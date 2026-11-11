//! Data sent to the webview.
use discard_core::model::{Block, Message};
use discard_core::ui_state::continues_group;
use discard_lua::element::Element;
use serde::Serialize;
use std::collections::BTreeMap;

pub type BlockDto = Block;

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDto {
    pub agent_id: String, pub project_id: Option<String>,
    pub id: String, pub name: String, pub agent: String, pub provider: String,
    pub workspace_mode: String, pub running: bool, pub unread: bool, pub pending_approvals: usize,
    /// Approvals that wait in the threads of this channel, followed or not. `pending_approvals` counts the channel only.
    pub thread_pending_approvals: usize,
    /// Followed threads of this channel, oldest first, with live state.
    pub threads: Vec<ThreadLinkDto>,
}

/// A followed thread in the channel bar, indented under its parent channel.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLinkDto {
    pub id: String,
    pub parent_channel_id: String,
    pub name: String,
    /// An agent message after the thread's own read marker.
    pub unread: bool,
    pub running: bool,
    pub pending_approvals: usize,
    /// `question` or `sub_agent`.
    pub kind: String,
    /// `running`, `done`, `failed`, `cancelled` or `timed_out` for a sub-agent thread. Null for a question thread.
    pub state: Option<String>,
}

/// A thread that has an approval waiting, followed or not. The Approvals page lists these.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ThreadApprovalDto {
    pub thread_id: String,
    pub parent_channel_id: String,
    pub name: String,
    pub kind: String,
    pub agent_name: String,
    pub pending_approvals: usize,
}

/// The newest user or agent message of a thread.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLastMessageDto {
    pub role: String,
    /// The agent that wrote it. Null for a user message.
    pub author_name: Option<String>,
    /// Plain text on one line, cut at 80 characters. Empty when the message has no text.
    pub preview: String,
}

/// The chip under an anchor message.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ThreadChipDto {
    pub channel_id: String,
    pub name: String,
    /// User and agent messages in the thread.
    pub message_count: usize,
    pub last_message: Option<ThreadLastMessageDto>,
    pub followed: bool,
    pub unread: bool,
    /// `question` or `sub_agent`.
    pub kind: String,
    /// `running`, `done`, `failed`, `cancelled` or `timed_out` for a sub-agent thread. Null for a question thread.
    pub state: Option<String>,
    /// The thread's agent: the sub-agent of a sub-agent thread, the default responder of a question thread.
    pub agent_name: String,
    /// Approvals that wait in the thread.
    pub pending_approvals: usize,
}

/// Where a reference row in a parent channel comes from.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PostedFromDto {
    pub source_message_id: String,
    pub thread_channel_id: String,
    pub thread_name: String,
}

/// The thread panel: one open thread beside the selected channel. The transcript comes from
/// `get_thread_history` and the `discard://thread-history` event; `history_rev` says which revision to hold.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpenThreadDto {
    pub channel_id: String,
    pub parent_channel_id: String,
    pub name: String,
    pub followed: bool,
    /// `question` or `sub_agent`.
    pub kind: String,
    /// The run state of a sub-agent thread. Null for a question thread.
    pub state: Option<String>,
    /// The thread's agent name (the sub-agent of a sub-agent thread).
    pub agent_name: String,
    /// Null for a sub-agent thread whose parent turn still runs.
    pub anchor_message_id: Option<String>,
    pub history_rev: u64,
    pub agent_id: String,
    pub agent: String,
    pub provider: String,
    pub responder: Option<discard_core::model::MessageAuthor>,
    pub agent_status: String,
    pub turn_running: bool,
    pub notice: Option<String>,
    pub live_blocks: Vec<BlockDto>,
    pub prompts: Vec<PromptDto>,
    pub pending_approvals: usize,
    pub unread: bool,
    pub composer_disabled: bool,
    /// Why the composer is off: `read_only` when the thread, its agent or its channel is archived or unavailable.
    /// Null when it is on. A sub-agent thread has its composer on: a message steers a task that runs, and starts a
    /// follow-up turn after the task ended.
    pub composer_disabled_reason: Option<String>,
    /// Says that threads are read-only. Always present.
    pub read_only_note: String,
}

