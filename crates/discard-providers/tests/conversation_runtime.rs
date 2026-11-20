//! Conversation runtime behavior against real SQLite and markdown memory.
use discard_core::memory::{MemoryStore, Scope};
use discard_core::memory_items::{ItemKind, MutationOrigin};
use discard_core::{
    agent::AgentConfig,
    capability::ApprovalPolicy,
    engine::Engine,
    model::{AgentId, Block, ChannelId, Message, MessageId, Role, WorkspaceMode},
    provider::SessionMode,
    store::Store,
    ui_state::{Command, Update},
};
use discard_mcp::service::MemoryRuntime;
use discard_providers::fake::{FakeProvider, Probe, Step};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct Rig {
    store: Arc<Mutex<Store>>,
    memory: Arc<MemoryStore>,
    runtime: Arc<MemoryRuntime>,
    agent: AgentId,
    channel: ChannelId,
    probe: Probe,
    commands: mpsc::UnboundedSender<Command>,
    updates: mpsc::UnboundedReceiver<Update>,
    task: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}
/// Count of 4000-byte messages that overflow the default budget `batches` times over.
fn messages_over_budget(batches: usize) -> usize {
    batches * discard_core::conversation_context::input_budget(None) / 4000 + 2
}
impl Rig {
    async fn new(script: Vec<Step>) -> Self {
        let provider = Arc::new(FakeProvider::new(script).with_resume_handle("must-not-resume"));
        let probe = provider.probe.clone();
        Self::with_provider(provider, probe).await
    }
    async fn with_provider(
        provider: Arc<dyn discard_core::provider::Provider>,
        probe: Probe,
    ) -> Self {
        Self::with_memory_service(provider, probe, |runtime, _| runtime).await
    }
    async fn with_memory_service(
        provider: Arc<dyn discard_core::provider::Provider>,
        probe: Probe,
        decorate: impl FnOnce(
            Arc<MemoryRuntime>,
            Arc<MemoryStore>,
        ) -> Arc<dyn discard_core::engine::MemoryService>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store.db")).unwrap();
        let agent = store
            .create_agent(&AgentConfig {
                workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
                reasoning_effort: None,
                name: "A".into(),
                avatar: None,
                provider: "fake".into(),
                model: Some("one".into()),
                system_prompt: "First personality".into(),
                policy: ApprovalPolicy::Ask,
                plugin_write: true,
            })
            .unwrap();
        let channel = store.default_conversation(&agent).unwrap();
        let session = store.latest_session(&channel.id).unwrap().unwrap();
        let store = Arc::new(Mutex::new(store));
        let memory = Arc::new(
            MemoryStore::open(&dir.path().join("memory"), &dir.path().join("index.db")).unwrap(),
        );
        let runtime = MemoryRuntime::start(memory.clone(), &dir.path().join("run"))
            .await
            .unwrap();
        let memory_service = decorate(runtime.clone(), memory.clone());
        let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
            reasoning_effort: None,
            mode: SessionMode::Conversation,
            providers: vec![provider.clone()],
            store: store.clone(),
            agent_id: agent.clone(),
            project_id: None,
            memory: Some(memory_service),
            plugin_write: true,
            workspace_mode: WorkspaceMode::Worktree,
            workspace: None,
            observer: None,
            channel_name: "General".into(),
            provider,
            channel_id: channel.id.clone(),
            session_id: session,
            cwd: dir.path().join("empty"),
            model: Some("stale".into()),
            system_prompt: Some("stale".into()),
            policy: ApprovalPolicy::Ask,
        };
        let (commands, rx) = mpsc::unbounded_channel();
        let (tx, updates) = mpsc::unbounded_channel();
        let task = tokio::spawn(engine.run(rx, tx));
        Self {
            store,
            memory,
            runtime,
            agent,
            channel: channel.id,
            probe,
            commands,
            updates,
            task,
            _dir: dir,
        }
    }
    async fn done(&mut self) -> Vec<Update> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut out = vec![];
            while let Some(update) = self.updates.recv().await {
                let done = matches!(update, Update::TurnFinished(_));
                out.push(update);
                if done {
                    return out;
                }
            }
            panic!("engine closed")
        })
        .await
        .unwrap()
    }
    fn edit(&self, edit: impl FnOnce(&mut AgentConfig)) {
        let store = self.store.lock().unwrap();
        let mut config = store.load_agent(&self.agent).unwrap();
        edit(&mut config);
        store.update_agent(&self.agent, &config).unwrap();
    }
    async fn close(self) {
        drop(self.commands);
        self.task.await.unwrap();
        self.runtime.shutdown().await;
    }
}

#[tokio::test]
async fn image_attachments_reach_conversation_turns_and_followup_history() {
    let mut rig = Rig::new(vec![Step::text("I can see it"), Step::idle()]).await;
    let image = discard_core::attachments::Attachment {
        id: "retained-image".into(), name: "diagram.png".into(), media_type: "image/png".into(),
        size: 100, path: rig._dir.path().join("image.png"), text: None,
    };
    rig.commands.send(Command::SendAttachments { text: "Describe this".into(), attachments: vec![image.clone()] }).unwrap();
    let updates = rig.done().await;
    assert!(!updates.iter().any(|u| matches!(u, Update::Error(_))), "{updates:?}");
    assert_eq!(rig.probe.images.lock().unwrap()[0], vec![image.clone()]);
    rig.commands.send(Command::Send("What color is it?".into())).unwrap();
    let updates = rig.done().await;
    assert!(!updates.iter().any(|u| matches!(u, Update::Error(_))), "{updates:?}");
    assert_eq!(rig.probe.images.lock().unwrap()[1], vec![image]);
    assert!(rig.probe.sent.lock().unwrap()[1].contains("diagram.png"));
}

#[tokio::test]
async fn each_turn_refreshes_personality_memory_model_and_retains_only_own_history() {
    let mut rig = Rig::new(vec![Step::text("answer one"), Step::idle()]).await;
    let item = rig
        .memory
        .create_item(
            &Scope::Agent(rig.agent.clone()),
            ItemKind::Instruction,
            "original memory",
            Some(&rig.channel),
            MutationOrigin::User,
        )
        .unwrap();
    rig.commands
        .send(Command::Send("first request".into()))
        .unwrap();
    rig.done().await;
    rig.edit(|c| {
        c.model = Some("two".into());
        c.reasoning_effort = Some("high".into());
        c.system_prompt = "Second personality".into();
    });
    rig.memory
        .update_item(
            &item.id,
            item.version,
            "edited memory",
            MutationOrigin::User,
        )
        .unwrap();
    rig.commands
        .send(Command::Send("second request".into()))
        .unwrap();
    let updates = rig.done().await;
    let specs = rig.probe.specs.lock().unwrap().clone();
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].model.as_deref(), Some("one"));
    assert_eq!(specs[1].model.as_deref(), Some("two"));
    assert_eq!(specs[0].reasoning_effort, None);
    assert_eq!(specs[1].reasoning_effort.as_deref(), Some("high"));
    assert!(
        specs[1]
            .system_prompt
            .as_ref()
            .unwrap()
            .contains("Second personality")
    );
    assert!(
        specs[1]
            .system_prompt
            .as_ref()
            .unwrap()
            .contains("edited memory")
    );
    assert!(
        !specs[1]
            .system_prompt
            .as_ref()
            .unwrap()
            .contains("original memory")
    );
    assert!(
        specs
            .iter()
            .all(|s| s.resume_handle.is_none() && s.mode == SessionMode::Conversation)
    );
    assert!(rig.probe.sent.lock().unwrap()[1].contains("first request"));
    assert!(rig.probe.sent.lock().unwrap()[1].contains("answer one"));
    assert!(
        updates
            .iter()
            .any(|u| matches!(u, Update::ConversationMarker(_)))
    );
    assert!(
        std::fs::read_dir(rig._dir.path().join("run"))
            .unwrap()
            .next()
            .is_none()
    );
    rig.memory.forget_item(&item.id, item.version + 1).unwrap();
    rig.commands
        .send(Command::Send("third request".into()))
        .unwrap();
    rig.done().await;
    assert!(
        !rig.probe.specs.lock().unwrap()[2]
            .system_prompt
            .as_ref()
            .unwrap()
            .contains("edited memory")
    );
    rig.close().await;
}

#[tokio::test]
async fn worktree_facts_reach_each_reply_without_entering_history_or_direct_chats() {
    use discard_core::{model::Channel, store::WorktreeRecord};
    let mut rig = Rig::new(vec![Step::text("direct answer"), Step::idle()]).await;
    rig.commands.send(Command::Send("direct request".into())).unwrap();
    rig.done().await;
    assert!(!rig.probe.sent.lock().unwrap()[0].contains("<worktree-snapshot>"));
    rig.task.abort();
    let _ = (&mut rig.task).await;
    let repo = rig._dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [vec!["init", "-q", "-b", "main"], vec!["commit", "--allow-empty", "-m", "Initial commit"], vec!["checkout", "--detach"]] {
        let output = discard_core::process::spawn_std(std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .args(args).current_dir(&repo).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
    let (channel, session) = {
        let store = rig.store.lock().unwrap();
        let project = store.import_project_folder(&repo).unwrap();
        let channel = Channel { id: ChannelId::new(), project_id: Some(project.id), name: "worktree".into(),
            agent_id: rig.agent.clone(), workspace_mode: WorkspaceMode::Worktree };
        let record = WorktreeRecord { channel_id: channel.id.clone(), path: repo.to_string_lossy().into(),
            branch: "saved-head".into(), base_commit: String::new(), repo: repo.to_string_lossy().into(), owned: false };
        store.create_named_repository_channel(&channel, "", None, Some(&record)).unwrap();
        let session = store.latest_session(&channel.id).unwrap().unwrap();
        (channel, session)
    };
    let provider = Arc::new(FakeProvider::new(vec![Step::text("worktree answer"), Step::idle()]));
    let probe = provider.probe.clone();
    let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
        reasoning_effort: None, mode: SessionMode::Conversation, providers: vec![provider.clone()],
        store: rig.store.clone(), agent_id: rig.agent.clone(), project_id: channel.project_id.clone(),
        memory: Some(rig.runtime.clone()), plugin_write: false, workspace_mode: WorkspaceMode::Worktree,
        workspace: None, observer: None, channel_name: channel.name.clone(), provider,
        channel_id: channel.id.clone(), session_id: session,
        // This unrelated path must never supply the worktree snapshot.
        cwd: rig._dir.path().join("empty"), model: None, system_prompt: None, policy: ApprovalPolicy::Ask,
    };
    let (commands, rx) = mpsc::unbounded_channel();
    let (tx, updates) = mpsc::unbounded_channel();
    rig.commands = commands;
    rig.updates = updates;
    rig.task = tokio::spawn(engine.run(rx, tx));
    rig.commands.send(Command::Send("What can we do with this worktree?".into())).unwrap();
    rig.done().await;
    std::fs::write(repo.join("new-file.txt"), "local change").unwrap();
    rig.commands.send(Command::Send("What changed?".into())).unwrap();
    rig.done().await;
    let sent = probe.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].contains("# branch.head (detached)"));
    assert!(sent[0].contains("Initial commit"));
    assert!(!sent[0].contains("new-file.txt"));
    assert!(sent[1].contains("? new-file.txt"));
    assert!(sent.iter().all(|text| text.matches("<worktree-snapshot>").count() == 1));
    assert!(!sent[0].contains("direct request"));
    assert!(!probe.specs.lock().unwrap().iter().any(|spec| spec.mode != SessionMode::Conversation));
    let history = rig.store.lock().unwrap().messages_for_channel(&channel.id).unwrap();
    assert!(!history.iter().any(|message| format!("{:?}", message.blocks).contains("<worktree-snapshot>")));
    rig.close().await;
}

