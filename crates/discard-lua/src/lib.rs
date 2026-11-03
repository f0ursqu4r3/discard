//! Luau plugin host: manifests, snapshots, element trees, theme tokens, and the sandboxed runtime.

pub mod element;
pub mod manifest;
pub mod snapshot;
pub mod theme;
pub mod queue;
pub mod runtime;

pub use runtime::{HookResult, LoadedInfo, PluginError, PluginHandle, PluginLimits, Status, Violation};
pub mod manager;

pub use manager::{PluginManager, Rendered};
pub mod bundled;
