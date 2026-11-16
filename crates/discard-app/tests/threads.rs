//! The bridge side of threads: app state for a selected channel and its open thread, and the channel service.
use discard_app::desktop::channels::{Channels, Launcher};
use discard_app::desktop::view::ViewId;
use discard_app::desktop::core::{AppCore, CoreInit};
use discard_app::desktop::pump::{Emit, pump_live};
use discard_core::agent::AgentConfig;
use discard_core::driver::TurnOutcome;
use discard_core::engine::Engine;
use discard_core::event::{Event, EventId, EventKind, SessionId};
use discard_core::model::{AgentId, Block, Channel, ChannelId, Message, MessageAuthor, MessageId, PostedSource, Role, ThreadInfo, ThreadKind};
use discard_core::store::Store;
use discard_core::ui_state::{Command, UiState, Update};
use discard_providers::fake::{FakeProvider, Step};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

fn text(chunk: &str) -> Update {
    Update::Event(Box::new(Event { id: EventId::new(), session_id: SessionId::new(), seq: 1, kind: EventKind::TextDelta { text: chunk.into() }, raw: None }))
}
fn message(id: &str, channel: &str, role: Role, body: &str) -> Message {
    Message { id: MessageId(id.into()), channel_id: ChannelId(channel.into()), role, blocks: vec![Block::Markdown { text: body.into() }] }
}
fn author(name: &str) -> MessageAuthor { MessageAuthor { agent_id: AgentId("agent".into()), name: name.into(), provider: "fake".into(), avatar: None } }
fn channel(id: &str) -> Channel {
    Channel { id: ChannelId(id.into()), project_id: Some(discard_core::model::ProjectId("p".into())), name: id.into(), agent_id: AgentId("agent".into()), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout }
}
fn thread_info(id: &str, parent: &str, anchor: &str) -> ThreadInfo {
    ThreadInfo { channel_id: ChannelId(id.into()), parent_channel_id: ChannelId(parent.into()), anchor_message_id: Some(MessageId(anchor.into())), kind: ThreadKind::Question, followed: true, name: "Why?".into(), state: None, agent_id: AgentId("g".into()), agent_name: "Gus".into() }
}

/// A selected channel `parent` with a question and an answer, and a loaded thread `thread` on the answer.
struct Rig {
    core: Arc<AppCore>,
    parent: mpsc::UnboundedReceiver<Command>,
    thread: mpsc::UnboundedReceiver<Command>,
    _dir: tempfile::TempDir,
}
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let (commands, parent) = mpsc::unbounded_channel();
    let history = vec![message("q", "parent", Role::User, "question"), message("a", "parent", Role::Agent, "answer")];
    let authors = [(MessageId("a".into()), author("Quinn"))].into_iter().collect();
    let core = Arc::new(AppCore::new(CoreInit { state: UiState::new(ChannelId("parent".into()), history).with_authors(authors), commands, project: "P".into(), channel: "parent".into(), agent: "Quinn".into(), provider: "fake".into() }));
    core.configure_read_marker(&ViewId::main(), dir.path(), "parent");
    let (commands, thread) = mpsc::unbounded_channel();
    core.add_thread(CoreInit { state: UiState::new(ChannelId("thread".into()), vec![]), commands, project: "P".into(), channel: "Why?".into(), agent: "Quinn".into(), provider: "fake".into() }, &channel("thread"), &thread_info("thread", "parent", "a"));
    Rig { core, parent, thread, _dir: dir }
}
/// Run one thread turn through the app state: the user message, a reply, and the end of the turn.
fn thread_turn(core: &AppCore, question: &str, reply: &str) {
    core.send_in_thread("thread", question.into(), vec![], false, Some(author("Quinn"))).unwrap();
    core.apply_channel("thread", Update::Responding(author("Quinn")));
    core.apply_channel("thread", text(reply));
    core.apply_channel("thread", Update::TurnFinished(TurnOutcome::Completed));
}

#[test]
fn a_thread_never_appears_in_the_channel_list_and_cannot_be_selected() {
    let r = rig();
    let snapshot = r.core.snapshot(&ViewId::main());
    assert_eq!(snapshot.channels.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["parent"]);
    assert_eq!(snapshot.channel_id, "parent");
    assert!(r.core.switch_channel(&ViewId::main(), "thread").is_err(), "a thread opens beside its channel and never replaces it");
    assert_eq!(r.core.active_channel_id(&ViewId::main()), "parent");
}

#[test]
fn the_anchor_message_carries_a_chip_with_the_thread_state() {
    let r = rig();
    assert!(r.core.history(&ViewId::main()).iter().all(|m| m.thread.is_none()), "a thread with no message has no chip");
    thread_turn(&r.core, "why  that\nway?", "Because.");
    let history = r.core.history(&ViewId::main());
    assert!(history[0].thread.is_none());
    let chip = history[1].thread.as_ref().expect("the anchor has a chip");
    assert_eq!((chip.channel_id.as_str(), chip.name.as_str(), chip.message_count, chip.followed), ("thread", "Why?", 2, true));
    let last = chip.last_message.as_ref().unwrap();
    assert_eq!((last.role.as_str(), last.preview.as_str()), ("agent", "Because."));
    assert!(chip.unread, "the reply arrived while nobody read the thread");
    assert!(r.core.history_snapshot(&ViewId::main()).messages[1].thread.is_some());
}

#[test]
fn a_chip_change_makes_the_parent_resend_its_transcript() {
    let r = rig();
    let before = r.core.history_snapshot(&ViewId::main()).rev;
    thread_turn(&r.core, "q", "a");
    let after_turn = r.core.history_snapshot(&ViewId::main()).rev;
    assert!(after_turn > before);
    r.core.set_thread_followed("thread", false);
    let after_leave = r.core.history_snapshot(&ViewId::main()).rev;
    assert!(after_leave > after_turn);
    assert!(!r.core.history(&ViewId::main())[1].thread.as_ref().unwrap().followed);
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    r.core.mark_read_in(Some("thread"), &r.core.thread_history_snapshot(&ViewId::main()).unwrap().messages[1].id).unwrap();
    assert!(r.core.history_snapshot(&ViewId::main()).rev > after_leave, "reading the thread clears the unread flag on the chip");
    assert!(!r.core.history(&ViewId::main())[1].thread.as_ref().unwrap().unread);
}