#[tokio::test]
async fn general_project_context_reaches_followups_without_polluting_history() {
    let mut rig = Rig::new(vec![Step::text("direct answer"), Step::idle()]).await;
    rig.commands.send(Command::Send("Hello".into())).unwrap();
    rig.done().await;
    assert!(!rig.probe.sent.lock().unwrap()[0].contains("<project-context>"));
    rig.task.abort();
    let _ = (&mut rig.task).await;
    let repo = rig._dir.path().join("project");
    std::fs::create_dir(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "Discard is an agent workspace.").unwrap();
    let (channel, session) = {
        let store = rig.store.lock().unwrap();
        let project = store.import_project_folder(&repo).unwrap();
        let channel = store.ensure_project_general_channel(&project.id, &rig.agent).unwrap();
        let session = store.latest_session(&channel.id).unwrap().unwrap();
        (channel, session)
    };
    let provider = Arc::new(FakeProvider::new(vec![Step::text("project answer"), Step::idle()]));
    let probe = provider.probe.clone();
    let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
        reasoning_effort: None, mode: SessionMode::Conversation, providers: vec![provider.clone()],
        store: rig.store.clone(), agent_id: rig.agent.clone(), project_id: channel.project_id.clone(),
        memory: Some(rig.runtime.clone()), plugin_write: false, workspace_mode: WorkspaceMode::MainCheckout,
        workspace: None, observer: None, channel_name: channel.name.clone(), provider,
        channel_id: channel.id.clone(), session_id: session, cwd: rig._dir.path().join("empty"),
        model: None, system_prompt: None, policy: ApprovalPolicy::Ask,
    };
    let (commands, rx) = mpsc::unbounded_channel();
    let (tx, updates) = mpsc::unbounded_channel();
    rig.commands = commands;
    rig.updates = updates;
    rig.task = tokio::spawn(engine.run(rx, tx));
    rig.commands.send(Command::Send("What can you tell me about this project?".into())).unwrap();
    rig.done().await;
    std::fs::write(repo.join("README.md"), "Discard manages projects and agents.").unwrap();
    rig.commands.send(Command::Send("What changed?".into())).unwrap();
    rig.done().await;
    let sent = probe.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].contains("Discard is an agent workspace."));
    assert!(sent[1].contains("Discard manages projects and agents."));
    assert!(!sent[1].contains("Discard is an agent workspace."));
    assert!(sent[1].contains("project answer"));
    assert!(sent.iter().all(|input| input.matches("<project-context>").count() == 1));
    assert!(sent.iter().all(|input| !input.contains("<worktree-snapshot>")));
    let history = rig.store.lock().unwrap().messages_for_channel(&channel.id).unwrap();
    assert!(!history.iter().any(|message| format!("{:?}", message.blocks).contains("<project-context>")));
    rig.close().await;
}

#[tokio::test]
async fn conversation_prompt_refreshes_topic_without_writing_it_to_the_transcript() {
    let mut rig = Rig::new(vec![Step::text("first answer"), Step::text("second answer"), Step::idle()]).await;
    rig.store.lock().unwrap().save_channel_topic(&rig.channel, 0, "Old topic", discard_core::model::TopicAuthorKind::User, None).unwrap();
    rig.commands.send(Command::Send("first request".into())).unwrap();
    rig.done().await;
    let prior = rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap();
    rig.store.lock().unwrap().save_channel_topic(&rig.channel, 1, "Fresh topic", discard_core::model::TopicAuthorKind::User, None).unwrap();
    rig.commands.send(Command::Send("second request".into())).unwrap();
    rig.done().await;

    let sent = rig.probe.sent.lock().unwrap().clone();
    assert!(sent[0].contains("Old topic"));
    assert!(sent[1].contains("Fresh topic"));
    assert!(!sent[1].contains("Old topic"));
    let messages = rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap();
    assert_eq!(&messages[..prior.len()], &prior[..], "topic edits do not delete or rewrite transcript messages");
    assert!(!messages.iter().any(|message| format!("{:?}", message.blocks).contains("Fresh topic")));
    rig.close().await;
}

#[tokio::test]
async fn agent_identity_reaches_each_turn_and_renames_preserve_history() {
    let mut rig = Rig::new(vec![Step::text("Hello"), Step::idle()]).await;
    rig.edit(|c| {
        c.name = "Quinn".into();
        c.system_prompt.clear();
    });
    rig.commands.send(Command::Send("What is your name?".into())).unwrap();
    rig.done().await;
    rig.edit(|c| {
        c.name = "Robin".into();
        c.system_prompt = "Be concise.".into();
    });
    rig.commands.send(Command::Send("What is your name now?".into())).unwrap();
    rig.done().await;
    let specs = rig.probe.specs.lock().unwrap().clone();
    let first = specs[0].system_prompt.as_deref().unwrap();
    let second = specs[1].system_prompt.as_deref().unwrap();
    assert!(first.contains("Your name is \"Quinn\"."));
    assert!(second.contains("Your name is \"Robin\"."));
    assert!(!second.contains("Your name is \"Quinn\"."));
    assert!(second.contains("Be concise."));
    assert!(rig.probe.sent.lock().unwrap()[1].contains("What is your name?"));
    rig.close().await;
}

#[tokio::test]
async fn chat_turns_tell_the_agent_that_plugin_tools_need_a_builder_session() {
    let mut rig = Rig::new(vec![Step::text("Hello"), Step::idle()]).await;
    rig.commands.send(Command::Send("Write a plugin.".into())).unwrap();
    rig.done().await;
    rig.edit(|c| c.plugin_write = false);
    rig.commands.send(Command::Send("Write a plugin.".into())).unwrap();
    rig.done().await;
    let specs = rig.probe.specs.lock().unwrap().clone();
    let on = specs[0].system_prompt.as_deref().unwrap();
    let off = specs[1].system_prompt.as_deref().unwrap();
    assert!(on.contains("Your profile allows plugin writes, but this chat has no plugin tools."));
    assert!(off.contains("This chat has no plugin tools."));
    assert!(off.contains("asks the Builder agent"));
    rig.close().await;
}

#[tokio::test]
async fn archive_and_unavailable_provider_prevent_new_turn_without_fallback() {
    let mut rig = Rig::new(vec![Step::text("ok"), Step::idle()]).await;
    rig.edit(|c| c.provider = "missing".into());
    rig.commands.send(Command::Send("first".into())).unwrap();
    let updates = rig.done().await;
    assert!(
        updates
            .iter()
            .any(|u| matches!(u,Update::Error(e) if e.contains("unavailable")))
    );
    assert!(rig.probe.specs.lock().unwrap().is_empty());
    rig.edit(|c| c.provider = "fake".into());
    rig.store
        .lock()
        .unwrap()
        .set_agent_archived(&rig.agent, true)
        .unwrap();
    rig.commands.send(Command::Send("second".into())).unwrap();
    rig.done().await;
    assert!(rig.probe.specs.lock().unwrap().is_empty());
    assert!(
        rig.store
            .lock()
            .unwrap()
            .messages_for_channel(&rig.channel)
            .unwrap()
            .is_empty()
    );
    rig.close().await;
}

#[tokio::test]
async fn active_response_keeps_configuration_snapshot_and_next_response_sees_edit() {
    let mut rig = Rig::new(vec![Step::text("started"), Step::Hang]).await;
    rig.commands.send(Command::Send("first".into())).unwrap();
    while !matches!(rig.updates.recv().await, Some(Update::Event(_))) {}
    rig.edit(|c| c.model = Some("edited".into()));
    assert_eq!(
        rig.probe.specs.lock().unwrap()[0].model.as_deref(),
        Some("one")
    );
    rig.commands.send(Command::Cancel).unwrap();
    rig.done().await;
    rig.commands.send(Command::Send("next".into())).unwrap();
    while !matches!(rig.updates.recv().await, Some(Update::Event(_))) {}
    assert_eq!(
        rig.probe.specs.lock().unwrap()[1].model.as_deref(),
        Some("edited")
    );
    rig.commands.send(Command::Cancel).unwrap();
    rig.done().await;
    rig.close().await;
}

#[tokio::test]
async fn summary_is_semantic_reusable_and_does_not_delete_original_messages() {
    let mut rig = Rig::new(vec![Step::text("A semantic summary."), Step::idle()]).await;
    for i in 0..messages_over_budget(1) {
        rig.store
            .lock()
            .unwrap()
            .insert_message(&Message {
                id: MessageId::new(),
                channel_id: rig.channel.clone(),
                role: Role::User,
                blocks: vec![Block::Markdown {
                    text: format!("older {i} {}", "x".repeat(4000)),
                }],
            })
            .unwrap();
    }
    rig.commands
        .send(Command::Send("current request".into()))
        .unwrap();
    let updates = rig.done().await;
    assert!(
        updates
            .iter()
            .any(|u| matches!(u,Update::Notice(s) if s.contains("summary")))
    );
    let state = rig
        .store
        .lock()
        .unwrap()
        .conversation_context(&rig.channel)
        .unwrap();
    assert_eq!(state.summary.as_deref(), Some("A semantic summary."));
    assert!(state.through.is_some());
    assert!(
        rig.probe
            .sent
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("current request")
    );
    assert!(
        rig.probe
            .sent
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("A semantic summary.")
    );
    assert!(
        rig.store
            .lock()
            .unwrap()
            .messages_for_channel(&rig.channel)
            .unwrap()
            .len()
            >= 12
    );
    assert!(
        rig.memory
            .list_items(&Scope::applicable(&rig.agent, None))
            .unwrap()
            .is_empty()
    );
    let calls = rig.probe.specs.lock().unwrap().len();
    rig.commands
        .send(Command::Send("next current".into()))
        .unwrap();
    rig.done().await;
    assert_eq!(rig.probe.specs.lock().unwrap().len(), calls + 1);
    rig.close().await;
}

#[tokio::test]
async fn a_thread_saves_its_own_summary_over_the_parent_prefix_and_excludes_later_parent_messages() {
    let mut rig = Rig::new(vec![Step::text("A semantic summary."), Step::idle()]).await;
    let say = |channel: &ChannelId, role: Role, text: String| {
        let message = Message { id: MessageId::new(), channel_id: channel.clone(), role, blocks: vec![Block::Markdown { text }] };
        rig.store.lock().unwrap().insert_message(&message).unwrap();
        message.id
    };
    let mut older = Vec::new();
    for i in 0..messages_over_budget(1) { older.push(say(&rig.channel, Role::User, format!("older {i} {}", "x".repeat(4000)))); }
    let anchor = say(&rig.channel, Role::Agent, "the anchor".into());
    say(&rig.channel, Role::User, "LATER PARENT MESSAGE".into());
    let thread = rig.store.lock().unwrap().create_thread(&rig.channel, &anchor, "t", &rig.agent).unwrap();
    rig.task.abort();
    let _ = (&mut rig.task).await;
    let provider = Arc::new(FakeProvider::new(vec![Step::text("A semantic summary."), Step::idle()]));
    let probe = provider.probe.clone();
    let session = rig.store.lock().unwrap().latest_session(&thread.id).unwrap().unwrap();
    let engine = Engine {
        main_checkout_grants: Arc::default(), workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
        reasoning_effort: None, mode: SessionMode::Coding, providers: vec![provider.clone()], store: rig.store.clone(), agent_id: rig.agent.clone(), project_id: None,
        memory: Some(rig.runtime.clone()), plugin_write: false, workspace_mode: WorkspaceMode::MainCheckout, workspace: None, observer: None,
        channel_name: thread.name.clone(), provider, channel_id: thread.id.clone(), session_id: session, cwd: rig._dir.path().into(), model: None, system_prompt: None, policy: ApprovalPolicy::Ask,
    };
    let (commands, rx) = mpsc::unbounded_channel();
    let (tx, updates) = mpsc::unbounded_channel();
    rig.commands = commands;
    rig.updates = updates;
    rig.task = tokio::spawn(engine.run(rx, tx));
    rig.commands.send(Command::Send("side question".into())).unwrap();
    let updates = rig.done().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Notice(s) if s.contains("summary"))));
    let (thread_state, parent_state) = {
        let store = rig.store.lock().unwrap();
        (store.conversation_context(&thread.id).unwrap(), store.conversation_context(&rig.channel).unwrap())
    };
    assert_eq!(thread_state.summary.as_deref(), Some("A semantic summary."));
    let through = thread_state.through.expect("a saved boundary").0;
    let source = through.split(":part:").next().unwrap().to_owned();
    assert!(older.iter().any(|id| id.0 == source), "the boundary sits inside the parent prefix, not after the anchor");
    assert_eq!(parent_state, Default::default(), "the parent's own summary state is untouched");
    let sent = probe.sent.lock().unwrap().last().unwrap().clone();
    assert!(sent.contains("side question") && sent.contains("A semantic summary."));
    assert!(!sent.contains("LATER PARENT MESSAGE"));
    assert_eq!(rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap().len(), older.len() + 2, "no original message is removed");
    rig.close().await;
}

