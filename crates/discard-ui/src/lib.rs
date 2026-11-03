//! GPUI views for Discard. Thin on purpose: all state logic lives in `discard_core::ui_state`.

mod plugin_element;
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
    application().run(move |cx| {
        init(cx);
        let shared = Shared {
            plugins,
            load_report: std::rc::Rc::new(load_report),
            settings,
            plugin_epoch: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            plugin_notice: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        cx.set_global(shared.clone());
        open_window(WindowOptions::default(), cx, move |window, cx| {
            cx.new(|cx| DiscardView::new(ViewInit { data, shared, state, commands, updates }, window, cx))
        })
        .expect("failed to open the main window");
        cx.activate(true);
    });
    RunOutcome::Quit
}
