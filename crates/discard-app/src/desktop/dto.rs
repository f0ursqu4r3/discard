//! Data sent to the webview.
use discard_core::model::{Block, Message};
use discard_core::ui_state::continues_group;
use discard_lua::element::Element;
use serde::Serialize;
use std::collections::BTreeMap;

pub type BlockDto = Block;

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
    pub archived: bool,
    pub archive_busy: bool,
    pub member_list_visible: bool,
    pub panels: PanelsDto,
    pub branches: Vec<BranchRowDto>,
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
        archived: false,
        archive_busy: false,
        member_list_visible: true,
        panels: PanelsDto {
            channel_list: 240.0,
            member_list: 200.0,
        },
        branches: vec![],
        slots: BTreeMap::new(),
        banners: BannersDto {
            review_count: 0,
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