#[tokio::test]
async fn summary_cancellation_revokes_tokens_and_keeps_boundary_unmodified() {
    let mut rig = Rig::new(vec![Step::Hang]).await;
    for _ in 0..messages_over_budget(1) {
        rig.store
            .lock()
            .unwrap()
            .insert_message(&Message {
                id: MessageId::new(),
                channel_id: rig.channel.clone(),
                role: Role::User,
                blocks: vec![Block::Markdown {
                    text: "x".repeat(4000),
                }],
            })
            .unwrap();
    }
    rig.commands.send(Command::Send("current".into())).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while rig.probe.specs.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    rig.commands.send(Command::Cancel).unwrap();
    rig.done().await;
    assert!(
        rig.store
            .lock()
            .unwrap()
            .conversation_context(&rig.channel)
            .unwrap()
            .through
            .is_none()
    );
    assert!(
        std::fs::read_dir(rig._dir.path().join("run"))
            .unwrap()
            .next()
            .is_none()
    );
    assert!(
        rig.probe
            .cancelled
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    rig.close().await;
}

#[tokio::test]
async fn oversized_current_request_is_an_actionable_error_without_provider_call() {
    let mut rig = Rig::new(vec![Step::idle()]).await;
    rig.commands.send(Command::Send("x".repeat(discard_core::conversation_context::input_budget(None)))).unwrap();
    let updates = rig.done().await;
    assert!(
        updates
            .iter()
            .any(|u| matches!(u,Update::Error(s) if s.contains("Shorten the request")))
    );
    assert!(rig.probe.specs.lock().unwrap().is_empty());
    rig.close().await;
}

#[tokio::test]
async fn saved_and_proposed_changes_attach_once_to_the_completed_response() {
    let mut rig = Rig::new(vec![Step::text("response"), Step::Hang]).await;
    rig.commands.send(Command::Send("remember".into())).unwrap();
    while !matches!(rig.updates.recv().await, Some(Update::Event(_))) {}
    let saved = rig
        .memory
        .capture_item(
            &rig.agent,
            None,
            &rig.channel,
            &Scope::Agent(rig.agent.clone()),
            ItemKind::Fact,
            "durable fact",
        )
        .unwrap();
    let proposal = rig
        .memory
        .capture_item(
            &rig.agent,
            None,
            &rig.channel,
            &Scope::Global,
            ItemKind::Fact,
            "shared fact",
        )
        .unwrap();
    assert!(matches!(
        proposal,
        discard_core::memory_items::SaveOutcome::Proposed(_)
    ));
    assert!(matches!(
        saved,
        discard_core::memory_items::SaveOutcome::Saved(_)
    ));
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.done().await;
    let response = updates
        .iter()
        .find_map(|u| {
            if let Update::ResponseCompleted(m) = u {
                Some(m.id.clone())
            } else {
                None
            }
        })
        .unwrap();
    let records = rig
        .store
        .lock()
        .unwrap()
        .response_memory(&rig.channel)
        .unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|(id, _)| id == &response));
    assert!(updates.iter().any(|u|matches!(u,Update::MemoryChanges {response:id,changes} if id==&response && changes.len()==2)));
    rig.commands.send(Command::Send("again".into())).unwrap();
    while !matches!(rig.updates.recv().await, Some(Update::Event(_))) {}
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.done().await;
    assert!(
        !updates
            .iter()
            .any(|u| matches!(u, Update::MemoryChanges { .. }))
    );
    assert_eq!(
        rig.store
            .lock()
            .unwrap()
            .response_memory(&rig.channel)
            .unwrap()
            .len(),
        2
    );
    rig.store
        .lock()
        .unwrap()
        .delete_agent(
            &rig.agent,
            discard_core::store::DeleteAgentOptions {
                delete_conversations: true,
                delete_private_memory: false,
            },
        )
        .unwrap();
    assert!(
        rig.store
            .lock()
            .unwrap()
            .response_memory(&rig.channel)
            .unwrap()
            .is_empty()
    );
    assert!(
        rig.store
            .lock()
            .unwrap()
            .conversation_context(&rig.channel)
            .unwrap()
            .summary
            .is_none()
    );
    rig.close().await;
}

#[tokio::test]
async fn summary_token_is_read_only_and_regular_token_has_no_project_or_plugin_server() {
    use discard_core::engine::MemoryService;
    use discard_mcp::tools::McpHandler;
    let rig = Rig::new(vec![Step::idle()]).await;
    let prepared = rig
        .runtime
        .prepare_summary(&rig.agent, None, &rig.channel)
        .unwrap();
    let identity = rig.runtime.registry().lookup(&prepared.token).unwrap();
    assert!(!identity.can_write);
    assert_eq!(identity.project, None);
    let handler = McpHandler::new(rig.memory.clone());
    let reply=handler.handle(&identity,&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"memory_save","arguments":{"scope":"agent","text":"forbidden"}}})).unwrap();
    assert_eq!(reply["result"]["isError"], true);
    rig.runtime.finish(&prepared.token);
    assert!(rig.runtime.registry().lookup(&prepared.token).is_none());
    assert!(!prepared.mcp_config.exists());
    let prepared = rig
        .runtime
        .prepare(&rig.agent, None, &rig.channel, None, false)
        .unwrap();
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&prepared.mcp_config).unwrap()).unwrap();
    assert_eq!(config["mcpServers"].as_object().unwrap().len(), 1);
    let identity = rig.runtime.registry().lookup(&prepared.token).unwrap();
    assert_eq!(identity.project, None);
    assert_eq!(identity.source_conversation, Some(rig.channel.clone()));
    rig.runtime.finish(&prepared.token);
    assert_eq!(rig.runtime.registry().active(), 0);
    rig.close().await;
}

struct NamedProvider {
    name: &'static str,
    inner: FakeProvider,
}
#[async_trait::async_trait]
impl discard_core::provider::Provider for NamedProvider {
    fn id(&self) -> &str {
        self.name
    }
    fn capabilities(&self) -> &discard_core::capability::CapabilityMatrix {
        discard_core::provider::Provider::capabilities(&self.inner)
    }
    async fn check_setup(&self) -> discard_core::provider::SetupStatus {
        discard_core::provider::Provider::check_setup(&self.inner).await
    }
    async fn start_session(
        &self,
        spec: discard_core::provider::SessionSpec,
    ) -> Result<
        Box<dyn discard_core::provider::ProviderSession>,
        discard_core::provider::ProviderError,
    > {
        discard_core::provider::Provider::start_session(&self.inner, spec).await
    }
}

#[tokio::test]
async fn provider_switch_preserves_conversation_and_records_visible_marker() {
    let mut rig = Rig::new(vec![Step::text("first answer"), Step::idle()]).await;
    rig.commands
        .send(Command::Send("first user".into()))
        .unwrap();
    rig.done().await;
    // Restart the runtime with a changed provider registry, preserving app-owned history.
    rig.task.abort();
    let _ = (&mut rig.task).await;
    rig.edit(|c| {
        c.provider = "other".into();
        c.model = Some("other-model".into());
    });
    let other = Arc::new(NamedProvider {
        name: "other",
        inner: FakeProvider::new(vec![Step::text("other answer"), Step::idle()]),
    });
    let probe = other.inner.probe.clone();
    let session = rig
        .store
        .lock()
        .unwrap()
        .latest_session(&rig.channel)
        .unwrap()
        .unwrap();
    let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
        reasoning_effort: None,
        mode: SessionMode::Conversation,
        providers: vec![other.clone()],
        store: rig.store.clone(),
        agent_id: rig.agent.clone(),
        project_id: None,
        memory: Some(rig.runtime.clone()),
        plugin_write: false,
        workspace_mode: WorkspaceMode::MainCheckout,
        workspace: None,
        observer: None,
        channel_name: "General".into(),
        provider: other,
        channel_id: rig.channel.clone(),
        session_id: session,
        cwd: rig._dir.path().into(),
        model: None,
        system_prompt: None,
        policy: ApprovalPolicy::Ask,
    };
    let (commands, rx) = mpsc::unbounded_channel();
    let (tx, updates) = mpsc::unbounded_channel();
    rig.commands = commands;
    rig.updates = updates;
    rig.task = tokio::spawn(engine.run(rx, tx));
    rig.commands
        .send(Command::Send("next user".into()))
        .unwrap();
    let updates = rig.done().await;
    assert!(probe.sent.lock().unwrap()[0].contains("first user"));
    assert!(probe.sent.lock().unwrap()[0].contains("first answer"));
    assert!(updates.iter().any(|u|matches!(u,Update::ConversationMarker(m) if matches!(&m.blocks[0],Block::Markdown{text} if text.contains("other-model")))));
    rig.close().await;
}

#[tokio::test]
async fn facts_follow_request_relevance_but_all_preferences_and_instructions_remain() {
    let mut rig = Rig::new(vec![Step::text("ok"), Step::idle()]).await;
    let scope = Scope::Agent(rig.agent.clone());
    for (kind, body) in [
        (ItemKind::Fact, "Preferred compiler is Rust"),
        (ItemKind::Fact, "Garden flowers are blue"),
        (ItemKind::Preference, "Use short sentences"),
        (ItemKind::Instruction, "Keep decisions explicit"),
    ] {
        rig.memory
            .create_item(&scope, kind, body, None, MutationOrigin::User)
            .unwrap();
    }
    rig.commands
        .send(Command::Send("Which Rust compiler?".into()))
        .unwrap();
    rig.done().await;
    {
        let specs = rig.probe.specs.lock().unwrap();
        let system = specs[0].system_prompt.as_ref().unwrap();
        assert!(system.contains("Preferred compiler is Rust"));
        assert!(!system.contains("Garden flowers"));
        assert!(system.contains("Use short sentences"));
        assert!(system.contains("Keep decisions explicit"));
    }
    rig.close().await;
}