/// The transcript of the open thread.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHistoryDto {
    pub channel_id: String,
    pub parent_channel_id: String,
    pub rev: u64,
    /// The message the thread starts from. Null when the parent transcript no longer holds it.
    pub anchor: Option<MessageDto>,
    pub messages: Vec<MessageDto>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChannelWorktreeDto {
    pub path: String,
    pub branch: String,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectChannelDto {
    pub worktree: Option<ChannelWorktreeDto>,
    pub repository: String,
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub agent_id: String,
    pub workspace_mode: String,
    pub kind: String,
    pub branch: Option<String>,
    pub archived: bool,
    pub archived_into: Option<String>,
    /// The channel, or a thread of the channel, has a saved message.
    pub has_messages: bool,
    /// Followed threads of this channel, from storage.
    pub threads: Vec<ThreadLinkDto>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChannelTopicDto {
    pub channel_id: String,
    pub text: String,
    pub version: u64,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChannelTopicRevisionDto {
    pub channel_id: String,
    pub text: String,
    pub version: u64,
    pub author_kind: String,
    pub author_id: Option<String>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDetailsDto {
    pub channel: ProjectChannelDto,
    pub topic: ChannelTopicDto,
    pub topic_history: Vec<ChannelTopicRevisionDto>,
    pub members: Vec<ChannelMemberDto>,
    pub main_agent_id: Option<String>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMemberDto {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub archived: bool,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDto { pub channel_id: String, pub rev: u64, pub messages: Vec<MessageDto> }
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessageDto {
    pub id: String,
    pub role: String,
    pub blocks: Vec<BlockDto>,
    pub continues_group: bool,
    /// Creation time in Unix milliseconds, recovered from the persisted UUIDv7 ID.
    pub created_at: Option<u64>,
    pub author: Option<discard_core::model::MessageAuthor>,
    /// The question thread that starts from this message, if any.
    pub thread: Option<ThreadChipDto>,
    /// The sub-agent threads of this response, in task order. Empty on any other message.
    pub sub_agent_threads: Vec<ThreadChipDto>,
    /// Set on a reference row: the thread message it shows.
    pub posted_from: Option<PostedFromDto>,
    /// Set on a thread message that is also posted to the parent channel.
    pub posted_to_channel: bool,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PromptDto {
    pub kind: String,
    pub always_allow: String,
    pub rule_words: Option<Vec<String>>,
    pub default_cut: Option<usize>,
    pub rule_host: Option<String>,
    pub always_allow_note: Option<String>,
    pub approval_id: String,
    pub tool: String,
    pub summary: String,
    pub view: String,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BranchRowDto {
    pub repository: String,
    pub key: String,
    pub label: String,
    pub kind: String,
    pub depth: u8,
    pub current: bool,
    pub collapsed: bool,
    pub has_children: bool,
    pub detached: bool,
    pub worktree: Option<String>,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDto {
    pub id: String,
    pub label: String,
    pub path: String,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiffColorsDto {
    pub added: String,
    pub removed: String,
    pub inline_added: String,
    pub inline_removed: String,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ThemeDto {
    pub mode: String,
    pub tokens: BTreeMap<String, String>,
    pub radius: Option<f32>,
    pub mono_font: Option<String>,
    pub font_scale: f32,
    pub spacing_scale: f32,
    pub protected_bg: String,
    pub protected_fg: String,
    pub protected_border: String,
    pub diff_colors: DiffColorsDto,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct SlotDto {
    pub plugin: String,
    pub element: Element,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PanelsDto {
    pub channel_list: f32,
    pub member_list: f32,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveNoticeDto {
    pub message: String,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BannersDto {
    pub review_count: usize,
    pub review_names: Vec<String>,
    pub problem_lines: Vec<String>,
    pub confirm: Option<String>,
    pub archive: Option<ArchiveNoticeDto>,
    pub safe_mode: bool,
}
/// Sandbox autorun for the running write response. Absent when the agent did not opt in.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AutorunDto {
    /// `on` or `unavailable`.
    pub state: String,
    pub reason: Option<String>,
}
impl AutorunDto {
    /// `None` when the session is not a write session or the agent did not opt in.
    pub fn from_profile(profile: &discard_core::ui_state::SessionProfile) -> Option<Self> {
        use discard_core::ui_state::{AutorunState, SessionProfile};
        match profile {
            SessionProfile::Workspace { autorun: AutorunState::On, .. } => Some(Self { state: "on".into(), reason: None }),
            SessionProfile::Workspace { autorun: AutorunState::Unavailable(reason), .. } => Some(Self { state: "unavailable".into(), reason: Some(reason.clone()) }),
            _ => None,
        }
    }
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    pub active_project_id: Option<String>,
    pub composer_disabled: bool,
    pub rev: u64,
    pub history_rev: u64,
    pub memory_rev: u64,
    pub project: String,
    pub project_id: String,
    pub project_path: String,
    pub project_display_path: String,
    pub channel_id: String,
    pub agent_id: String,
    pub channel: String,
    pub agent: String,
    pub provider: String,
    pub responder: Option<discard_core::model::MessageAuthor>,
    pub agent_status: String,
    pub turn_running: bool,
    pub autorun: Option<AutorunDto>,
    pub notice: Option<String>,
    pub live_blocks: Vec<BlockDto>,
    pub prompts: Vec<PromptDto>,
    pub pending_approvals: usize,
    pub unread: bool,
    pub new_anchor_id: Option<String>,
    pub archived: bool,
    pub archive_busy: bool,
    pub member_list_visible: bool,
    pub panels: PanelsDto,
    pub branches: Vec<BranchRowDto>,
    pub repositories: Vec<RepositoryDto>,
    pub repositories_loading: bool,
    pub channels: Vec<ChannelDto>,
    /// The sub-agent threads of the running turn of the selected channel, in task order. They have no anchor yet.
    /// When the turn ends they move to `MessageDto.subAgentThreads` of the saved response.
    pub live_sub_agent_threads: Vec<ThreadChipDto>,
    /// Threads with an approval waiting, followed or not, in the loaded channels.
    pub thread_approvals: Vec<ThreadApprovalDto>,
    pub open_thread: Option<OpenThreadDto>,
    pub slots: BTreeMap<String, SlotDto>,
    pub banners: BannersDto,
    pub theme: ThemeDto,
}

pub fn history_dto(messages: &[Message]) -> Vec<MessageDto> {
    history_dto_with_authors(messages, &std::collections::HashMap::new())
}

pub fn history_dto_with_authors(
    messages: &[Message],
    authors: &std::collections::HashMap<discard_core::model::MessageId, discard_core::model::MessageAuthor>,
) -> Vec<MessageDto> {
    let mut previous = None;
    let mut previous_author = None;
    messages.iter().map(|message| {
        let author = authors.get(&message.id).cloned();
        let author_id = author.as_ref().map(|a| a.agent_id.clone());
        let dto = MessageDto {
            id: message.id.0.clone(),
            created_at: uuid::Uuid::parse_str(&message.id.0).ok()
                .filter(|id| id.get_version_num() == 7)
                .and_then(|id| id.get_timestamp())
                .map(|timestamp| {
                    let (seconds, nanos) = timestamp.to_unix();
                    seconds * 1000 + u64::from(nanos) / 1_000_000
                }),
            role: message.role.as_str().into(),
            blocks: message.blocks.clone(),
            continues_group: continues_group(previous, message.role) && previous_author == author_id,
            author,
            thread: None,
            sub_agent_threads: Vec::new(),
            posted_from: None,
            posted_to_channel: false,
        };
        previous = Some(message.role);
        previous_author = author_id;
        dto
    }).collect()
}

#[cfg(test)]
fn sample_snapshot() -> SnapshotDto {
    SnapshotDto {
        active_project_id: Some("project".into()),
        composer_disabled: false,
        rev: 1,
        history_rev: 0,
        memory_rev: 0,
        project: "Example".into(),
        project_id: "project".into(),
        project_path: "/project".into(),
        project_display_path: "/project".into(),
        channel_id: "channel".into(),
        agent_id: "agent".into(),
        channel: "general".into(),
        agent: "Agent".into(),
        provider: "example".into(),
        responder: None,
        agent_status: "working".into(),
        turn_running: true,
        autorun: None,
        notice: None,
        live_blocks: vec![Block::Markdown {
            text: "Hello".into(),
        }],
        prompts: vec![],
        pending_approvals: 0,
        unread: false,
        new_anchor_id: None,
        archived: false,
        archive_busy: false,
        member_list_visible: true,
        panels: PanelsDto {
            channel_list: 240.0,
            member_list: 200.0,
        },
        branches: vec![],
        repositories: vec![],
        repositories_loading: false,
        channels: vec![],
        live_sub_agent_threads: vec![],
        thread_approvals: vec![],
        open_thread: None,
        slots: BTreeMap::new(),
        banners: BannersDto {
            review_count: 0,
            review_names: vec![],
            problem_lines: vec![],
            confirm: None,
            archive: None,
            safe_mode: false,
        },
        theme: ThemeDto {
            mono_font: None,
            mode: "dark".into(),
            tokens: BTreeMap::new(),
            radius: None,
            font_scale: 1.0,
            spacing_scale: 1.0,
            protected_bg: "#000000".into(),
            protected_fg: "#ffffff".into(),
            protected_border: "#ffffff".into(),
            diff_colors: DiffColorsDto { added: "#ffffff".into(), removed: "#ffffff".into(), inline_added: "#ffffff".into(), inline_removed: "#ffffff".into() },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::model::{ChannelId, Message, MessageId, Role};

    #[test]
    fn history_dto_marks_continued_groups() {
        let messages: Vec<_> = [Role::User, Role::Agent, Role::Agent]
            .into_iter()
            .map(|role| Message {
                id: MessageId::new(),
                channel_id: ChannelId::new(),
                role,
                blocks: vec![],
            })
            .collect();
        assert_eq!(
            history_dto(&messages)
                .iter()
                .map(|m| m.continues_group)
                .collect::<Vec<_>>(),
            [false, false, true]
        );
    }

#[test]
    fn different_reply_authors_start_new_groups() {
        use discard_core::model::{AgentId, MessageAuthor};
        let messages: Vec<_> = (0..3).map(|_| Message { id: MessageId::new(), channel_id: ChannelId::new(), role: Role::Agent, blocks: vec![] }).collect();
        let quinn = MessageAuthor { agent_id: AgentId::new(), name: "Quinn".into(), provider: "test".into(), avatar: None };
        let jordan = MessageAuthor { agent_id: AgentId::new(), name: "Jordan".into(), provider: "test".into(), avatar: None };
        let authors = [(messages[0].id.clone(), quinn), (messages[1].id.clone(), jordan.clone()), (messages[2].id.clone(), jordan)].into_iter().collect();
        let rows = history_dto_with_authors(&messages, &authors);
        assert_eq!(rows.iter().map(|m| m.continues_group).collect::<Vec<_>>(), [false, false, true]);
        assert_eq!(rows[0].author.as_ref().unwrap().name, "Quinn");
        assert_eq!(rows[1].author.as_ref().unwrap().name, "Jordan");
    }

    #[test]
    fn history_exposes_uuid_v7_creation_time_without_inventing_legacy_dates() {
        let messages: Vec<_> = [
            "01890f3e-4c00-7000-8000-000000000000",
            "01890f3e-4c00-4000-8000-000000000000",
            "legacy-message",
        ].into_iter().map(|id| Message {
            id: MessageId(id.into()),
            channel_id: ChannelId::new(),
            role: Role::User,
            blocks: vec![],
        }).collect();
        let rows = history_dto(&messages);
        assert_eq!(rows[0].created_at, Some(0x0189_0f3e_4c00));
        assert_eq!(rows[1].created_at, None);
        assert_eq!(rows[2].created_at, None);
        assert_eq!(serde_json::to_value(&rows[0]).unwrap()["createdAt"], 0x0189_0f3e_4c00u64);
    }

    #[test]
    fn block_dto_serializes_with_type_tag() {
        let block = BlockDto::Markdown {
            text: "hello".into(),
        };
        assert_eq!(serde_json::to_value(block).unwrap()["type"], "markdown");
    }

    fn golden(name: &str, actual: String) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/bridge/fixtures").join(name);
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, &actual).unwrap();
        }
        assert_eq!(actual, std::fs::read_to_string(path).unwrap());
    }

    #[test]
    fn thread_payloads_match_golden_file() {
        let message = |id: &str, role: &str, text: &str| MessageDto {
            id: id.into(), role: role.into(), blocks: vec![Block::Markdown { text: text.into() }], continues_group: false, created_at: None,
            author: None, thread: None, sub_agent_threads: vec![], posted_from: None, posted_to_channel: false,
        };
        let mut anchor = message("answer", "agent", "A thread is a side conversation.");
        anchor.thread = Some(ThreadChipDto {
            channel_id: "thread".into(), name: "Why a thread?".into(), message_count: 2,
            last_message: Some(ThreadLastMessageDto { role: "agent".into(), author_name: Some("Agent".into()), preview: "It keeps the channel clean.".into() }),
            followed: true, unread: true, kind: "question".into(), state: None, agent_name: "Agent".into(), pending_approvals: 0,
        });
        let mut reference = message("reference", "agent", "It keeps the channel clean.");
        reference.posted_from = Some(PostedFromDto { source_message_id: "reply".into(), thread_channel_id: "thread".into(), thread_name: "Why a thread?".into() });
        let mut posted = message("reply", "agent", "It keeps the channel clean.");
        posted.posted_to_channel = true;
        let mut snapshot = sample_snapshot();
        snapshot.channels = vec![ChannelDto {
            agent_id: "agent".into(), project_id: Some("project".into()), id: "channel".into(), name: "general".into(), agent: "Agent".into(), provider: "example".into(),
            workspace_mode: "main_checkout".into(), running: false, unread: false, pending_approvals: 0, thread_pending_approvals: 0,
            threads: vec![ThreadLinkDto { id: "thread".into(), parent_channel_id: "channel".into(), name: "Why a thread?".into(), unread: true, running: false, pending_approvals: 0, kind: "question".into(), state: None }],
        }];
        snapshot.open_thread = Some(OpenThreadDto {
            channel_id: "thread".into(), parent_channel_id: "channel".into(), name: "Why a thread?".into(), followed: true, kind: "question".into(), state: None, agent_name: "Agent".into(), anchor_message_id: Some("answer".into()), history_rev: 4,
            agent_id: "agent".into(), agent: "Agent".into(), provider: "example".into(), responder: None, agent_status: "idle".into(), turn_running: false, notice: None,
            live_blocks: vec![], prompts: vec![], pending_approvals: 0, unread: true, composer_disabled: false, composer_disabled_reason: None,
            read_only_note: discard_core::workspace_session::ReadOnlyReason::Thread.note(),
        });
        let value = serde_json::json!({
            "snapshot": snapshot,
            "history": HistoryDto { channel_id: "channel".into(), rev: 2, messages: vec![anchor.clone(), reference] },
            "threadHistory": ThreadHistoryDto { channel_id: "thread".into(), parent_channel_id: "channel".into(), rev: 4, anchor: Some(anchor), messages: vec![message("ask", "user", "Why a thread?"), posted] },
        });
        golden("thread.json", serde_json::to_string_pretty(&value).unwrap() + "\n");
    }

    #[test]
    fn sub_agent_payloads_match_golden_file() {
        let message = |id: &str, role: &str, text: &str| MessageDto {
            id: id.into(), role: role.into(), blocks: vec![Block::Markdown { text: text.into() }], continues_group: false, created_at: None,
            author: None, thread: None, sub_agent_threads: vec![], posted_from: None, posted_to_channel: false,
        };
        let chip = |id: &str, name: &str, state: &str, preview: &str, count: usize| ThreadChipDto {
            channel_id: id.into(), name: name.into(), message_count: count,
            last_message: (count > 1).then(|| ThreadLastMessageDto { role: "agent".into(), author_name: Some("Reviewer".into()), preview: preview.into() }),
            followed: false, unread: count > 1, kind: "sub_agent".into(), state: Some(state.into()), agent_name: "Reviewer".into(), pending_approvals: 0,
        };
        let link = |id: &str, name: &str, state: &str, pending: usize| ThreadLinkDto {
            id: id.into(), parent_channel_id: "channel".into(), name: name.into(), unread: false, running: state == "running", pending_approvals: pending, kind: "sub_agent".into(), state: Some(state.into()),
        };
        // Two sub-agent threads run now. They have no anchor: the snapshot carries their chips.
        let mut snapshot = sample_snapshot();
        snapshot.live_sub_agent_threads = vec![chip("find-callers", "Find callers", "running", "", 1), chip("read-docs", "Read the docs", "done", "The docs say two things.", 2)];
        snapshot.channels = vec![ChannelDto {
            agent_id: "agent".into(), project_id: Some("project".into()), id: "channel".into(), name: "general".into(), agent: "Agent".into(), provider: "example".into(),
            workspace_mode: "main_checkout".into(), running: true, unread: false, pending_approvals: 0, thread_pending_approvals: 1,
            threads: vec![link("find-callers", "Find callers", "running", 1)],
        }];
        snapshot.thread_approvals = vec![ThreadApprovalDto { thread_id: "find-callers".into(), parent_channel_id: "channel".into(), name: "Find callers".into(), kind: "sub_agent".into(), agent_name: "Reviewer".into(), pending_approvals: 1 }];
        snapshot.open_thread = Some(OpenThreadDto {
            channel_id: "find-callers".into(), parent_channel_id: "channel".into(), name: "Find callers".into(), followed: true, kind: "sub_agent".into(), state: Some("running".into()), agent_name: "Reviewer".into(),
            anchor_message_id: None, history_rev: 3, agent_id: "reviewer".into(), agent: "Reviewer".into(), provider: "example".into(), responder: None, agent_status: "awaiting_approval".into(), turn_running: true, notice: None,
            live_blocks: vec![], prompts: vec![], pending_approvals: 1, unread: false, composer_disabled: false, composer_disabled_reason: None,
            read_only_note: discard_core::workspace_session::ReadOnlyReason::Thread.note(),
        });
        // After the turn the chips hang under the saved response, in task order.
        let mut response = message("response", "agent", "Both are done.");
        response.sub_agent_threads = vec![chip("find-callers", "Find callers", "done", "Callers: a, b.", 2), chip("read-docs", "Read the docs", "failed", "", 1)];
        let request = AgentRequestDto {
            id: "request".into(), channel_id: "channel".into(), requesting_agent_id: "agent".into(), requesting_agent_name: "Agent".into(), agent_id: "reviewer".into(), agent_name: "Reviewer".into(),
            provider: "example".into(), model: Some("model".into()), reason: "It reads code well.".into(), state: "pending".into(), result: None, created_at: 1_700_000_000_000,
        };
        let value = serde_json::json!({
            "snapshot": snapshot,
            "history": HistoryDto { channel_id: "channel".into(), rev: 5, messages: vec![message("ask", "user", "Research this."), response] },
            "threadHistory": ThreadHistoryDto { channel_id: "find-callers".into(), parent_channel_id: "channel".into(), rev: 3, anchor: None, messages: vec![message("brief", "user", "Find the callers of foo.")] },
            "agentRequests": vec![request],
        });
        golden("subagent.json", serde_json::to_string_pretty(&value).unwrap() + "\n");
    }

    #[test]
    fn sample_snapshot_matches_golden_file() {
        let snapshot = sample_snapshot();
        let actual = serde_json::to_string_pretty(&snapshot).unwrap() + "\n";
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../web/src/bridge/fixtures/snapshot.json");
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, &actual).unwrap();
        }
        assert_eq!(actual, std::fs::read_to_string(path).unwrap());
    }
}

/// A context menu target sent by the webview.
#[derive(serde::Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ContextTargetDto {
    Message { id: String, has_text: bool },
    CodeBlock { message_id: String, block: usize },
    DiffBlock { message_id: String, block: usize },
    ToolCall { message_id: String, block: usize, has_output: bool },
    Channel { id: String },
    Agent { id: String, name: String },
    Server { id: String, name: String, path: String },
}
impl From<ContextTargetDto> for discard_core::context_menu::ContextTarget {
    fn from(target: ContextTargetDto) -> Self {
        use discard_core::context_menu::ContextTarget as T;
        match target {
            ContextTargetDto::Message { id, has_text } => T::Message { id, has_text },
            ContextTargetDto::CodeBlock { message_id, block } => T::CodeBlock { message_id, block },
            ContextTargetDto::DiffBlock { message_id, block } => T::DiffBlock { message_id, block },
            ContextTargetDto::ToolCall { message_id, block, has_output } => T::ToolCall { message_id, block, has_output },
            ContextTargetDto::Channel { id } => T::Channel { id },
            ContextTargetDto::Agent { id, name } => T::Agent { id, name },
            ContextTargetDto::Server { id, name, path } => T::Server { id, name, path },
        }
    }
}
/// A context menu entry for the webview.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MenuEntryDto {
    Action { id: String, label: String, enabled: bool },
    Plugin { plugin: String, index: usize, label: String },
    Separator,
}
impl MenuEntryDto {
    pub fn from_entry(entry: discard_core::action::MenuEntry) -> Option<Self> {
        use discard_core::action::MenuEntry;
        match entry {
            MenuEntry::Action { action, label, enabled, .. } => Some(Self::Action { id: action.id().into(), label, enabled }),
            MenuEntry::Plugin { plugin, index, label } => Some(Self::Plugin { plugin, index, label }),
            MenuEntry::Separator => Some(Self::Separator),
            _ => None,
        }
    }
}

/// A request from an agent to add another agent to the project. The user decides on a card in the parent channel.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AgentRequestDto {
    pub id: String,
    pub channel_id: String,
    pub requesting_agent_id: String,
    pub requesting_agent_name: String,
    /// The agent to add.
    pub agent_id: String,
    pub agent_name: String,
    pub provider: String,
    pub model: Option<String>,
    /// What the requesting agent wrote. At most 500 characters.
    pub reason: String,
    /// `pending`, `added`, `denied`, `failed` or `expired`.
    pub state: String,
    pub result: Option<String>,
    /// Unix milliseconds.
    pub created_at: u64,
}
impl From<discard_core::agent_requests::AgentAddRequest> for AgentRequestDto {
    fn from(request: discard_core::agent_requests::AgentAddRequest) -> Self {
        Self {
            id: request.id, channel_id: request.channel_id, requesting_agent_id: request.requesting_agent_id, requesting_agent_name: request.requesting_agent_name,
            agent_id: request.agent_id, agent_name: request.agent_name, provider: request.provider, model: request.model, reason: request.reason,
            state: request.state, result: request.result, created_at: request.created_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleDto {
    pub id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub kind: String,
    pub text: String,
    pub created_at: i64,
}
