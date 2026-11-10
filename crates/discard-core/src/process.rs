//! Serialize child setup so a concurrent spawn cannot inherit a pipe before CLOEXEC is set.
//! The lock covers only spawn, never a child's runtime or output collection.
use std::{io, sync::Mutex};

static SPAWN_LOCK: Mutex<()> = Mutex::new(());

#[allow(
    clippy::disallowed_methods,
    reason = "This helper owns the process-wide spawn lock."
)]
pub fn spawn_std(command: &mut std::process::Command) -> io::Result<std::process::Child> {
    let _guard = SPAWN_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    command.spawn()
}

#[allow(
    clippy::disallowed_methods,
    reason = "This helper owns the same lock as spawn_std."
)]
pub fn spawn_tokio(command: &mut tokio::process::Command) -> io::Result<tokio::process::Child> {
    let _guard = SPAWN_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    command.spawn()
}
