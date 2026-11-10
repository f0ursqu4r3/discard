//! Codex provider: wraps the official `codex app-server` (JSON-RPC over stdio).

pub mod parse;
mod provider;
mod text;

pub use provider::{CodexProvider, thread_params};
