//! Headless core of Discard: events, turn and approval state machines,
//! policy, storage, and the provider trait. No GPUI dependency.

pub mod event;
pub mod approval;
pub mod turn;
pub mod capability;
pub mod agent;
pub mod model;
pub mod store;
pub mod driver;
pub mod provider;
pub mod redact;
pub mod logging;
pub mod timeline;
pub mod ui_state;
pub mod engine;
pub mod bootstrap;
pub mod memory;