#[tokio::test]
async fn response_insert_failure_keeps_live_reply_and_never_claims_saved_id() {
    let mut rig = Rig::new(vec![Step::text("recover this reply"), Step::Hang]).await;
    let connection = rusqlite::Connection::open(rig._dir.path().join("store.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_agent_message BEFORE INSERT ON messages WHEN NEW.role='agent' BEGIN SELECT RAISE(ABORT, 'test disk write failed'); END;").unwrap();
    rig.commands.send(Command::Send("request".into())).unwrap();
    let mut updates = vec![rig.updates.recv().await.unwrap()];
    rig.memory
        .capture_item(
            &rig.agent,
            None,
            &rig.channel,
            &Scope::Agent(rig.agent.clone()),
            ItemKind::Fact,
            "saved before transcript failure",
        )
        .unwrap();
    rig.commands.send(Command::Cancel).unwrap();
    updates.extend(rig.done().await);
    assert!(
        !updates
            .iter()
            .any(|u| matches!(u, Update::MemoryChanges { .. }))
    );
    assert!(
        !updates
            .iter()
            .any(|u| matches!(u, Update::ResponseCompleted(_)))
    );
    let mut ui = discard_core::ui_state::UiState::new(rig.channel.clone(), vec![]);
    ui.user_sent("request");
    for update in updates {
        ui.apply(update);
    }
    assert!(ui.notice().is_some_and(|s| s.contains("Copy")));
    assert!(!ui.turn_running());
    assert_eq!(
        ui.live_blocks(),
        &[Block::Markdown {
            text: "recover this reply".into()
        }]
    );
    assert_eq!(ui.history().len(), 1);
    assert!(
        rig.store
            .lock()
            .unwrap()
            .response_memory(&rig.channel)
            .unwrap()
            .is_empty()
    );
    assert_eq!(rig.runtime.registry().active(), 0);
    rig.close().await;
}

#[tokio::test]
async fn user_insert_failure_prevents_provider_turn() {
    let mut rig = Rig::new(vec![Step::text("must not run"), Step::idle()]).await;
    let connection = rusqlite::Connection::open(rig._dir.path().join("store.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_user_message BEFORE INSERT ON messages WHEN NEW.role='user' BEGIN SELECT RAISE(ABORT, 'test disk write failed'); END;").unwrap();
    rig.commands.send(Command::Send("request".into())).unwrap();
    let updates = rig.done().await;
    assert!(
        updates
            .iter()
            .any(|u| matches!(u,Update::Error(s) if s.contains("could not save")))
    );
    assert!(rig.probe.specs.lock().unwrap().is_empty());
    assert!(
        rig.store
            .lock()
            .unwrap()
            .messages_for_channel(&rig.channel)
            .unwrap()
            .is_empty()
    );
    rig.close().await;
}

struct BatchProvider {
    inner: FakeProvider,
    calls: std::sync::atomic::AtomicUsize,
    cancel: bool,
}
#[async_trait::async_trait]
impl discard_core::provider::Provider for BatchProvider {
    fn id(&self) -> &str {
        "fake"
    }
    fn capabilities(&self) -> &discard_core::capability::CapabilityMatrix {
        discard_core::provider::Provider::capabilities(&self.inner)
    }
    async fn check_setup(&self) -> discard_core::provider::SetupStatus {
        discard_core::provider::SetupStatus::Ready
    }
    async fn start_session(
        &self,
        spec: discard_core::provider::SessionSpec,
    ) -> Result<
        Box<dyn discard_core::provider::ProviderSession>,
        discard_core::provider::ProviderError,
    > {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut fake = FakeProvider::new(if call == 0 {
            vec![Step::text("successful first summary"), Step::idle()]
        } else if self.cancel {
            vec![Step::Hang]
        } else {
            vec![Step::Emit(discard_core::event::EventKind::Error {
                message: "second batch failed".into(),
            })]
        });
        fake.probe = self.inner.probe.clone();
        discard_core::provider::Provider::start_session(&fake, spec).await
    }
}

#[tokio::test]
async fn later_summary_batch_failure_or_cancel_keeps_original_boundary_and_cleans_up() {
    for cancel in [false, true] {
        let provider = Arc::new(BatchProvider {
            inner: FakeProvider::new(vec![]),
            calls: std::sync::atomic::AtomicUsize::new(0),
            cancel,
        });
        let probe = provider.inner.probe.clone();
        let mut rig = Rig::with_provider(provider, probe).await;
        let old = discard_core::conversation_context::ConversationContext {
            provider: Some("fake".into()),
            model: Some("one".into()),
            summary: Some("previous durable summary".into()),
            through: None,
            ..Default::default()
        };
        rig.store
            .lock()
            .unwrap()
            .set_conversation_context(&rig.channel, &old)
            .unwrap();
        for _ in 0..messages_over_budget(2) {
            rig.store
                .lock()
                .unwrap()
                .insert_message(&Message {
                    id: MessageId::new(),
                    channel_id: rig.channel.clone(),
                    role: Role::User,
                    blocks: vec![Block::Markdown {
                        text: "x".repeat(4000),
                    }],
                })
                .unwrap();
        }
        rig.commands.send(Command::Send("current".into())).unwrap();
        if cancel {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while rig.probe.sent.lock().unwrap().len() < 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            rig.commands.send(Command::Cancel).unwrap();
        }
        let updates = rig.done().await;
        let expected = if cancel {
            discard_core::driver::TurnOutcome::Cancelled
        } else {
            discard_core::driver::TurnOutcome::Failed
        };
        assert!(
            updates
                .iter()
                .any(|u| matches!(u,Update::TurnFinished(outcome) if *outcome==expected))
        );
        if cancel {
            assert!(!updates.iter().any(|u| matches!(u, Update::Error(_))));
        }
        assert_eq!(
            rig.store
                .lock()
                .unwrap()
                .conversation_context(&rig.channel)
                .unwrap(),
            old
        );
        assert_eq!(rig.probe.specs.lock().unwrap().len(), 2);
        assert_eq!(rig.runtime.registry().active(), 0);
        assert!(
            std::fs::read_dir(rig._dir.path().join("run"))
                .unwrap()
                .next()
                .is_none()
        );
        rig.close().await;
    }
}

#[tokio::test]
async fn user_memory_edit_during_response_is_not_an_assistant_save_marker() {
    let mut rig = Rig::new(vec![Step::text("reply"), Step::Hang]).await;
    rig.commands.send(Command::Send("request".into())).unwrap();
    while !matches!(rig.updates.recv().await, Some(Update::Event(_))) {}
    rig.memory
        .create_item(
            &Scope::Agent(rig.agent.clone()),
            ItemKind::Fact,
            "user's own edit",
            Some(&rig.channel),
            MutationOrigin::User,
        )
        .unwrap();
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.done().await;
    assert!(
        !updates
            .iter()
            .any(|u| matches!(u, Update::MemoryChanges { .. }))
    );
    assert!(
        rig.store
            .lock()
            .unwrap()
            .response_memory(&rig.channel)
            .unwrap()
            .is_empty()
    );
    rig.close().await;
}

struct ApproveAfterSnapshot {
    runtime: Arc<MemoryRuntime>,
    memory: Arc<MemoryStore>,
}
impl discard_core::engine::MemoryService for ApproveAfterSnapshot {
    fn prepare(
        &self,
        agent: &AgentId,
        project: Option<&discard_core::model::ProjectId>,
        source: &ChannelId,
        base: Option<&str>,
        plugin: bool,
    ) -> Result<discard_core::engine::PreparedMemory, String> {
        self.runtime.prepare(agent, project, source, base, plugin)
    }
    fn changes(
        &self,
        source: &ChannelId,
    ) -> Result<Vec<discard_core::memory_items::MemoryChange>, String> {
        let snapshot = self.runtime.changes(source)?;
        for change in &snapshot {
            if let discard_core::memory_items::MemoryChange::Proposed(p) = change {
                let approved = self.memory.approve_proposal(&p.id).unwrap();
                self.memory
                    .update_item(
                        &approved.id,
                        approved.version,
                        "later user edit",
                        MutationOrigin::User,
                    )
                    .unwrap();
            }
        }
        Ok(snapshot)
    }
    fn approved_changes(
        &self,
        source: &ChannelId,
    ) -> Result<Vec<discard_core::memory_items::MemoryChange>, String> {
        self.runtime.approved_changes(source)
    }
    fn finish(&self, token: &str) {
        self.runtime.finish(token)
    }
}

#[tokio::test]
async fn approval_between_change_snapshot_and_row_insert_keeps_original_approved_card() {
    let provider = Arc::new(FakeProvider::new(vec![Step::text("response"), Step::Hang]));
    let probe = provider.probe.clone();
    let mut rig = Rig::with_memory_service(provider, probe, |runtime, memory| {
        Arc::new(ApproveAfterSnapshot { runtime, memory })
    })
    .await;
    rig.commands.send(Command::Send("remember".into())).unwrap();
    while !matches!(rig.updates.recv().await, Some(Update::Event(_))) {}
    let proposal = rig
        .memory
        .capture_item(
            &rig.agent,
            None,
            &rig.channel,
            &Scope::Global,
            ItemKind::Fact,
            "original approved value",
        )
        .unwrap();
    assert!(matches!(
        proposal,
        discard_core::memory_items::SaveOutcome::Proposed(_)
    ));
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.done().await;
    let response = updates
        .iter()
        .find_map(|u| {
            if let Update::ResponseCompleted(m) = u {
                Some(m.id.clone())
            } else {
                None
            }
        })
        .unwrap();
    let records = rig
        .store
        .lock()
        .unwrap()
        .response_memory(&rig.channel)
        .unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|(id, _)| id == &response));
    assert!(records.iter().any(|(_,c)|matches!(c,discard_core::memory_items::MemoryChange::Saved{item,..} if item.version==1 && item.content=="original approved value" && item.origin==MutationOrigin::Approved)));
    assert!(updates.iter().any(|u|matches!(u,Update::MemoryChanges{response:id,changes} if id==&response && changes.iter().any(|c|matches!(c,discard_core::memory_items::MemoryChange::Saved{item,..} if item.version==1)))));
    rig.close().await;
}

#[tokio::test]
async fn explicit_mention_routes_one_reply_and_persists_its_author() {
    let mut rig = Rig::new(vec![Step::idle()]).await;
    rig.task.abort();
    let _ = (&mut rig.task).await;
    let (channel, jordan) = {
        let store = rig.store.lock().unwrap();
        let project = store.import_project_folder(rig._dir.path()).unwrap();
        store.ensure_project_general_channel(&project.id, &rig.agent).unwrap();
        let channel = store.create_conversation(&rig.agent, Some(&project.id), "Existing worktree").unwrap();
        let mut config = store.load_agent(&rig.agent).unwrap();
        config.name = "Jordan Lee".into();
        config.model = Some("jordan-model".into());
        config.system_prompt = "Jordan's review instructions".into();
        let jordan = store.create_agent(&config).unwrap();
        store.save_channel_topic(&channel.id, 0, "@Unknown must not dispatch", discard_core::model::TopicAuthorKind::User, None).unwrap();
        (channel, jordan)
    };
    rig.memory.create_item(&Scope::Agent(jordan.clone()), ItemKind::Instruction, "Jordan private instruction", Some(&channel.id), MutationOrigin::User).unwrap();
    rig.memory.create_item(&Scope::Agent(rig.agent.clone()), ItemKind::Instruction, "Default private instruction", Some(&channel.id), MutationOrigin::User).unwrap();
    let provider = Arc::new(FakeProvider::new(vec![Step::text("answer from selected agent"), Step::idle()]));
    let probe = provider.probe.clone();
    let session = rig.store.lock().unwrap().latest_session(&channel.id).unwrap().unwrap();
    let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
        reasoning_effort: None, mode: SessionMode::Conversation, providers: vec![provider.clone()],
        store: rig.store.clone(), agent_id: rig.agent.clone(), project_id: channel.project_id.clone(),
        memory: Some(rig.runtime.clone()), plugin_write: false, workspace_mode: WorkspaceMode::MainCheckout,
        workspace: None, observer: None, channel_name: channel.name.clone(), provider,
        channel_id: channel.id.clone(), session_id: session, cwd: rig._dir.path().into(),
        model: None, system_prompt: None, policy: ApprovalPolicy::Ask,
    };
    let (commands, rx) = mpsc::unbounded_channel();
    let (tx, updates) = mpsc::unbounded_channel();
    rig.commands = commands;
    rig.updates = updates;
    rig.task = tokio::spawn(engine.run(rx, tx));
    rig.commands.send(Command::Send("@\"Jordan Lee\" review this worktree".into())).unwrap();
    let first = rig.done().await;
    assert!(first.iter().any(|u| matches!(u, Update::Responding(author) if author.agent_id == jordan && author.name == "Jordan Lee")));
    {
        let store = rig.store.lock().unwrap();
        assert!(store.project_agents(channel.project_id.as_ref().unwrap()).unwrap().contains(&jordan));
        assert!(store.channel_agents(&channel.id).unwrap().contains(&jordan));
        assert_eq!(store.load_channel(&channel.id).unwrap().agent_id, rig.agent);
    }
    // Mentions in history, topics, quotes and code must not select the next responder.
    rig.commands.send(Command::Send("Follow up\n> @Unknown\n`@Unknown`".into())).unwrap();
    let second = rig.done().await;
    assert!(second.iter().any(|u| matches!(u, Update::Responding(author) if author.agent_id == rig.agent)));
    {
        let specs = probe.specs.lock().unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].model.as_deref(), Some("jordan-model"));
        assert_eq!(specs[1].model.as_deref(), Some("one"));
        let first = specs[0].system_prompt.as_deref().unwrap();
        assert!(first.contains("Jordan's review instructions"));
        assert!(first.contains("Jordan private instruction"));
        assert!(!first.contains("Default private instruction"));
        let second = specs[1].system_prompt.as_deref().unwrap();
        assert!(second.contains("Default private instruction"));
        assert!(!second.contains("Jordan private instruction"));
    }
    assert!(probe.sent.lock().unwrap()[1].contains("\"name\":\"Jordan Lee\""));
    // Read from a second connection to prove attribution survives runtime disposal.
    let reopened = Store::open(&rig._dir.path().join("store.db")).unwrap();
    let authors = reopened.message_authors(&channel.id).unwrap();
    let history = reopened.messages_for_channel(&channel.id).unwrap();
    let replies: Vec<_> = history.iter().filter(|m| m.role == Role::Agent).collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(authors[&replies[0].id].agent_id, jordan);
    assert_eq!(authors[&replies[1].id].agent_id, rig.agent);
    // The project default must also change an existing worktree/branch channel's next reply.
    // Keep its saved agent stale to reproduce channels created before a default change.
    {
        let store = rig.store.lock().unwrap();
        store.set_project_main_agent(channel.project_id.as_ref().unwrap(), &jordan).unwrap();
        assert_eq!(store.load_channel(&channel.id).unwrap().agent_id, rig.agent);
    }
    rig.commands.send(Command::Send("Use the new project default".into())).unwrap();
    let third = rig.done().await;
    assert!(third.iter().any(|u| matches!(u, Update::Responding(author) if author.agent_id == jordan)));
    assert_eq!(probe.specs.lock().unwrap()[2].model.as_deref(), Some("jordan-model"));
    rig.close().await;
}

