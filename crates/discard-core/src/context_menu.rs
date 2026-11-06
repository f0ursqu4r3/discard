//! Pure context menu models and clipboard content resolution.
use crate::action::{append_plugin_items, Action, MenuContext, MenuEntry, MenuTarget};
use crate::model::{Block, Message};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextTarget {
    DiffBlock { message_id:String, block:usize },
    ToolCall { message_id:String, block:usize, has_output:bool },
    Agent { id:String, name:String },
    Server { id:String, name:String, path:String },
    Message { id: String, has_text: bool },
    CodeBlock { message_id: String, block: usize },
    Channel { id: String },
}

impl ContextTarget {
    pub fn kind(&self) -> MenuTarget {
        match self { Self::DiffBlock { .. } => MenuTarget::Diff, Self::ToolCall { .. } => MenuTarget::ToolCall, Self::Agent { .. } => MenuTarget::Agent, Self::Server { .. } => MenuTarget::Server, Self::Message { .. } => MenuTarget::Message, Self::CodeBlock { .. } => MenuTarget::CodeBlock, Self::Channel { .. } => MenuTarget::Channel }
    }

    pub fn target_id(&self) -> String {
        match self { Self::Agent { id, .. } | Self::Server { id, .. } | Self::Message { id, .. } | Self::Channel { id } => id.clone(), Self::DiffBlock { message_id, block } | Self::ToolCall { message_id, block, .. } | Self::CodeBlock { message_id, block } => format!("{message_id}:{block}") }
    }
}

