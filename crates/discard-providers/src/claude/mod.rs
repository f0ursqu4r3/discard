//! Claude provider: wraps the official `claude` CLI in stream-json mode.

mod provider;
pub mod parse;

pub use provider::{ClaudeProvider, build_args};