#[tokio::test]
async fn invalid_mentions_fail_before_saving_or_calling_a_provider() {
    let mut rig = Rig::new(vec![Step::text("must not run"), Step::idle()]).await;
    {
        let store = rig.store.lock().unwrap();
        let mut config = store.load_agent(&rig.agent).unwrap();
        config.name = "Jordan".into();
        store.create_agent(&config).unwrap();
        config.name = "Duplicate".into();
        store.create_agent(&config).unwrap();
        store.create_agent(&config).unwrap();
    }
    for (message, reason) in [("@Nobody hello", "No active"), ("@A @Jordan hello", "one agent"), ("@Duplicate hello", "More than one"), ("@Jordan hello", "project channel")] {
        rig.commands.send(Command::Send(message.into())).unwrap();
        let updates = rig.done().await;
        assert!(updates.iter().any(|u| matches!(u, Update::Error(error) if error.contains(reason))), "{updates:?}");
    }
    assert!(rig.probe.specs.lock().unwrap().is_empty());
    assert!(rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap().is_empty());
    rig.close().await;
}

#[tokio::test]
async fn provider_notice_is_saved_once_across_two_turns() {
    let notice = "Codex 0.167.0 is newer than the tested version 0.166.0. Discard verified the isolation controls before this response.";
    let provider = Arc::new(FakeProvider::new(vec![Step::text("answer"), Step::idle()]).with_notice(notice));
    let probe = provider.probe.clone();
    let mut rig = Rig::with_provider(provider, probe).await;
    let mut markers = Vec::new();
    for text in ["first turn", "second turn"] {
        rig.commands.send(Command::Send(text.into())).unwrap();
        let updates = rig.done().await;
        assert!(updates.iter().any(|u| matches!(u, Update::TurnFinished(discard_core::driver::TurnOutcome::Completed))));
        markers.extend(updates.into_iter().filter_map(|u| match u { Update::ConversationMarker(m) => Some(m), _ => None }));
    }
    // Marker storage runs independently of the turn. Wait only in the test.
    if markers.is_empty() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(update) = rig.updates.recv().await {
                if let Update::ConversationMarker(message) = update { markers.push(message); break; }
            }
        }).await.unwrap();
    }
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].role, Role::System);
    assert_eq!(markers[0].blocks, vec![Block::Markdown { text: notice.into() }]);
    let messages = rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap();
    assert_eq!(messages.iter().filter(|m| m.role == Role::System && m.blocks == markers[0].blocks).count(), 1);
    rig.close().await;
}

// Native provider resume (engine side).

/// A fake that answers each session with the next text, and reports the handle `h1`.
fn replies(texts: &[&str]) -> FakeProvider {
    let mut texts = texts.iter();
    let mut provider = FakeProvider::new(vec![Step::text(texts.next().unwrap()), Step::idle()]);
    for text in texts { provider = provider.then_session(vec![Step::text(text), Step::idle()]); }
    provider.with_resume_handle("h1")
}

/// A rig whose agent has `native_resume` on.
async fn native_rig(provider: FakeProvider) -> Rig {
    let provider = Arc::new(provider);
    let probe = provider.probe.clone();
    let rig = Rig::with_provider(provider, probe).await;
    rig.edit(|c| c.native_resume = true);
    rig
}

impl Rig {
    async fn say(&mut self, text: &str) -> Vec<Update> {
        self.commands.send(Command::Send(text.into())).unwrap();
        let updates = self.done().await;
        assert!(updates.iter().any(|u| matches!(u, Update::TurnFinished(discard_core::driver::TurnOutcome::Completed))), "{updates:?}");
        updates
    }
    fn row(&self, agent: &AgentId) -> Option<discard_core::store::AgentSession> {
        self.store.lock().unwrap().agent_session(&self.channel, agent).unwrap()
    }
    fn sent(&self, turn: usize) -> String {
        self.probe.sent.lock().unwrap()[turn].clone()
    }
    fn started_with(&self) -> Vec<Option<String>> {
        self.probe.started_with.lock().unwrap().clone()
    }
    fn started_resumable(&self) -> Vec<bool> {
        self.probe.started_resumable.lock().unwrap().clone()
    }
    /// Run a new engine on `channel` with these providers, keeping the store and the memory runtime.
    async fn respawn(&mut self, providers: Vec<Arc<dyn discard_core::provider::Provider>>, probe: Probe, channel: ChannelId) {
        self.task.abort();
        let _ = (&mut self.task).await;
        let (project, session) = {
            let store = self.store.lock().unwrap();
            (store.load_channel(&channel).unwrap().project_id, store.latest_session(&channel).unwrap().unwrap())
        };
        let engine = Engine {
            main_checkout_grants: Arc::default(), workspace_locks: Arc::default(),
            write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
            reasoning_effort: None, mode: SessionMode::Conversation, provider: providers[0].clone(), providers,
            store: self.store.clone(), agent_id: self.agent.clone(), project_id: project,
            memory: Some(self.runtime.clone()), plugin_write: false, workspace_mode: WorkspaceMode::MainCheckout,
            workspace: None, observer: None, channel_name: "talk".into(),
            channel_id: channel.clone(), session_id: session, cwd: self._dir.path().join("empty"),
            model: None, system_prompt: None, policy: ApprovalPolicy::Ask,
        };
        let (commands, rx) = mpsc::unbounded_channel();
        let (tx, updates) = mpsc::unbounded_channel();
        self.commands = commands;
        self.updates = updates;
        self.channel = channel;
        self.probe = probe;
        self.task = tokio::spawn(engine.run(rx, tx));
    }
}

fn notices(updates: &[Update]) -> Vec<String> {
    updates.iter().filter_map(|u| match u { Update::Notice(text) => Some(text.clone()), _ => None }).collect()
}

fn resume_states(updates: &[Update]) -> Vec<discard_core::ui_state::ResumeState> {
    updates.iter().filter_map(|u| match u { Update::ResumeState(state) => Some(state.clone()), _ => None }).collect()
}

#[tokio::test]
async fn the_engine_reports_resumed_and_replay_states() {
    use discard_core::ui_state::ResumeState;
    let mut rig = native_rig(replies(&["answer one", "answer two", "answer three"])).await;
    // A first session is fresh: no state.
    let first = rig.say("first request").await;
    assert_eq!(resume_states(&first), vec![]);
    let second = rig.say("second request").await;
    assert_eq!(resume_states(&second), vec![ResumeState::Resumed]);
    rig.edit(|c| c.model = Some("other-model".into()));
    let third = rig.say("third request").await;
    let reasons = resume_states(&third);
    assert_eq!(reasons.len(), 1, "{third:?}");
    let ResumeState::Replay { reason } = &reasons[0] else { panic!("{reasons:?}") };
    assert!(!reason.starts_with("Fresh replay: ") && reason.contains("changed"), "{reason}");
    assert!(notices(&third).contains(&format!("Fresh replay: {reason}")), "{third:?}");
    rig.close().await;
}

#[tokio::test]
async fn a_non_native_agent_reports_no_resume_state() {
    let mut rig = Rig::with_provider(Arc::new(replies(&["answer one", "answer two"])), Probe::default()).await;
    rig.say("first request").await;
    let second = rig.say("second request").await;
    assert_eq!(resume_states(&second), vec![]);
    rig.close().await;
}

#[tokio::test]
async fn a_non_native_agent_still_replays_every_turn() {
    let mut rig = Rig::with_provider(Arc::new(replies(&["answer one", "answer two"])), Probe::default()).await;
    let provider = Arc::new(replies(&["answer one", "answer two"]));
    let probe = provider.probe.clone();
    let channel = rig.channel.clone();
    rig.respawn(vec![provider], probe, channel).await;
    rig.say("first request").await;
    rig.say("second request").await;
    assert_eq!(rig.started_with(), vec![None, None]);
    assert_eq!(rig.started_resumable(), vec![false, false]);
    assert!(rig.sent(1).contains("first request") && rig.sent(1).contains("answer one"));
    assert!(rig.row(&rig.agent.clone()).is_none());
    rig.close().await;
}

#[tokio::test]
async fn second_response_resumes_and_sends_only_new_messages() {
    let mut rig = native_rig(replies(&["answer one", "answer two", "answer three"])).await;
    rig.say("first request").await;
    rig.say("second request").await;
    rig.say("third request").await;
    assert_eq!(rig.started_with(), vec![None, Some("h1".into()), Some("h1".into())]);
    assert_eq!(rig.started_resumable(), vec![true, true, true]);
    assert!(rig.sent(0).contains("first request"));
    let second = rig.sent(1);
    assert!(second.contains("second request"), "{second}");
    assert!(!second.contains("first request") && !second.contains("answer one"), "{second}");
    let third = rig.sent(2);
    assert!(third.contains("third request") && !third.contains("second request") && !third.contains("answer two"), "{third}");
    let specs = rig.probe.specs.lock().unwrap().clone();
    let sessions = rig._dir.path().join("sessions");
    assert!(specs.iter().all(|s| s.cwd.starts_with(&sessions) && s.cwd == specs[0].cwd), "{:?}", specs[0].cwd);
    assert!(specs.iter().all(|s| s.system_prompt == specs[0].system_prompt));
    let row = rig.row(&rig.agent.clone()).expect("a saved session");
    assert_eq!(row.handle, "h1");
    let messages = rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap();
    let last = messages.iter().rfind(|m| m.role == Role::Agent).unwrap();
    assert_eq!(row.delivered_through.as_ref().map(|id| id.0.split(":part:").next().unwrap().to_owned()), Some(last.id.0.clone()));
    assert!(row.bytes_in_session > 0 && row.dropped_reason.is_none());
    rig.close().await;
}

#[tokio::test]
async fn a_changed_non_mandatory_memory_item_goes_in_the_context_update_and_keeps_the_handle() {
    let mut rig = native_rig(replies(&["one", "two", "three"])).await;
    let fact = rig.memory.create_item(&Scope::Agent(rig.agent.clone()), ItemKind::Fact, "The Rust compiler is rustc 1.90", None, MutationOrigin::User).unwrap();
    rig.say("Which Rust compiler?").await;
    let system = rig.probe.specs.lock().unwrap()[0].system_prompt.clone().unwrap();
    assert!(!system.contains("rustc 1.90"), "selected facts stay out of the fixed system prompt");
    assert!(rig.sent(0).contains("<context-update>") && rig.sent(0).contains("rustc 1.90"));
    rig.memory.update_item(&fact.id, fact.version, "The Rust compiler is rustc 1.91", MutationOrigin::User).unwrap();
    rig.say("And the Rust version now?").await;
    assert_eq!(rig.started_with()[1].as_deref(), Some("h1"));
    assert!(rig.sent(1).contains("<context-update>") && rig.sent(1).contains("rustc 1.91"), "{}", rig.sent(1));
    rig.say("Rust once more").await;
    assert_eq!(rig.started_with()[2].as_deref(), Some("h1"));
    assert!(!rig.sent(2).contains("<context-update>"), "{}", rig.sent(2));
    let row = rig.row(&rig.agent.clone()).unwrap();
    assert_eq!(row.delivered_memory.get(&fact.id), Some(&(fact.version + 1)));
    rig.close().await;
}

#[tokio::test]
async fn a_changed_instruction_item_replays_with_a_notice() {
    let mut rig = native_rig(replies(&["one", "two", "three", "four"])).await;
    let rule = rig.memory.create_item(&Scope::Agent(rig.agent.clone()), ItemKind::Instruction, "Keep decisions explicit", None, MutationOrigin::User).unwrap();
    rig.say("first request").await;
    rig.say("second request").await;
    rig.memory.update_item(&rule.id, rule.version, "Keep every decision explicit", MutationOrigin::User).unwrap();
    let updates = rig.say("third request").await;
    assert_eq!(rig.started_with(), vec![None, Some("h1".into()), None]);
    assert!(notices(&updates).iter().any(|n| n == "Fresh replay: the instructions changed. Discard sends the history again."), "{updates:?}");
    assert!(updates.iter().any(|u| matches!(u, Update::ConversationMarker(m) if matches!(&m.blocks[0], Block::Markdown { text } if text.starts_with("Fresh replay:")))));
    let third = rig.sent(2);
    assert!(third.contains("first request") && third.contains("two"), "a replay sends the whole history");
    assert!(rig.probe.specs.lock().unwrap()[2].system_prompt.as_deref().unwrap().contains("Keep every decision explicit"));
    rig.say("fourth request").await;
    assert_eq!(rig.started_with()[3].as_deref(), Some("h1"));
    rig.close().await;
}

