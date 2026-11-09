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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectChannelKind {
    General,
    Branch,
    ArchivedLegacy,
    /// Active project channel created by the retained workspace bootstrap path.
    Legacy,
}

impl ProjectChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Branch => "branch",
            Self::ArchivedLegacy => "archived_legacy",
            Self::Legacy => "legacy",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "general" => Some(Self::General),
            "branch" => Some(Self::Branch),
            "archived_legacy" => Some(Self::ArchivedLegacy),
            "legacy" => Some(Self::Legacy),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectChannelMetadata {
    pub repository: String,
    pub project_id: ProjectId,
    pub channel_id: ChannelId,
    pub kind: ProjectChannelKind,
    pub branch: Option<String>,
    pub archived_into: Option<ChannelId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelTopic {
    pub channel_id: ChannelId,
    pub text: String,
    pub version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicAuthorKind {
    User,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelTopicRevision {
    pub channel_id: ChannelId,
    pub text: String,
    pub version: u64,
    pub author_kind: TopicAuthorKind,
    pub author_id: Option<AgentId>,
}

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
    pub project_id: Option<ProjectId>,
    pub name: String,
    pub agent_id: AgentId,
    pub workspace_mode: WorkspaceMode,
}

/// A message is a list of typed blocks. Plugins may add block types later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Changes { root: String, state: crate::workspace_changes::ChangesState, commits: Vec<crate::workspace_changes::ChangedCommit>, files: Vec<crate::workspace_changes::ChangedFile>, truncated: bool },
    Attachment { attachment: crate::attachments::Attachment },
    Markdown {
        text: String,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    Diff {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        input: serde_json::Value,
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    /// Rendered by Rust. Not overridable by plugins.
    Approval {
        #[serde(default)]
        kind: crate::approval::ApprovalKind,
        approval_id: String,
        tool: String,
        summary: String,
    },
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageAuthor {
    pub agent_id: AgentId,
    pub name: String,
    pub provider: String,
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub role: Role,
    pub blocks: Vec<Block>,
}
