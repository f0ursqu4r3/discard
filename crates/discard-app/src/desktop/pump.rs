//! Apply engine updates and limit webview events.
use super::{
    core::AppCore,
    dto::{HistoryDto, SnapshotDto, ThreadHistoryDto},
    view::ViewId,
};
use discard_core::ui_state::Update;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

/// Receives what the pump sends to the window of each view.
pub trait Emit: Send + Sync {
    fn snapshot(&self, view: &ViewId, s: &SnapshotDto);
    fn history(&self, view: &ViewId, h: &HistoryDto);
    /// The transcript of the open thread of a view. Called when its revision changes.
    fn thread_history(&self, _: &ViewId, _: &ThreadHistoryDto) {}
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
/// What the pump sent last to one view.
#[derive(Default)]
struct Sent {
    history_rev: u64,
    thread_rev: (String, u64),
}
async fn run(core: Arc<AppCore>, mut updates: UnboundedReceiver<Update>, emit: Arc<dyn Emit>, min_interval: Duration, keep_alive: bool) {
    let mut connected = true;
    // The first engine belongs to the channel that the main view selects at startup.
    let startup = ViewId::main();
    let channel = core.active_channel_id(&startup);
    let generation = core.runtime_generation(&startup);
    let mut last = tokio::time::Instant::now() - min_interval;
    let mut sent: HashMap<ViewId, Sent> = HashMap::new();
    let mut conversations_rev = core.conversations_rev();
    let mut dirty = false;
    loop {
        tokio::select! {
            update = updates.recv(), if connected => match update {
                Some(update) => {
                    if core.apply_runtime(&channel, Some(generation), update) { core.refresh_branches_selecting(&channel); }
                    dirty = true;
                }
                None => {
                    if dirty { emit_current(&core, &*emit, &mut sent, &mut conversations_rev); }
                    if keep_alive { connected = false; } else { break; }
                }
            },
            _ = core.changed.notified() => { dirty = true; }
            _ = tokio::time::sleep_until(last + min_interval), if dirty => {
                emit_current(&core, &*emit, &mut sent, &mut conversations_rev);
                last = tokio::time::Instant::now();
                dirty = false;
            }
        }
    }
}

/// Send each view the state of its own window. A view that closed leaves no record behind.
fn emit_current(core: &AppCore, emit: &dyn Emit, sent: &mut HashMap<ViewId, Sent>, conversations_rev: &mut u64) {
    let views = core.view_ids();
    sent.retain(|view, _| views.contains(view));
    let conversations = core.conversations_rev();
    if conversations != *conversations_rev { *conversations_rev = conversations; emit.conversations(); }
    for view in &views { emit_view(core, view, emit, sent.entry(view.clone()).or_default()); }
}

fn emit_view(core: &AppCore, view: &ViewId, emit: &dyn Emit, sent: &mut Sent) {
    let snapshot = core.snapshot(view);
    // Send the open thread's transcript once per revision. Closing the thread forgets the revision.
    match snapshot.open_thread.as_ref() {
        Some(open) if (open.channel_id.as_str(), open.history_rev) != (sent.thread_rev.0.as_str(), sent.thread_rev.1) => {
            if let Some(history) = core.thread_history_snapshot(view).filter(|h| h.channel_id == open.channel_id && h.rev == open.history_rev) {
                emit.thread_history(view, &history);
                sent.thread_rev = (open.channel_id.clone(), open.history_rev);
            }
        }
        Some(_) => {}
        None => sent.thread_rev = (String::new(), 0),
    }
    if snapshot.history_rev != sent.history_rev {
        let history = core.history_snapshot(view);
        if history.channel_id == snapshot.channel_id && history.rev == snapshot.history_rev { emit.history(view, &history); }
        sent.history_rev = snapshot.history_rev;
    }
    emit.snapshot(view, &snapshot);
}