#[test]
fn the_open_thread_streams_beside_a_selected_channel_that_streams_too() {
    let mut r = rig();
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    // Both turns run at once. Each update lands in its own conversation.
    r.core.send_to(Some("parent"), "parent task".into()).unwrap();
    r.core.send_in_thread("thread", "side question".into(), vec![], false, Some(author("Quinn"))).unwrap();
    assert!(matches!(r.parent.try_recv(), Ok(Command::Send(t)) if t == "parent task"));
    assert!(matches!(r.thread.try_recv(), Ok(Command::SendInThread { text, also_post: false, .. }) if text == "side question"));
    r.core.apply_channel("parent", text("parent stream"));
    r.core.apply_channel("thread", text("thread stream"));
    let snapshot = r.core.snapshot(&ViewId::main());
    assert!(snapshot.turn_running && snapshot.live_blocks == vec![Block::Markdown { text: "parent stream".into() }]);
    let open = snapshot.open_thread.expect("the thread is open");
    assert!(open.turn_running);
    assert_eq!(open.live_blocks, vec![Block::Markdown { text: "thread stream".into() }]);
    assert_eq!(open.parent_channel_id, "parent");
    assert_eq!(open.anchor_message_id.as_deref(), Some("a"));
    assert!(open.read_only_note.contains("threads are read-only"), "{}", open.read_only_note);
    assert!(!open.composer_disabled);
    assert!(r.core.send_in_thread("thread", "again".into(), vec![], false, None).is_err(), "one turn at a time in a thread");
    r.core.apply_channel("thread", Update::TurnFinished(TurnOutcome::Completed));
    assert!(r.core.snapshot(&ViewId::main()).turn_running, "the thread turn ending leaves the parent turn running");
    assert!(!r.core.snapshot(&ViewId::main()).open_thread.unwrap().turn_running);
    let history = r.core.thread_history_snapshot(&ViewId::main()).unwrap();
    assert_eq!(history.anchor.as_ref().map(|m| m.id.as_str()), Some("a"));
    assert_eq!(history.messages.len(), 2, "the user message and the finished reply");
}

#[test]
fn the_thread_history_revision_moves_with_the_thread_transcript() {
    let r = rig();
    assert!(r.core.thread_history_snapshot(&ViewId::main()).is_none(), "no thread is open");
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    let opened = r.core.snapshot(&ViewId::main()).open_thread.unwrap().history_rev;
    assert_eq!(r.core.thread_history_snapshot(&ViewId::main()).unwrap().rev, opened);
    r.core.apply_channel("thread", text("streaming only"));
    assert_eq!(r.core.snapshot(&ViewId::main()).open_thread.unwrap().history_rev, opened, "live text is not history");
    thread_turn(&r.core, "q", "a");
    assert!(r.core.snapshot(&ViewId::main()).open_thread.unwrap().history_rev > opened);
    r.core.close_thread(&ViewId::main());
    assert!(r.core.snapshot(&ViewId::main()).open_thread.is_none());
    assert!(r.core.thread_history_snapshot(&ViewId::main()).is_none());
}

#[test]
fn only_a_thread_of_the_selected_channel_opens_and_selecting_another_channel_closes_it() {
    let r = rig();
    let (commands, _rx) = mpsc::unbounded_channel();
    let other = channel("other");
    r.core.add_channel(CoreInit { state: UiState::new(other.id.clone(), vec![]), commands, project: "P".into(), channel: "other".into(), agent: "A".into(), provider: "fake".into() }, &other);
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    assert!(r.core.open_thread(&ViewId::main(), "missing").is_err());
    r.core.switch_channel(&ViewId::main(), "other").unwrap();
    assert!(r.core.snapshot(&ViewId::main()).open_thread.is_none(), "the panel belongs to the channel that holds the anchor");
    assert!(r.core.open_thread(&ViewId::main(), "thread").is_err(), "the thread's channel is not selected");
    r.core.switch_channel(&ViewId::main(), "parent").unwrap();
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
}

#[test]
fn approvals_in_a_thread_are_decided_and_stopped_by_the_thread_id() {
    let mut r = rig();
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    r.core.send_in_thread("thread", "run it".into(), vec![], false, None).unwrap();
    let _ = r.thread.try_recv();
    let approval = discard_core::event::ApprovalId("approval-1".into());
    r.core.apply_channel("thread", Update::Event(Box::new(Event { id: EventId::new(), session_id: SessionId::new(), seq: 2, kind: EventKind::ApprovalRequest { kind: Default::default(), paths: vec![], approval_id: approval.clone(), call_id: "call-1".into(), tool: "Bash".into(), input: serde_json::json!({"command": "ls"}) }, raw: None })));
    let open = r.core.snapshot(&ViewId::main()).open_thread.unwrap();
    assert_eq!((open.pending_approvals, open.prompts.len(), open.agent_status.as_str()), (1, 1, "awaiting_approval"));
    assert_eq!(r.core.snapshot(&ViewId::main()).channels[0].threads[0].pending_approvals, 1, "the channel bar shows it too");
    r.core.decide_in(Some("other"), approval.to_string(), "approve".into(), None);
    assert!(r.thread.try_recv().is_err(), "an unknown channel decides nothing");
    r.core.decide_in(Some("thread"), approval.to_string(), "approve".into(), None);
    assert!(matches!(r.thread.try_recv(), Ok(Command::Decide { approval_id, .. }) if approval_id == approval));
    assert!(r.parent.try_recv().is_err(), "the parent never sees a thread decision");
    r.core.cancel_in(Some("thread"));
    assert_eq!(r.thread.try_recv().unwrap(), Command::Cancel);
    assert!(r.parent.try_recv().is_err());
}

