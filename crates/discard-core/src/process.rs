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

/// Run `setup` while holding the spawn lock. Use it for code that creates pipes or pseudo terminals and
/// then starts a child through a library that calls `spawn` itself.
/// Keep `setup` short: it covers pipe creation and child start, never a child's runtime or output.
/// `setup` must not call `spawn_std` or `spawn_tokio`, because the lock is not reentrant.
pub fn with_spawn_lock<T>(setup: impl FnOnce() -> T) -> T {
    let _guard = SPAWN_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    setup()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_spawn_lock_holds_the_lock_and_returns_the_value() {
        let value = with_spawn_lock(|| {
            assert!(SPAWN_LOCK.try_lock().is_err(), "the lock must be held inside");
            7
        });
        assert_eq!(value, 7);
    }
}