pub fn context_menu(target: &ContextTarget, ctx: &MenuContext) -> Vec<MenuEntry> {
    let entry = |label: &str, action: Action, enabled: bool| MenuEntry::Action { label: label.into(), action, checked: false, enabled };
    let mut entries = match target {
        ContextTarget::DiffBlock { .. } => vec![entry("Copy Diff", Action::CopyDiff, true)],
        ContextTarget::ToolCall { has_output, .. } => vec![entry("Copy Tool Input", Action::CopyToolInput, true),entry("Copy Tool Output", Action::CopyToolOutput, *has_output)],
        ContextTarget::Agent { .. } => vec![entry("Copy Agent Name", Action::CopyAgentName, true),entry("View Profile", Action::OpenAgentSettings, true)],
        ContextTarget::Server { .. } => vec![entry("Copy Project Name", Action::CopyProjectName, true),entry("Copy Project Path", Action::CopyProjectPath, true),entry("Workspace Settings…", Action::OpenWorkspaceSettings, true)],
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

pub fn resolve_copy(messages: &[Message], channel_name: &str, target: &ContextTarget, action: Action) -> Option<String> {
    match target {
        ContextTarget::Agent { name, .. } if action == Action::CopyAgentName => Some(name.clone()),
        ContextTarget::Server { name, .. } if action == Action::CopyProjectName => Some(name.clone()),
        ContextTarget::Server { path, .. } if action == Action::CopyProjectPath => Some(path.clone()),
        ContextTarget::DiffBlock { message_id, block } => match messages.iter().find(|m|m.id.0==*message_id)?.blocks.get(*block)? { Block::Diff { text } if action==Action::CopyDiff => Some(text.clone()), _=>None },
        ContextTarget::ToolCall { message_id, block, .. } => match messages.iter().find(|m|m.id.0==*message_id)?.blocks.get(*block)? { Block::ToolCall { input, .. } if action==Action::CopyToolInput => serde_json::to_string_pretty(input).ok(), Block::ToolCall { output, .. } if action==Action::CopyToolOutput => output.clone(), _=>None },
        ContextTarget::Agent { .. } | ContextTarget::Server { .. } => None,
        ContextTarget::Channel { .. } if action==Action::CopyChannelName => Some(channel_name.into()),
        ContextTarget::Message { id, .. } if action==Action::CopyMessageText => messages.iter().find(|message| message.id.0 == *id).map(|message| message_text(&message.blocks)),
        ContextTarget::CodeBlock { message_id, block } if action==Action::CopyCode => match messages.iter().find(|message| message.id.0 == *message_id)?.blocks.get(*block)? {
            Block::Code { text, .. } => Some(text.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, MenuContext, MenuEntry, MenuTarget, PluginMenuItem};
    use crate::model::Block;

    #[test] fn diff_block_menu_is_copy_diff_only() { let t=ContextTarget::DiffBlock{message_id:"m".into(),block:2}; assert_eq!(t.target_id(),"m:2"); assert!(matches!(&context_menu(&t,&MenuContext::default())[..],[MenuEntry::Action{action:Action::CopyDiff,..}])); }
    #[test] fn tool_call_menu_greys_copy_output_until_there_is_output() { let t=ContextTarget::ToolCall{message_id:"m".into(),block:1,has_output:false}; assert!(matches!(&context_menu(&t,&MenuContext::default())[..],[_,MenuEntry::Action{action:Action::CopyToolOutput,enabled:false,..}])); }
    #[test] fn agent_menu_lists_copy_name_then_agent_settings() { let t=ContextTarget::Agent{id:"a".into(),name:"Agent".into()}; assert_eq!(t.kind(),MenuTarget::Agent); assert_eq!(context_menu(&t,&MenuContext::default()).len(),2); }
    #[test] fn server_menu_lists_copy_name_copy_path_then_workspace_settings() { let t=ContextTarget::Server{id:"p".into(),name:"P".into(),path:"/p".into()}; assert_eq!(t.kind(),MenuTarget::Server); assert_eq!(context_menu(&t,&MenuContext::default()).len(),3); }
    #[test]
    fn resolve_copy_finds_diff_tool_input_and_tool_output_and_none_for_the_wrong_block_type() {
        use crate::model::{ChannelId,MessageId,Role};
        let messages=vec![Message{id:MessageId("m".into()),channel_id:ChannelId("c".into()),role:Role::User,blocks:vec![Block::Diff{text:"diff".into()},Block::ToolCall{call_id:"c".into(),name:"run".into(),input:serde_json::json!({"a":1}),output:Some("done".into())}]}];
        let diff=ContextTarget::DiffBlock{message_id:"m".into(),block:0}; let tool=ContextTarget::ToolCall{message_id:"m".into(),block:1,has_output:true};
        assert_eq!(resolve_copy(&messages,"",&diff,Action::CopyDiff).as_deref(),Some("diff"));
        assert_eq!(resolve_copy(&messages,"",&tool,Action::CopyToolOutput).as_deref(),Some("done"));
        assert!(resolve_copy(&messages,"",&tool,Action::CopyToolInput).unwrap().contains("\"a\": 1"));
        assert_eq!(resolve_copy(&messages,"",&diff,Action::CopyToolInput),None);
    }
    #[test] fn resolve_copy_refuses_an_action_for_the_wrong_target() { let t=ContextTarget::Channel{id:"c".into()}; assert_eq!(resolve_copy(&[],"general",&t,Action::CopyDiff),None); }
    #[test]
    fn target_ids_for_the_new_targets() { let cases=[(ContextTarget::DiffBlock{message_id:"m".into(),block:1},"m:1"),(ContextTarget::ToolCall{message_id:"m".into(),block:2,has_output:false},"m:2"),(ContextTarget::Agent{id:"a".into(),name:"n".into()},"a"),(ContextTarget::Server{id:"s".into(),name:"n".into(),path:"/".into()},"s")]; for (target,id) in cases {assert_eq!(target.target_id(),id);} }
    #[test]
    fn plugin_items_follow_each_new_menu_after_a_separator() { for target in [ContextTarget::DiffBlock{message_id:"m".into(),block:0},ContextTarget::ToolCall{message_id:"m".into(),block:0,has_output:false},ContextTarget::Agent{id:"a".into(),name:"n".into()},ContextTarget::Server{id:"s".into(),name:"n".into(),path:"/".into()}] {let ctx=MenuContext {plugin_items:vec![PluginMenuItem{plugin:"p".into(),index:0,target:target.kind(),label:"Run".into()}],..Default::default()};let items=context_menu(&target,&ctx);assert!(matches!(&items[items.len()-2..],[MenuEntry::Separator,MenuEntry::Plugin{..}]));} }
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
        assert_eq!(resolve_copy(&messages, "general", &message, Action::CopyMessageText).as_deref(), Some("text\n\ncode\n\ndiff"));
        assert_eq!(resolve_copy(&messages, "general", &ContextTarget::Channel { id: "c".into() }, Action::CopyChannelName).as_deref(), Some("general"));
        for (index, expected) in [(0, None), (1, Some("code")), (2, None), (3, None)] {
            assert_eq!(resolve_copy(&messages, "general", &ContextTarget::CodeBlock { message_id: "m".into(), block: index }, Action::CopyCode).as_deref(), expected);
        }
        assert_eq!(resolve_copy(&messages, "general", &ContextTarget::Message { id: "unknown".into(), has_text: true }, Action::CopyMessageText), None);
    }
}
