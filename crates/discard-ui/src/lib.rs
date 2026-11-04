//! GPUI views for Discard. Thin on purpose: all state logic lives in `discard_core::ui_state`.

mod appearance;
mod dispatch;
mod menus;
mod plugin_element;
mod problems;
mod settings;
mod shared;
mod view;

pub use settings::{SettingsView, open_settings};
pub use shared::{ProviderInfo, Recheck, Shared};
pub use view::{DiscardView, ViewData, ViewInit};

use appearance::AppearanceState;
use discard_core::ghostty::live::LiveTheme;
use discard_core::settings::SettingsStore;
use discard_core::ui_state::{Command, UiState, Update};
use gpui_kit::{AppContext as _, WindowOptions, WindowBounds, Bounds, point, size, px, application, init, open_window};
use discard_core::window_state::{StateStore, Bounds as SavedBounds};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Starts a process that runs Discard again after this one exits. The argument is the new safe-mode state.
/// `Err` holds a message for the user. On `Ok` the dispatcher quits this process.
pub type Relaunch = std::sync::Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>;

/// Open the main window and run until it quits. Blocks the calling (main) thread.
/// On macOS it never returns: GPUI's quit ends the process inside the event loop.
#[allow(clippy::too_many_arguments)] // Each argument is one piece the app hands over once. A wrapper struct adds nothing.
pub fn run(
    data: ViewData,
    plugins: std::sync::Arc<discard_lua::PluginService>,
    load_report: discard_lua::LoadReport,
    state: UiState,
    commands: UnboundedSender<Command>,
    updates: UnboundedReceiver<Update>,
    settings: std::sync::Arc<std::sync::Mutex<SettingsStore>>,
    discard_home: std::path::PathBuf,
    relaunch: Relaunch,
    workspace: Arc<discard_core::workspace::GitWorkspace>,
    channel_id: discard_core::model::ChannelId,
    providers: Vec<ProviderInfo>,
    recheck: Recheck,
    store: Arc<Mutex<discard_core::store::Store>>,
    memory: Arc<discard_core::memory::MemoryStore>,
) {
    // On macOS `cx.quit()` ends the process inside this call, so nothing after it may matter.
    application().run(move |cx| {
        init(cx);
        cx.set_global(dispatch::Relauncher(relaunch));
        // Bind keys before the first menu install, so the menu shows each shortcut.
        menus::register_handlers(cx);
        let keymap = settings.lock().unwrap_or_else(|p| p.into_inner()).settings().keymap.clone();
        menus::bind_keys(cx, &keymap);
        let appearance = {
            let settings = settings.lock().unwrap_or_else(|p| p.into_inner()).settings().appearance.clone();
            let (dirs, configs) = appearance::theme_env(&discard_home);
            let system = appearance::system_appearance(cx.window_appearance());
            let live = LiveTheme::new(dirs, configs, &settings, system);
            let plugin_theme = plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).theme();
            let state = AppearanceState::new(live, &settings, plugin_theme);
            appearance::apply(&state, cx);
            std::sync::Arc::new(std::sync::Mutex::new(state))
        };
        let window_state = Arc::new(Mutex::new(StateStore::open(&discard_home.join("state.toml"))));
        let saved = window_state.lock().unwrap_or_else(|p| p.into_inner()).state().bounds;
        let displays: Vec<SavedBounds> = cx.displays().iter().map(|display| {
            let b = display.bounds();
            SavedBounds { x: f32::from(b.origin.x), y: f32::from(b.origin.y), width: f32::from(b.size.width), height: f32::from(b.size.height) }
        }).collect();
        let window_bounds = saved.filter(|b| b.is_restorable(&displays)).map(|b| WindowBounds::Windowed(Bounds::new(point(px(b.x), px(b.y)), size(px(b.width), px(b.height)))));
        let options = WindowOptions { window_bounds, ..Default::default() };
        let shared = Shared {
            workspace,
            channel_id,
            providers,
            recheck,
            store,
            memory,
            discard_home,
            plugins,
            load_report: std::rc::Rc::new(load_report),
            settings,
            appearance,
            plugin_epoch: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            plugin_notice: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        cx.set_global(shared.clone());
        let (main_window, main_view) = open_window(options, cx, move |window, cx| {
            // The red close button ends the app, so it follows the Quit confirm rule. `false` keeps the window open.
            window.on_window_should_close(cx, |_, cx| dispatch::main_window_should_close(cx));
            cx.new(|cx| DiscardView::new(ViewInit { data, shared, state, commands, updates, window_state }, window, cx))
        })
        .expect("failed to open the main window");
        cx.set_global(dispatch::MainWindow { handle: main_window, view: main_view.downgrade() });
        // A quit from any path (Dock, Cmd+Q, logout) tells the engine to cancel a running turn. Best effort only:
        // GPUI gives quit handlers a short window and then the process exits, so provider child processes can survive.
        cx.on_app_quit(|cx| {
            dispatch::cancel_running_turn(cx);
            async {}
        })
        .detach();
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
}