#[test]
fn thread_activity_never_marks_the_parent_unread_but_a_posted_agent_message_does() {
    let r = rig();
    assert!(!r.core.snapshot(&ViewId::main()).unread);
    thread_turn(&r.core, "q", "an unread reply");
    assert!(!r.core.snapshot(&ViewId::main()).unread, "thread activity stays in the thread");
    assert!(r.core.snapshot(&ViewId::main()).channels[0].threads[0].unread, "the channel bar marks the thread");
    let reply = r.core.send_in_thread("thread", "more".into(), vec![], false, None);
    assert!(reply.is_ok());
    r.core.apply_channel("thread", Update::TurnFinished(TurnOutcome::Completed));
    // The engine reports a posted agent message.
    let source = PostedSource { source_message_id: MessageId("reply".into()), thread_channel_id: ChannelId("thread".into()), thread_name: "Why?".into() };
    r.core.apply_channel("thread", Update::MessagePosted { reference: message("ref", "parent", Role::Agent, "an unread reply"), source: source.clone() });
    let history = r.core.history(&ViewId::main());
    assert_eq!(history.last().unwrap().id, "ref", "the reference row comes last");
    assert_eq!(history.last().unwrap().role, "agent", "it keeps the source's role");
    let from = history.last().unwrap().posted_from.as_ref().expect("a link to the thread");
    assert_eq!((from.source_message_id.as_str(), from.thread_channel_id.as_str(), from.thread_name.as_str()), ("reply", "thread", "Why?"));
    assert!(r.core.snapshot(&ViewId::main()).unread, "a posted agent message counts like any agent message");
    assert!(r.core.snapshot(&ViewId::main()).new_anchor_id.as_deref() == Some("ref"));
    r.core.mark_read(&ViewId::main(), "ref").unwrap();
    assert!(!r.core.snapshot(&ViewId::main()).unread);
    // The same post again changes nothing.
    let rev = r.core.history_snapshot(&ViewId::main()).messages.len();
    r.core.apply_channel("thread", Update::MessagePosted { reference: message("ref", "parent", Role::Agent, "an unread reply"), source });
    assert_eq!(r.core.history_snapshot(&ViewId::main()).messages.len(), rev);
    assert!(!r.core.snapshot(&ViewId::main()).unread);
}

#[test]
fn a_posted_user_message_is_never_unread_and_a_background_parent_gets_it_too() {
    let r = rig();
    let (commands, _rx) = mpsc::unbounded_channel();
    let other = channel("other");
    r.core.add_channel(CoreInit { state: UiState::new(other.id.clone(), vec![]), commands, project: "P".into(), channel: "other".into(), agent: "A".into(), provider: "fake".into() }, &other);
    r.core.apply_channel("thread", Update::MessagePosted { reference: message("ref", "parent", Role::User, "my note"), source: PostedSource { source_message_id: MessageId("note".into()), thread_channel_id: ChannelId("thread".into()), thread_name: "Why?".into() } });
    assert!(!r.core.snapshot(&ViewId::main()).unread);
    r.core.switch_channel(&ViewId::main(), "other").unwrap();
    r.core.apply_channel("thread", Update::MessagePosted { reference: message("ref2", "parent", Role::Agent, "background"), source: PostedSource { source_message_id: MessageId("bg".into()), thread_channel_id: ChannelId("thread".into()), thread_name: "Why?".into() } });
    assert!(r.core.snapshot(&ViewId::main()).channels.iter().find(|c| c.id == "parent").unwrap().unread, "a parent in the background shows the posted agent message as unread");
    r.core.switch_channel(&ViewId::main(), "parent").unwrap();
    assert_eq!(r.core.history(&ViewId::main()).iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["q", "a", "ref", "ref2"]);
}

#[test]
fn the_thread_has_its_own_read_marker_and_the_parents_state_does_not_change() {
    let r = rig();
    thread_turn(&r.core, "q", "reply");
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    assert!(r.core.snapshot(&ViewId::main()).open_thread.unwrap().unread);
    let ids: Vec<_> = r.core.thread_history_snapshot(&ViewId::main()).unwrap().messages.iter().map(|m| m.id.clone()).collect();
    assert_eq!(r.core.last_read("thread"), None);
    r.core.mark_read_in(Some("thread"), &ids[1]).unwrap();
    assert_eq!(r.core.last_read("thread").as_deref(), Some(ids[1].as_str()));
    assert_eq!(r.core.last_read("parent").as_deref(), Some("a"), "the parent marker stays");
    assert!(!r.core.snapshot(&ViewId::main()).open_thread.unwrap().unread);
    assert!(!r.core.snapshot(&ViewId::main()).channels[0].threads[0].unread);
    // An older marker never replaces a newer one.
    r.core.mark_read_in(Some("thread"), &ids[0]).unwrap();
    assert_eq!(r.core.last_read("thread").as_deref(), Some(ids[1].as_str()));
}

#[test]
fn followed_threads_list_under_their_channel_and_leaving_removes_them() {
    let r = rig();
    assert!(r.core.snapshot(&ViewId::main()).channels[0].threads.is_empty(), "a thread with no message is not listed");
    thread_turn(&r.core, "q", "a");
    let links = &r.core.snapshot(&ViewId::main()).channels[0].threads;
    assert_eq!((links[0].id.as_str(), links[0].name.as_str(), links[0].parent_channel_id.as_str()), ("thread", "Why?", "parent"));
    r.core.set_thread_followed("thread", false);
    assert!(r.core.snapshot(&ViewId::main()).channels[0].threads.is_empty());
    r.core.set_thread_followed("thread", true);
    assert_eq!(r.core.snapshot(&ViewId::main()).channels[0].threads.len(), 1);
}

#[test]
fn the_saved_id_replaces_the_local_id_and_a_read_marker_follows_it() {
    let dir = tempfile::tempdir().unwrap();
    let (commands, _rx) = mpsc::unbounded_channel();
    let core = AppCore::new(CoreInit { state: UiState::new(ChannelId("c".into()), vec![]), commands, project: "P".into(), channel: "c".into(), agent: "A".into(), provider: "fake".into() });
    core.configure_read_marker(&ViewId::main(), dir.path(), "c");
    core.send_to(Some("c"), "hello".into()).unwrap();
    let local = core.history(&ViewId::main())[0].id.clone();
    core.mark_read(&ViewId::main(), &local).unwrap();
    assert_eq!(core.last_read("c").as_deref(), Some(local.as_str()));
    let before = core.history_snapshot(&ViewId::main()).rev;
    core.apply_channel("c", Update::UserMessageSaved(MessageId("saved".into())));
    assert_eq!(core.history(&ViewId::main())[0].id, "saved");
    assert!(core.history_snapshot(&ViewId::main()).rev > before, "the transcript is sent again with the saved ID");
    assert_eq!(core.last_read("c").as_deref(), Some("saved"));
}

#[test]
fn copy_and_plugin_targets_find_a_message_inside_the_open_thread() {
    use discard_core::{action::Action, context_menu::ContextTarget};
    let r = rig();
    thread_turn(&r.core, "side question", "side answer");
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    let ids: Vec<_> = r.core.thread_history_snapshot(&ViewId::main()).unwrap().messages.iter().map(|m| m.id.clone()).collect();
    let target = ContextTarget::Message { id: ids[1].clone(), has_text: true };
    assert_eq!(r.core.resolve_copy(&ViewId::main(), &target, Action::CopyMessageText).as_deref(), Some("side answer"));
    let parent_target = ContextTarget::Message { id: "a".into(), has_text: true };
    assert_eq!(r.core.resolve_copy(&ViewId::main(), &parent_target, Action::CopyMessageText).as_deref(), Some("answer"));
    r.core.close_thread(&ViewId::main());
    assert_eq!(r.core.resolve_copy(&ViewId::main(), &target, Action::CopyMessageText), None, "a closed thread is not searched");
}

