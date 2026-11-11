//! Apply engine updates and limit webview events.
use super::{
    core::AppCore,
    dto::{HistoryDto, SnapshotDto, ThreadHistoryDto},
};
use discard_core::ui_state::Update;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

pub trait Emit: Send + Sync {
    fn snapshot(&self, s: &SnapshotDto);
    fn history(&self, h: &HistoryDto);
    /// The transcript of the open thread. Called when its revision changes.
    fn thread_history(&self, _: &ThreadHistoryDto) {}
    /// A thread list that the webview reads from storage changed without a command of the webview: the engine
    /// followed a thread, or a followed sub-agent thread changed state. Called at most once per change burst.
    fn conversations(&self) {}
}
pub async fn pump(
    core: Arc<AppCore>,
    updates: UnboundedReceiver<Update>,
    emit: Arc<dyn Emit>,
    min_interval: Duration,
) {
    run(core, updates, emit, min_interval, false).await;
}
pub async fn pump_live(core: Arc<AppCore>, updates: UnboundedReceiver<Update>, emit: Arc<dyn Emit>, min_interval: Duration) {
    run(core, updates, emit, min_interval, true).await;
}
async fn run(core: Arc<AppCore>, mut updates: UnboundedReceiver<Update>, emit: Arc<dyn Emit>, min_interval: Duration, keep_alive: bool) {
    let mut connected = true;
    let channel = core.active_channel_id();
    let generation = core.runtime_generation();
    let mut last = tokio::time::Instant::now() - min_interval;
    let mut history_rev = 0;
    let mut thread_rev = (String::new(), 0);
    let mut conversations_rev = core.conversations_rev();
    let mut dirty = false;
    loop {
        tokio::select! {
            update = updates.recv(), if connected => match update {
                Some(update) => {
                    if core.apply_runtime(&channel, Some(generation), update) {
                        let generation = core.request_branch_refresh();
                        let branch_core = core.clone();
                        tokio::task::spawn_blocking(move || branch_core.refresh_branches(generation));
                    }
                    dirty = true;
                }
                None => {
                    if dirty { emit_current(&core, &*emit, &mut history_rev, &mut thread_rev, &mut conversations_rev); }
                    if keep_alive { connected = false; } else { break; }
                }
            },
            _ = core.changed.notified() => { dirty = true; }
            _ = tokio::time::sleep_until(last + min_interval), if dirty => {
                emit_current(&core, &*emit, &mut history_rev, &mut thread_rev, &mut conversations_rev);
                last = tokio::time::Instant::now();
                dirty = false;
            }
        }
    }
}

fn emit_current(core: &AppCore, emit: &dyn Emit, history_rev: &mut u64, thread_rev: &mut (String, u64), conversations_rev: &mut u64) {
    let snapshot = core.snapshot();
    let conversations = core.conversations_rev();
    if conversations != *conversations_rev { *conversations_rev = conversations; emit.conversations(); }
    // Send the open thread's transcript once per revision. Closing the thread forgets the revision.
    match snapshot.open_thread.as_ref() {
        Some(open) if (open.channel_id.as_str(), open.history_rev) != (thread_rev.0.as_str(), thread_rev.1) => {
            if let Some(history) = core.thread_history_snapshot().filter(|h| h.channel_id == open.channel_id && h.rev == open.history_rev) {
                emit.thread_history(&history);
                *thread_rev = (open.channel_id.clone(), open.history_rev);
            }
        }
        Some(_) => {}
        None => *thread_rev = (String::new(), 0),
    }
    if snapshot.history_rev != *history_rev {
        let history = core.history_snapshot();
        if history.channel_id == snapshot.channel_id && history.rev == snapshot.history_rev { emit.history(&history); }
        *history_rev = snapshot.history_rev;
    }
    emit.snapshot(&snapshot);
}
