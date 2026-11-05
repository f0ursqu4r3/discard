pub mod commands;
pub mod core;
pub mod dto;
pub mod pump;

use core::{AppCore, CoreInit};
use discard_core::ghostty::{live::LiveTheme, spec::Appearance};
use discard_core::settings::SettingsStore;
use discard_core::ui_state::Update;
use discard_lua::PluginService;
use dto::{MessageDto, SnapshotDto};
use pump::Emit;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc::UnboundedReceiver;

struct WebEmit(tauri::AppHandle);
impl Emit for WebEmit {
    fn snapshot(&self, value: &SnapshotDto) {
        let _ = self.0.emit("discard://snapshot", value);
    }
    fn history(&self, value: &[MessageDto]) {
        let _ = self.0.emit("discard://history", value);
    }
}
pub fn run(
    init: CoreInit,
    updates: UnboundedReceiver<Update>,
    runtime: tokio::runtime::Handle,
    settings: Arc<Mutex<SettingsStore>>,
    plugins: Arc<PluginService>,
    data_dir: PathBuf,
) -> Result<(), tauri::Error> {
    let core = Arc::new(AppCore::new(init));
    core.set_plugins(plugins.clone());
    let dark = Arc::new(AtomicBool::new(true));
    let event_dark = dark.clone();
    tauri::Builder::default()
        .manage(core.clone())
        .on_window_event(move |_window, event| {
            if let tauri::WindowEvent::ThemeChanged(value) = event {
                event_dark.store(*value == tauri::Theme::Dark, Ordering::Relaxed);
            }
        })
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main")
                && let Ok(value) = window.theme()
            {
                dark.store(value == tauri::Theme::Dark, Ordering::Relaxed);
            }
            let mode = if dark.load(Ordering::Relaxed) {
                Appearance::Dark
            } else {
                Appearance::Light
            };
            let appearance = settings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .settings()
                .appearance
                .clone();
            let plugin_theme = plugins
                .manager()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .theme();
            let (dirs, paths) = theme::theme_env(&data_dir);
            let mut theme_state = theme::AppearanceState::new(
                LiveTheme::new(dirs, paths, &appearance, mode),
                &appearance,
                plugin_theme,
            );
            core.set_theme(theme::theme_dto(&theme_state, mode));
            let theme_core = core.clone();
            runtime.spawn(async move {
                let mut timer = tokio::time::interval(Duration::from_millis(700));
                loop {
                    timer.tick().await;
                    let mode = if dark.load(Ordering::Relaxed) {
                        Appearance::Dark
                    } else {
                        Appearance::Light
                    };
                    let appearance = settings
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .settings()
                        .appearance
                        .clone();
                    let plugin_theme = plugins
                        .manager()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .theme();
                    if theme_state.update(&appearance, mode, &plugin_theme) {
                        theme_core.set_theme(theme::theme_dto(&theme_state, mode));
                    }
                }
            });
            runtime.spawn(pump::pump(
                core,
                updates,
                Arc::new(WebEmit(app.handle().clone())),
                Duration::from_millis(16),
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_history,
            commands::send_message,
            commands::decide_approval,
            commands::cancel_turn
        ])
        .run(tauri::generate_context!())
}

pub mod theme;
