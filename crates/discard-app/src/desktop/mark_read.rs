//! Mark a project, or every project of a rail group, as read.
use super::core::AppCore;
use discard_core::model::{ChannelId, ProjectId, Role};
use discard_core::store::{RailItem, RailLayout, Store};
use std::sync::Mutex;

/// The member project IDs of a rail group, in order.
pub fn group_project_ids(layout: &RailLayout, group_id: &str) -> Result<Vec<String>, String> {
    layout.items.iter().find_map(|item| match item {
        RailItem::Group(g) if g.id == group_id => Some(g.project_ids.iter().map(|id| id.0.clone()).collect()),
        _ => None,
    }).ok_or_else(|| "unknown group".to_string())
}

/// `(thread id, last user or agent message id)` for every thread of these channels, followed or not.
/// A thread without such a message is left out.
pub fn thread_markers(store: &Store, channel_ids: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut markers = Vec::new();
    for channel in channel_ids {
        for thread in store.threads_for_channel(&ChannelId(channel.clone())).map_err(|e| e.to_string())? {
            let messages = store.messages_for_channel(&thread.channel_id).map_err(|e| e.to_string())?;
            if let Some(last) = messages.iter().rev().find(|m| m.role != Role::System) {
                markers.push((thread.channel_id.0.clone(), last.id.0.clone()));
            }
        }
    }
    Ok(markers)
}

