//! GPUI views for Discard. Thin on purpose: all state logic lives in `discard_core::ui_state`.

mod plugin_element;
mod view;

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
    // Task 5 uses the settings.
    let _ = settings;
    application().run(move |cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, move |window, cx| {
            cx.new(|cx| DiscardView::new(ViewInit { data, plugins, load_report, state, commands, updates }, window, cx))
        })
        .expect("failed to open the main window");
        cx.activate(true);
    });
    RunOutcome::Quit
}
