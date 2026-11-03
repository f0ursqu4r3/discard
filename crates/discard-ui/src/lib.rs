//! GPUI views for Discard. Thin on purpose: all state logic lives in `discard_core::ui_state`.

mod dispatch;
mod menus;
mod plugin_element;
mod problems;
mod settings;
mod shared;
mod view;

pub use settings::{SettingsView, open_settings};
pub use shared::Shared;
pub use view::{DiscardView, ViewData, ViewInit};

use discard_core::settings::SettingsStore;
use discard_core::ui_state::{Command, UiState, Update};
use gpui_kit::{AppContext as _, WindowOptions, application, init, open_window};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// How the app ended. `Relaunch` asks the caller to start a new process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Quit,
    Relaunch { safe_mode: bool },
}

/// Open the main window and run until it quits. Blocks the calling (main) thread.
pub fn run(
    data: ViewData,
    plugins: std::sync::Arc<discard_lua::PluginService>,
    load_report: discard_lua::LoadReport,
    state: UiState,
    commands: UnboundedSender<Command>,
    updates: UnboundedReceiver<Update>,
    settings: std::sync::Arc<std::sync::Mutex<SettingsStore>>,
) -> RunOutcome {
    // The dispatcher stores a relaunch request here before it quits. Read it after the event loop ends.
    let relaunch = dispatch::RelaunchSlot::default();
    let slot = relaunch.clone();
    application().run(move |cx| {
        init(cx);
        cx.set_global(slot);
        // Bind keys before the first menu install, so the menu shows each shortcut.
        menus::register_handlers(cx);
        menus::bind_keys(cx);
        let shared = Shared {
            plugins,
            load_report: std::rc::Rc::new(load_report),
            settings,
            plugin_epoch: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            plugin_notice: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        cx.set_global(shared.clone());
        let (main_window, main_view) = open_window(WindowOptions::default(), cx, move |window, cx| {
            cx.new(|cx| DiscardView::new(ViewInit { data, shared, state, commands, updates }, window, cx))
        })
        .expect("failed to open the main window");
        cx.set_global(dispatch::MainWindow { handle: main_window, view: main_view.downgrade() });
        // Discard has one main window and the engine runs for it. When it closes, the app ends on every platform,
        // and Settings goes with it. GPUI's default quit mode on macOS is Explicit, so it would keep running without this.
        // Closing the Settings window alone does not match the main window id, so it does not quit.
        let main_id = main_window.window_id();
        cx.on_window_closed(move |cx, closed| {
            if closed == main_id {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
    outcome(*relaunch.0.lock().unwrap_or_else(|p| p.into_inner()))
}

/// The outcome for what the dispatcher stored. Nothing stored means a plain quit.
fn outcome(stored: Option<bool>) -> RunOutcome {
    match stored {
        Some(safe_mode) => RunOutcome::Relaunch { safe_mode },
        None => RunOutcome::Quit,
    }
}

#[cfg(test)]
mod tests {
    use super::{RunOutcome, outcome};

    #[test]
    fn stored_relaunch_becomes_the_outcome() {
        assert_eq!(outcome(None), RunOutcome::Quit);
        assert_eq!(outcome(Some(true)), RunOutcome::Relaunch { safe_mode: true });
        assert_eq!(outcome(Some(false)), RunOutcome::Relaunch { safe_mode: false });
    }
}