#[tokio::test]
async fn topic_and_capture_pause_changes_are_delivered_as_updates() {
    let mut rig = native_rig(replies(&["one", "two", "three", "four"])).await;
    rig.store.lock().unwrap().save_channel_topic(&rig.channel, 0, "Old topic", discard_core::model::TopicAuthorKind::User, None).unwrap();
    rig.say("first request").await;
    assert!(rig.sent(0).contains("Old topic"));
    rig.say("second request").await;
    assert!(!rig.sent(1).contains("Old topic") && !rig.sent(1).contains("<channel-topic>"), "{}", rig.sent(1));
    rig.store.lock().unwrap().save_channel_topic(&rig.channel, 1, "Fresh topic", discard_core::model::TopicAuthorKind::User, None).unwrap();
    let fact = rig.memory.create_item(&Scope::Agent(rig.agent.clone()), ItemKind::Fact, "unrelated", None, MutationOrigin::User).unwrap();
    rig.memory.forget_item_in(&fact.id, fact.version, Some(&rig.channel)).unwrap();
    rig.say("third request").await;
    let third = rig.sent(2);
    assert!(third.contains("Fresh topic") && third.contains("<memory-capture>Automatic capture is paused for this conversation. Do not save or propose memory. Only the user can resume capture in the app.</memory-capture>"), "{third}");
    rig.say("fourth request").await;
    let fourth = rig.sent(3);
    assert!(!fourth.contains("Fresh topic") && !fourth.contains("<memory-capture>"), "{fourth}");
    assert_eq!(rig.started_with(), vec![None, Some("h1".into()), Some("h1".into()), Some("h1".into())]);
    let row = rig.row(&rig.agent.clone()).unwrap();
    assert!(row.delivered_capture_paused);
    assert_eq!(row.delivered_topic_version, Some(2));
    rig.close().await;
}

/// A project channel with a second native agent named Bee. Returns Bee's id.
async fn two_agent_rig(texts: &[&str]) -> (Rig, AgentId) {
    let mut rig = native_rig(replies(&["unused"])).await;
    let project_dir = rig._dir.path().join("project");
    std::fs::create_dir(&project_dir).unwrap();
    std::fs::write(project_dir.join("README.md"), "A small project.").unwrap();
    let (channel, bee) = {
        let store = rig.store.lock().unwrap();
        let project = store.import_project_folder(&project_dir).unwrap();
        store.ensure_project_general_channel(&project.id, &rig.agent).unwrap();
        let channel = store.create_conversation(&rig.agent, Some(&project.id), "talk").unwrap();
        let mut config = store.load_agent(&rig.agent).unwrap();
        config.name = "Bee".into();
        let bee = store.create_agent(&config).unwrap();
        (channel, bee)
    };
    // Each fresh session gets its own handle; a resumed session keeps the handle it was started with.
    let provider = Arc::new(replies(texts).with_fresh_handles(&["h1", "h2"]));
    let probe = provider.probe.clone();
    rig.respawn(vec![provider], probe, channel.id).await;
    (rig, bee)
}

#[tokio::test]
async fn two_agents_in_one_channel_keep_separate_rows() {
    let (mut rig, bee) = two_agent_rig(&["reply a1", "reply b1", "reply a2", "reply b2"]).await;
    rig.say("hello there").await;
    rig.say("@Bee what do you think").await;
    rig.say("and again").await;
    rig.say("@Bee last word").await;
    // Each agent resumes only its own session: the first agent got h1, Bee got h2.
    assert_eq!(rig.started_with(), vec![None, None, Some("h1".into()), Some("h2".into())]);
    let a = rig.row(&rig.agent.clone()).expect("a row for the first agent");
    let b = rig.row(&bee).expect("a row for Bee");
    assert_eq!((a.handle.as_str(), b.handle.as_str()), ("h1", "h2"));
    assert_ne!(a.delivered_through, b.delivered_through);
    rig.close().await;
}

#[tokio::test]
async fn agent_a_delta_includes_agent_b_reply_but_not_its_own() {
    let (mut rig, _bee) = two_agent_rig(&["reply a1", "reply b1", "reply a2", "reply b2"]).await;
    rig.say("hello there").await;
    rig.say("@Bee what do you think").await;
    rig.say("and again").await;
    let a2 = rig.sent(2);
    assert!(a2.contains("what do you think") && a2.contains("reply b1"), "{a2}");
    assert!(!a2.contains("reply a1") && !a2.contains("hello there"), "{a2}");
    rig.say("@Bee last word").await;
    let b2 = rig.sent(3);
    assert!(b2.contains("and again") && b2.contains("reply a2"), "{b2}");
    assert!(!b2.contains("reply b1") && !b2.contains("what do you think"), "{b2}");
    rig.close().await;
}

#[tokio::test]
async fn a_message_posted_during_the_turn_is_delivered_next_turn() {
    let provider = FakeProvider::new(vec![Step::Approval { tool: "Bash".into(), input: serde_json::json!({"command": "ls"}) }, Step::text("answer one"), Step::idle()])
        .then_session(vec![Step::text("answer two"), Step::idle()])
        .with_resume_handle("h1");
    let mut rig = native_rig(provider).await;
    rig.commands.send(Command::Send("first request".into())).unwrap();
    let approval = loop {
        if let Some(Update::Event(event)) = rig.updates.recv().await
            && let discard_core::event::EventKind::ApprovalRequest { approval_id, .. } = &event.kind { break approval_id.clone(); }
    };
    rig.store.lock().unwrap().insert_message(&Message { id: MessageId::new(), channel_id: rig.channel.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "posted mid-turn".into() }] }).unwrap();
    rig.commands.send(Command::Decide { approval_id: approval, decision: discard_core::approval::Decision::Approve, rule_cut: None }).unwrap();
    let updates = rig.done().await;
    assert!(updates.iter().any(|u| matches!(u, Update::TurnFinished(discard_core::driver::TurnOutcome::Completed))), "{updates:?}");
    rig.say("second request").await;
    assert_eq!(rig.started_with(), vec![None, Some("h1".into())]);
    let second = rig.sent(1);
    assert!(second.contains("posted mid-turn") && second.contains("second request"), "{second}");
    assert!(!second.contains("first request"), "{second}");
    rig.close().await;
}

#[tokio::test]
async fn deleting_an_earlier_message_forces_a_replay() {
    let mut rig = native_rig(replies(&["one", "two", "three"])).await;
    rig.say("first request").await;
    rig.say("second request").await;
    let first = rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap()[0].id.clone();
    let connection = rusqlite::Connection::open(rig._dir.path().join("store.db")).unwrap();
    connection.execute("DELETE FROM messages WHERE id=?1", [&first.0]).unwrap();
    let updates = rig.say("third request").await;
    assert_eq!(rig.started_with(), vec![None, Some("h1".into()), None]);
    assert!(notices(&updates).iter().any(|n| n.contains("earlier messages changed")), "{updates:?}");
    let third = rig.sent(2);
    assert!(third.contains("second request") && !third.contains("first request"), "{third}");
    rig.close().await;
}

#[tokio::test]
async fn clearing_the_context_starts_a_new_provider_session_with_no_earlier_messages() {
    let mut rig = native_rig(replies(&["alpha-reply", "beta-reply", "gamma-reply", "delta-reply"])).await;
    let agent = rig.agent.clone();
    rig.say("first request").await;
    rig.say("second request").await;
    assert!(rig.row(&agent).is_some());
    rig.store.lock().unwrap().reset_channel_context(&rig.channel).unwrap();
    assert!(rig.row(&agent).is_none(), "the session record is gone");
    rig.say("third request").await;
    assert_eq!(rig.started_with(), vec![None, Some("h1".into()), None], "the third turn starts a new session");
    let third = rig.sent(2);
    assert!(third.contains("third request"), "{third}");
    assert!(!third.contains("first request") && !third.contains("second request") && !third.contains("alpha-reply"), "{third}");
    assert!(rig.row(&agent).is_some(), "the new session is saved");
    rig.say("fourth request").await;
    assert_eq!(rig.started_with().last(), Some(&Some("h1".into())), "the new session resumes");
    let fourth = rig.sent(3);
    assert!(fourth.contains("fourth request") && !fourth.contains("first request"), "{fourth}");
    rig.close().await;
}

#[tokio::test]
async fn clearing_the_context_cuts_the_replay_of_an_agent_without_a_saved_session() {
    let provider = Arc::new(replies(&["alpha-reply", "beta-reply"]));
    let probe = provider.probe.clone();
    let mut rig = Rig::with_provider(provider, probe).await;
    rig.say("first request").await;
    rig.store.lock().unwrap().reset_channel_context(&rig.channel).unwrap();
    rig.say("second request").await;
    let sent = rig.sent(1);
    assert!(sent.contains("second request") && !sent.contains("first request") && !sent.contains("alpha-reply"), "{sent}");
    rig.close().await;
}

#[tokio::test]
async fn changing_model_provider_or_turning_the_setting_off_leaves_no_row() {
    let mut rig = native_rig(replies(&["one", "two", "three", "four", "five", "six"])).await;
    let agent = rig.agent.clone();
    rig.say("first request").await;
    assert!(rig.row(&agent).is_some());
    rig.edit(|c| c.model = Some("two".into()));
    let updates = rig.say("second request").await;
    assert!(notices(&updates).iter().any(|n| n == "Fresh replay: the model changed. Discard sends the history again."), "{updates:?}");
    assert!(rig.row(&agent).unwrap().fingerprint.contains("\"model\":\"two\""));
    rig.say("third request").await;
    assert_eq!(rig.started_with(), vec![None, None, Some("h1".into())]);
    // The provider changes: the old handle is never offered to the new provider.
    let fake = Arc::new(replies(&["unused"]));
    let other = Arc::new(NamedProvider { name: "other", inner: replies(&["four", "five"]) });
    let probe = other.inner.probe.clone();
    let channel = rig.channel.clone();
    rig.respawn(vec![fake, other], probe, channel).await;
    rig.edit(|c| c.provider = "other".into());
    let updates = rig.say("fourth request").await;
    assert_eq!(rig.started_with(), vec![None]);
    assert!(notices(&updates).iter().any(|n| n == "Fresh replay: the provider changed. Discard sends the history again."), "{updates:?}");
    assert!(rig.row(&agent).unwrap().fingerprint.contains("\"provider\":\"other\""));
    rig.edit(|c| c.native_resume = false);
    rig.say("fifth request").await;
    assert_eq!(rig.started_with(), vec![None, None]);
    assert_eq!(rig.started_resumable(), vec![true, false]);
    assert!(rig.row(&agent).is_none());
    rig.close().await;
}

#[tokio::test]
async fn a_thread_turn_and_a_summary_never_resume() {
    // Summary: the summary session never resumes. The response session starts fresh and resumable.
    let mut rig = native_rig(replies(&["A semantic summary.", "answer"])).await;
    for i in 0..messages_over_budget(1) {
        rig.store.lock().unwrap().insert_message(&Message { id: MessageId::new(), channel_id: rig.channel.clone(), role: Role::User, blocks: vec![Block::Markdown { text: format!("older {i} {}", "x".repeat(4000)) }] }).unwrap();
    }
    rig.say("current request").await;
    assert_eq!(rig.started_with(), vec![None, None]);
    assert_eq!(rig.started_resumable(), vec![false, true]);
    // Thread: a thread turn never resumes and saves no row.
    let anchor = rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap().last().unwrap().id.clone();
    let thread = rig.store.lock().unwrap().create_thread(&rig.channel, &anchor, "t", &rig.agent).unwrap();
    let provider = Arc::new(replies(&["thread one", "thread two"]));
    let probe = provider.probe.clone();
    rig.respawn(vec![provider], probe, thread.id.clone()).await;
    rig.say("side question").await;
    rig.say("another side question").await;
    // The thread replays the long parent prefix, so it summarizes too.
    assert!(rig.started_with().len() >= 2 && rig.started_with().iter().all(Option::is_none));
    assert!(rig.started_resumable().iter().all(|resumable| !resumable));
    assert!(rig.row(&rig.agent.clone()).is_none());
    rig.close().await;
}

