//! Luau plugin host: manifests, snapshots, element trees, theme tokens, and the sandboxed runtime.

pub mod element;
pub mod manifest;
pub mod snapshot;
pub mod theme;
pub mod queue;
pub mod runtime;

pub use runtime::{HookResult, LoadedInfo, PluginError, PluginHandle, PluginLimits, Status, Violation};
pub mod host;
pub mod manager;

pub use manager::{PluginManager, Rendered};
pub use host::{FsAccess, Host, MemoryFile, MemoryHit, MemoryHost, MemoryText};
pub mod bundled;
pub mod api_spec;
pub mod keystore;
pub mod ledger;
pub mod drafts;
pub mod repo;
pub mod service;

pub use service::{LoadReport, PluginService, PluginState, Review, ReviewKind, ServiceError, TestReport};