// The service tests below use a real store and a fake provider.

struct World {
    _dir: tempfile::TempDir,
    store: Arc<Mutex<Store>>,
    core: Arc<AppCore>,
    channels: Arc<Channels>,
    parent: Channel,
    agent: AgentId,
    question: MessageId,
    answer: MessageId,
}

fn put(store: &Store, channel: &ChannelId, role: Role, body: &str, author: Option<&AgentId>) -> MessageId {
    let message = Message { id: MessageId::new(), channel_id: channel.clone(), role, blocks: vec![Block::Markdown { text: body.into() }] };
    match author {
        Some(agent) => store.insert_authored_message(&message, &MessageAuthor { agent_id: agent.clone(), name: "Quinn".into(), provider: "fake".into(), avatar: None }).unwrap(),
        None => store.insert_message(&message).unwrap(),
    }
    message.id
}

fn launcher(store: Arc<Mutex<Store>>, fail_threads: bool) -> Launcher {
    Arc::new(move |channel: &Channel| {
        let (agent, session, history, authors, is_thread) = {
            let store = store.lock().unwrap();
            (store.load_agent(&channel.agent_id).unwrap(), store.latest_session(&channel.id).unwrap().unwrap(), store.messages_for_channel(&channel.id).unwrap(),
             store.message_authors(&channel.id).unwrap(), store.thread_info(&channel.id).unwrap().is_some())
        };
        if fail_threads && is_thread { return Err("The provider is unavailable.".into()); }
        let engine = Engine {
            main_checkout_grants: Arc::default(), workspace_locks: Arc::default(),
            write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
            reasoning_effort: None, mode: discard_core::provider::SessionMode::Conversation, providers: vec![],
            store: store.clone(), agent_id: channel.agent_id.clone(), project_id: channel.project_id.clone(), memory: None, plugin_write: false,
            workspace_mode: channel.workspace_mode, workspace: None, observer: None, channel_name: channel.name.clone(),
            provider: Arc::new(FakeProvider::new(vec![Step::working(), Step::text("the answer"), Step::idle()])),
            channel_id: channel.id.clone(), session_id: session, cwd: ".".into(), model: Some("test".into()), system_prompt: None,
            policy: discard_core::capability::ApprovalPolicy::Ask,
        };
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates_rx) = mpsc::unbounded_channel();
        tokio::spawn(engine.run(commands_rx, updates_tx));
        Ok((CoreInit { state: UiState::new(channel.id.clone(), history).with_authors(authors), commands, project: String::new(), channel: channel.name.clone(), agent: agent.name, provider: agent.provider }, updates_rx))
    })
}

async fn world(fail_threads: bool) -> World {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let agent = store.create_agent(&AgentConfig::from_toml("name='Quinn'\nprovider='fake'\nmodel='test'\npolicy='ask'").unwrap()).unwrap();
    let parent = store.create_conversation(&agent, None, "chat").unwrap();
    let question = put(&store, &parent.id, Role::User, "what is a thread?", None);
    let answer = put(&store, &parent.id, Role::Agent, "a side conversation", Some(&agent));
    let store = Arc::new(Mutex::new(store));
    let launch = launcher(store.clone(), fail_threads);
    let (init, _updates) = (launch.clone())(&parent).unwrap();
    let core = Arc::new(AppCore::new(init));
    core.configure_read_marker(&ViewId::main(), dir.path(), &parent.id.0);
    let channels = Arc::new(Channels::new(core.clone(), store.clone(), tokio::runtime::Handle::current(), launch));
    channels.select(&ViewId::main(), &parent.id.0).unwrap();
    World { _dir: dir, store, core, channels, parent, agent, question, answer }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async { while !done() { tokio::time::sleep(Duration::from_millis(5)).await; } })
        .await.unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

#[tokio::test]
async fn start_thread_creates_the_thread_with_its_first_message_and_opens_it() {
    let w = world(false).await;
    assert!(w.core.history(&ViewId::main()).iter().all(|m| m.thread.is_none()));
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "Why  a\nthread?".into(), vec![], false).unwrap();
    let info = w.store.lock().unwrap().thread_info(&ChannelId(thread.clone())).unwrap().expect("the thread row exists");
    assert_eq!((info.name.as_str(), info.parent_channel_id.clone(), info.anchor_message_id.clone(), info.followed), ("Why a thread?", w.parent.id.clone(), Some(w.answer.clone()), true));
    assert_eq!(info.kind, ThreadKind::Question);
    assert_eq!(w.core.snapshot(&ViewId::main()).open_thread.as_ref().map(|t| t.channel_id.clone()), Some(thread.clone()), "the new thread is open");
    until("the thread reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && !w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.turn_running)).await;
    // The UI shows the stored message IDs, so any thread message can be posted.
    let stored = w.store.lock().unwrap().messages_for_channel(&ChannelId(thread.clone())).unwrap();
    let shown = w.core.thread_history_snapshot(&ViewId::main()).unwrap();
    assert_eq!(shown.messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), stored.iter().map(|m| m.id.0.clone()).collect::<Vec<_>>());
    assert_eq!(shown.anchor.as_ref().map(|m| m.id.clone()), Some(w.answer.0.clone()));
    assert_eq!(shown.messages[1].author.as_ref().map(|a| a.name.as_str()), Some("Quinn"));
    let chip = w.core.history(&ViewId::main()).into_iter().find(|m| m.id == w.answer.0).and_then(|m| m.thread).expect("a chip on the anchor");
    assert_eq!((chip.channel_id, chip.message_count, chip.followed), (thread.clone(), 2, true));
    assert_eq!(chip.last_message.unwrap().author_name.as_deref(), Some("Quinn"));
    assert!(w.core.snapshot(&ViewId::main()).channels.iter().all(|c| c.id == w.parent.id.0), "the thread is not a channel");
    assert!(!w.core.snapshot(&ViewId::main()).unread, "thread replies never mark the parent unread");
    assert_eq!(w.core.snapshot(&ViewId::main()).channels[0].threads.len(), 1);
    assert_eq!(w.store.lock().unwrap().messages_for_channel(&w.parent.id).unwrap().len(), 2, "the parent transcript is unchanged");
}