// Native provider resume: summary headroom.

/// 20 messages of 4000 bytes: the first summary batch leaves about 36 KB, which fits the budget but not half of it.
fn insert_twenty_long_messages(rig: &Rig) {
    for i in 0..20 {
        rig.store.lock().unwrap().insert_message(&Message { id: MessageId::new(), channel_id: rig.channel.clone(), role: Role::User, blocks: vec![Block::Markdown { text: format!("older {i} {}", "x".repeat(4000)) }] }).unwrap();
    }
}

#[tokio::test]
async fn a_channel_that_overflowed_once_resumes_on_the_next_turn() {
    let mut rig = native_rig(replies(&["summary one", "summary two", "answer one", "answer two"])).await;
    insert_twenty_long_messages(&rig);
    rig.say("current request").await;
    // Two summary sessions, then the response session.
    assert_eq!(rig.started_with(), vec![None, None, None]);
    assert_eq!(rig.started_resumable(), vec![false, false, true]);
    let updates = rig.say("second request").await;
    assert_eq!(rig.started_with().last().unwrap().as_deref(), Some("h1"));
    assert_eq!(rig.started_resumable().last(), Some(&true));
    assert!(!notices(&updates).iter().any(|n| n.starts_with("Fresh replay")), "{updates:?}");
    assert!(!rig.sent(3).contains("current request"), "{}", rig.sent(3));
    rig.close().await;
}

#[tokio::test]
async fn a_non_resumable_agent_keeps_todays_summary_target() {
    let provider = Arc::new(replies(&["summary one", "answer one"]));
    let probe = provider.probe.clone();
    let rig = Rig::with_provider(provider, probe).await;
    insert_twenty_long_messages(&rig);
    let mut rig = rig;
    rig.say("current request").await;
    // One summary session only: the input fits the full budget after the first batch.
    assert_eq!(rig.started_with().len(), 2);
    assert_eq!(rig.started_resumable(), vec![false, false]);
    assert!(rig.sent(1).contains("summary one"));
    assert!(rig.sent(1).len() > discard_core::conversation_context::input_budget(None) / 2, "{}", rig.sent(1).len());
    rig.close().await;
}

// Native provider resume: drop rules.

const UNSAFE_CANCELLED: &str = "the previous response was cancelled";
const UNSAFE_FAILED: &str = "the previous response failed";
const UNSAFE_NOT_SAVED: &str = "the previous response was not saved";
const REJECTED_NOTICE: &str = "Fresh replay: the provider rejected the saved session. Discard sends the history again.";

fn unsafe_notice(why: &str) -> String {
    format!("Fresh replay: the last turn did not finish cleanly ({why}). Discard sends the history again.")
}

fn finished(updates: &[Update]) -> Option<discard_core::driver::TurnOutcome> {
    updates.iter().find_map(|u| match u { Update::TurnFinished(outcome) => Some(*outcome), _ => None })
}

fn errors(updates: &[Update]) -> Vec<String> {
    updates.iter().filter_map(|u| match u { Update::Error(text) => Some(text.clone()), _ => None }).collect()
}

/// After a drop, the next response replays the whole history with the notice, and saves a valid row.
async fn assert_next_response_replays(rig: &mut Rig, notice: &str) {
    let updates = rig.say("after the drop").await;
    let last = rig.started_with().len() - 1;
    assert_eq!(rig.started_with()[last], None, "a dropped handle is never offered");
    assert!(notices(&updates).iter().any(|n| n == notice), "{updates:?}");
    let input = rig.probe.sent.lock().unwrap().last().unwrap().clone();
    assert!(input.contains("first request") && input.contains("after the drop"), "{input}");
    let row = rig.row(&rig.agent.clone()).unwrap();
    assert_eq!(row.dropped_reason, None);
}

#[tokio::test]
async fn a_cancelled_turn_drops_the_handle_and_the_next_response_replays_with_a_notice() {
    let provider = FakeProvider::new(vec![Step::text("one"), Step::idle()])
        .then_session(vec![Step::text("partial"), Step::Hang])
        .then_session(vec![Step::text("three"), Step::idle()])
        .with_resume_handle("h1");
    let mut rig = native_rig(provider).await;
    rig.say("first request").await;
    rig.commands.send(Command::Send("second request".into())).unwrap();
    loop {
        if let Some(Update::Event(event)) = rig.updates.recv().await
            && matches!(&event.kind, discard_core::event::EventKind::TextDelta { .. }) { break; }
    }
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.done().await;
    assert_eq!(finished(&updates), Some(discard_core::driver::TurnOutcome::Cancelled));
    assert_eq!(rig.started_with(), vec![None, Some("h1".into())]);
    let row = rig.row(&rig.agent.clone()).unwrap();
    assert_eq!(row.handle, "h1", "a dropped row keeps its handle");
    assert_eq!(row.dropped_reason.as_deref(), Some(UNSAFE_CANCELLED));
    assert_next_response_replays(&mut rig, &unsafe_notice(UNSAFE_CANCELLED)).await;
    rig.close().await;
}

#[tokio::test]
async fn a_failed_turn_drops_the_handle() {
    let provider = FakeProvider::new(vec![Step::text("one"), Step::idle()])
        .then_session(vec![Step::text("partial"), Step::Crash])
        .then_session(vec![Step::text("three"), Step::idle()])
        .with_resume_handle("h1");
    let mut rig = native_rig(provider).await;
    rig.say("first request").await;
    rig.commands.send(Command::Send("second request".into())).unwrap();
    let updates = rig.done().await;
    assert_eq!(finished(&updates), Some(discard_core::driver::TurnOutcome::Failed));
    assert_eq!(rig.row(&rig.agent.clone()).unwrap().dropped_reason.as_deref(), Some(UNSAFE_FAILED));
    assert_next_response_replays(&mut rig, &unsafe_notice(UNSAFE_FAILED)).await;
    assert_eq!(rig.started_with(), vec![None, Some("h1".into()), None]);
    rig.close().await;
}

#[tokio::test]
async fn a_response_that_fails_to_save_drops_the_handle() {
    let mut rig = native_rig(replies(&["one", "two", "three"])).await;
    rig.say("first request").await;
    let connection = rusqlite::Connection::open(rig._dir.path().join("store.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_agent_message BEFORE INSERT ON messages WHEN NEW.role='agent' BEGIN SELECT RAISE(ABORT, 'test disk write failed'); END;").unwrap();
    rig.commands.send(Command::Send("second request".into())).unwrap();
    let updates = rig.done().await;
    assert!(updates.iter().any(|u| matches!(u, Update::ResponseSaveFailed(_))), "{updates:?}");
    assert_eq!(rig.row(&rig.agent.clone()).unwrap().dropped_reason.as_deref(), Some(UNSAFE_NOT_SAVED));
    connection.execute_batch("DROP TRIGGER reject_agent_message;").unwrap();
    assert_next_response_replays(&mut rig, &unsafe_notice(UNSAFE_NOT_SAVED)).await;
    rig.close().await;
}

#[tokio::test]
async fn a_rejected_handle_replays_once_in_the_same_response() {
    // Codex shape: the start fails. Claude shape: the start works and the first message fails.
    for first_send in [false, true] {
        let provider = if first_send {
            FakeProvider::new(vec![Step::text("one"), Step::idle()])
                .then_session(vec![Step::text("from the rejected session"), Step::idle()])
                .then_session(vec![Step::text("two"), Step::idle()])
                .with_rejecting_resume_on_first_send()
        } else {
            FakeProvider::new(vec![Step::text("one"), Step::idle()])
                .then_session(vec![Step::text("two"), Step::idle()])
                .with_rejecting_resume()
        };
        let mut rig = native_rig(provider.with_resume_handle("h1")).await;
        rig.say("first request").await;
        let updates = rig.say("second request").await;
        assert_eq!(rig.started_with(), vec![None, Some("h1".into()), None], "first_send={first_send}");
        assert_eq!(rig.started_resumable(), vec![true, true, true]);
        assert_eq!(notices(&updates).iter().filter(|n| *n == REJECTED_NOTICE).count(), 1, "{updates:?}");
        assert!(errors(&updates).is_empty(), "{updates:?}");
        // The last state wins: a rejected handle ends as a replay with its reason, never as a stale `Resumed`.
        assert_eq!(resume_states(&updates).last(), Some(&discard_core::ui_state::ResumeState::Replay { reason: "the provider rejected the saved session. Discard sends the history again.".into() }), "first_send={first_send} {updates:?}");
        assert!(updates.iter().any(|u| matches!(u, Update::ConversationMarker(m) if matches!(&m.blocks[0], Block::Markdown { text } if text == REJECTED_NOTICE))));
        let sent = rig.probe.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), if first_send { 3 } else { 2 });
        if first_send { assert!(!sent[1].contains("first request") && sent[1].contains("second request"), "the rejected session got the delta"); }
        let replay = sent.last().unwrap();
        assert!(replay.contains("first request") && replay.contains("one") && replay.contains("second request"), "{replay}");
        assert_eq!(replay.matches("second request").count(), 1, "{replay}");
        // The rejected session shows nothing.
        let texts: Vec<String> = updates.iter().filter_map(|u| match u { Update::Event(e) => match &e.kind { discard_core::event::EventKind::TextDelta { text } => Some(text.clone()), _ => None }, _ => None }).collect();
        assert_eq!(texts, vec!["two".to_string()]);
        let messages = rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap();
        assert!(!messages.iter().any(|m| m.blocks.iter().any(|b| matches!(b, Block::Markdown { text } if text.contains("rejected session") && !text.starts_with("Fresh replay")))));
        let row = rig.row(&rig.agent.clone()).unwrap();
        assert_eq!((row.handle.as_str(), row.dropped_reason.as_deref()), ("h1", None));
        let last = messages.iter().rfind(|m| m.role == Role::Agent).unwrap();
        assert_eq!(row.delivered_through.as_ref().map(|id| id.0.split(":part:").next().unwrap().to_owned()), Some(last.id.0.clone()));
        rig.close().await;
    }
}

#[tokio::test]
async fn a_rejected_handle_whose_replay_also_fails_ends_the_turn() {
    for first_send in [false, true] {
        let provider = FakeProvider::new(vec![Step::text("one"), Step::idle()]).with_resume_handle("h1").with_failed_start(2, "the replay broke");
        let provider = if first_send { provider.with_rejecting_resume_on_first_send() } else { provider.with_rejecting_resume() };
        let mut rig = native_rig(provider).await;
        rig.say("first request").await;
        rig.commands.send(Command::Send("second request".into())).unwrap();
        let updates = rig.done().await;
        assert_eq!(finished(&updates), Some(discard_core::driver::TurnOutcome::Failed));
        assert_eq!(rig.started_with(), vec![None, Some("h1".into()), None], "no third start; first_send={first_send}");
        assert!(errors(&updates).iter().any(|e| e.contains("the replay broke")), "{updates:?}");
        assert!(!errors(&updates).iter().any(|e| e.contains("Start a new session")), "{updates:?}");
        assert_eq!(rig.row(&rig.agent.clone()).unwrap().dropped_reason.as_deref(), Some(UNSAFE_FAILED));
        rig.close().await;
    }
}

#[tokio::test]
async fn over_budget_bytes_replay_on_the_next_response() {
    let mut rig = native_rig(replies(&["one", "two"])).await;
    rig.say("first request").await;
    let mut row = rig.row(&rig.agent.clone()).unwrap();
    row.bytes_in_session = discard_core::conversation_context::input_budget(None) as u64;
    rig.store.lock().unwrap().put_agent_session(&row).unwrap();
    assert_next_response_replays(&mut rig, "Fresh replay: the session grew past its size limit. Discard sends the history again.").await;
    assert_eq!(rig.started_with(), vec![None, None]);
    assert!(rig.row(&rig.agent.clone()).unwrap().bytes_in_session < row.bytes_in_session);
    rig.close().await;
}

