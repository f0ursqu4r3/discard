//! Plain data types shared by storage and the UI.

use serde::{Deserialize, Serialize};

macro_rules! text_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);
        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::now_v7().to_string())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

text_id!(ProjectId);
text_id!(AgentId);
text_id!(ChannelId);
text_id!(MessageId);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: String,
}

/// Where a channel's agent works. Recorded when the channel is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    /// The project's main checkout. All channels created before worktree support use this.
    MainCheckout,
    /// A per-channel git worktree (created in milestone M6).
    Worktree,
}

impl WorkspaceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkspaceMode::MainCheckout => "main_checkout",
            WorkspaceMode::Worktree => "worktree",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "main_checkout" => Some(Self::MainCheckout),
            "worktree" => Some(Self::Worktree),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: ChannelId,
    pub project_id: ProjectId,
    pub name: String,
    pub agent_id: AgentId,
    pub workspace_mode: WorkspaceMode,
}

/// A message is a list of typed blocks. Plugins may add block types later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Markdown { text: String },
    Code { language: Option<String>, text: String },
    Diff { text: String },
    ToolCall { call_id: String, name: String, input: serde_json::Value, output: Option<String> },
    /// Rendered by Rust. Not overridable by plugins.
    Approval { approval_id: String, tool: String, summary: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Agent,
    System,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Agent => "agent",
            Role::System => "system",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "agent" => Some(Self::Agent),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub role: Role,
    pub blocks: Vec<Block>,
}
