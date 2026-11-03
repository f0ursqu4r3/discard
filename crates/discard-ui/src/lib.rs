//! GPUI views for Discard. Thin on purpose: all state logic lives in `discard_core::ui_state`.

mod plugin_element;
mod view;

pub use view::{DiscardView, ViewData, ViewInit};

use discard_core::ui_state::{Command, UiState, Update};
use gpui_kit::{AppContext as _, WindowOptions, application, init, open_window};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Open the main window and run until it closes. Blocks the calling (main) thread.
pub fn run(
    data: ViewData,
    plugins: std::sync::Arc<discard_lua::PluginService>,
    load_report: discard_lua::LoadReport,
    state: UiState,
    commands: UnboundedSender<Command>,
    updates: UnboundedReceiver<Update>,
) {
    application().run(move |cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, move |window, cx| {
            cx.new(|cx| DiscardView::new(ViewInit { data, plugins, load_report, state, commands, updates }, window, cx))
        })
        .expect("failed to open the main window");
        cx.activate(true);
    });
}
