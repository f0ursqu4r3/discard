//! Local MCP server that exposes memory tools to provider sessions.

pub mod identity;
pub mod tools;
pub mod server;

pub use server::McpServer;
pub mod service;

pub use service::MemoryRuntime;