#[tokio::test]
async fn start_thread_surfaces_core_rejections_and_leaves_no_row() {
    let w = world(false).await;
    let first = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    until("the first reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2)).await;
    let error = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "second".into(), vec![], false).unwrap_err();
    assert!(error.contains("already has a thread"), "{error}");
    assert_eq!(w.store.lock().unwrap().threads_for_channel(&w.parent.id).unwrap().len(), 1);
    // A thread cannot start a thread: the parent must be the selected channel.
    let nested = w.channels.start_thread(&ViewId::main(), &first, &w.answer.0, "nested".into(), vec![], false).unwrap_err();
    assert!(nested.contains("channel changed"), "{nested}");
    let empty = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.question.0, "  ".into(), vec![], false).unwrap_err();
    assert!(empty.contains("Enter a message"), "{empty}");
    let unknown = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, "no-such-message", "hi".into(), vec![], false).unwrap_err();
    assert!(unknown.contains("saved message"), "{unknown}");
    let mention = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.question.0, "@Nobody hi".into(), vec![], false).unwrap_err();
    assert!(mention.contains("Nobody"), "{mention}");
    assert_eq!(w.store.lock().unwrap().threads_for_channel(&w.parent.id).unwrap().len(), 1, "a refused start creates no thread");
}

#[tokio::test]
async fn a_failed_first_send_removes_the_new_thread() {
    let w = world(true).await;
    let error = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "hi".into(), vec![], false).unwrap_err();
    assert_eq!(error, "The provider is unavailable.");
    assert!(w.store.lock().unwrap().threads_for_channel(&w.parent.id).unwrap().is_empty(), "no empty thread stays in storage");
    assert!(w.core.snapshot(&ViewId::main()).open_thread.is_none());
    assert!(w.core.history(&ViewId::main()).iter().all(|m| m.thread.is_none()));
    let retry = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "hi".into(), vec![], false).unwrap_err();
    assert_eq!(retry, "The provider is unavailable.", "the anchor is free for a new thread");
}

#[tokio::test]
async fn also_post_is_an_error_outside_a_thread_and_posts_the_first_message_in_one() {
    let w = world(false).await;
    let error = w.channels.send_with_options(&w.parent.id.0, "hello".into(), vec![], true).unwrap_err();
    assert!(error.contains("Only a message in a thread"), "{error}");
    assert_eq!(w.core.history(&ViewId::main()).len(), 2, "nothing was sent");
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.question.0, "share this".into(), vec![], true).unwrap();
    until("the post", || w.core.history(&ViewId::main()).iter().any(|m| m.posted_from.is_some())).await;
    let history = w.core.history(&ViewId::main());
    let reference = history.last().unwrap();
    assert_eq!(reference.role, "user");
    assert!(matches!(&reference.blocks[..], [Block::Markdown { text }] if text == "share this"));
    let from = reference.posted_from.as_ref().unwrap();
    assert_eq!((from.thread_channel_id.as_str(), from.thread_name.as_str()), (thread.as_str(), "share this"));
    let stored = w.store.lock().unwrap().messages_for_channel(&w.parent.id).unwrap();
    assert_eq!(stored.last().unwrap().id.0, reference.id, "the reference shown is the stored row");
    until("the reply", || !w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.turn_running)).await;
    assert!(!w.core.snapshot(&ViewId::main()).unread, "a posted user message is not unread");
}

