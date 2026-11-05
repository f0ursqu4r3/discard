//! Apply engine updates and limit webview events.
use super::{
    core::AppCore,
    dto::{MessageDto, SnapshotDto},
};
use discard_core::ui_state::Update;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

pub trait Emit: Send + Sync {
    fn snapshot(&self, s: &SnapshotDto);
    fn history(&self, h: &[MessageDto]);
}
pub async fn pump(
    core: Arc<AppCore>,
    mut updates: UnboundedReceiver<Update>,
    emit: Arc<dyn Emit>,
    min_interval: Duration,
) {
    let mut last = tokio::time::Instant::now() - min_interval;
    let mut history_rev = 0;
    let mut dirty = false;
    loop {
        tokio::select! {
            update = updates.recv() => match update {
                Some(update) => { core.apply(update); dirty = true; }
                None => {
                    if dirty { emit_current(&core, &*emit, &mut history_rev); }
                    break;
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
        emit.history(&core.history());
        *history_rev = snapshot.history_rev;
    }
    emit.snapshot(&snapshot);
}
