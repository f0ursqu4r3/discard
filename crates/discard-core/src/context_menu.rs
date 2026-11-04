//! Pure context menu models and clipboard content resolution.
use crate::action::{append_plugin_items, Action, MenuContext, MenuEntry, MenuTarget};
use crate::model::{Block, Message};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextTarget {
    Message { id: String, has_text: bool },
    CodeBlock { message_id: String, block: usize },
    Channel { id: String },
}

impl ContextTarget {
    pub fn kind(&self) -> MenuTarget {
        match self { Self::Message { .. } => MenuTarget::Message, Self::CodeBlock { .. } => MenuTarget::CodeBlock, Self::Channel { .. } => MenuTarget::Channel }
    }

    pub fn target_id(&self) -> String {
        match self { Self::Message { id, .. } | Self::Channel { id } => id.clone(), Self::CodeBlock { message_id, block } => format!("{message_id}:{block}") }
    }
}

pub fn context_menu(target: &ContextTarget, ctx: &MenuContext) -> Vec<MenuEntry> {
    let entry = |label: &str, action: Action, enabled: bool| MenuEntry::Action { label: label.into(), action, checked: false, enabled };
    let mut entries = match target {
        ContextTarget::Message { has_text, .. } => vec![entry("Copy Message Text", Action::CopyMessageText, *has_text)],
        ContextTarget::CodeBlock { .. } => vec![entry("Copy Code", Action::CopyCode, true)],
        ContextTarget::Channel { .. } => vec![entry("Copy Channel Name", Action::CopyChannelName, true), MenuEntry::Separator, entry("Archive Channel…", Action::ArchiveChannel, !ctx.turn_running && !ctx.archived)],
    };
    append_plugin_items(&mut entries, ctx, target.kind());
    entries
}

pub fn message_text(blocks: &[Block]) -> String {
    blocks.iter().filter_map(|block| match block { Block::Markdown { text } | Block::Code { text, .. } | Block::Diff { text } => Some(text.as_str()), Block::ToolCall { .. } | Block::Approval { .. } => None }).collect::<Vec<_>>().join("\n\n")
}

pub fn resolve_copy(messages: &[Message], channel_name: &str, target: &ContextTarget) -> Option<String> {
    match target {
        ContextTarget::Channel { .. } => Some(channel_name.into()),
        ContextTarget::Message { id, .. } => messages.iter().find(|message| message.id.0 == *id).map(|message| message_text(&message.blocks)),
        ContextTarget::CodeBlock { message_id, block } => match messages.iter().find(|message| message.id.0 == *message_id)?.blocks.get(*block)? {
            Block::Code { text, .. } => Some(text.clone()),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, MenuContext, MenuEntry, MenuTarget, PluginMenuItem};
    use crate::model::Block;

    #[test]
    fn message_menu_has_copy_and_it_is_disabled_without_text() {
        let menu = context_menu(&ContextTarget::Message { id: "m".into(), has_text: false }, &MenuContext::default());
        assert!(matches!(&menu[..], [MenuEntry::Action { action: Action::CopyMessageText, enabled: false, .. }]));
    }

    #[test]
    fn code_block_menu_is_copy_code_only() {
        let target = ContextTarget::CodeBlock { message_id: "m".into(), block: 3 };
        assert_eq!(target.target_id(), "m:3");
        assert!(matches!(&context_menu(&target, &MenuContext::default())[..], [MenuEntry::Action { action: Action::CopyCode, enabled: true, .. }]));
    }

    #[test]
    fn channel_menu_lists_copy_name_then_archive() {
        let ctx = MenuContext { turn_running: true, ..Default::default() };
        let target = ContextTarget::Channel { id: "c".into() };
        assert!(matches!(&context_menu(&target, &ctx)[..], [MenuEntry::Action { action: Action::CopyChannelName, .. }, MenuEntry::Separator, MenuEntry::Action { action: Action::ArchiveChannel, enabled: false, .. }]));
    }

    #[test]
    fn plugin_items_come_last_after_a_separator_and_other_targets_are_not_listed() {
        let ctx = MenuContext { plugin_items: vec![PluginMenuItem { plugin: "p".into(), index: 0, target: MenuTarget::Channel, label: "Run".into() }], ..Default::default() };
        let message = ContextTarget::Message { id: "m".into(), has_text: true };
        assert_eq!(context_menu(&message, &ctx).len(), 1);
        let channel = ContextTarget::Channel { id: "c".into() };
        assert!(matches!(&context_menu(&channel, &ctx)[3..], [MenuEntry::Separator, MenuEntry::Plugin { .. }]));
    }

    #[test]
    fn message_text_joins_text_blocks_and_skips_tool_calls_and_approvals() {
        let blocks = vec![Block::Markdown { text: "hello".into() }, Block::Code { language: None, text: "code".into() }, Block::Diff { text: "diff".into() }, Block::ToolCall { call_id: "c".into(), name: "n".into(), input: serde_json::Value::Null, output: None }, Block::Approval { approval_id: "a".into(), tool: "t".into(), summary: "s".into() }];
        assert_eq!(message_text(&blocks), "hello\n\ncode\n\ndiff");
    }

    #[test]
    fn resolve_copy_finds_message_code_and_channel_name() {
        use crate::model::{ChannelId, Message, MessageId, Role};
        let messages = vec![Message { id: MessageId("m".into()), channel_id: ChannelId("c".into()), role: Role::User, blocks: vec![Block::Markdown { text: "text".into() }, Block::Code { language: None, text: "code".into() }, Block::Diff { text: "diff".into() }] }];
        let message = ContextTarget::Message { id: "m".into(), has_text: true };
        assert_eq!(resolve_copy(&messages, "general", &message).as_deref(), Some("text\n\ncode\n\ndiff"));
        assert_eq!(resolve_copy(&messages, "general", &ContextTarget::Channel { id: "c".into() }).as_deref(), Some("general"));
        for (index, expected) in [(0, None), (1, Some("code")), (2, None), (3, None)] {
            assert_eq!(resolve_copy(&messages, "general", &ContextTarget::CodeBlock { message_id: "m".into(), block: index }).as_deref(), expected);
        }
        assert_eq!(resolve_copy(&messages, "general", &ContextTarget::Message { id: "unknown".into(), has_text: true }), None);
    }
}
