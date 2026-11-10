//! Headless core of Discard: events, turn and approval state machines,
//! policy, storage, and the provider trait. No GUI dependency.

pub mod attachments;
pub mod action;
pub mod context_menu;
pub mod provider_page;
pub mod agent_edit;
pub mod event;
pub mod approval;
pub mod turn;
pub mod capability;
pub mod agent;
pub mod model;
pub mod store;
pub mod driver;
pub mod provider;
pub mod personality;
pub mod redact;
pub mod logging;
pub mod timeline;
pub mod ui_state;
pub mod engine;
pub mod observer;
pub mod bootstrap;
pub mod memory;
pub mod worktree;
pub mod branch_tree;
pub mod workspace;
pub mod settings;
pub mod settings_page;
pub mod ghostty;
pub mod window_state;
pub mod diff;
pub mod keymap;
pub mod log_file;
pub mod log_view;
pub mod memory_page;
pub mod workspace_page;
pub mod advanced;

pub mod memory_items;

pub mod mentions;
pub mod conversation_context;
pub mod worktree_context;
pub mod project_context;
pub mod worktree_access;

pub mod workspace_session;
pub mod workspace_commit;
pub mod workspace_lock;
pub mod workspace_changes;

pub mod always_allow;

pub mod write_requests;

pub mod workspace_grants;

pub mod process;
