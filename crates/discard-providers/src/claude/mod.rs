//! Claude provider: wraps the official `claude` CLI in stream-json mode.

mod api;
mod provider;
mod text;
pub mod parse;

pub use provider::{ClaudeProvider, build_args};