/// Mark every non-thread channel of these projects, archived ones included, and the threads under them, as read.
/// A loaded channel uses its in-memory tail. Any other channel uses its last stored message.
/// Returns whether any marker changed.
pub fn mark_projects_read(core: &AppCore, store: &Mutex<Store>, project_ids: &[String]) -> Result<bool, String> {
    let loaded: std::collections::HashMap<String, Option<String>> = core.loaded_channel_tails(project_ids).into_iter().collect();
    let (mut markers, threads) = {
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        let mut channel_ids = Vec::new();
        for project in project_ids {
            for (channel, _archived) in store.all_channels_for_project(&ProjectId(project.clone())).map_err(|e| e.to_string())? { channel_ids.push(channel.id.0); }
        }
        for id in loaded.keys() { if !channel_ids.contains(id) { channel_ids.push(id.clone()); } }
        let mut markers = Vec::new();
        for id in &channel_ids {
            let tail = match loaded.get(id) {
                Some(tail) => tail.clone(),
                None => store.last_message_id(&ChannelId(id.clone())).map_err(|e| e.to_string())?.map(|m| m.0),
            };
            if let Some(tail) = tail { markers.push((id.clone(), tail)); }
        }
        (markers, thread_markers(&store, &channel_ids)?)
    };
    markers.extend(threads);
    core.apply_read_markers(&markers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::capability::ApprovalPolicy;
    use discard_core::agent::AgentConfig;
    use discard_core::model::{AgentId, Block, Channel, Message, MessageId, Project, ProjectId, WorkspaceMode};
    use discard_core::ui_state::UiState;
    use super::super::core::CoreInit;
    use super::super::view::ViewId;

    fn config() -> AgentConfig {
        AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Scout".into(), avatar: None, provider: "claude".into(), model: Some("model".into()), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }
    }
    fn say(store: &Store, channel: &ChannelId, role: Role) -> MessageId {
        let message = Message { id: MessageId::new(), channel_id: channel.clone(), role, blocks: vec![Block::Markdown { text: "x".into() }] };
        store.insert_message(&message).unwrap();
        message.id
    }
    struct World { store: Store, project: Project, agent: AgentId, one: Channel, two: Channel }
    fn world() -> World {
        let store = Store::open_in_memory().unwrap();
        let agent = store.create_agent(&config()).unwrap();
        let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
        store.insert_project(&project).unwrap();
        let channel = |name: &str| Channel { id: ChannelId::new(), agent_id: agent.clone(), project_id: Some(project.id.clone()), name: name.into(), workspace_mode: WorkspaceMode::MainCheckout };
        let (one, two) = (channel("one"), channel("two"));
        store.insert_channel(&one).unwrap();
        store.insert_channel(&two).unwrap();
        World { store, project, agent, one, two }
    }
    fn thread_in(w: &World) -> ChannelId {
        let anchor = say(&w.store, &w.one.id, Role::Agent);
        w.store.create_thread(&w.one.id, &anchor, "Why?", &w.agent).unwrap().id
    }
    fn ids(channel: &ChannelId) -> Vec<String> { vec![channel.0.clone()] }

    #[test]
    fn group_project_ids_returns_the_members_in_order() {
        let store = Store::open_in_memory().unwrap();
        let ids = ["a", "b", "c"].map(|id| ProjectId(id.into()));
        for id in &ids { store.insert_project(&Project { id: id.clone(), name: id.0.clone(), path: format!("/{}", id.0) }).unwrap(); }
        let group = store.rail_group_projects(&ids[2], &ids[0]).unwrap();
        let layout = store.rail_layout().unwrap();
        assert_eq!(group_project_ids(&layout, &group).unwrap(), vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn group_project_ids_rejects_an_unknown_group() {
        let layout = Store::open_in_memory().unwrap().rail_layout().unwrap();
        assert_eq!(group_project_ids(&layout, "nope"), Err("unknown group".to_string()));
    }

    #[test]
    fn an_empty_group_gives_no_projects() {
        let layout = RailLayout { items: vec![RailItem::Group(discard_core::store::RailGroup { id: "g".into(), name: "G".into(), color: None, collapsed: false, project_ids: vec![] })] };
        assert_eq!(group_project_ids(&layout, "g"), Ok(vec![]));
    }

    #[test]
    fn thread_markers_use_the_last_user_or_agent_message_and_skip_system_messages() {
        let w = world();
        let thread = thread_in(&w);
        say(&w.store, &thread, Role::User);
        let last = say(&w.store, &thread, Role::Agent);
        say(&w.store, &thread, Role::System);
        assert_eq!(thread_markers(&w.store, &ids(&w.one.id)).unwrap(), vec![(thread.0, last.0)]);
    }

    #[test]
    fn a_thread_with_only_a_user_message_still_gets_a_marker() {
        let w = world();
        let thread = thread_in(&w);
        let only = say(&w.store, &thread, Role::User);
        assert_eq!(thread_markers(&w.store, &ids(&w.one.id)).unwrap(), vec![(thread.0, only.0)]);
    }

    #[test]
    fn a_thread_with_no_message_gets_no_marker() {
        let w = world();
        let thread = thread_in(&w);
        say(&w.store, &thread, Role::System);
        assert!(thread_markers(&w.store, &ids(&w.one.id)).unwrap().is_empty());
    }

    #[test]
    fn mark_projects_read_marks_channels_and_threads_in_one_write() {
        let w = world();
        let thread = thread_in(&w);
        let thread_last = say(&w.store, &thread, Role::Agent);
        let dir = tempfile::tempdir().unwrap();
        let init = |id: &ChannelId, history: Vec<Message>| {
            let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
            std::mem::forget(receiver);
            CoreInit { state: UiState::new(id.clone(), history), commands, project: w.project.id.0.clone(), channel: id.0.clone(), agent: "a".into(), provider: "fake".into() }
        };
        let history = |id: &ChannelId, message: &str| vec![Message { id: MessageId(message.into()), channel_id: id.clone(), role: Role::Agent, blocks: vec![] }];
        let core = AppCore::new(init(&w.one.id, history(&w.one.id, "m1")));
        core.configure_targets(&ViewId::main(), w.project.id.0.clone(), std::path::Path::new("/p"), "agent".into());
        core.configure_read_marker(&ViewId::main(), dir.path(), &w.one.id.0);
        core.add_channel(init(&w.two.id, history(&w.two.id, "m2")), &w.two);
        let project = vec![w.project.id.0.clone()];
        let store = Mutex::new(w.store);
        assert_eq!(mark_projects_read(&core, &store, &project), Ok(true));
        assert_eq!(core.last_read(&w.one.id.0), Some("m1".to_string()));
        assert_eq!(core.last_read(&w.two.id.0), Some("m2".to_string()));
        assert_eq!(core.last_read(&thread.0), Some(thread_last.0));
        assert_eq!(mark_projects_read(&core, &store, &project), Ok(false));
    }

    #[test]
    fn mark_projects_read_covers_channels_that_are_not_loaded_and_archived_ones() {
        let w = world();
        let thread = thread_in(&w);
        let thread_last = say(&w.store, &thread, Role::Agent);
        say(&w.store, &w.one.id, Role::Agent);
        let one_last = say(&w.store, &w.one.id, Role::Agent);
        let archived = Channel { id: ChannelId::new(), agent_id: w.agent.clone(), project_id: Some(w.project.id.clone()), name: "old".into(), workspace_mode: WorkspaceMode::MainCheckout };
        w.store.insert_channel(&archived).unwrap();
        say(&w.store, &archived.id, Role::User);
        let archived_last = say(&w.store, &archived.id, Role::Agent);
        w.store.archive_channel(&archived.id).unwrap();
        let dir = tempfile::tempdir().unwrap();
        discard_core::window_state::StateStore::open(&dir.path().join("state.toml")).set_last_read(&archived.id.0, "stale").unwrap();
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        std::mem::forget(receiver);
        // Only `two` is loaded. `one` and the archived channel are not.
        let core = AppCore::new(CoreInit { state: UiState::new(w.two.id.clone(), vec![]), commands, project: w.project.id.0.clone(), channel: w.two.id.0.clone(), agent: "a".into(), provider: "fake".into() });
        core.configure_read_marker(&ViewId::main(), dir.path(), &w.two.id.0);
        let store = Mutex::new(w.store);
        assert_eq!(mark_projects_read(&core, &store, std::slice::from_ref(&w.project.id.0)), Ok(true));
        assert_eq!(core.last_read(&w.one.id.0), Some(one_last.0));
        assert_eq!(core.last_read(&archived.id.0), Some(archived_last.0));
        assert_eq!(core.last_read(&thread.0), Some(thread_last.0));
    }
}
