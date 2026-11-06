use discard_app::desktop::settings::memory::{memory_view, mutate, resolve_proposal, resume};
use discard_core::{
    agent::AgentConfig,
    capability::ApprovalPolicy,
    memory::{MemoryStore, Scope},
    memory_items::{ItemKind, MutationOrigin, SaveOutcome},
    model::{ChannelId, Message, MessageId, Role},
    store::Store,
};

fn setup() -> (
    tempfile::TempDir,
    MemoryStore,
    Store,
    discard_core::model::AgentId,
    ChannelId,
) {
    let dir = tempfile::tempdir().unwrap();
    let memory = MemoryStore::open_in_memory_index(dir.path()).unwrap();
    let store = Store::open_in_memory().unwrap();
    let agent = store
        .create_agent(&AgentConfig {
            reasoning_effort: None,
            name: "Ada".into(),
            avatar: None,
            provider: "codex".into(),
            model: Some("exact".into()),
            system_prompt: String::new(),
            policy: ApprovalPolicy::DenyAll,
            plugin_write: false,
        })
        .unwrap();
    let channel = store.default_conversation(&agent).unwrap().id;
    (dir, memory, store, agent, channel)
}
#[test]
fn scoped_view_uses_stored_conversation_and_all_view_keeps_orphan_scopes() {
    let (_d, memory, store, agent, channel) = setup();
    let mine = memory
        .create_item(
            &Scope::Agent(agent),
            ItemKind::Preference,
            "short answers",
            Some(&channel),
            MutationOrigin::User,
        )
        .unwrap();
    let orphan = memory
        .create_item(
            &Scope::Agent(discard_core::model::AgentId::new()),
            ItemKind::Fact,
            "orphan fact",
            None,
            MutationOrigin::User,
        )
        .unwrap();
    let view = memory_view(&memory, &store, Some(&channel.0)).unwrap();
    assert_eq!(view["items"].as_array().unwrap().len(), 1);
    assert_eq!(view["items"][0]["id"], mine.id);
    assert_eq!(view["items"][0]["sourceAvailable"], true);
    assert!(memory_view(&memory, &store, Some("")).is_err());
    let all = memory_view(&memory, &store, None).unwrap();
    assert!(
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == orphan.id)
    );
    assert!(
        mutate(
            &memory,
            &store,
            &orphan.id,
            1,
            "forget",
            None,
            None,
            Some(&channel.0)
        )
        .is_err()
    );
}
#[test]
fn edits_moves_forget_conflicts_pause_and_app_resume_use_real_versions() {
    let (_d, memory, store, agent, source) = setup();
    let other = store
        .create_conversation(&agent, None, "Requester")
        .unwrap()
        .id;
    let item = memory
        .create_item(
            &Scope::Agent(agent),
            ItemKind::Fact,
            "old",
            Some(&source),
            MutationOrigin::Automatic,
        )
        .unwrap();
    mutate(
        &memory,
        &store,
        &item.id,
        1,
        "edit",
        Some("new"),
        None,
        Some(&other.0),
    )
    .unwrap();
    assert!(
        mutate(
            &memory,
            &store,
            &item.id,
            1,
            "undo",
            None,
            None,
            Some(&other.0)
        )
        .unwrap_err()
        .contains("Reload")
    );
    mutate(
        &memory,
        &store,
        &item.id,
        2,
        "move",
        None,
        Some("global"),
        Some(&other.0),
    )
    .unwrap();
    assert_eq!(memory.read_item(&item.id).unwrap().history.len(), 2);
    mutate(
        &memory,
        &store,
        &item.id,
        3,
        "forget",
        None,
        None,
        Some(&other.0),
    )
    .unwrap();
    assert!(memory.capture_paused(&other).unwrap());
    assert_eq!(
        memory_view(&memory, &store, Some(&other.0)).unwrap()["paused"],
        true
    );
    resume(&memory, &store, &other.0).unwrap();
    assert!(!memory.capture_paused(&other).unwrap());
}
#[test]
fn approval_revalidates_agent_source_context_and_versions_and_keeps_response_link() {
    let (_d, memory, store, agent, channel) = setup();
    let SaveOutcome::Proposed(proposal) = memory
        .capture_item(
            &agent,
            None,
            &channel,
            &Scope::Global,
            ItemKind::Fact,
            "shared",
        )
        .unwrap()
    else {
        panic!()
    };
    let response = MessageId::new();
    store
        .insert_message(&Message {
            id: response.clone(),
            channel_id: channel.clone(),
            role: Role::Agent,
            blocks: vec![],
        })
        .unwrap();
    store
        .record_response_memory(
            &channel,
            &response,
            &discard_core::memory_items::MemoryChange::Proposed(proposal.clone()),
        )
        .unwrap();
    store.set_agent_archived(&agent, true).unwrap();
    assert!(resolve_proposal(&memory, &store, &proposal.id, true, Some(&channel.0)).is_err());
    store.set_agent_archived(&agent, false).unwrap();
    resolve_proposal(&memory, &store, &proposal.id, true, Some(&channel.0)).unwrap();
    let view = memory_view(&memory, &store, Some(&channel.0)).unwrap();
    assert!(
        view["responses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["responseId"] == response.0 && r["type"] == "approved")
    );
    assert!(resolve_proposal(&memory, &store, &proposal.id, true, Some(&channel.0)).is_err());
    let item = memory.list_items(&[Scope::Global]).unwrap().remove(0);
    let p = memory
        .propose_update(
            &agent,
            None,
            &channel,
            &item.id,
            item.version,
            "inferred correction",
        )
        .unwrap();
    memory
        .update_item(
            &item.id,
            item.version,
            "user correction",
            MutationOrigin::User,
        )
        .unwrap();
    assert!(resolve_proposal(&memory, &store, &p.id, true, Some(&channel.0)).is_err());
    resolve_proposal(&memory, &store, &p.id, false, Some(&channel.0)).unwrap();
}

#[test]
fn forged_proposer_and_project_context_cannot_authorize_shared_memory() {
    let (_d, memory, store, agent, channel) = setup();
    let impostor = discard_core::model::AgentId::new();
    let SaveOutcome::Proposed(p) = memory
        .capture_item(
            &impostor,
            None,
            &channel,
            &Scope::Global,
            ItemKind::Fact,
            "forged source",
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(resolve_proposal(&memory, &store, &p.id, true, None).is_err());
    let project_dir = tempfile::tempdir().unwrap();
    let project = store.import_project_folder(project_dir.path()).unwrap();
    let SaveOutcome::Proposed(p) = memory
        .capture_item(
            &agent,
            Some(&project.id),
            &channel,
            &Scope::Project(project.id.clone()),
            ItemKind::Decision,
            "wrong conversation context",
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(resolve_proposal(&memory, &store, &p.id, true, None).is_err());
    let project_channel = store
        .create_conversation(&agent, Some(&project.id), "Project")
        .unwrap();
    let SaveOutcome::Proposed(p) = memory
        .capture_item(
            &agent,
            Some(&project.id),
            &project_channel.id,
            &Scope::Project(project.id.clone()),
            ItemKind::Decision,
            "project decision",
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(resolve_proposal(&memory, &store, &p.id, true, Some(&channel.0)).is_err());
    resolve_proposal(&memory, &store, &p.id, true, Some(&project_channel.id.0)).unwrap();
    assert_eq!(
        memory
            .list_items(&[Scope::Project(project.id)])
            .unwrap()
            .len(),
        1
    );
    assert!(memory.list_items(&[Scope::Global]).unwrap().is_empty());
}

#[test]
fn external_file_conflict_import_reload_and_retained_deleted_source_are_real() {
    let (dir, memory, store, agent, channel) = setup();
    let item = memory
        .create_item(
            &Scope::Global,
            ItemKind::Fact,
            "committed",
            Some(&channel),
            MutationOrigin::User,
        )
        .unwrap();
    let path = dir.path().join(".items").join(format!("{}.md", item.id));
    let text = std::fs::read_to_string(&path).unwrap();
    let metadata = text.split_once("\n\n").unwrap().0;
    std::fs::write(&path, format!("{metadata}\n\nexternal edit")).unwrap();
    assert!(
        mutate(
            &memory,
            &store,
            &item.id,
            1,
            "edit",
            Some("draft"),
            None,
            Some(&channel.0)
        )
        .unwrap_err()
        .contains("Import file edits")
    );
    memory.reconcile().unwrap();
    let view = memory_view(&memory, &store, Some(&channel.0)).unwrap();
    assert_eq!(view["items"][0]["content"], "external edit");
    assert_eq!(view["items"][0]["version"], 2);
    mutate(
        &memory,
        &store,
        &item.id,
        2,
        "edit",
        Some("accepted draft"),
        None,
        Some(&channel.0),
    )
    .unwrap();
    store
        .delete_agent(
            &agent,
            discard_core::store::DeleteAgentOptions {
                delete_conversations: false,
                delete_private_memory: false,
            },
        )
        .unwrap();
    assert_eq!(
        memory_view(&memory, &store, None).unwrap()["items"][0]["sourceAvailable"],
        true
    );
    assert!(store.load_channel(&channel).is_ok());
}

#[test]
fn source_deletion_is_unavailable_and_cannot_approve_a_pending_proposal() {
    let (_d, memory, store, agent, channel) = setup();
    let item = memory
        .create_item(
            &Scope::Global,
            ItemKind::Fact,
            "retained shared fact",
            Some(&channel),
            MutationOrigin::User,
        )
        .unwrap();
    let SaveOutcome::Proposed(p) = memory
        .capture_item(
            &agent,
            None,
            &channel,
            &Scope::Global,
            ItemKind::Fact,
            "pending shared fact",
        )
        .unwrap()
    else {
        panic!()
    };
    store
        .delete_agent(
            &agent,
            discard_core::store::DeleteAgentOptions {
                delete_conversations: true,
                delete_private_memory: false,
            },
        )
        .unwrap();
    let view = memory_view(&memory, &store, None).unwrap();
    assert_eq!(view["items"][0]["id"], item.id);
    assert_eq!(view["items"][0]["sourceAvailable"], false);
    assert!(resolve_proposal(&memory, &store, &p.id, true, None).is_err());
    resolve_proposal(&memory, &store, &p.id, false, None).unwrap();
}

#[test]
fn archived_agent_domain_keeps_its_name_and_pause_stays_with_old_conversation() {
    let (_d, memory, store, agent, channel) = setup();
    let item = memory
        .create_item(
            &Scope::Agent(agent.clone()),
            ItemKind::Fact,
            "old source",
            Some(&channel),
            MutationOrigin::Automatic,
        )
        .unwrap();
    mutate(
        &memory,
        &store,
        &item.id,
        item.version,
        "undo",
        None,
        None,
        Some(&channel.0),
    )
    .unwrap();
    assert!(memory.capture_paused(&channel).unwrap());
    let fresh = store
        .create_conversation(&agent, None, "Fresh conversation")
        .unwrap();
    assert!(!memory.capture_paused(&fresh.id).unwrap());
    assert!(store.messages_for_channel(&fresh.id).unwrap().is_empty());
    store.set_agent_archived(&agent, true).unwrap();
    let view = memory_view(&memory, &store, None).unwrap();
    assert!(
        view["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["scope"] == format!("agent:{}", agent.0)
                && s["title"].as_str().unwrap().contains("Ada"))
    );
}

#[test]
fn runtime_memory_notification_refreshes_cards_without_changing_transcript_ids() {
    use discard_app::desktop::core::{AppCore, CoreInit};
    use discard_core::ui_state::{UiState, Update};
    let channel = ChannelId::new();
    let (commands, _) = tokio::sync::mpsc::unbounded_channel();
    let core = AppCore::new(CoreInit {
        state: UiState::new(channel.clone(), vec![]),
        commands,
        project: String::new(),
        channel: "Direct".into(),
        agent: "Ada".into(),
        provider: "codex".into(),
    });
    let before = core.snapshot().memory_rev;
    core.apply_channel(
        &channel.0,
        Update::MemoryChanges {
            response: MessageId::new(),
            changes: vec![],
        },
    );
    assert_eq!(core.snapshot().memory_rev, before + 1);
    assert!(core.history_snapshot().messages.is_empty());
}

#[test]
fn approval_before_proposal_row_persistence_reconciles_to_original_response() {
    use discard_core::memory_items::MemoryChange;
    let (_dir, memory, store, agent, channel) = setup();
    let SaveOutcome::Proposed(proposal) = memory
        .capture_item(
            &agent,
            None,
            &channel,
            &Scope::Global,
            ItemKind::Fact,
            "approved while response is saving",
        )
        .unwrap()
    else {
        panic!()
    };
    // The runtime has copied this pending snapshot, but has not saved its response row.
    let snapshot = MemoryChange::Proposed(proposal.clone());
    resolve_proposal(&memory, &store, &proposal.id, true, Some(&channel.0)).unwrap();
    let response = MessageId::new();
    store
        .insert_message(&Message {
            id: response.clone(),
            channel_id: channel.clone(),
            role: Role::Agent,
            blocks: vec![],
        })
        .unwrap();
    store
        .record_response_memory(&channel, &response, &snapshot)
        .unwrap();
    let view = memory_view(&memory, &store, Some(&channel.0)).unwrap();
    assert!(
        view["responses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["responseId"] == response.0 && r["type"] == "approved")
    );
    assert!(store.response_memory(&channel).unwrap().iter().any(|(id,change)|id==&response && matches!(change, MemoryChange::Saved{item,..} if item.origin==MutationOrigin::Approved)));
}

#[test]
fn committed_approval_survives_sql_notice_failure_reopen_and_later_edit() {
    use discard_core::memory_items::MemoryChange;
    let (dir, memory, _unused, _a, _c) = setup();
    let store = Store::open(&dir.path().join("approval.db")).unwrap();
    let agent = store
        .create_agent(&AgentConfig {
            reasoning_effort: None,
            name: "Ada".into(),
            avatar: None,
            provider: "codex".into(),
            model: Some("exact".into()),
            system_prompt: String::new(),
            policy: ApprovalPolicy::DenyAll,
            plugin_write: false,
        })
        .unwrap();
    let channel = store.default_conversation(&agent).unwrap().id;
    let SaveOutcome::Proposed(proposal) = memory
        .capture_item(
            &agent,
            None,
            &channel,
            &Scope::Global,
            ItemKind::Fact,
            "original approved value",
        )
        .unwrap()
    else {
        panic!()
    };
    let response = MessageId::new();
    store
        .insert_message(&Message {
            id: response.clone(),
            channel_id: channel.clone(),
            role: Role::Agent,
            blocks: vec![],
        })
        .unwrap();
    store
        .record_response_memory(
            &channel,
            &response,
            &MemoryChange::Proposed(proposal.clone()),
        )
        .unwrap();
    let sql = rusqlite::Connection::open(dir.path().join("approval.db")).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_memory_notice BEFORE INSERT ON response_memory WHEN instr(NEW.change_key, ':') > 0 BEGIN SELECT RAISE(ABORT, 'test notice disk failure'); END;").unwrap();
    let outcome = resolve_proposal(&memory, &store, &proposal.id, true, Some(&channel.0))
        .expect("the approved memory already committed");
    assert!(outcome.committed);
    assert!(outcome.association_pending);
    assert!(outcome.warning.as_ref().unwrap().contains("approved"));
    assert!(memory.pending_proposals().unwrap().is_empty());
    let failed_view = memory_view(&memory, &store, Some(&channel.0)).unwrap();
    assert!(!failed_view["warnings"].as_array().unwrap().is_empty());
    assert!(
        failed_view["responses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["type"] == "approved" && r["item"]["version"] == 1)
    );
    let item = memory.list_items(&[Scope::Global]).unwrap().remove(0);
    memory
        .update_item(
            &item.id,
            item.version,
            "later user edit",
            MutationOrigin::User,
        )
        .unwrap();
    drop(memory);
    let memory = MemoryStore::open_in_memory_index(dir.path()).unwrap();
    sql.execute_batch("DROP TRIGGER reject_memory_notice;")
        .unwrap();
    let view = memory_view(&memory, &store, Some(&channel.0)).unwrap();
    let card = view["responses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["type"] == "approved")
        .unwrap();
    assert_eq!(card["responseId"], response.0);
    assert_eq!(card["item"]["version"], 1);
    assert_eq!(card["item"]["content"], "original approved value");
    assert!(
        mutate(
            &memory,
            &store,
            &item.id,
            1,
            "undo",
            None,
            None,
            Some(&channel.0)
        )
        .is_err()
    );
    let records = store.response_memory(&channel).unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().any(|(_,c)|matches!(c,MemoryChange::Saved{item,..} if item.version==1 && item.content=="original approved value")));
    sql.execute_batch("CREATE TRIGGER reject_memory_notice BEFORE INSERT ON response_memory BEGIN SELECT RAISE(ABORT, 'no more writes'); END;").unwrap();
    let settled = memory_view(&memory, &store, Some(&channel.0)).unwrap();
    assert!(
        settled["warnings"].as_array().unwrap().is_empty(),
        "settled associations must not attempt more writes"
    );
    assert_eq!(store.response_memory(&channel).unwrap().len(), 2);
}

#[test]
fn approval_observed_by_a_later_response_keeps_its_original_response_id() {
    use discard_core::memory_items::MemoryChange;
    let (_dir, memory, store, agent, channel) = setup();
    let SaveOutcome::Proposed(proposal) = memory
        .capture_item(
            &agent,
            None,
            &channel,
            &Scope::Global,
            ItemKind::Fact,
            "shared value",
        )
        .unwrap()
    else {
        panic!()
    };
    let original = MessageId::new();
    let later = MessageId::new();
    for id in [&original, &later] {
        store
            .insert_message(&Message {
                id: id.clone(),
                channel_id: channel.clone(),
                role: Role::Agent,
                blocks: vec![],
            })
            .unwrap();
    }
    store
        .record_response_memory(
            &channel,
            &original,
            &MemoryChange::Proposed(proposal.clone()),
        )
        .unwrap();
    memory.approve_proposal(&proposal.id).unwrap();
    let receipt = memory.approved_changes(&channel).unwrap().remove(0);
    assert_eq!(
        store
            .record_response_memory_at(&channel, &later, &receipt)
            .unwrap(),
        Some(original.clone())
    );
    assert!(
        store
            .response_memory(&channel)
            .unwrap()
            .iter()
            .all(|(id, _)| id == &original)
    );
}

#[test]
fn fresh_plugin_memory_has_versioned_controls_in_settings_and_conversation() {
    use discard_app::plugin_memory::PluginMemory;
    use discard_lua::MemoryHost;
    use std::sync::Arc;
    let (_dir, memory, store, agent, conversation) = setup();
    let memory = Arc::new(memory);
    let host = PluginMemory::new(memory.clone(), agent.clone(), None);
    let original = host
        .append("agent", "core/plugin.md", "mandatorypluginoriginal")
        .unwrap();
    let settings = memory_view(&memory, &store, None).unwrap();
    let contextual = memory_view(&memory, &store, Some(&conversation.0)).unwrap();
    assert_eq!(
        settings["items"].as_array().unwrap().len(),
        1,
        "fresh plugin write must be manageable"
    );
    assert_eq!(settings["items"], contextual["items"]);
    let id = settings["items"][0]["id"].as_str().unwrap();
    assert_eq!(settings["items"][0]["kind"], "instruction");
    assert_eq!(host.list("agent").unwrap()[0].path, "core/plugin.md");
    assert_eq!(
        host.read("agent", "core/plugin.md").unwrap().version,
        original
    );
    assert_eq!(
        host.search(&["agent".into()], "mandatorypluginoriginal", 5)
            .unwrap()[0]
            .path,
        "core/plugin.md"
    );
    assert!(
        memory
            .core_blocks(&Scope::applicable(&agent, None))
            .unwrap()
            .iter()
            .any(|b| b.text == "mandatorypluginoriginal")
    );
    mutate(
        &memory,
        &store,
        id,
        1,
        "edit",
        Some("mandatorypluginedited"),
        None,
        Some(&conversation.0),
    )
    .unwrap();
    assert!(
        host.edit(
            "agent",
            "core/plugin.md",
            "mandatorypluginedited",
            "bad",
            &original
        )
        .is_err()
    );
    let current = host.read("agent", "core/plugin.md").unwrap();
    host.edit(
        "agent",
        "core/plugin.md",
        "mandatorypluginedited",
        "mandatorypluginfinal",
        &current.version,
    )
    .unwrap();
    assert!(mutate(&memory, &store, id, 2, "edit", Some("stale"), None, None).is_err());
    mutate(
        &memory,
        &store,
        id,
        3,
        "move",
        None,
        Some("global"),
        Some(&conversation.0),
    )
    .unwrap();
    assert!(host.read("agent", "core/plugin.md").is_err());
    assert!(
        host.append("agent", "core/plugin.md", "do not recreate moved item")
            .is_err()
    );
    let moved = host.list("global").unwrap();
    assert_eq!(moved.len(), 1);
    assert_eq!(
        host.read("global", &moved[0].path).unwrap().text,
        "mandatorypluginfinal"
    );
    assert_eq!(memory.read_item(id).unwrap().history.len(), 3);
    assert!(
        memory
            .search(
                &[Scope::Global, Scope::Agent(agent.clone())],
                "mandatorypluginoriginal",
                5
            )
            .unwrap()
            .is_empty()
    );
    mutate(
        &memory,
        &store,
        id,
        4,
        "forget",
        None,
        None,
        Some(&conversation.0),
    )
    .unwrap();
    assert!(host.list("global").unwrap().is_empty());
    assert!(
        host.append("agent", "core/plugin.md", "mandatorypluginoriginal")
            .is_err()
    );
    assert!(
        host.append("global", &moved[0].path, "mandatorypluginfinal")
            .is_err()
    );
    assert!(
        memory_view(&memory, &store, None).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        memory_view(&memory, &store, Some(&conversation.0)).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        memory
            .core_blocks(&Scope::applicable(&agent, None))
            .unwrap()
            .is_empty()
    );
    assert!(
        host.search(&["global".into()], "mandatorypluginfinal", 5)
            .unwrap()
            .is_empty()
    );
}