#[tokio::test]
async fn a_compacted_session_replays_on_the_next_response() {
    let mut rig = native_rig(replies(&["one", "two"]).with_compaction()).await;
    rig.say("first request").await;
    let row = rig.row(&rig.agent.clone()).unwrap();
    assert_eq!((row.handle.as_str(), row.dropped_reason.as_deref()), ("h1", Some("provider_compacted")));
    let updates = rig.say("after the drop").await;
    assert_eq!(rig.started_with(), vec![None, None]);
    assert!(notices(&updates).iter().any(|n| n == "Fresh replay: the provider compacted the session. Discard sends the history again."), "{updates:?}");
    assert!(rig.sent(1).contains("first request"));
    rig.close().await;
}

#[tokio::test]
async fn a_context_limit_error_drops_the_handle_with_over_budget() {
    let provider = FakeProvider::new(vec![Step::text("one"), Step::idle()])
        .then_session(vec![Step::Emit(discard_core::event::EventKind::Error { message: "Prompt is too long: the context window limit was exceeded".into() }), Step::idle()])
        .then_session(vec![Step::text("three"), Step::idle()])
        .with_resume_handle("h1");
    let mut rig = native_rig(provider).await;
    rig.say("first request").await;
    rig.commands.send(Command::Send("second request".into())).unwrap();
    let updates = rig.done().await;
    assert_eq!(finished(&updates), Some(discard_core::driver::TurnOutcome::Failed));
    assert_eq!(rig.row(&rig.agent.clone()).unwrap().dropped_reason.as_deref(), Some("over_budget"));
    assert_next_response_replays(&mut rig, "Fresh replay: the session grew past its size limit. Discard sends the history again.").await;
    rig.close().await;
}

#[tokio::test]
async fn a_non_native_agent_is_unaffected_by_failures() {
    let provider = FakeProvider::new(vec![Step::text("partial"), Step::Crash])
        .then_session(vec![Step::text("two"), Step::idle()])
        .with_resume_handle("h1")
        .with_compaction();
    let probe = provider.probe.clone();
    let mut rig = Rig::with_provider(Arc::new(provider), probe).await;
    rig.commands.send(Command::Send("first request".into())).unwrap();
    let updates = rig.done().await;
    assert_eq!(finished(&updates), Some(discard_core::driver::TurnOutcome::Failed));
    let updates = rig.say("second request").await;
    assert!(!notices(&updates).iter().any(|n| n.starts_with("Fresh replay")), "{updates:?}");
    assert_eq!(rig.started_with(), vec![None, None]);
    assert_eq!(rig.started_resumable(), vec![false, false]);
    assert!(rig.row(&rig.agent.clone()).is_none());
    rig.close().await;
}

#[tokio::test]
async fn a_cancel_during_a_rejected_first_send_ends_the_turn_without_a_replay() {
    let provider = FakeProvider::new(vec![Step::text("one"), Step::idle()])
        .then_session(vec![Step::text("from the rejected session"), Step::idle()])
        .then_session(vec![Step::text("must not run"), Step::idle()])
        .with_resume_handle("h1")
        .with_slow_rejecting_resume_on_first_send(std::time::Duration::from_millis(300));
    let mut rig = native_rig(provider).await;
    rig.say("first request").await;
    rig.commands.send(Command::Send("second request".into())).unwrap();
    // The rejected session received its message: the send is now waiting.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while rig.probe.sent.lock().unwrap().len() < 2 { tokio::time::sleep(std::time::Duration::from_millis(5)).await; }
    }).await.unwrap();
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.done().await;
    assert_eq!(finished(&updates), Some(discard_core::driver::TurnOutcome::Cancelled), "{updates:?}");
    assert_eq!(rig.started_with(), vec![None, Some("h1".into())], "no replay starts after a cancel");
    assert_eq!(rig.probe.sent.lock().unwrap().len(), 2, "no second input");
    assert!(!notices(&updates).iter().any(|n| n == REJECTED_NOTICE), "{updates:?}");
    assert_eq!(rig.row(&rig.agent.clone()).unwrap().dropped_reason.as_deref(), Some("handle_rejected"));
    rig.close().await;
}

// Native provider resume: final review fixes.

const UNSAFE_UNFINISHED: &str = "the previous response did not finish";

#[tokio::test]
async fn an_interrupted_resumed_turn_replays_with_a_notice_after_a_restart() {
    let provider = FakeProvider::new(vec![Step::text("one"), Step::idle()])
        .then_session(vec![Step::text("partial"), Step::Hang])
        .with_resume_handle("h1");
    let mut rig = native_rig(provider).await;
    rig.say("first request").await;
    rig.commands.send(Command::Send("second request".into())).unwrap();
    loop {
        if let Some(Update::Event(event)) = rig.updates.recv().await
            && matches!(&event.kind, discard_core::event::EventKind::TextDelta { .. }) { break; }
    }
    assert_eq!(rig.started_with(), vec![None, Some("h1".into())]);
    // The provider session now holds a partial reply. The row says so before the turn ends.
    let row = rig.row(&rig.agent.clone()).unwrap();
    assert_eq!((row.handle.as_str(), row.dropped_reason.as_deref()), ("h1", Some(UNSAFE_UNFINISHED)));
    // The app exits mid-turn: a new engine runs on the same store.
    let provider = Arc::new(replies(&["three"]));
    let probe = provider.probe.clone();
    let channel = rig.channel.clone();
    rig.respawn(vec![provider], probe, channel).await;
    assert_next_response_replays(&mut rig, &unsafe_notice(UNSAFE_UNFINISHED)).await;
    assert_eq!(rig.started_with(), vec![None]);
    rig.close().await;
}

#[tokio::test]
async fn a_changed_item_that_is_not_selected_is_retracted_then_sent_fresh() {
    let mut rig = native_rig(replies(&["one", "two", "three", "four"])).await;
    let fact = rig.memory.create_item(&Scope::Agent(rig.agent.clone()), ItemKind::Fact, "Deploys go to staging-A", None, MutationOrigin::User).unwrap();
    rig.say("Where do deploys go?").await;
    assert!(rig.sent(0).contains("staging-A"), "{}", rig.sent(0));
    rig.say("hello there").await;
    rig.memory.update_item(&fact.id, fact.version, "Deploys go to staging-B", MutationOrigin::User).unwrap();
    // No word of this turn or the two latest messages matches the item.
    rig.say("and more").await;
    let third = rig.sent(2);
    assert!(third.contains(&format!("<changed-memory id=\"{}\">", fact.id)), "{third}");
    assert!(!third.contains("staging-A") && !third.contains("staging-B"), "{third}");
    assert!(!rig.row(&rig.agent.clone()).unwrap().delivered_memory.contains_key(&fact.id));
    rig.say("deploys now?").await;
    let fourth = rig.sent(3);
    assert!(fourth.contains("staging-B") && !fourth.contains("changed-memory"), "{fourth}");
    assert_eq!(rig.started_with(), vec![None, Some("h1".into()), Some("h1".into()), Some("h1".into())]);
    assert_eq!(rig.row(&rig.agent.clone()).unwrap().delivered_memory.get(&fact.id), Some(&(fact.version + 1)));
    rig.close().await;
}

/// The memory runtime, with the session-state lines of every turn replaced by `state`.
struct StateOverride {
    inner: Arc<MemoryRuntime>,
    state: Arc<Mutex<String>>,
}

impl discard_core::engine::MemoryService for StateOverride {
    fn prepare_response(&self, agent: &AgentId, project: Option<&discard_core::model::ProjectId>, source: &ChannelId, base_prompt: Option<&str>, plugin_write: bool, options: discard_core::engine::MemoryOptions<'_>) -> Result<discard_core::engine::PreparedMemory, String> {
        let mut prepared = self.inner.prepare_response(agent, project, source, base_prompt, plugin_write, options)?;
        if let Some(parts) = &mut prepared.parts { parts.session_state = self.state.lock().unwrap().clone(); }
        Ok(prepared)
    }
    fn prepare(&self, agent: &AgentId, project: Option<&discard_core::model::ProjectId>, source: &ChannelId, base_prompt: Option<&str>, plugin_write: bool) -> Result<discard_core::engine::PreparedMemory, String> {
        self.inner.prepare(agent, project, source, base_prompt, plugin_write)
    }
    fn prepare_summary(&self, agent: &AgentId, project: Option<&discard_core::model::ProjectId>, source: &ChannelId) -> Result<discard_core::engine::PreparedMemory, String> {
        self.inner.prepare_summary(agent, project, source)
    }
    fn resume_dir(&self, channel: &ChannelId, agent: &AgentId) -> Option<std::path::PathBuf> { self.inner.resume_dir(channel, agent) }
    fn finish(&self, token: &str) { self.inner.finish(token) }
}

const MODERATOR: &str = "You are this project's Primary / Moderator agent.";

async fn state_rig(native: bool, state: Arc<Mutex<String>>) -> Rig {
    let provider = Arc::new(replies(&["one", "two", "three"]));
    let probe = provider.probe.clone();
    let rig = Rig::with_memory_service(provider, probe, move |runtime, _| Arc::new(StateOverride { inner: runtime, state })).await;
    rig.edit(|c| c.native_resume = native);
    rig
}

#[tokio::test]
async fn a_session_state_line_that_disappears_is_replaced_by_none() {
    let state = Arc::new(Mutex::new(MODERATOR.to_string()));
    let mut rig = state_rig(true, state.clone()).await;
    let block = |body: &str| format!("This block replaces every earlier session-state block in this conversation.\n<session-state>\n{body}\n</session-state>");
    rig.say("first request").await;
    assert!(rig.sent(0).contains(&block(MODERATOR)), "{}", rig.sent(0));
    assert!(rig.sent(0).contains("Current snapshot: none. It replaces earlier snapshots."), "{}", rig.sent(0));
    rig.say("second request").await;
    assert!(rig.sent(1).contains(&block(MODERATOR)), "{}", rig.sent(1));
    // The user makes another agent the moderator.
    *state.lock().unwrap() = String::new();
    rig.say("third request").await;
    let third = rig.sent(2);
    assert!(third.contains(&block("none")) && !third.contains(MODERATOR), "{third}");
    assert!(third.contains("Current snapshot: none. It replaces earlier snapshots."), "{third}");
    assert_eq!(rig.started_with(), vec![None, Some("h1".into()), Some("h1".into())]);
    rig.close().await;
}

#[tokio::test]
async fn a_non_native_agent_gets_no_session_state_or_snapshot_labels() {
    let mut rig = state_rig(false, Arc::new(Mutex::new(MODERATOR.to_string()))).await;
    rig.say("first request").await;
    rig.say("second request").await;
    for turn in 0..2 {
        let sent = rig.sent(turn);
        assert!(!sent.contains("<session-state>") && !sent.contains("replaces earlier snapshots") && !sent.contains(MODERATOR), "{sent}");
    }
    rig.close().await;
}

#[tokio::test]
async fn the_byte_estimate_counts_full_tool_outputs() {
    // The fake has no context window, so the budget is the default 48 KB. A 30 KB output is far past the 1 KB that
    // replay keeps of a tool output.
    let big = "x".repeat(30_000);
    let provider = FakeProvider::new(vec![Step::text("one"), Step::idle()])
        .then_session(vec![
            Step::Emit(discard_core::event::EventKind::ToolCallStart { call_id: "c1".into(), name: "Bash".into(), input: serde_json::json!({"command": "cat big"}) }),
            Step::Emit(discard_core::event::EventKind::ToolCallResult { call_id: "c1".into(), output: big, is_error: false }),
            Step::text("two"),
            Step::idle(),
        ])
        .then_session(vec![Step::text("three"), Step::idle()])
        .with_resume_handle("h1");
    let mut rig = native_rig(provider).await;
    rig.say("first request").await;
    // The session is near the budget, with room for one small turn but not for 30 KB more.
    let budget = discard_core::conversation_context::input_budget(None) as u64;
    let mut row = rig.row(&rig.agent.clone()).unwrap();
    row.bytes_in_session = budget - 20_000;
    rig.store.lock().unwrap().put_agent_session(&row).unwrap();
    rig.say("second request").await;
    assert_eq!(rig.started_with()[1].as_deref(), Some("h1"));
    let after = rig.row(&rig.agent.clone()).unwrap().bytes_in_session;
    assert!(after >= row.bytes_in_session + 30_000, "{after} < {} + 30000", row.bytes_in_session);
    let updates = rig.say("third request").await;
    assert_eq!(rig.started_with()[2], None);
    assert!(notices(&updates).iter().any(|n| n == "Fresh replay: the session grew past its size limit. Discard sends the history again."), "{updates:?}");
    rig.close().await;
}
