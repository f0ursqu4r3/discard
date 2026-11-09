//! A queued, process-local write lease per canonical worktree path.

use crate::model::ChannelId;
use std::{collections::HashMap, path::{Path, PathBuf}, sync::{Arc, Mutex}};

#[derive(Default)]
struct Entry { gate: Arc<tokio::sync::Mutex<()>>, holder: Mutex<Option<ChannelId>>, commit_barrier: Mutex<()> }
#[derive(Default)]
pub struct WorkspaceLocks { entries: Mutex<HashMap<PathBuf, Arc<Entry>>> }
pub struct WorkspaceLockGuard { entry: Arc<Entry>, _guard: tokio::sync::OwnedMutexGuard<()> }
fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> { value.lock().unwrap_or_else(|e| e.into_inner()) }
impl WorkspaceLocks {
    fn key(root: &Path) -> PathBuf { root.canonicalize().unwrap_or_else(|_| root.to_owned()) }
    fn entry(&self, root: &Path) -> Arc<Entry> { lock(&self.entries).entry(Self::key(root)).or_default().clone() }
    pub async fn acquire(&self, root: &Path, holder: ChannelId) -> WorkspaceLockGuard {
        let entry = self.entry(root); let guard = entry.gate.clone().lock_owned().await;
        *lock(&entry.holder) = Some(holder); WorkspaceLockGuard { entry, _guard: guard }
    }
    pub fn holder(&self, root: &Path) -> Option<ChannelId> { lock(&self.entry(root).holder).clone() }
    pub(crate) fn with_holder<T>(&self, root: &Path, channel: &ChannelId, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let entry = self.entry(root); let _commit = lock(&entry.commit_barrier);
        if lock(&entry.holder).as_ref() != Some(channel) { return Err("This response does not hold the worktree write lock.".into()); }
        // Keep the lease alive even if the response is stopped during the Git command.
        action()
    }
}
impl WorkspaceLockGuard {
    /// Close commit authority and wait for an active mutation. Retain the path gate for response storage.
    pub async fn stop_writes(&self) {
        let entry = self.entry.clone();
        tokio::task::spawn_blocking(move || { let _commit = lock(&entry.commit_barrier); *lock(&entry.holder) = None; }).await.expect("workspace commit barrier completes");
    }
}
impl Drop for WorkspaceLockGuard { fn drop(&mut self) { let _commit = lock(&self.entry.commit_barrier); *lock(&self.entry.holder) = None; } }

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn holder_returns_within_100ms_while_a_commit_is_busy() {
        let dir = tempfile::tempdir().unwrap(); let root = dir.path().to_owned(); let locks = Arc::new(WorkspaceLocks::default());
        let channel = ChannelId("a".into()); let _guard = locks.acquire(&root, channel.clone()).await;
        let (entered_tx, entered_rx) = std::sync::mpsc::channel(); let (release_tx, release_rx) = std::sync::mpsc::channel();
        let writer = { let locks = locks.clone(); let root = root.clone(); let channel = channel.clone(); std::thread::spawn(move || locks.with_holder(&root, &channel, || { entered_tx.send(()).unwrap(); release_rx.recv().unwrap(); Ok(()) })) };
        entered_rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || result_tx.send(locks.holder(&root)).unwrap());
        let result = result_rx.recv_timeout(std::time::Duration::from_millis(100));
        release_tx.send(()).unwrap(); writer.join().unwrap().unwrap(); reader.join().unwrap();
        assert_eq!(result.unwrap(), Some(channel));
    }
    #[tokio::test]
    async fn second_acquire_waits_until_the_guard_drops() {
        let dir = tempfile::tempdir().unwrap(); let locks = WorkspaceLocks::default();
        let guard = locks.acquire(dir.path(), ChannelId("a".into())).await;
        let next = locks.acquire(dir.path(), ChannelId("b".into())); tokio::pin!(next);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut next).await.is_err());
        drop(guard); let _guard = next.await; assert_eq!(locks.holder(dir.path()), Some(ChannelId("b".into())));
    }
    #[tokio::test]
    async fn holder_reports_the_owning_channel() {
        let dir = tempfile::tempdir().unwrap(); let locks = WorkspaceLocks::default();
        let guard = locks.acquire(dir.path(), ChannelId("a".into())).await;
        assert_eq!(locks.holder(&dir.path().canonicalize().unwrap()), Some(ChannelId("a".into())));
        drop(guard); assert_eq!(locks.holder(dir.path()), None);
    }
    #[tokio::test]
    async fn dropping_a_waiting_future_does_not_leak_the_lock() {
        let dir = tempfile::tempdir().unwrap(); let locks = WorkspaceLocks::default();
        let guard = locks.acquire(dir.path(), ChannelId("a".into())).await;
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), locks.acquire(dir.path(), ChannelId("b".into()))).await.is_err());
        drop(guard); let _guard = locks.acquire(dir.path(), ChannelId("c".into())).await;
        assert_eq!(locks.holder(dir.path()), Some(ChannelId("c".into())));
    }
}