#[tokio::test]
async fn sending_in_a_thread_keeps_it_followed_and_posts_on_request() {
    let w = world(false).await;
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    until("the first reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && !w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.turn_running)).await;
    w.channels.set_thread_followed(&ChannelId(thread.clone()), false).unwrap();
    assert!(w.core.snapshot(&ViewId::main()).channels[0].threads.is_empty(), "leaving removes it from the channel bar");
    assert!(!w.core.history(&ViewId::main()).into_iter().find(|m| m.id == w.answer.0).unwrap().thread.unwrap().followed, "the chip stays");
    assert_eq!(w.channels.thread_links(&w.parent.id).unwrap().len(), 0);
    w.channels.send_with_options(&thread, "second".into(), vec![], false).unwrap();
    until("the second reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 4) && !w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.turn_running)).await;
    assert!(w.store.lock().unwrap().thread_info(&ChannelId(thread.clone())).unwrap().unwrap().followed, "a reply follows the thread again");
    assert_eq!(w.channels.thread_links(&w.parent.id).unwrap().len(), 1);
    assert_eq!(w.core.snapshot(&ViewId::main()).channels[0].threads.len(), 1);
    // Also post one message.
    w.channels.send_with_options(&thread, "post me".into(), vec![], true).unwrap();
    until("the post", || w.core.history(&ViewId::main()).iter().any(|m| m.posted_from.is_some())).await;
    until("the third reply", || !w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.turn_running)).await;
    let thread_view = w.core.thread_history_snapshot(&ViewId::main()).unwrap();
    assert_eq!(thread_view.messages.iter().filter(|m| m.posted_to_channel).count(), 1, "the thread marks what it posted");
    assert!(thread_view.messages.iter().any(|m| m.posted_to_channel && matches!(&m.blocks[..], [Block::Markdown { text }] if text == "post me")));
}

#[tokio::test]
async fn posting_a_thread_message_is_idempotent_and_works_for_any_role() {
    let w = world(false).await;
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    until("the reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && !w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.turn_running)).await;
    assert!(!w.core.snapshot(&ViewId::main()).unread);
    let reply = w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages[1].id.clone();
    let reference = w.channels.post_thread_message(&reply).unwrap();
    let history = w.core.history(&ViewId::main());
    assert_eq!(history.last().unwrap().id, reference);
    assert_eq!(history.last().unwrap().role, "agent");
    assert_eq!(history.last().unwrap().author.as_ref().map(|a| a.name.as_str()), Some("Quinn"));
    assert_eq!(history.last().unwrap().posted_from.as_ref().unwrap().source_message_id, reply);
    assert!(w.core.snapshot(&ViewId::main()).unread, "a posted agent message counts as unread in the parent");
    assert_eq!(w.channels.post_thread_message(&reply).unwrap(), reference, "posting again returns the same row");
    assert_eq!(w.core.history(&ViewId::main()).len(), 3);
    assert_eq!(w.store.lock().unwrap().messages_for_channel(&w.parent.id).unwrap().len(), 3);
    // A user message posts too.
    let first = w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages[0].id.clone();
    let user_reference = w.channels.post_thread_message(&first).unwrap();
    assert_eq!(w.core.history(&ViewId::main()).last().unwrap().role, "user");
    assert_ne!(user_reference, reference);
    // A parent message or a reference row is not a thread message.
    assert!(w.channels.post_thread_message(&w.answer.0).is_err());
    assert!(w.channels.post_thread_message(&reference).is_err());
    assert!(w.channels.post_thread_message("missing").is_err());
    assert_eq!(thread, w.core.snapshot(&ViewId::main()).open_thread.unwrap().channel_id);
}

#[tokio::test]
async fn selecting_a_thread_selects_its_parent_and_opens_the_panel_after_a_restart() {
    let w = world(false).await;
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    until("the reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && !w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.turn_running)).await;
    // Another channel is selected, the way the channel bar leaves it.
    let other = w.store.lock().unwrap().create_conversation(&w.agent, None, "other").unwrap();
    w.channels.select(&ViewId::main(), &other.id.0).unwrap();
    assert_eq!(w.core.snapshot(&ViewId::main()).channel_id, other.id.0);
    assert!(w.core.snapshot(&ViewId::main()).open_thread.is_none());
    assert_eq!(w.channels.select_thread(&ViewId::main(), &ChannelId(thread.clone())).unwrap(), w.parent.id.0);
    assert_eq!(w.core.snapshot(&ViewId::main()).channel_id, w.parent.id.0);
    assert_eq!(w.core.snapshot(&ViewId::main()).open_thread.unwrap().channel_id, thread);
    // switch_channel with a thread id does the same, and a thread can never become the selected channel.
    w.channels.select(&ViewId::main(), &other.id.0).unwrap();
    w.channels.select(&ViewId::main(), &thread).unwrap();
    assert_eq!((w.core.snapshot(&ViewId::main()).channel_id, w.core.snapshot(&ViewId::main()).open_thread.map(|t| t.channel_id)), (w.parent.id.0.clone(), Some(thread.clone())));
    w.channels.open_thread(&ViewId::main(), &ChannelId(thread.clone())).unwrap();
    assert!(w.channels.open_thread(&ViewId::main(), &ChannelId("nope".into())).is_err());
    w.channels.select(&ViewId::main(), &other.id.0).unwrap();
    assert!(w.channels.open_thread(&ViewId::main(), &ChannelId(thread)).is_err(), "the thread's channel is not selected");
}

#[tokio::test]
async fn a_new_process_loads_the_chips_from_storage_when_the_channel_is_selected() {
    let w = world(false).await;
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    until("the reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && !w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.turn_running)).await;
    let reply = w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages[1].id.clone();
    w.channels.post_thread_message(&reply).unwrap();
    // A second app over the same store: nothing is loaded yet.
    let dir = tempfile::tempdir().unwrap();
    let launch = launcher(w.store.clone(), false);
    let (init, _updates) = launch(&w.parent).unwrap();
    let core = Arc::new(AppCore::new(init));
    core.configure_read_marker(&ViewId::main(), dir.path(), &w.parent.id.0);
    let channels = Channels::new(core.clone(), w.store.clone(), tokio::runtime::Handle::current(), launch);
    channels.refresh_threads(&w.parent.id).unwrap();
    let history = core.history(&ViewId::main());
    let chip = history.iter().find(|m| m.id == w.answer.0).and_then(|m| m.thread.clone()).expect("the chip comes from storage");
    assert_eq!((chip.channel_id.as_str(), chip.message_count, chip.unread), (thread.as_str(), 2, true));
    assert!(history.last().unwrap().posted_from.is_some(), "the reference row links to its thread after a restart");
    assert_eq!(core.snapshot(&ViewId::main()).channels[0].threads.len(), 1);
    assert_eq!(channels.thread_links(&w.parent.id).unwrap().len(), 1);
    channels.open_thread(&ViewId::main(), &ChannelId(thread)).unwrap();
    assert_eq!(core.thread_history_snapshot(&ViewId::main()).unwrap().messages.len(), 2, "the thread transcript loads from storage");
}

#[derive(Default)]
struct Recorder { snapshots: Mutex<Vec<(u64, String)>>, threads: Mutex<Vec<(String, u64, usize)>> }
impl Emit for Recorder {
    fn snapshot(&self, _: &ViewId, value: &discard_app::desktop::dto::SnapshotDto) { self.snapshots.lock().unwrap().push((value.rev, value.channel_id.clone())); }
    fn history(&self, _: &ViewId, _: &discard_app::desktop::dto::HistoryDto) {}
    fn thread_history(&self, _: &ViewId, value: &discard_app::desktop::dto::ThreadHistoryDto) { self.threads.lock().unwrap().push((value.channel_id.clone(), value.rev, value.messages.len())); }
}

#[tokio::test]
async fn the_pump_sends_the_open_thread_transcript_once_per_revision() {
    let r = rig();
    let recorder = Arc::new(Recorder::default());
    let (tx, rx) = mpsc::unbounded_channel::<Update>();
    let task = tokio::spawn(pump_live(r.core.clone(), rx, recorder.clone(), Duration::from_millis(1)));
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    until("the first thread history", || !recorder.threads.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(recorder.threads.lock().unwrap().len(), 1, "an unchanged revision is not sent again");
    thread_turn(&r.core, "q", "a");
    until("the second thread history", || recorder.threads.lock().unwrap().len() == 2).await;
    assert_eq!(recorder.threads.lock().unwrap().last().unwrap().2, 2);
    r.core.close_thread(&ViewId::main());
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    until("a history after reopening", || recorder.threads.lock().unwrap().len() == 3).await;
    drop(tx);
    task.abort();
}

#[tokio::test]
async fn the_project_channel_list_carries_the_followed_threads_with_their_unread_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let agent = store.create_agent(&AgentConfig::from_toml("name='Quinn'\nprovider='fake'\nmodel='test'\npolicy='ask'").unwrap()).unwrap();
    let project = discard_core::model::Project { id: discard_core::model::ProjectId::new(), name: "demo".into(), path: dir.path().to_string_lossy().into() };
    store.insert_project(&project).unwrap();
    store.set_project_main_agent(&project.id, &agent).unwrap();
    let general = store.ensure_project_general_channel(&project.id, &agent).unwrap();
    let anchor = put(&store, &general.id, Role::Agent, "an answer", Some(&agent));
    let later = put(&store, &general.id, Role::Agent, "another answer", Some(&agent));
    let followed = store.create_thread(&general.id, &anchor, "followed", &agent).unwrap();
    let left = store.create_thread(&general.id, &later, "left", &agent).unwrap();
    for thread in [&followed, &left] {
        put(&store, &thread.id, Role::User, "q", None);
        put(&store, &thread.id, Role::Agent, "a", Some(&agent));
    }
    store.set_thread_followed(&left.id, false).unwrap();
    let store = Arc::new(Mutex::new(store));
    let launch = launcher(store.clone(), false);
    let (init, _updates) = launch(&general).unwrap();
    let core = Arc::new(AppCore::new(init));
    core.configure_read_marker(&ViewId::main(), dir.path(), &general.id.0);
    let channels = Channels::new(core.clone(), store.clone(), tokio::runtime::Handle::current(), launch);
    let listed = channels.list_project_channels(&project.id).unwrap();
    let links = &listed.iter().find(|c| c.id == general.id.0).unwrap().threads;
    assert_eq!(links.len(), 1, "a thread that is not followed is absent");
    assert_eq!((links[0].id.as_str(), links[0].name.as_str(), links[0].parent_channel_id.as_str(), links[0].unread), (followed.id.0.as_str(), "followed", general.id.0.as_str(), true));
    let details = channels.channel_details(&project.id, &general.id).unwrap();
    assert_eq!(details.channel.threads, *links);
    // Reading the thread clears the flag in the next listing.
    let reply = store.lock().unwrap().messages_for_channel(&followed.id).unwrap()[1].id.0.clone();
    channels.refresh_threads(&general.id).unwrap();
    channels.open_thread(&ViewId::main(), &followed.id).unwrap();
    core.mark_read_in(Some(&followed.id.0), &reply).unwrap();
    assert!(!channels.list_project_channels(&project.id).unwrap().iter().find(|c| c.id == general.id.0).unwrap().threads[0].unread);
}

#[test]
fn a_thread_of_an_archived_channel_is_read_only() {
    let r = rig();
    r.core.open_thread(&ViewId::main(), "thread").unwrap();
    assert!(!r.core.snapshot(&ViewId::main()).open_thread.unwrap().composer_disabled);
    assert!(r.core.archive_started(&ViewId::main()));
    r.core.archive_finished(&ViewId::main(), discard_app::desktop::plan::ArchiveStep::Done("Archived.".into()));
    assert!(r.core.snapshot(&ViewId::main()).open_thread.unwrap().composer_disabled);
    assert_eq!(r.core.send_in_thread("thread", "hi".into(), vec![], false, None).unwrap_err(), "This thread cannot send a message right now.");
}

fn idle(w: &World) -> impl FnMut() -> bool + '_ {
    || !w.core.snapshot(&ViewId::main()).open_thread.is_some_and(|t| t.turn_running)
}

#[tokio::test]
async fn the_threads_list_holds_every_thread_latest_activity_first() {
    let w = world(false).await;
    assert!(w.core.snapshot(&ViewId::main()).channel_threads.is_empty(), "no thread, no list");
    let first = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    until("the first reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && idle(&w)()).await;
    let second = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.question.0, "second".into(), vec![], false).unwrap();
    until("the second reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && idle(&w)()).await;
    let ids = |w: &World| w.core.snapshot(&ViewId::main()).channel_threads.iter().map(|c| c.channel_id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&w), [second.clone(), first.clone()], "the newest thread is first");
    // A thread that is not followed is still listed, and activity in an older thread moves it up.
    w.channels.set_thread_followed(&ChannelId(first.clone()), false).unwrap();
    w.channels.send_with_options(&first, "again".into(), vec![], false).unwrap();
    until("the reply in the first thread", || w.core.snapshot(&ViewId::main()).channel_threads.iter().find(|c| c.channel_id == first).is_some_and(|c| c.message_count == 4)).await;
    assert_eq!(ids(&w), [first.clone(), second.clone()]);
    let listed = w.core.snapshot(&ViewId::main()).channel_threads;
    assert_eq!((listed[0].followed, listed[0].kind.as_str(), listed[0].name.as_str()), (true, "question", "first"), "a message follows the thread again");
    w.channels.set_thread_followed(&ChannelId(second.clone()), false).unwrap();
    assert!(!w.core.snapshot(&ViewId::main()).channel_threads.iter().find(|c| c.channel_id == second).unwrap().followed);
    // A thread of another channel is not listed.
    let other = w.store.lock().unwrap().create_conversation(&w.agent, None, "other").unwrap();
    w.channels.select(&ViewId::main(), &other.id.0).unwrap();
    assert!(w.core.snapshot(&ViewId::main()).channel_threads.is_empty());
    w.channels.select(&ViewId::main(), &w.parent.id.0).unwrap();
    assert_eq!(ids(&w).len(), 2);
}

#[tokio::test]
async fn renaming_a_thread_updates_the_record_the_chip_the_panel_and_the_posted_label() {
    let w = world(false).await;
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    until("the reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && idle(&w)()).await;
    let reply = w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages[1].id.clone();
    w.channels.post_thread_message(&reply).unwrap();
    let revision = w.core.history_snapshot(&ViewId::main()).rev;
    assert_eq!(w.channels.rename_thread(&ChannelId(thread.clone()), "  Latency\nplan  ").unwrap(), "Latency plan");
    assert!(w.core.history_snapshot(&ViewId::main()).rev > revision, "the parent sends its transcript again");
    let history = w.core.history(&ViewId::main());
    assert_eq!(history.iter().find(|m| m.id == w.answer.0).unwrap().thread.as_ref().unwrap().name, "Latency plan", "the chip");
    assert_eq!(history.last().unwrap().posted_from.as_ref().unwrap().thread_name, "Latency plan", "the posted row");
    assert_eq!(w.core.snapshot(&ViewId::main()).open_thread.unwrap().name, "Latency plan", "the panel header");
    assert_eq!(w.core.snapshot(&ViewId::main()).channel_threads[0].name, "Latency plan", "the threads list");
    assert_eq!(w.core.snapshot(&ViewId::main()).channels[0].threads[0].name, "Latency plan", "the channel bar");
    assert_eq!(w.store.lock().unwrap().thread_info(&ChannelId(thread.clone())).unwrap().unwrap().name, "Latency plan");
    assert_eq!(w.channels.rename_thread(&ChannelId(thread.clone()), "").unwrap_err(), "Enter a thread name of 1 to 80 characters.");
    assert!(w.channels.rename_thread(&ChannelId(thread.clone()), &"a".repeat(81)).is_err());
    assert!(w.channels.rename_thread(&w.parent.id, "x").is_err(), "a channel is not a thread");
    assert_eq!(w.core.snapshot(&ViewId::main()).open_thread.unwrap().name, "Latency plan", "a refused name changes nothing");
}

#[tokio::test]
async fn deleting_a_thread_keeps_posted_messages_closes_the_panel_and_drops_the_marker() {
    let w = world(false).await;
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    assert!(w.channels.delete_thread(&ChannelId(thread.clone())).unwrap_err().contains("turn runs"), "the first turn still runs");
    until("the reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && idle(&w)()).await;
    let messages = w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages;
    let reference = w.channels.post_thread_message(&messages[1].id).unwrap();
    w.core.mark_read_in(Some(&thread), &messages[1].id).unwrap();
    assert!(w.core.last_read(&thread).is_some());
    assert!(w.core.history(&ViewId::main()).iter().any(|m| m.thread.is_some()), "the anchor has a chip");
    assert_eq!(w.channels.thread_posted_count(&ChannelId(thread.clone())).unwrap(), 1, "the dialog says how many stay");
    let kept = w.channels.delete_thread(&ChannelId(thread.clone())).unwrap();
    assert_eq!(kept, 1, "one posted message stays in the channel");
    assert!(w.core.snapshot(&ViewId::main()).open_thread.is_none(), "the panel closes");
    assert!(w.core.thread_history_snapshot(&ViewId::main()).is_none());
    assert!(w.core.snapshot(&ViewId::main()).channel_threads.is_empty() && w.core.snapshot(&ViewId::main()).channels[0].threads.is_empty());
    assert!(w.core.last_read(&thread).is_none(), "the read marker goes");
    assert!(!w.core.has_channel(&thread), "the session is closed");
    let history = w.core.history(&ViewId::main());
    assert!(history.iter().all(|m| m.thread.is_none()), "the chip under the anchor is gone");
    let stayed = history.iter().find(|m| m.id == reference).expect("the posted message stays");
    assert!(stayed.posted_from.is_none(), "it has no thread link");
    assert_eq!((stayed.role.as_str(), stayed.author.as_ref().map(|a| a.name.as_str())), ("agent", Some("Quinn")));
    assert!(matches!(&stayed.blocks[..], [Block::Markdown { text }] if text == "the answer"));
    assert_eq!(history.len(), 3);
    let store = w.store.lock().unwrap();
    assert!(store.thread_info(&ChannelId(thread.clone())).unwrap().is_none());
    assert!(store.posted_sources(&w.parent.id).unwrap().is_empty());
    assert_eq!(store.messages_for_channel(&w.parent.id).unwrap().len(), 3);
    drop(store);
    assert!(w.channels.delete_thread(&ChannelId(thread)).is_err(), "it is gone");
    // The anchor can start a new thread.
    w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "again".into(), vec![], false).unwrap();
}

#[tokio::test]
async fn a_thread_with_a_pending_approval_or_a_running_sub_agent_task_cannot_be_deleted() {
    let w = world(false).await;
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    until("the reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && idle(&w)()).await;
    let approval = discard_core::event::ApprovalId("approval-1".into());
    w.core.apply_channel(&thread, Update::Event(Box::new(Event { id: EventId::new(), session_id: SessionId::new(), seq: 9, kind: EventKind::ApprovalRequest { kind: Default::default(), paths: vec![], approval_id: approval.clone(), call_id: "call-1".into(), tool: "Bash".into(), input: serde_json::json!({"command": "ls"}) }, raw: None })));
    assert_eq!(w.core.thread_delete_blocker(&thread).map(|reason| reason.contains("approval")), Some(true));
    assert!(w.channels.delete_thread(&ChannelId(thread.clone())).is_err());
    assert!(w.store.lock().unwrap().thread_info(&ChannelId(thread.clone())).unwrap().is_some(), "a refused delete changes nothing");
    assert!(w.core.snapshot(&ViewId::main()).open_thread.is_some() && w.core.has_channel(&thread), "the thread stays open and loaded");
    // A running sub-agent task blocks the delete too.
    let sub = w.store.lock().unwrap().create_sub_agent_thread(&w.parent.id, "Research", &w.agent).unwrap();
    w.channels.refresh_threads(&w.parent.id).unwrap();
    let error = w.channels.delete_thread(&sub.id).unwrap_err();
    assert!(error.contains("task still runs"), "{error}");
    assert!(w.store.lock().unwrap().thread_info(&sub.id).unwrap().is_some());
    w.store.lock().unwrap().finish_sub_agent_thread(&sub.id, discard_core::model::ThreadState::Cancelled).unwrap();
}

#[tokio::test]
async fn removing_a_posted_message_from_the_channel_keeps_it_in_the_thread() {
    let w = world(false).await;
    let thread = w.channels.start_thread(&ViewId::main(), &w.parent.id.0, &w.answer.0, "first".into(), vec![], false).unwrap();
    until("the reply", || w.core.thread_history_snapshot(&ViewId::main()).is_some_and(|h| h.messages.len() == 2) && idle(&w)()).await;
    let reply = w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages[1].id.clone();
    let reference = w.channels.post_thread_message(&reply).unwrap();
    assert!(w.core.thread_history_snapshot(&ViewId::main()).unwrap().messages[1].posted_to_channel);
    // The channel read marker sits on the reference row. It moves to the row before it.
    w.core.mark_read(&ViewId::main(), &reference).unwrap();
    let revision = (w.core.history_snapshot(&ViewId::main()).rev, w.core.thread_history_snapshot(&ViewId::main()).unwrap().rev);
    w.channels.remove_posted_message(&reference).unwrap();
    assert_eq!(w.core.history(&ViewId::main()).len(), 2, "the row is gone");
    assert!(w.core.history(&ViewId::main()).iter().all(|m| m.id != reference && m.posted_from.is_none()));
    assert_eq!(w.core.last_read(&w.parent.id.0).as_deref(), Some(w.answer.0.as_str()), "the marker moves to the row before");
    let thread_view = w.core.thread_history_snapshot(&ViewId::main()).unwrap();
    assert!(!thread_view.messages[1].posted_to_channel, "Post to channel is available again");
    assert_eq!(thread_view.messages.len(), 2, "the message stays in the thread");
    assert!(w.core.history_snapshot(&ViewId::main()).rev > revision.0 && thread_view.rev > revision.1);
    assert_eq!(w.store.lock().unwrap().messages_for_channel(&w.parent.id).unwrap().len(), 2);
    assert!(w.channels.remove_posted_message(&reference).is_err(), "it is gone");
    assert!(w.channels.remove_posted_message(&reply).is_err(), "a thread message is not a reference");
    let again = w.channels.post_thread_message(&reply).unwrap();
    assert_ne!(again, reference);
    assert_eq!(w.core.history(&ViewId::main()).len(), 3);
    assert_eq!(w.core.snapshot(&ViewId::main()).open_thread.unwrap().channel_id, thread);
}
