//! Apply engine updates and limit webview events.
use super::{
    core::AppCore,
    dto::{HistoryDto, SnapshotDto},
};
use discard_core::ui_state::Update;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

pub trait Emit: Send + Sync {
    fn snapshot(&self, s: &SnapshotDto);
    fn history(&self, h: &HistoryDto);
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
                    if dirty { emit_current(&core, &*emit, &mut history_rev); }
                    if keep_alive { connected = false; } else { break; }
                }
            },
            _ = core.changed.notified() => { dirty = true; }
            _ = tokio::time::sleep_until(last + min_interval), if dirty => {
                emit_current(&core, &*emit, &mut history_rev);
                last = tokio::time::Instant::now();
                dirty = false;
            }
        }
    }
}

fn emit_current(core: &AppCore, emit: &dyn Emit, history_rev: &mut u64) {
    let snapshot = core.snapshot();
    if snapshot.history_rev != *history_rev {
        let history = core.history_snapshot();
        if history.channel_id == snapshot.channel_id && history.rev == snapshot.history_rev { emit.history(&history); }
        *history_rev = snapshot.history_rev;
    }
    emit.snapshot(&snapshot);
}
