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
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc::UnboundedReceiver;

struct WebEmit {
    app: tauri::AppHandle,
    core: Arc<AppCore>,
    settings: Arc<Mutex<SettingsStore>>,
    menu_state: Mutex<Option<(discard_core::action::MenuContext, discard_core::keymap::Keymap)>>,
}
impl Emit for WebEmit {
    fn snapshot(&self, value: &SnapshotDto) {
        let ctx = self.core.menu_context();
        let map = self.settings.lock().unwrap_or_else(|e| e.into_inner()).settings().keymap.clone();
        let mut previous = self.menu_state.lock().unwrap_or_else(|e| e.into_inner());
        if previous.as_ref() != Some(&(ctx.clone(), map.clone())) {
            match menu::build_menu(&self.app, &ctx, &map).and_then(|menu| self.app.set_menu(menu).map(|_| ())) {
                Ok(()) => {
                    let _ = self.app.emit("discard://shortcuts", menu::web_shortcuts(&map));
                    *previous = Some((ctx, map));
                },
                Err(error) => tracing::warn!(%error, "cannot update native menu"),
            }
        }
        let _ = self.app.emit("discard://snapshot", value);
    }
    fn history(&self, value: &[MessageDto]) {
        let _ = self.app.emit("discard://history", value);
    }
}
/// Startup data for the main desktop window.
pub struct DesktopConfig {
    pub project_dir: PathBuf,
    pub project_id: String,
    pub agent_id: String,
    pub channel_dir: Option<PathBuf>,
    pub channel_id: discard_core::model::ChannelId,
    pub load_report: discard_lua::LoadReport,
    pub workspace: Arc<discard_core::workspace::GitWorkspace>,
    pub store: Arc<Mutex<discard_core::store::Store>>,
    pub workspace_mode: discard_core::model::WorkspaceMode,
    pub relaunch: actions::Relaunch,
    pub providers: Vec<discard_ui::ProviderInfo>,
    pub recheck: discard_ui::Recheck,
    pub memory: Arc<discard_core::memory::MemoryStore>,
}
pub fn run(
    init: CoreInit,
    updates: UnboundedReceiver<Update>,
    runtime: tokio::runtime::Handle,
    settings: Arc<Mutex<SettingsStore>>,
    plugins: Arc<PluginService>,
    data_dir: PathBuf,
    config: DesktopConfig,
) -> Result<(), tauri::Error> {
    let core = Arc::new(AppCore::new(init));
    core.set_plugins(plugins.clone());
    core.set_load_report(config.load_report.clone());
    core.configure_window(&data_dir, &config.project_dir, config.channel_dir.as_deref());
    core.configure_targets(config.project_id, &config.project_dir, config.agent_id);
    core.configure_branch_source(config.store.clone(), config.channel_id.clone(), config.workspace_mode, config.project_dir.clone());
    core.configure_read_marker(&data_dir, &config.channel_id.0);
    let actions = Arc::new(actions::DesktopActions { settings: settings.clone(), workspace: config.workspace, channel_id: config.channel_id.clone(), relaunch: config.relaunch, exiting: AtomicBool::new(false) });
    let event_actions = actions.clone();
    let menu_actions = actions.clone();
    let menu_core = core.clone();
    let exit_core = core.clone();
    let exit_actions = actions.clone();
    let dark = Arc::new(AtomicBool::new(true));
    let event_dark = dark.clone();
    let event_core = core.clone();
    let event_runtime = runtime.clone();
    let geometry_generation = Arc::new(AtomicU64::new(0));
    let app = tauri::Builder::default()
        .manage(core.clone())
        .manage(actions.clone())
        .manage(windows::SettingsRoute::default())
        .manage(settings::SettingsContext { home: data_dir.clone(), plugins: plugins.clone(), providers: config.providers, recheck: config.recheck, store: config.store, memory: config.memory, channel_id: config.channel_id, load_report: config.load_report, trusted_reviews: Mutex::new(std::collections::HashSet::new()), logs: Mutex::new(settings::logs::LogState::open(&data_dir)) })
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();
            if let Some(action) = discard_core::action::Action::from_id(id) {
                actions::run(app, &menu_core, &menu_actions, action, None);
            } else if let Some(rest) = id.strip_prefix("plugin:")
                && let Some((index, plugin)) = rest.split_once(':')
                && let Ok(index) = index.parse()
                && let Err(error) = menu_core.run_plugin_menu(plugin, index, None)
            { menu_core.plugin_failure(plugin, &error); }
        })
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::ThemeChanged(value) = event {
                event_dark.store(*value == tauri::Theme::Dark, Ordering::Relaxed);
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let ctx = event_core.menu_context();
                if windows::request_main_close(&event_core, window.label() == "main", event_actions.confirms()) {
                    api.prevent_close();
                } else if window.label() == "main" && ctx.turn_running { event_core.cancel(); }
            }
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) {
                let generation = geometry_generation.fetch_add(1, Ordering::Relaxed) + 1;
                let counter = geometry_generation.clone();
                let core = event_core.clone();
                let window = window.clone();
                event_runtime.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    if counter.load(Ordering::Relaxed) == generation
                        && let Some((bounds, screens)) = windows::window_geometry(&window)
                        && let Err(error) = core.save_bounds(bounds, &screens)
                    { tracing::warn!(%error, "cannot save window bounds"); }
                });
            }
        })
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(value) = window.theme() {
                    dark.store(value == tauri::Theme::Dark, Ordering::Relaxed);
                }
                if let Some(saved) = core.window_bounds()
                    && let Some((_, screens)) = windows::window_geometry(&window.as_ref().window())
                    && saved.is_restorable(&screens)
                {
                    let _ = window.set_position(tauri::LogicalPosition::new(saved.x, saved.y));
                    let _ = window.set_size(tauri::LogicalSize::new(saved.width, saved.height));
                }
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
            let emitter_settings = settings.clone();
            let settings_events = app.handle().clone();
            runtime.spawn(async move {
                let mut timer = tokio::time::interval(Duration::from_millis(700));
                loop {
                    timer.tick().await;
                    let mode = if dark.load(Ordering::Relaxed) {
                        Appearance::Dark
                    } else {
                        Appearance::Light
                    };
                    let (appearance, changed) = {
                        let mut settings = settings.lock().unwrap_or_else(|e| e.into_inner());
                        let changed = settings.reload_if_changed();
                        (settings.settings().appearance.clone(), changed)
                    };
                    if changed { theme_core.clear_slots(); let _ = settings_events.emit("discard://settings", ()); }
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
            let maintenance_core = core.clone();
            runtime.spawn(async move {
                let mut timer = tokio::time::interval(Duration::from_secs(1));
                loop { timer.tick().await; maintenance_core.maintain_plugins(); }
            });
            runtime.spawn(pump::pump(
                core.clone(),
                updates,
                Arc::new(WebEmit { app: app.handle().clone(), core: core.clone(), settings: emitter_settings, menu_state: Mutex::new(None) }),
                Duration::from_millis(16),
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_history,
            commands::send_message,
            commands::decide_approval,
            commands::cancel_turn,
            commands::set_panels,
            commands::set_member_list_visible,
            commands::toggle_branch,
            commands::mark_read,
            commands::confirm_resolve,
            commands::archive_confirm,
            commands::plugin_action,
            commands::context_menu,
            commands::run_action,
            commands::plugin_menu_action,
            commands::web_shortcuts,
            windows::settings_current_page,
            settings::settings_search,
            settings::settings_get,
            settings::settings_set_general,
            settings::settings_set_appearance,
            settings::settings_copy_theme,
            settings::settings_choose_theme,
            settings::providers::settings_providers,
            settings::providers::settings_recheck_provider,
            settings::agents::settings_agent_get,
            settings::agents::settings_agent_review,
            settings::agents::settings_agent_save,
            settings::memory::settings_memory_scopes,
            settings::memory::settings_memory_read,
            settings::workspaces::settings_workspaces,
            settings::workspaces::settings_reveal_workspace,
            settings::memory::settings_reveal_memory,
            settings::advanced::settings_advanced_actions,
            settings::advanced::settings_advanced_run,
            settings::keybindings::settings_keymap_rows,
            settings::keybindings::settings_keymap_set,
            settings::plugins::settings_plugins,
            settings::plugins::settings_plugin_trust,
            settings::plugins::settings_plugin_approve,
            settings::plugins::settings_plugin_reject,
            settings::plugins::settings_plugin_rollback,
            settings::plugins::settings_plugin_modified_diff,
            settings::plugins::settings_plugin_dismiss_notice,
            settings::logs::settings_logs
        ])
        .build(tauri::generate_context!())?;
    app.run(move |_handle, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event && !exit_actions.exiting.load(Ordering::SeqCst) {
            let ctx = exit_core.menu_context();
            if let plan::Close::Ask(ending) = plan::close_plan(true, ctx.turn_running, exit_actions.confirms()) {
                api.prevent_exit();
                exit_core.ask(ending);
            } else if ctx.turn_running { exit_core.cancel(); }
        }
    });
    Ok(())
}

pub mod theme;

pub mod windows;

pub mod problems;

pub mod slots;

pub mod plan;
pub mod menu;

pub mod actions;

pub mod settings;
