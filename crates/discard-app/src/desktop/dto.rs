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
    pub id: String, pub name: String, pub agent: String, pub provider: String,
    pub workspace_mode: String, pub running: bool, pub unread: bool, pub pending_approvals: usize,
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
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PromptDto {
    pub approval_id: String,
    pub tool: String,
    pub summary: String,
    pub view: String,
}
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BranchRowDto {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub depth: u8,
    pub current: bool,
    pub collapsed: bool,
    pub has_children: bool,
    pub detached: bool,
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
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    pub rev: u64,
    pub history_rev: u64,
    pub project: String,
    pub project_id: String,
    pub project_path: String,
    pub channel_id: String,
    pub agent_id: String,
    pub channel: String,
    pub agent: String,
    pub provider: String,
    pub agent_status: String,
    pub turn_running: bool,
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
    pub channels: Vec<ChannelDto>,
    pub slots: BTreeMap<String, SlotDto>,
    pub banners: BannersDto,
    pub theme: ThemeDto,
}

pub fn history_dto(messages: &[Message]) -> Vec<MessageDto> {
    let mut previous = None;
    messages
        .iter()
        .map(|message| {
            let dto = MessageDto {
                id: message.id.0.clone(),
                role: message.role.as_str().into(),
                blocks: message.blocks.clone(),
                continues_group: continues_group(previous, message.role),
            };
            previous = Some(message.role);
            dto
        })
        .collect()
}

#[cfg(test)]
fn sample_snapshot() -> SnapshotDto {
    SnapshotDto {
        rev: 1,
        history_rev: 0,
        project: "Example".into(),
        project_id: "project".into(),
        project_path: "/project".into(),
        channel_id: "channel".into(),
        agent_id: "agent".into(),
        channel: "general".into(),
        agent: "Agent".into(),
        provider: "example".into(),
        agent_status: "working".into(),
        turn_running: true,
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
        channels: vec![],
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
    fn block_dto_serializes_with_type_tag() {
        let block = BlockDto::Markdown {
            text: "hello".into(),
        };
        assert_eq!(serde_json::to_value(block).unwrap()["type"], "markdown");
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
