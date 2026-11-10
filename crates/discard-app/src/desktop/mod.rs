pub mod commands;
pub mod attachments;
pub mod channels;
pub mod core;
pub mod dto;
pub mod pump;

use core::{AppCore, CoreInit};
use discard_core::ghostty::{live::LiveTheme, spec::Appearance};
use discard_core::settings::SettingsStore;
use discard_core::ui_state::Update;
use discard_lua::PluginService;
use dto::{HistoryDto, SnapshotDto};
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
    fn history(&self, value: &HistoryDto) {
        let _ = self.app.emit("discard://history", value);
    }
}
/// Startup data for the main desktop window.
pub struct DesktopConfig {
    pub startup_channel: Option<discard_core::model::Channel>,
    pub sidebar_project: Option<discard_core::model::Project>,
    pub launcher: channels::Launcher,
    pub on_ready: channels::OnReady,
    pub load_report: discard_lua::LoadReport,
    pub workspace: Arc<discard_core::workspace::GitWorkspace>,
    pub store: Arc<Mutex<discard_core::store::Store>>,
    pub relaunch: actions::Relaunch,
    pub providers: Vec<actions::ProviderInfo>,
    pub recheck: actions::Recheck,
    pub memory: Arc<discard_core::memory::MemoryStore>,
    pub main_checkout_grants: Arc<discard_core::workspace_grants::MainCheckoutGrants>,
    pub write_requests: Arc<discard_core::write_requests::WriteRequests>,
    pub worktree_access: Arc<discard_core::worktree_access::WorktreeAccess>,
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
    let project_dir: PathBuf = config.sidebar_project.as_ref().map(|p| p.path.clone().into()).unwrap_or_default();
    core.configure_window(&data_dir, &project_dir, None);
    core.select_sidebar_project(config.sidebar_project.as_ref());
    if let Some(channel) = &config.startup_channel {
        core.configure_targets(channel.project_id.as_ref().map(|p| p.0.clone()).unwrap_or_default(), &project_dir, channel.agent_id.0.clone());
        core.configure_read_marker(&data_dir, &channel.id.0);
        if channel.project_id.is_some() { core.configure_branch_source(config.store.clone(), channel.id.clone(), channel.workspace_mode, project_dir.clone()); }
    }
    (config.on_ready)(&core);
    let channels = Arc::new(channels::Channels::new(core.clone(), config.store.clone(), runtime.clone(), config.launcher));
    if config.startup_channel.is_none()
        && let Err(error) = channels.restore() { core.notice(format!("Could not restore conversation: {error}")); }
    core.refresh_branches(core.request_branch_refresh());
    let actions = Arc::new(actions::DesktopActions { settings: settings.clone(), workspace: config.workspace.clone(), relaunch: config.relaunch, exiting: AtomicBool::new(false) });
    let refresh_core = core.clone();
    runtime.spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let core = refresh_core.clone();
            let generation = core.request_branch_refresh();
            let _ = tokio::task::spawn_blocking(move || core.refresh_branches(generation)).await;
        }
    });
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
        .plugin(tauri_plugin_dialog::init())
        .manage(core.clone())
        .manage(channels)
        .manage(config.worktree_access)
        .manage(config.write_requests)
        .manage(config.main_checkout_grants)
        .manage(config.workspace)
        .manage(actions.clone())
        .manage(windows::SettingsRoute::default())
        .manage(settings::SettingsContext { home: data_dir.clone(), plugins: plugins.clone(), providers: config.providers, recheck: config.recheck, store: config.store, memory: config.memory, core: core.clone(), load_report: config.load_report, trusted_reviews: Mutex::new(std::collections::HashSet::new()), logs: Mutex::new(settings::logs::LogState::open(&data_dir)) })
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
                if windows::request_main_close(&event_core, window.label() == "main", event_actions.confirms()) {
                    api.prevent_close();
                } else if window.label() == "main" && event_core.any_running() { event_core.cancel_all(); }
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
            runtime.spawn(pump::pump_live(
                core.clone(),
                updates,
                Arc::new(WebEmit { app: app.handle().clone(), core: core.clone(), settings: emitter_settings, menu_state: Mutex::new(None) }),
                Duration::from_millis(16),
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_write_requests, commands::decide_write_request,
            commands::get_always_allow_rules, commands::revoke_always_allow_rule,
            commands::get_snapshot, commands::choose_project_folder, commands::get_projects, commands::update_project_profile, commands::select_project, commands::remove_project,
            commands::get_project_channels, commands::get_project_members, commands::get_channel_details, commands::update_channel_topic, commands::set_project_main_agent,
            commands::invite_channel_member, commands::select_canonical_branch, commands::select_canonical_project_channel,
            commands::search_agent_history, commands::restore_conversation, commands::get_conversations, commands::create_conversation, commands::open_agent_chat,
            commands::create_channel, commands::switch_channel, commands::open_branch_channel,
            commands::get_history,
            commands::send_message,
            attachments::choose_attachments,
            attachments::import_attachments,
            attachments::attachment_preview,
            commands::get_worktree_requests,
            commands::decide_worktree_removal,
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
            settings::providers::settings_provider_models,
            settings::agents::settings_agent_get,
            settings::agents::settings_agent_review,
            settings::agents::settings_agent_save,
            settings::agents::settings_agent_defaults, settings::agents::settings_agent_create, settings::agents::settings_agent_archive, settings::agents::settings_agent_delete,
            settings::memory::memory_fresh_conversation, settings::memory::memory_get, settings::memory::memory_create, settings::memory::memory_mutate, settings::memory::memory_resolve, settings::memory::memory_resume, settings::memory::memory_reconcile, settings::memory::memory_reveal_item,
            settings::workspaces::settings_workspaces,
            settings::workspaces::settings_reveal_workspace,
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
            if let plan::Close::Ask(ending) = plan::close_plan(true, exit_core.any_running(), exit_actions.confirms()) {
                api.prevent_exit();
                exit_core.ask(ending);
            } else if exit_core.any_running() { exit_core.cancel_all(); }
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
