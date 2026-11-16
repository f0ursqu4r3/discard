//! The bridge side of sub-agent threads: the `Delegator` of the app, the cancel cascade, live and anchored chips,
//! follow triggers, and agent add requests. The parent channels are mocks that record their commands. The sub-agent
//! threads run real engines over a fake provider.
use discard_app::desktop::channels::{Channels, Launcher};
use discard_app::desktop::view::ViewId;
use discard_app::desktop::core::{AppCore, CoreInit};
use discard_app::desktop::delegation::{AppDelegator, Limits};
use discard_app::desktop::pump::{Emit, pump_live};
use discard_core::agent::AgentConfig;
use discard_core::agent_requests::AgentRequests;
use discard_core::delegation::{DelegateResult, DelegateTask, Delegator, format_results, steer_note};
use discard_core::driver::TurnOutcome;
use discard_core::engine::Engine;
use discard_core::model::{AgentId, Block, Channel, ChannelId, Message, MessageAuthor, MessageId, Role, ThreadState};
use discard_core::store::Store;
use discard_core::ui_state::{Command, UiState, Update};
use discard_providers::fake::{FakeProvider, Step};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// What the test keeps of a parent channel: its commands, and the sender of its updates.
struct Parent { commands: mpsc::UnboundedReceiver<Command>, _updates: mpsc::UnboundedSender<Update> }

#[derive(Default)]
struct Rigs {
    parents: Mutex<HashMap<String, Parent>>,
    /// The script of a sub-agent thread, by thread name. A thread with none answers "done: <name>".
    scripts: Mutex<HashMap<String, Vec<Step>>>,
    /// The scripts of the sessions after the first of a sub-agent thread, by thread name: one for each later turn.
    later_scripts: Mutex<HashMap<String, Vec<Vec<Step>>>>,
    /// Thread names whose engine cannot start.
    broken: Mutex<Vec<String>>,
    /// The launcher of the thread "gate" says it arrived on `entered`, then waits for `release`: the call stands
    /// between the creation of one task and the next.
    entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// The thread "vanish" has a stub for an engine. It ends its update stream when this fires.
    vanish: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

fn bash() -> Step { Step::Approval { tool: "Bash".into(), input: serde_json::json!({"command": "ls"}) } }

fn launcher(store: Arc<Mutex<Store>>, rigs: Arc<Rigs>, handle: tokio::runtime::Handle) -> Launcher {
    Arc::new(move |channel: &Channel| {
        let (agent, session, history, authors, thread) = {
            let store = store.lock().unwrap();
            (store.load_agent(&channel.agent_id).unwrap(), store.latest_session(&channel.id).unwrap().unwrap(), store.messages_for_channel(&channel.id).unwrap(),
             store.message_authors(&channel.id).unwrap(), store.thread_info(&channel.id).unwrap())
        };
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates_rx) = mpsc::unbounded_channel();
        match thread {
            None => { rigs.parents.lock().unwrap().insert(channel.id.0.clone(), Parent { commands: commands_rx, _updates: updates_tx }); }
            Some(_) => {
                if rigs.broken.lock().unwrap().contains(&channel.name) { return Err("The provider is unavailable.".into()); }
                if channel.name == "gate" {
                    if let Some(entered) = rigs.entered.lock().unwrap().take() { let _ = entered.send(()); }
                    let release = rigs.release.lock().unwrap().take();
                    if let Some(release) = release { let _ = release.recv(); }
                }
                let script = rigs.scripts.lock().unwrap().get(&channel.name).cloned().unwrap_or_else(|| vec![Step::working(), Step::text(&format!("done: {}", channel.name)), Step::idle()]);
                let engine = Engine {
                    main_checkout_grants: Arc::default(), workspace_locks: Arc::default(),
                    write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
                    reasoning_effort: None, mode: discard_core::provider::SessionMode::Conversation, providers: vec![],
                    store: store.clone(), agent_id: channel.agent_id.clone(), project_id: channel.project_id.clone(), memory: None, plugin_write: false,
                    workspace_mode: channel.workspace_mode, workspace: None, observer: None, channel_name: channel.name.clone(),
                    provider: Arc::new(match channel.name.as_str() {
                        "nosetup" => FakeProvider::new(script).with_setup(discard_core::provider::SetupStatus::NotInstalled { install_hint: "Install the fake CLI.".into() }),
                        name => rigs.later_scripts.lock().unwrap().get(name).cloned().unwrap_or_default().into_iter().fold(FakeProvider::new(script), |provider, later| provider.then_session(later)),
                    }),
                    channel_id: channel.id.clone(), session_id: session, cwd: ".".into(), model: Some("test".into()), system_prompt: None,
                    policy: discard_core::capability::ApprovalPolicy::Ask,
                };
                match channel.name.as_str() {
                    // The engine is gone before the brief: the commands cannot be sent.
                    "dead" => { drop(commands_rx); drop(updates_tx); }
                    // The engine takes commands, but its update stream ends when the test says so.
                    "vanish" => {
                        let release = rigs.vanish.lock().unwrap().take().expect("a release for the stub");
                        handle.spawn(async move { let _keep = commands_rx; let _ = release.await; drop(updates_tx); });
                    }
                    _ => { handle.spawn(engine.run(commands_rx, updates_tx)); }
                }
            }
        }
        Ok((CoreInit { state: UiState::new(channel.id.clone(), history).with_authors(authors), commands, project: String::new(), channel: channel.name.clone(), agent: agent.name, provider: agent.provider }, updates_rx))
    })
}

struct World {
    _dir: tempfile::TempDir,
    store: Arc<Mutex<Store>>,
    core: Arc<AppCore>,
    channels: Arc<Channels>,
    rigs: Arc<Rigs>,
    delegator: Arc<AppDelegator>,
    /// The calling agent, with delegation on.
    agent: AgentId,
    parent: Channel,
    question: MessageId,
}

fn config(name: &str) -> AgentConfig { AgentConfig::from_toml(&format!("name='{name}'\nprovider='fake'\nmodel='test'\npolicy='ask'\ndelegation=true")).unwrap() }

fn put(store: &Store, channel: &ChannelId, role: Role, body: &str, author: Option<&AgentId>) -> MessageId {
    let message = Message { id: MessageId::new(), channel_id: channel.clone(), role, blocks: vec![Block::Markdown { text: body.into() }] };
    match author {
        Some(agent) => store.insert_authored_message(&message, &MessageAuthor { agent_id: agent.clone(), name: "Quinn".into(), provider: "fake".into(), avatar: None }).unwrap(),
        None => store.insert_message(&message).unwrap(),
    }
    message.id
}

fn world_with(limits: Limits) -> World {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let agent = store.create_agent(&config("Quinn")).unwrap();
    let parent = store.create_conversation(&agent, None, "chat").unwrap();
    let question = put(&store, &parent.id, Role::User, "research this", None);
    let store = Arc::new(Mutex::new(store));
    let rigs = Arc::new(Rigs::default());
    let launch = launcher(store.clone(), rigs.clone(), tokio::runtime::Handle::current());
    let (init, _updates) = (launch.clone())(&parent).unwrap();
    let core = Arc::new(AppCore::new(init));
    core.configure_read_marker(&ViewId::main(), dir.path(), &parent.id.0);
    let channels = Arc::new(Channels::new(core.clone(), store.clone(), tokio::runtime::Handle::current(), launch));
    channels.select(&ViewId::main(), &parent.id.0).unwrap();
    let delegator = Arc::new(AppDelegator::new(Arc::downgrade(&channels)).with_limits(limits));
    World { _dir: dir, store, core, channels, rigs, delegator, agent, parent, question }
}

