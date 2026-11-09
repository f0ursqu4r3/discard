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
        let output = std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .args(args).current_dir(&repo).output().unwrap();
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
    for i in 0..10 {
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
async fn summary_cancellation_revokes_tokens_and_keeps_boundary_unmodified() {
    let mut rig = Rig::new(vec![Step::Hang]).await;
    for _ in 0..10 {
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
    rig.commands.send(Command::Send("x".repeat(30000))).unwrap();
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
        };
        rig.store
            .lock()
            .unwrap()
            .set_conversation_context(&rig.channel, &old)
            .unwrap();
        for _ in 0..15 {
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