fn world() -> World { world_with(Limits { timeout: Duration::from_secs(60), grace: Duration::from_secs(5) }) }

fn task(name: &str, agent: &AgentId) -> DelegateTask { DelegateTask { name: name.into(), brief: format!("Look into {name}."), agent: agent.clone() } }

type Call = std::thread::JoinHandle<Result<Vec<DelegateResult>, String>>;

/// A `delegate` call on its own thread, the way the MCP server makes it.
fn call(w: &World, parent: &ChannelId, tasks: Vec<DelegateTask>) -> Call {
    let (delegator, parent, agent) = (w.delegator.clone(), parent.clone(), w.agent.clone());
    std::thread::spawn(move || delegator.delegate(&parent, &agent, tasks))
}

async fn finish(call: Call) -> Vec<DelegateResult> {
    tokio::task::spawn_blocking(move || call.join().expect("the call did not panic")).await.unwrap().expect("the call succeeded")
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async { while !done() { tokio::time::sleep(Duration::from_millis(5)).await; } })
        .await.unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

impl World {
    fn threads(&self, parent: &ChannelId) -> Vec<discard_core::model::ThreadInfo> { self.store.lock().unwrap().threads_for_channel(parent).unwrap() }
    fn state(&self, thread: &ChannelId) -> Option<ThreadState> { self.store.lock().unwrap().thread_info(thread).unwrap().unwrap().state }
    fn thread_running(&self, thread: &ChannelId) -> bool { self.core.channel_running(&thread.0) }
    /// Wait until the call has created and started `count` threads of `parent`, and return them in task order.
    async fn started(&self, parent: &ChannelId, count: usize) -> Vec<ChannelId> {
        until("the threads to start", || self.threads(parent).len() == count && self.threads(parent).iter().all(|t| self.core.has_channel(&t.channel_id.0) && self.thread_running(&t.channel_id))).await;
        self.threads(parent).into_iter().map(|t| t.channel_id).collect()
    }
    /// Like `started`, for tasks that may end at once: wait until the call created and loaded `count` threads.
    async fn loaded(&self, parent: &ChannelId, count: usize) -> Vec<ChannelId> {
        until("the threads to load", || self.threads(parent).len() == count && self.threads(parent).iter().all(|t| self.core.has_channel(&t.channel_id.0))).await;
        self.threads(parent).into_iter().map(|t| t.channel_id).collect()
    }
    /// Mark the parent turn as running, as a sent message does. Its mock engine never answers.
    fn parent_turn_running(&self) { self.core.send_to(Some(&self.parent.id.0), "research this".into()).unwrap(); }
    fn parent_commands(&self, parent: &ChannelId) -> Vec<Command> {
        let mut parents = self.rigs.parents.lock().unwrap();
        let rig = parents.get_mut(&parent.0).expect("a mock parent");
        std::iter::from_fn(|| rig.commands.try_recv().ok()).collect()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_tasks_run_together_and_the_results_come_back_in_task_order() {
    let w = world();
    // Task 1 waits for an approval. Task 2 finishes at once, so it ends first.
    w.rigs.scripts.lock().unwrap().insert("slow".into(), vec![Step::working(), bash(), Step::text("slow answer"), Step::idle()]);
    w.parent_turn_running();
    let call = call(&w, &w.parent.id, vec![task("slow", &w.agent), task("quick", &w.agent)]);
    let threads = w.loaded(&w.parent.id, 2).await.into_iter().map(|t| t.0).collect::<Vec<_>>();
    until("the quick task to finish", || w.state(&ChannelId(threads[1].clone())) == Some(ThreadState::Done) && w.core.snapshot(&ViewId::main()).live_sub_agent_threads.get(1).is_some_and(|c| c.state.as_deref() == Some("done"))).await;
    assert_eq!(w.state(&ChannelId(threads[0].clone())), Some(ThreadState::Running), "both run at the same time");
    assert!(!call.is_finished(), "the call blocks until every task ended");
    // The live chips are on the snapshot of the running parent, in task order, with no anchor yet.
    let snapshot = w.core.snapshot(&ViewId::main());
    assert_eq!(snapshot.live_sub_agent_threads.iter().map(|c| (c.channel_id.as_str(), c.name.as_str(), c.kind.as_str(), c.state.as_deref())).collect::<Vec<_>>(),
        [(threads[0].as_str(), "slow", "sub_agent", Some("running")), (threads[1].as_str(), "quick", "sub_agent", Some("done"))]);
    assert_eq!(snapshot.live_sub_agent_threads[0].agent_name, "Quinn");
    assert!(w.core.history(&ViewId::main()).iter().all(|m| m.sub_agent_threads.is_empty()), "nothing hangs under a message before the turn ends");
    // The thread of an unanchored run opens. Its composer is on: a message steers the task.
    w.channels.open_thread(&ViewId::main(), &ChannelId(threads[0].clone())).unwrap();
    let open = w.core.snapshot(&ViewId::main()).open_thread.unwrap();
    assert_eq!((open.kind.as_str(), open.state.as_deref(), open.anchor_message_id.clone(), open.agent_name.as_str()), ("sub_agent", Some("running"), None, "Quinn"));
    assert!(!open.composer_disabled && open.composer_disabled_reason.is_none());
    assert!(open.read_only_note.contains("threads are read-only"), "{}", open.read_only_note);
    let history = w.core.thread_history_snapshot(&ViewId::main()).unwrap();
    assert!(history.anchor.is_none(), "an unanchored thread has no anchor message");
    until("the approval", || w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.pending_approvals == 1)).await;
    // A pending approval follows the thread and shows on the Approvals data, with its channel's counts.
    let snapshot = w.core.snapshot(&ViewId::main());
    assert_eq!(snapshot.channels[0].pending_approvals, 0, "the channel's own count stays per channel");
    assert_eq!(snapshot.channels[0].thread_pending_approvals, 1);
    assert_eq!(snapshot.thread_approvals.iter().map(|a| (a.thread_id.as_str(), a.name.as_str(), a.pending_approvals)).collect::<Vec<_>>(), [(threads[0].as_str(), "slow", 1)]);
    assert_eq!(snapshot.channels[0].threads.iter().map(|l| (l.id.as_str(), l.kind.as_str(), l.state.as_deref(), l.pending_approvals)).collect::<Vec<_>>(), [(threads[0].as_str(), "sub_agent", Some("running"), 1)], "only the thread with the approval is followed");
    assert!(w.store.lock().unwrap().thread_info(&ChannelId(threads[0].clone())).unwrap().unwrap().followed);
    assert!(!w.store.lock().unwrap().thread_info(&ChannelId(threads[1].clone())).unwrap().unwrap().followed, "a run does not follow its thread");
    let approval = w.core.snapshot(&ViewId::main()).open_thread.unwrap().prompts[0].approval_id.clone();
    w.core.decide_in(Some(&threads[0]), approval, "approve".into(), None);
    let results = finish(call).await;
    assert_eq!(results.iter().map(|r| (r.name.as_str(), r.state, r.text.as_str(), r.thread_channel_id.0.as_str())).collect::<Vec<_>>(),
        [("slow", ThreadState::Done, "slow answer", threads[0].as_str()), ("quick", ThreadState::Done, "done: quick", threads[1].as_str())], "in task order, not in finishing order");
    // The sub-agent threads were never opened by the run, and the brief is the parent agent's message.
    let stored = w.store.lock().unwrap().messages_for_channel(&ChannelId(threads[1].clone())).unwrap();
    assert_eq!(stored.iter().map(|m| m.role).collect::<Vec<_>>(), [Role::User, Role::Agent]);
    let author = w.store.lock().unwrap().message_author(&stored[0].id).unwrap().unwrap();
    assert_eq!(author.agent_id, w.agent);
    assert!(w.parent_commands(&w.parent.id).iter().all(|c| !matches!(c, Command::Cancel)), "the parent was not stopped");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_follows_nothing_and_opens_no_panel() {
    let w = world();
    w.parent_turn_running();
    let results = finish(call(&w, &w.parent.id, vec![task("one", &w.agent)])).await;
    assert_eq!(results[0].state, ThreadState::Done);
    let thread = results[0].thread_channel_id.clone();
    let snapshot = w.core.snapshot(&ViewId::main());
    assert!(snapshot.open_thread.is_none(), "the thread panel stays as the user left it");
    assert!(snapshot.channels[0].threads.is_empty(), "the thread is not in the channel bar");
    assert!(!w.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().followed);
    // It is still a thread of the parent, and its state is final everywhere.
    assert_eq!(snapshot.live_sub_agent_threads[0].state.as_deref(), Some("done"));
    assert_eq!(w.state(&thread), Some(ThreadState::Done));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_chips_move_to_the_saved_response_when_the_parent_turn_ends() {
    let w = world();
    w.rigs.scripts.lock().unwrap().insert("b".into(), vec![Step::working(), Step::text("b answer"), Step::idle()]);
    w.parent_turn_running();
    let results = finish(call(&w, &w.parent.id, vec![task("a", &w.agent), task("b", &w.agent)])).await;
    let threads: Vec<_> = results.iter().map(|r| r.thread_channel_id.0.clone()).collect();
    assert_eq!(w.core.snapshot(&ViewId::main()).live_sub_agent_threads.len(), 2);
    // The parent engine saves its response and anchors the threads, then sends both updates.
    let response = put(&w.store.lock().unwrap(), &w.parent.id, Role::Agent, "Both are done.", Some(&w.agent));
    let anchored = w.store.lock().unwrap().anchor_sub_agent_threads(&w.parent.id, Some(&response)).unwrap();
    let before = w.core.history_snapshot(&ViewId::main()).rev;
    w.core.apply_channel(&w.parent.id.0, Update::ResponseCompleted(Message { id: response.clone(), channel_id: w.parent.id.clone(), role: Role::Agent, blocks: vec![Block::Markdown { text: "Both are done.".into() }] }));
    w.core.apply_channel(&w.parent.id.0, Update::ThreadsAnchored(anchored));
    w.core.apply_channel(&w.parent.id.0, Update::TurnFinished(TurnOutcome::Completed));
    assert!(w.core.history_snapshot(&ViewId::main()).rev > before, "the parent transcript is sent again, with the chips, without a reload");
    let history = w.core.history(&ViewId::main());
    let message = history.iter().find(|m| m.id == response.0).unwrap();
    assert_eq!(message.sub_agent_threads.iter().map(|c| (c.channel_id.clone(), c.name.as_str(), c.state.as_deref(), c.kind.as_str())).collect::<Vec<_>>(),
        [(threads[0].clone(), "a", Some("done"), "sub_agent"), (threads[1].clone(), "b", Some("done"), "sub_agent")], "in task order");
    assert!(message.thread.is_none(), "the question thread slot is separate");
    assert_eq!(message.sub_agent_threads[1].last_message.as_ref().map(|m| m.preview.as_str()), Some("b answer"));
    assert!(w.core.snapshot(&ViewId::main()).live_sub_agent_threads.is_empty(), "the live chips are gone");
    // A response can hold a question thread too.
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &response.0, "why?".into(), vec![], false);
    assert!(thread.is_ok(), "{thread:?}");
    until("the question thread chip", || w.core.history(&ViewId::main()).iter().find(|m| m.id == response.0).is_some_and(|m| m.thread.is_some())).await;
    let message = w.core.history(&ViewId::main()).into_iter().find(|m| m.id == response.0).unwrap();
    assert_eq!((message.thread.unwrap().kind.as_str(), message.sub_agent_threads.len()), ("question", 2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_parent_in_the_background_still_gets_its_sub_agents_and_shows_them_when_selected() {
    let w = world();
    let other = w.store.lock().unwrap().create_conversation(&w.agent, None, "other").unwrap();
    w.parent_turn_running();
    w.channels.select(&ViewId::main(), &other.id.0).unwrap();
    assert!(w.core.snapshot(&ViewId::main()).live_sub_agent_threads.is_empty(), "the chips are for the selected channel only");
    let results = finish(call(&w, &w.parent.id, vec![task("one", &w.agent), task("two", &w.agent)])).await;
    assert!(results.iter().all(|r| r.state == ThreadState::Done));
    let snapshot = w.core.snapshot(&ViewId::main());
    assert_eq!((snapshot.channel_id.as_str(), snapshot.open_thread.is_none(), snapshot.live_sub_agent_threads.len()), (other.id.0.as_str(), true, 0));
    assert!(results.iter().all(|r| w.core.has_channel(&r.thread_channel_id.0)), "the thread state exists although nobody looked");
    w.channels.select(&ViewId::main(), &w.parent.id.0).unwrap();
    assert_eq!(w.core.snapshot(&ViewId::main()).live_sub_agent_threads.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["one", "two"]);
    assert!(w.core.snapshot(&ViewId::main()).open_thread.is_none(), "selecting the parent does not open a thread");
    // A thread that nobody opened can be opened from its chip, and the whole run is in it.
    w.channels.open_thread(&ViewId::main(), &results[0].thread_channel_id).unwrap();
    assert_eq!(w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages.len(), 2);
    // From another channel, selecting the unanchored thread selects its parent and opens the panel.
    w.channels.select(&ViewId::main(), &other.id.0).unwrap();
    assert!(w.core.snapshot(&ViewId::main()).open_thread.is_none());
    assert_eq!(w.channels.select_thread(&ViewId::main(), &results[1].thread_channel_id).unwrap(), (ViewId::main(), w.parent.id.0.clone()));
    let snapshot = w.core.snapshot(&ViewId::main());
    assert_eq!((snapshot.channel_id.as_str(), snapshot.open_thread.map(|t| t.channel_id)), (w.parent.id.0.as_str(), Some(results[1].thread_channel_id.0.clone())));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_calls_from_different_channels_run_at_the_same_time() {
    let w = world();
    let other = w.store.lock().unwrap().create_conversation(&w.agent, None, "other").unwrap();
    for name in ["x", "y"] { w.rigs.scripts.lock().unwrap().insert(name.into(), vec![Step::working(), Step::Hang]); }
    put(&w.store.lock().unwrap(), &other.id, Role::User, "other request", None);
    w.channels.select(&ViewId::main(), &other.id.0).unwrap();
    w.core.send_to(Some(&other.id.0), "other request".into()).unwrap();
    w.channels.select(&ViewId::main(), &w.parent.id.0).unwrap();
    w.parent_turn_running();
    let first = call(&w, &w.parent.id, vec![task("x", &w.agent)]);
    let second = call(&w, &other.id, vec![task("y", &w.agent)]);
    let (a, b) = (w.started(&w.parent.id, 1).await, w.started(&other.id, 1).await);
    assert!(w.thread_running(&a[0]) && w.thread_running(&b[0]), "neither call waits for the other");
    w.core.cancel_in(Some(&w.parent.id.0));
    w.core.cancel_sub_agents_of(&other.id.0);
    let (first, second) = (finish(first).await, finish(second).await);
    assert_eq!((first[0].state, second[0].state), (ThreadState::Cancelled, ThreadState::Cancelled));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_that_runs_too_long_times_out_and_the_first_final_state_wins() {
    let w = world_with(Limits { timeout: Duration::from_millis(150), grace: Duration::from_secs(5) });
    w.rigs.scripts.lock().unwrap().insert("stuck".into(), vec![Step::working(), Step::text("partial"), Step::Hang]);
    w.parent_turn_running();
    let started = std::time::Instant::now();
    let results = finish(call(&w, &w.parent.id, vec![task("stuck", &w.agent), task("fine", &w.agent)])).await;
    assert!(started.elapsed() < Duration::from_secs(5), "the injected limit applies");
    assert_eq!((results[0].state, results[1].state), (ThreadState::TimedOut, ThreadState::Done), "one slow task does not fail the others");
    let thread = results[0].thread_channel_id.clone();
    until("the stopped engine", || !w.thread_running(&thread)).await;
    assert_eq!(w.state(&thread), Some(ThreadState::TimedOut), "the engine's own Cancelled does not replace TimedOut");
    assert_eq!(w.core.snapshot(&ViewId::main()).live_sub_agent_threads[0].state.as_deref(), Some("timed_out"), "the chip shows the stored state");
    let partial = w.store.lock().unwrap().messages_for_channel(&thread).unwrap();
    assert_eq!(partial.len(), 2, "the partial transcript stays");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_the_parent_turn_stops_every_sub_agent_it_started() {
    for stop in ["cancel_turn", "cancel_all", "the turn ended"] {
        let w = world();
        for name in ["one", "two"] { w.rigs.scripts.lock().unwrap().insert(name.into(), vec![Step::working(), Step::text("partial"), Step::Hang]); }
        w.parent_turn_running();
        let call = call(&w, &w.parent.id, vec![task("one", &w.agent), task("two", &w.agent)]);
        let threads = w.started(&w.parent.id, 2).await;
        match stop {
            "cancel_turn" => w.core.cancel_in(Some(&w.parent.id.0)),
            "cancel_all" => w.core.cancel_all(),
            // The provider of the parent died while the call waited: the turn ends without the call returning.
            _ => { w.core.apply_channel(&w.parent.id.0, Update::TurnFinished(TurnOutcome::Failed)); }
        };
        let results = finish(call).await;
        assert!(results.iter().all(|r| r.state == ThreadState::Cancelled), "{stop}: {results:?}");
        assert!(threads.iter().all(|t| w.state(t) == Some(ThreadState::Cancelled)), "{stop}");
        if stop == "cancel_turn" { assert!(w.parent_commands(&w.parent.id).iter().any(|c| matches!(c, Command::Cancel)), "the parent got its stop too"); }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_one_sub_agent_ends_only_its_task() {
    let w = world();
    for name in ["one", "two"] { w.rigs.scripts.lock().unwrap().insert(name.into(), vec![Step::working(), Step::Hang]); }
    w.parent_turn_running();
    let call = call(&w, &w.parent.id, vec![task("one", &w.agent), task("two", &w.agent)]);
    let threads = w.started(&w.parent.id, 2).await;
    w.core.cancel_in(Some(&threads[0].0));
    until("the first task to end", || w.state(&threads[0]) == Some(ThreadState::Cancelled)).await;
    assert!(w.thread_running(&threads[1]) && !call.is_finished(), "the other task and the call go on");
    assert!(w.core.channel_running(&w.parent.id.0), "the parent turn goes on");
    assert!(w.parent_commands(&w.parent.id).iter().all(|c| !matches!(c, Command::Cancel)), "the parent got no stop");
    w.core.cancel_in(Some(&threads[1].0));
    let results = finish(call).await;
    assert_eq!(results.iter().map(|r| r.state).collect::<Vec<_>>(), [ThreadState::Cancelled, ThreadState::Cancelled]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_that_cannot_start_fails_and_the_others_run() {
    let w = world();
    w.rigs.broken.lock().unwrap().push("broken".into());
    w.parent_turn_running();
    let results = finish(call(&w, &w.parent.id, vec![task("fine one", &w.agent), task("broken", &w.agent), task("fine two", &w.agent)])).await;
    assert_eq!(results.iter().map(|r| r.state).collect::<Vec<_>>(), [ThreadState::Done, ThreadState::Failed, ThreadState::Done]);
    assert_eq!(results[1].text, "The provider is unavailable.");
    assert_eq!(w.state(&results[1].thread_channel_id), Some(ThreadState::Failed));
    // The failed thread still hangs under the response, with its state, although it never loaded.
    let anchored = w.store.lock().unwrap().anchor_sub_agent_threads(&w.parent.id, None).unwrap();
    w.core.apply_channel(&w.parent.id.0, Update::ThreadsAnchored(anchored));
    let history = w.core.history(&ViewId::main());
    let chips = &history.iter().find(|m| m.id == w.question.0).unwrap().sub_agent_threads;
    assert_eq!(chips.iter().map(|c| c.state.as_deref()).collect::<Vec<_>>(), [Some("done"), Some("failed"), Some("done")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_run_keeps_the_error_text_as_the_detail() {
    let w = world();
    w.rigs.scripts.lock().unwrap().insert("crash".into(), vec![Step::working(), Step::Crash]);
    w.parent_turn_running();
    let results = finish(call(&w, &w.parent.id, vec![task("crash", &w.agent), task("nosetup", &w.agent)])).await;
    assert_eq!((results[0].state, results[1].state), (ThreadState::Failed, ThreadState::Failed));
    assert!(results[1].text.contains("Install the fake CLI."), "the setup text reaches the agent as the detail: {:?}", results[1].text);
    until("the chip", || w.core.snapshot(&ViewId::main()).live_sub_agent_threads.iter().all(|c| c.state.as_deref() == Some("failed"))).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finished_sub_agent_thread_supports_post_follow_leave_and_read() {
    let w = world();
    w.parent_turn_running();
    let results = finish(call(&w, &w.parent.id, vec![task("one", &w.agent)])).await;
    let thread = results[0].thread_channel_id.clone();
    // It cannot start a thread.
    let brief = w.store.lock().unwrap().messages_for_channel(&thread).unwrap()[0].id.clone();
    assert!(w.channels.start_thread(&ViewId::main(), &thread.0, &brief.0, "nested".into(), vec![], false).unwrap_err().contains("sub-agent thread cannot start"));
    assert_eq!(w.store.lock().unwrap().messages_for_channel(&thread).unwrap().len(), 2, "nothing was added");
    // Follow and leave, with the sub-agent fields on the link.
    w.channels.set_thread_followed(&thread, true).unwrap();
    let link = w.core.snapshot(&ViewId::main()).channels[0].threads[0].clone();
    assert_eq!((link.id, link.kind.as_str(), link.state.as_deref(), link.running), (thread.0.clone(), "sub_agent", Some("done"), false));
    let stored = w.channels.thread_links(&w.parent.id).unwrap();
    assert_eq!((stored[0].kind.as_str(), stored[0].state.as_deref()), ("sub_agent", Some("done")));
    w.channels.set_thread_followed(&thread, false).unwrap();
    assert!(w.core.snapshot(&ViewId::main()).channels[0].threads.is_empty());
    // Open it, read it, and post its result to the channel.
    w.channels.open_thread(&ViewId::main(), &thread).unwrap();
    let messages = w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages;
    assert_eq!(messages[0].author.as_ref().map(|a| a.name.as_str()), Some("Quinn"));
    assert!(w.core.snapshot(&ViewId::main()).open_thread.unwrap().unread, "the reply is unread");
    w.core.mark_read_in(Some(&thread.0), &messages[1].id).unwrap();
    assert!(!w.core.snapshot(&ViewId::main()).open_thread.unwrap().unread);
    let reference = w.channels.post_thread_message(&messages[1].id).unwrap();
    let history = w.core.history(&ViewId::main());
    assert_eq!(history.last().unwrap().id, reference);
    assert_eq!(history.last().unwrap().posted_from.as_ref().map(|p| p.thread_name.as_str()), Some("one"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_in_a_finished_sub_agent_thread_starts_a_follow_up_turn_and_keeps_the_state() {
    let w = world();
    w.rigs.scripts.lock().unwrap().insert("one".into(), vec![Step::working(), Step::text("first answer"), Step::idle()]);
    w.rigs.later_scripts.lock().unwrap().insert("one".into(), vec![vec![Step::working(), Step::text("held"), Step::Hang]]);
    w.parent_turn_running();
    let results = finish(call(&w, &w.parent.id, vec![task("one", &w.agent)])).await;
    let thread = results[0].thread_channel_id.clone();
    assert_eq!((results[0].state, results[0].text.as_str()), (ThreadState::Done, "first answer"));
    assert_eq!(results[0].steer_note, None, "a task without a steer has no note");
    until("the thread turn to end", || !w.thread_running(&thread)).await;
    w.channels.open_thread(&ViewId::main(), &thread).unwrap();
    w.channels.send_with_options(&thread.0, "Where is it defined?".into(), vec![], false).unwrap();
    // The message shows at once, and the follow-up turn runs.
    let open = w.core.snapshot(&ViewId::main()).open_thread.unwrap();
    assert!(open.turn_running && open.followed);
    until("the follow-up reply to stream", || w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| !t.live_blocks.is_empty())).await;
    // While that turn runs, a second message is not a steer: the task ended.
    assert!(w.channels.send_with_options(&thread.0, "and here?".into(), vec![], false).unwrap_err().contains("cannot send a message right now"));
    w.core.cancel_in(Some(&thread.0));
    until("the follow-up turn to end", || !w.thread_running(&thread)).await;
    assert_eq!(w.state(&thread), Some(ThreadState::Done), "a follow-up turn leaves the state alone");
    let roles = w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages.iter().map(|m| m.role.clone()).collect::<Vec<_>>();
    assert_eq!(roles, ["user", "agent", "user", "agent"]);
    assert_eq!(w.store.lock().unwrap().messages_for_channel(&thread).unwrap().len(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_steer_keeps_the_task_open_and_the_call_returns_the_text_of_the_last_turn() {
    let w = world();
    w.rigs.scripts.lock().unwrap().insert("steered".into(), vec![Step::working(), Step::text("partial notes"), Step::Hang]);
    w.rigs.later_scripts.lock().unwrap().insert("steered".into(), vec![vec![Step::working(), Step::text("Only the tests call it."), Step::idle()]]);
    w.parent_turn_running();
    let call = call(&w, &w.parent.id, vec![task("steered", &w.agent)]);
    let thread = w.started(&w.parent.id, 1).await.remove(0);
    w.channels.open_thread(&ViewId::main(), &thread).unwrap();
    until("the partial text", || w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| !t.live_blocks.is_empty())).await;
    let open = w.core.snapshot(&ViewId::main()).open_thread.unwrap();
    assert!(!open.composer_disabled && open.turn_running && open.state.as_deref() == Some("running"), "the composer takes a steer");
    w.channels.send_with_options(&thread.0, "Only look in tests.".into(), vec![], false).unwrap();
    let results = finish(call).await;
    assert_eq!((results[0].state, results[0].text.as_str()), (ThreadState::Done, "Only the tests call it."), "the result is the last turn's message");
    let note = steer_note(1, "Only look in tests.");
    assert_eq!(results[0].steer_note, note, "the note counts the steer and quotes it");
    let rendered: serde_json::Value = serde_json::from_str(&format_results(&results)).unwrap();
    assert_eq!(rendered["results"][0]["result"], format!("{}\n\nOnly the tests call it.", note.unwrap()).as_str(), "the note leads the text of the last turn");
    until("the steer turn to end", || !w.thread_running(&thread)).await;
    assert_eq!(w.state(&thread), Some(ThreadState::Done));
    // The panel shows the partial response, then the steer, then the final reply.
    let messages = w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages;
    assert_eq!(messages.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(), ["user", "agent", "user", "agent"]);
    assert_eq!(w.store.lock().unwrap().messages_for_channel(&thread).unwrap().len(), 4);
    let open = w.core.snapshot(&ViewId::main()).open_thread.unwrap();
    assert!(open.followed && !open.turn_running);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_steered_task_that_ends_cancelled_still_carries_the_note() {
    let w = world();
    w.rigs.scripts.lock().unwrap().insert("steered".into(), vec![Step::working(), Step::text("partial notes"), Step::Hang]);
    w.rigs.later_scripts.lock().unwrap().insert("steered".into(), vec![vec![Step::working(), Step::text("steer partial"), Step::Hang]]);
    w.parent_turn_running();
    let call = call(&w, &w.parent.id, vec![task("steered", &w.agent)]);
    let thread = w.started(&w.parent.id, 1).await.remove(0);
    w.channels.open_thread(&ViewId::main(), &thread).unwrap();
    until("the partial text", || w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| !t.live_blocks.is_empty())).await;
    w.channels.send_with_options(&thread.0, "Only look in \"tests\".".into(), vec![], false).unwrap();
    until("the steer turn to run", || w.store.lock().unwrap().messages_for_channel(&thread).unwrap().len() >= 3).await;
    w.core.cancel_in(Some(&thread.0));
    let results = finish(call).await;
    assert_eq!(results[0].state, ThreadState::Cancelled);
    assert_eq!(results[0].steer_note, steer_note(1, "Only look in 'tests'."), "the quotes of the message are swapped");
    let rendered: serde_json::Value = serde_json::from_str(&format_results(&results)).unwrap();
    let error = rendered["results"][0]["error"].as_str().unwrap();
    assert!(error.starts_with("Note: the user sent this sub-agent 1 message during the task."), "{error}");
    assert!(error.ends_with("The sub-agent task was cancelled."), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_process_opens_a_finished_sub_agent_thread_from_storage() {
    let w = world();
    w.parent_turn_running();
    let results = finish(call(&w, &w.parent.id, vec![task("one", &w.agent), task("two", &w.agent)])).await;
    let response = put(&w.store.lock().unwrap(), &w.parent.id, Role::Agent, "Done.", Some(&w.agent));
    w.store.lock().unwrap().anchor_sub_agent_threads(&w.parent.id, Some(&response)).unwrap();
    // A second app over the same store: nothing is loaded yet.
    let dir = tempfile::tempdir().unwrap();
    let launch = launcher(w.store.clone(), w.rigs.clone(), tokio::runtime::Handle::current());
    let (init, _updates) = launch(&w.parent).unwrap();
    let core = Arc::new(AppCore::new(init));
    core.configure_read_marker(&ViewId::main(), dir.path(), &w.parent.id.0);
    let channels = Channels::new(core.clone(), w.store.clone(), tokio::runtime::Handle::current(), launch);
    channels.refresh_threads(&w.parent.id).unwrap();
    let history = core.history(&ViewId::main());
    let chips = &history.iter().find(|m| m.id == response.0).expect("the saved response").sub_agent_threads;
    assert_eq!(chips.iter().map(|c| (c.name.as_str(), c.state.as_deref(), c.message_count)).collect::<Vec<_>>(), [("one", Some("done"), 2), ("two", Some("done"), 2)], "the chips come from storage, in task order");
    assert!(core.snapshot(&ViewId::main()).live_sub_agent_threads.is_empty(), "an anchored thread is not live");
    channels.open_thread(&ViewId::main(), &results[1].thread_channel_id).unwrap();
    let open = core.snapshot(&ViewId::main()).open_thread.unwrap();
    assert_eq!((open.kind.as_str(), open.state.as_deref(), open.anchor_message_id.as_deref(), open.composer_disabled_reason.as_deref()), ("sub_agent", Some("done"), Some(response.0.as_str()), None));
    assert!(!open.composer_disabled, "a finished sub-agent thread takes a reply");
    assert_eq!(core.thread_history_snapshot(&ViewId::main()).unwrap().messages.len(), 2, "the transcript comes from storage");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selecting_a_channel_hangs_the_threads_of_a_lost_turn_under_their_user_message() {
    let w = world();
    // A thread that its parent turn never anchored: the app stopped. The parent shows no running turn.
    let thread = w.store.lock().unwrap().create_sub_agent_thread(&w.parent.id, "lost", &w.agent).unwrap();
    w.store.lock().unwrap().finish_sub_agent_thread(&thread.id, ThreadState::Cancelled).unwrap();
    w.channels.refresh_threads(&w.parent.id).unwrap();
    let info = w.store.lock().unwrap().thread_info(&thread.id).unwrap().unwrap();
    assert_eq!(info.anchor_message_id, Some(w.question.clone()));
    let chips = &w.core.history(&ViewId::main()).into_iter().find(|m| m.id == w.question.0).unwrap().sub_agent_threads;
    assert_eq!((chips[0].name.as_str(), chips[0].state.as_deref()), ("lost", Some("cancelled")));
    assert!(w.core.snapshot(&ViewId::main()).live_sub_agent_threads.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selecting_a_channel_never_anchors_the_threads_of_a_turn_that_runs() {
    let w = world();
    w.parent_turn_running();
    let thread = w.store.lock().unwrap().create_sub_agent_thread(&w.parent.id, "live", &w.agent).unwrap();
    w.channels.refresh_threads(&w.parent.id).unwrap();
    assert_eq!(w.store.lock().unwrap().thread_info(&thread.id).unwrap().unwrap().anchor_message_id, None);
    assert_eq!(w.core.snapshot(&ViewId::main()).live_sub_agent_threads.len(), 1);
}

#[derive(Default)]
struct Recorder { conversations: Mutex<usize> }
impl Emit for Recorder {
    fn snapshot(&self, _: &ViewId, _: &discard_app::desktop::dto::SnapshotDto) {}
    fn history(&self, _: &ViewId, _: &discard_app::desktop::dto::HistoryDto) {}
    fn conversations(&self) { *self.conversations.lock().unwrap() += 1; }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_follow_by_the_engine_tells_the_webview_to_load_the_thread_lists_again() {
    let w = world();
    w.rigs.scripts.lock().unwrap().insert("ask".into(), vec![Step::working(), bash(), Step::text("done"), Step::idle()]);
    let recorder = Arc::new(Recorder::default());
    let (_tx, rx) = mpsc::unbounded_channel::<Update>();
    let pump = tokio::spawn(pump_live(w.core.clone(), rx, recorder.clone(), Duration::from_millis(1)));
    w.parent_turn_running();
    let call = call(&w, &w.parent.id, vec![task("ask", &w.agent)]);
    let thread = w.started(&w.parent.id, 1).await.remove(0);
    until("the follow", || w.core.snapshot(&ViewId::main()).channels[0].threads.len() == 1).await;
    until("the event", || *recorder.conversations.lock().unwrap() >= 1).await;
    assert_eq!(w.channels.thread_links(&w.parent.id).unwrap().len(), 1, "storage lists the followed thread");
    w.channels.open_thread(&ViewId::main(), &thread).unwrap();
    let approval = w.core.snapshot(&ViewId::main()).open_thread.unwrap().prompts[0].approval_id.clone();
    w.core.decide_in(Some(&thread.0), approval, "deny".into(), None);
    finish(call).await;
    pump.abort();
}

// ---- Stops, limits, and ends that do not come the usual way ----

async fn failure(call: Call) -> String {
    tokio::task::spawn_blocking(move || call.join().expect("the call did not panic")).await.unwrap().expect_err("the call failed")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_between_two_tasks_cancels_the_task_that_did_not_start_and_makes_no_thread_for_it() {
    let w = world();
    w.rigs.scripts.lock().unwrap().insert("gate".into(), vec![Step::working(), Step::Hang]);
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *w.rigs.entered.lock().unwrap() = Some(entered_tx);
    *w.rigs.release.lock().unwrap() = Some(release_rx);
    w.parent_turn_running();
    let call = call(&w, &w.parent.id, vec![task("gate", &w.agent), task("second", &w.agent)]);
    // The call made the first task and waits in its launcher. The second task does not exist yet.
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap()).await.unwrap();
    w.core.cancel_in(Some(&w.parent.id.0));
    release_tx.send(()).unwrap();
    let results = finish(call).await;
    assert_eq!(results.iter().map(|r| r.state).collect::<Vec<_>>(), [ThreadState::Cancelled, ThreadState::Cancelled], "{results:?}");
    assert!(results[1].thread_channel_id.0.is_empty(), "a task that did not start has no thread");
    assert!(results[1].text.contains("stopped before this sub-agent started"), "{}", results[1].text);
    let threads = w.threads(&w.parent.id);
    assert_eq!(threads.len(), 1, "no thread for the second task, so no chip can stay running");
    until("the first thread to end", || w.state(&threads[0].channel_id) == Some(ThreadState::Cancelled)).await;
    assert!(w.core.snapshot(&ViewId::main()).live_sub_agent_threads.iter().all(|c| c.state.as_deref() == Some("cancelled")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_for_a_parent_with_no_running_turn_or_with_a_stopped_turn_starts_nothing() {
    let w = world();
    // No turn runs: the call fails before it starts anything.
    let error = failure(call(&w, &w.parent.id, vec![task("one", &w.agent)])).await;
    assert!(error.contains("no running turn"), "{error}");
    assert!(w.threads(&w.parent.id).is_empty());
    // A turn runs and the user stops it. The provider has not ended the turn yet when the call arrives.
    w.parent_turn_running();
    w.core.cancel_in(Some(&w.parent.id.0));
    let results = finish(call(&w, &w.parent.id, vec![task("one", &w.agent), task("two", &w.agent)])).await;
    assert_eq!(results.iter().map(|r| (r.state, r.thread_channel_id.0.as_str())).collect::<Vec<_>>(), [(ThreadState::Cancelled, ""), (ThreadState::Cancelled, "")]);
    assert!(w.threads(&w.parent.id).is_empty(), "nothing started");
    // The turn ends. The stop does not carry into the next turn.
    w.core.apply_channel(&w.parent.id.0, Update::TurnFinished(TurnOutcome::Cancelled));
    assert!(!w.core.sub_agents().parent_stopped(&w.parent.id.0));
    assert!(failure(call(&w, &w.parent.id, vec![task("late", &w.agent)])).await.contains("no running turn"), "a call after the turn ended starts nothing");
    w.parent_turn_running();
    let results = finish(call(&w, &w.parent.id, vec![task("again", &w.agent)])).await;
    assert_eq!(results[0].state, ThreadState::Done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn at_most_four_sub_agents_run_for_a_parent_even_when_calls_come_in_parallel() {
    let w = world();
    for name in ["a", "b", "c", "d", "e", "f"] { w.rigs.scripts.lock().unwrap().insert(name.into(), vec![Step::working(), Step::Hang]); }
    w.parent_turn_running();
    // Two calls of three tasks at the same time: only one fits.
    let first = call(&w, &w.parent.id, vec![task("a", &w.agent), task("b", &w.agent), task("c", &w.agent)]);
    let second = call(&w, &w.parent.id, vec![task("d", &w.agent), task("e", &w.agent), task("f", &w.agent)]);
    until("one call to fail", || first.is_finished() || second.is_finished()).await;
    let (failed, running) = if first.is_finished() { (first, second) } else { (second, first) };
    let error = tokio::task::spawn_blocking(move || failed.join().unwrap()).await.unwrap().expect_err("the call that did not fit failed");
    assert!(error.contains("at most 4") && error.contains("No sub-agent started"), "{error}");
    let started = w.started(&w.parent.id, 3).await;
    assert_eq!(started.len(), 3, "the failed call started nothing");
    // One more task fits. A fifth does not.
    w.rigs.scripts.lock().unwrap().insert("g".into(), vec![Step::working(), Step::Hang]);
    let third = call(&w, &w.parent.id, vec![task("g", &w.agent)]);
    w.started(&w.parent.id, 4).await;
    let error = failure(call(&w, &w.parent.id, vec![task("h", &w.agent)])).await;
    assert!(error.contains("at most 4") && error.contains("4 already run"), "{error}");
    assert_eq!(w.threads(&w.parent.id).len(), 4);
    w.core.cancel_in(Some(&w.parent.id.0));
    let results = finish(running).await;
    assert!(results.iter().all(|r| r.state == ThreadState::Cancelled));
    assert!(finish(third).await.iter().all(|r| r.state == ThreadState::Cancelled));
    // The places are free again.
    w.core.apply_channel(&w.parent.id.0, Update::TurnFinished(TurnOutcome::Cancelled));
    w.parent_turn_running();
    assert_eq!(finish(call(&w, &w.parent.id, vec![task("i", &w.agent)])).await[0].state, ThreadState::Done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anchoring_never_sets_a_final_thread_back_to_running_or_clears_its_follow() {
    let w = world();
    w.parent_turn_running();
    let results = finish(call(&w, &w.parent.id, vec![task("one", &w.agent)])).await;
    let thread = results[0].thread_channel_id.clone();
    w.channels.set_thread_followed(&thread, true).unwrap();
    // The parent engine read the thread before the sub-agent ended: its info says running and not followed.
    let mut stale = w.store.lock().unwrap().thread_info(&thread).unwrap().unwrap();
    stale.state = Some(ThreadState::Running);
    stale.followed = false;
    let response = put(&w.store.lock().unwrap(), &w.parent.id, Role::Agent, "Done.", Some(&w.agent));
    stale.anchor_message_id = Some(response.clone());
    w.core.apply_channel(&w.parent.id.0, Update::ResponseCompleted(Message { id: response.clone(), channel_id: w.parent.id.clone(), role: Role::Agent, blocks: vec![Block::Markdown { text: "Done.".into() }] }));
    w.core.apply_channel(&w.parent.id.0, Update::ThreadsAnchored(vec![stale]));
    let history = w.core.history(&ViewId::main());
    let chips = &history.iter().find(|m| m.id == response.0).unwrap().sub_agent_threads;
    assert_eq!((chips.len(), chips[0].state.as_deref(), chips[0].followed), (1, Some("done"), true), "the chip sits under the response and keeps its state and follow");
    assert_eq!(w.core.snapshot(&ViewId::main()).channels[0].threads[0].state.as_deref(), Some("done"), "the channel bar too");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_task_ends_when_the_thread_runtime_is_gone() {
    // The runtime is removed while the engine still runs. The end that its stream reports later reaches the task.
    let w = world();
    w.rigs.scripts.lock().unwrap().insert("lost".into(), vec![Step::working(), Step::Hang]);
    w.parent_turn_running();
    let call = call(&w, &w.parent.id, vec![task("lost", &w.agent)]);
    let thread = w.started(&w.parent.id, 1).await.remove(0);
    let generation = w.core.channel_runtime_generation(&thread.0).unwrap();
    w.core.remove_thread(&thread.0);
    assert!(!w.core.has_channel(&thread.0));
    w.core.apply_runtime(&thread.0, Some(generation), Update::SubAgentFinished { state: ThreadState::Done, text: "late answer".into() });
    let results = finish(call).await;
    assert_eq!((results[0].state, results[0].text.as_str()), (ThreadState::Done, "late answer"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_update_stream_fails_the_task_even_when_the_runtime_is_gone() {
    // The call would wait for the timeout if the closed stream did not reach the task.
    let w = world_with(Limits { timeout: Duration::from_secs(3), grace: Duration::from_secs(1) });
    let (release, vanish) = tokio::sync::oneshot::channel();
    *w.rigs.vanish.lock().unwrap() = Some(vanish);
    w.parent_turn_running();
    let call = call(&w, &w.parent.id, vec![task("vanish", &w.agent)]);
    let thread = w.loaded(&w.parent.id, 1).await.remove(0);
    w.core.remove_thread(&thread.0);
    release.send(()).unwrap();
    let started = std::time::Instant::now();
    let results = finish(call).await;
    assert_eq!(results[0].state, ThreadState::Failed, "{results:?}");
    assert!(results[0].text.contains("ended before it finished"), "{}", results[0].text);
    assert!(started.elapsed() < Duration::from_secs(2), "it did not wait for the timeout");
    assert_eq!(w.state(&thread), Some(ThreadState::Failed));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_start_removes_the_parked_runtime_and_the_chip_shows_failed() {
    let w = world();
    w.parent_turn_running();
    // The engine of "dead" is gone before the brief, so the brief cannot be sent after the thread was added.
    let results = finish(call(&w, &w.parent.id, vec![task("dead", &w.agent), task("fine", &w.agent)])).await;
    assert_eq!((results[0].state, results[1].state), (ThreadState::Failed, ThreadState::Done));
    let dead = results[0].thread_channel_id.clone();
    assert!(!w.core.has_channel(&dead.0), "the parked runtime of the failed thread is gone");
    assert!(w.core.has_channel(&results[1].thread_channel_id.0), "the other thread stays");
    assert_eq!(w.state(&dead), Some(ThreadState::Failed));
    let chips = w.core.snapshot(&ViewId::main()).live_sub_agent_threads;
    assert_eq!(chips.iter().map(|c| c.state.as_deref()).collect::<Vec<_>>(), [Some("failed"), Some("done")], "no chip stays running");
    // It can still be opened later: the transcript loads from storage again.
    w.channels.open_thread(&ViewId::main(), &dead).unwrap();
    assert_eq!(w.core.open_thread_id(&ViewId::main()), Some(dead.0.clone()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failure_to_save_the_response_reaches_the_task_detail() {
    let w = world();
    w.rigs.scripts.lock().unwrap().insert("save".into(), vec![Step::working(), Step::Hang]);
    w.parent_turn_running();
    let call = call(&w, &w.parent.id, vec![task("save", &w.agent)]);
    let thread = w.started(&w.parent.id, 1).await.remove(0);
    w.core.apply_channel(&thread.0, Update::ResponseSaveFailed("Discard could not save the agent message: disk full.".into()));
    w.core.apply_channel(&thread.0, Update::SubAgentFinished { state: ThreadState::Failed, text: String::new() });
    let results = finish(call).await;
    assert_eq!(results[0].state, ThreadState::Failed);
    assert!(results[0].text.contains("disk full"), "{:?}", results[0].text);
}

// ---- Agent add requests ----

struct Project {
    w: World,
    requests: Arc<AgentRequests>,
    reviewer: AgentId,
}

/// A project channel with a main agent that has delegation on, and an agent outside the project.
fn project_world() -> Project {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let agent = store.create_agent(&config("Quinn")).unwrap();
    let reviewer = store.create_agent(&config("Reviewer")).unwrap();
    let project = discard_core::model::Project { id: discard_core::model::ProjectId::new(), name: "demo".into(), path: dir.path().to_string_lossy().into_owned() };
    store.insert_project(&project).unwrap();
    store.set_project_main_agent(&project.id, &agent).unwrap();
    let parent = store.ensure_project_general_channel(&project.id, &agent).unwrap();
    let question = put(&store, &parent.id, Role::User, "who should review?", None);
    let store = Arc::new(Mutex::new(store));
    let rigs = Arc::new(Rigs::default());
    let launch = launcher(store.clone(), rigs.clone(), tokio::runtime::Handle::current());
    let (init, _updates) = (launch.clone())(&parent).unwrap();
    let core = Arc::new(AppCore::new(init));
    core.configure_read_marker(&ViewId::main(), dir.path(), &parent.id.0);
    let channels = Arc::new(Channels::new(core.clone(), store.clone(), tokio::runtime::Handle::current(), launch));
    let delegator = Arc::new(AppDelegator::new(Arc::downgrade(&channels)));
    Project { w: World { _dir: dir, store, core, channels, rigs, delegator, agent, parent, question }, requests: Arc::new(AgentRequests::default()), reviewer }
}

fn ask(p: &Project) -> discard_core::agent_requests::AgentAddRequest {
    p.requests.request(&p.w.store.lock().unwrap(), &p.w.parent.id, &p.w.agent, "Reviewer", "It reads code well.").unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adding_an_agent_makes_it_a_member_and_continues_the_parent() {
    let p = project_world();
    let request = ask(&p);
    assert!(request.created_at > 0);
    let dto: discard_app::desktop::dto::AgentRequestDto = request.clone().into();
    assert_eq!((dto.requesting_agent_name.as_str(), dto.agent_name.as_str(), dto.provider.as_str(), dto.model.as_deref(), dto.reason.as_str(), dto.state.as_str()), ("Quinn", "Reviewer", "fake", Some("test"), "It reads code well.", "pending"));
    let decided = p.w.channels.decide_agent_request(&p.requests, &p.w.parent.id.0, &request.id, true).unwrap();
    assert_eq!(decided.state, "added");
    assert!(discard_core::delegation::is_member(&p.w.store.lock().unwrap(), p.w.parent.project_id.as_ref().unwrap(), &p.reviewer).unwrap());
    let commands = p.w.parent_commands(&p.w.parent.id);
    assert!(matches!(&commands[..], [Command::Continue { agent, note }] if *agent == p.w.agent && note.contains("added Reviewer")), "{commands:?}");
    assert!(p.w.core.snapshot(&ViewId::main()).turn_running, "the parent continues");
    let note = p.w.core.history(&ViewId::main()).into_iter().last().unwrap();
    assert_eq!(note.role, "system");
    assert!(matches!(&note.blocks[..], [Block::Markdown { text }] if text.contains("Reviewer is now a project member")));
    assert_eq!(p.requests.list(&p.w.parent.id)[0].state, "added");
    assert!(p.w.channels.decide_agent_request(&p.requests, &p.w.parent.id.0, &request.id, true).is_err(), "decided once");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn denying_continues_the_parent_with_the_declined_command_and_adds_nobody() {
    let p = project_world();
    let request = ask(&p);
    let decided = p.w.channels.decide_agent_request(&p.requests, &p.w.parent.id.0, &request.id, false).unwrap();
    assert_eq!(decided.state, "denied");
    assert!(!discard_core::delegation::is_member(&p.w.store.lock().unwrap(), p.w.parent.project_id.as_ref().unwrap(), &p.reviewer).unwrap());
    let commands = p.w.parent_commands(&p.w.parent.id);
    assert!(matches!(&commands[..], [Command::ContinueDeclined { agent, note }] if *agent == p.w.agent && note.contains("declined")), "{commands:?}");
    assert!(p.w.core.snapshot(&ViewId::main()).turn_running);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_decision_is_refused_before_it_saves_when_the_parent_cannot_continue() {
    let p = project_world();
    let request = ask(&p);
    p.w.core.send_to(Some(&p.w.parent.id.0), "still talking".into()).unwrap();
    let error = p.w.channels.decide_agent_request(&p.requests, &p.w.parent.id.0, &request.id, true).unwrap_err();
    assert!(error.contains("Wait for the current response"), "{error}");
    assert_eq!(p.requests.list(&p.w.parent.id)[0].state, "pending", "nothing was decided");
    assert!(!discard_core::delegation::is_member(&p.w.store.lock().unwrap(), p.w.parent.project_id.as_ref().unwrap(), &p.reviewer).unwrap());
    let other = p.w.store.lock().unwrap().create_conversation(&p.w.agent, None, "other").unwrap();
    assert!(p.w.channels.decide_agent_request(&p.requests, &other.id.0, &request.id, true).is_err(), "only the selected channel");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_add_shows_the_failure_and_continues_nobody() {
    let p = project_world();
    let request = ask(&p);
    p.w.store.lock().unwrap().set_agent_archived(&p.reviewer, true).unwrap();
    let decided = p.w.channels.decide_agent_request(&p.requests, &p.w.parent.id.0, &request.id, true).unwrap();
    assert_eq!(decided.state, "failed");
    assert!(p.w.parent_commands(&p.w.parent.id).is_empty(), "the parent is not continued");
}
