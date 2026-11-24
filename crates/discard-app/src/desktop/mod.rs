pub mod commands;
pub mod attachments;
pub mod channels;
pub mod core;
pub mod mark_read;
pub mod delegation;
pub mod dto;
pub mod notify;
pub mod pump;
pub mod review;
pub mod terminal;
pub mod threads;
pub mod view;

use core::{AppCore, CoreInit};
use discard_core::ghostty::{live::LiveTheme, spec::Appearance};
use discard_core::settings::SettingsStore;
use discard_core::ui_state::Update;
use discard_lua::PluginService;
use dto::{HistoryDto, SnapshotDto, ThreadHistoryDto};
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
impl WebEmit {
    /// Rebuild the native menu when its context or the keymap changed.
    fn update_menu(&self, view: &view::ViewId) {
        let ctx = self.core.menu_context(view);
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
    }
}
// A view event goes to the window of the view. The web store listens with the label of its window as the target.
impl Emit for WebEmit {
    fn snapshot(&self, view: &view::ViewId, value: &SnapshotDto) {
        // The native menu follows the focused view. Each tick sends every view its snapshot, so a change of focus reaches it.
        if *view == self.core.focused_view() { self.update_menu(view); }
        let _ = self.app.emit_to(view.label(), "discard://snapshot", value);
    }
    fn history(&self, view: &view::ViewId, value: &HistoryDto) {
        let _ = self.app.emit_to(view.label(), "discard://history", value);
    }
    fn thread_history(&self, view: &view::ViewId, value: &ThreadHistoryDto) {
        let _ = self.app.emit_to(view.label(), "discard://thread-history", value);
    }
    fn conversations(&self) {
        let _ = self.app.emit("discard://conversations", ());
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
    /// The services the MCP server uses for `delegate`. `run` installs the app's `Delegator` in its slot.
    pub delegation: discard_core::delegation::DelegationServices,
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
    let main = view::ViewId::main();
    core.configure_window(&main, &data_dir, &project_dir, None);
    core.select_sidebar_project(&main, config.sidebar_project.as_ref());
    if let Some(channel) = &config.startup_channel {
        core.configure_targets(&main, channel.project_id.as_ref().map(|p| p.0.clone()).unwrap_or_default(), &project_dir, channel.agent_id.0.clone());
        core.configure_read_marker(&main, &data_dir, &channel.id.0);
        if channel.project_id.is_some() { core.configure_branch_source(&main, config.store.clone(), channel.id.clone(), channel.workspace_mode, project_dir.clone()); }
    }
    (config.on_ready)(&core);
    let channels = Arc::new(channels::Channels::new(core.clone(), config.store.clone(), runtime.clone(), config.launcher));
    config.delegation.delegator.install(Arc::new(delegation::AppDelegator::new(Arc::downgrade(&channels))));
    if let Some(channel) = &config.startup_channel && let Err(error) = channels.refresh_threads(&channel.id) { core.notice(&main, format!("Could not load threads: {error}")); }
    if config.startup_channel.is_none()
        && let Err(error) = channels.restore(&main) { core.notice(&main, format!("Could not restore conversation: {error}")); }
    core.refresh_branches(&main, core.request_branch_refresh(&main));
    let terminals = Arc::new(terminal::Terminals::default());
    let actions = Arc::new(actions::DesktopActions { settings: settings.clone(), workspace: config.workspace.clone(), relaunch: config.relaunch, exiting: AtomicBool::new(false), terminals: terminals.clone() });
    let sweep_terminals = terminals.clone();
    runtime.spawn(async move {
        // A shell must not outlive its folder: archive, removal, or a deletion by hand.
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            sweep_terminals.close_missing();
        }
    });
    let exit_terminals = terminals.clone();
    let refresh_core = core.clone();
    runtime.spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            // Every window lists the branches of its own project.
            for view in refresh_core.view_ids() {
                let core = refresh_core.clone();
                let generation = core.request_branch_refresh(&view);
                let _ = tokio::task::spawn_blocking(move || core.refresh_branches(&view, generation)).await;
            }
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
    let setup_channels = channels.clone();
    let notify_settings = settings.clone();
    let event_runtime = runtime.clone();
    let setup_runtime = runtime.clone();
    let geometry_generation = Arc::new(AtomicU64::new(0));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(core.clone())
        .manage(channels)
        .manage(config.worktree_access)
        .manage(config.write_requests)
        .manage(config.delegation.requests.clone())
        .manage(config.main_checkout_grants)
        .manage(config.workspace)
        .manage(actions.clone())
        .manage(terminals)
        .manage(windows::SettingsRoute::default())
        .manage(windows::ProjectWindows::default())
        .manage(settings::SettingsContext { home: data_dir.clone(), plugins: plugins.clone(), providers: config.providers, recheck: config.recheck, store: config.store, memory: config.memory, core: core.clone(), load_report: config.load_report, trusted_reviews: Mutex::new(std::collections::HashSet::new()), logs: Mutex::new(settings::logs::LogState::open(&data_dir)) })
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();
            if let Some(action) = discard_core::action::Action::from_id(id) {
                actions::run(app, &menu_core, &menu_actions, &menu_core.focused_view(), action, None);
            } else if let Some(rest) = id.strip_prefix("plugin:")
                && let Some((index, plugin)) = rest.split_once(':')
                && let Ok(index) = index.parse()
                && let Err(error) = menu_core.run_plugin_menu(&menu_core.focused_view(), plugin, index, None)
            { menu_core.plugin_failure(plugin, &error); }
        })
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::ThemeChanged(value) = event {
                event_dark.store(*value == tauri::Theme::Dark, Ordering::Relaxed);
            }
            // Focus of every window decides whether the app notifies. A window that closes loses its focus.
            if let tauri::WindowEvent::Focused(focused) = event {
                event_core.window_focus(window.label(), *focused);
            }
            if matches!(event, tauri::WindowEvent::Destroyed) { event_core.window_closed(window.label()); }
            // The last app window that had focus decides the view of the menu, the quit question, and the plugins.
            if let tauri::WindowEvent::Focused(true) = event && let Some(view) = view::ViewId::from_label(window.label()) {
                event_core.set_focused_view(&view);
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event && windows::on_close_requested(window, &event_core, &event_actions) {
                api.prevent_close();
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
            // The handler must exist before the first notification shows: the plugin reports no click for one that
            // was shown earlier. The plugin calls it on a background thread, so it only starts the work and returns.
            {
                use tauri_plugin_notification::NotificationExt;
                let click_app = app.handle().clone();
                let click_channels = setup_channels.clone();
                let click_runtime = setup_runtime.clone();
                if let Err(error) = app.notification().on_action(move |performed| {
                    let Some(extra) = performed.notification().map(|notification| notification.extra()) else { return };
                    if let Some(channel_id) = notify::channel_from_click(performed.action_id(), extra) {
                        open_notified_channel(click_app.clone(), click_channels.clone(), click_runtime.clone(), channel_id);
                    }
                }) {
                    tracing::warn!(%error, "cannot listen for clicks on desktop notifications");
                }
            }
            let enabled_settings = notify_settings.clone();
            core.set_notifier(
                Arc::new(TauriNotifier { app: app.handle().clone() }),
                Arc::new(move || {
                    let mut store = enabled_settings.lock().unwrap_or_else(|e| e.into_inner());
                    store.reload_if_changed();
                    store.settings().general.desktop_notifications
                }),
            );
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
            // Windows and Linux get a tray icon: it is the way back to an app with no window (spec T2). macOS has the Dock.
            // The code compiles on every platform. The app decides at run time.
            let tray_state = if cfg!(target_os = "macos") { Ok(()) } else { tray::create(app.handle()) };
            if let Err(error) = &tray_state { tracing::warn!(%error, "cannot create the tray icon: the app quits when the last window closes"); }
            app.manage(tray::TrayState(tray_state));
            let mut applied_icon = appearance.app_icon;
            app_icon::apply(app.handle(), applied_icon);
            let icon_app = app.handle().clone();
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
                    if appearance.app_icon != applied_icon {
                        applied_icon = appearance.app_icon;
                        app_icon::apply(&icon_app, applied_icon);
                    }
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
            commands::get_agent_requests, commands::decide_agent_request,
            commands::get_always_allow_rules, commands::revoke_always_allow_rule,
            commands::get_snapshot, commands::choose_project_folder, commands::get_projects, commands::update_project_profile, commands::set_project_show_archived, commands::get_rail_layout, commands::move_rail_project, commands::group_rail_projects, commands::create_rail_group, commands::move_rail_group, commands::rename_rail_group, commands::set_rail_group_color, commands::set_rail_group_collapsed, commands::ungroup_rail_group, commands::select_project, commands::remove_project, commands::open_project_window, commands::show_main_window,
            commands::get_project_channels, commands::get_project_members, commands::get_channel_details, commands::update_channel_topic, commands::set_project_main_agent,
            commands::invite_channel_member, commands::select_canonical_branch, commands::select_canonical_project_channel,
            commands::search_agent_history, commands::restore_conversation, commands::get_conversations, commands::create_conversation, commands::open_agent_chat,
            commands::create_channel, commands::create_text_channel, commands::switch_channel, commands::open_branch_channel,
            commands::get_history, commands::channel_changes,
            commands::channel_review, commands::review_decide, commands::review_retry_post, commands::review_request_summary,
            commands::send_message,
            commands::get_thread_history, commands::start_thread, commands::open_thread, commands::close_thread, commands::select_thread,
            commands::post_thread_message, commands::follow_thread, commands::leave_thread,
            commands::rename_thread, commands::thread_posted_count, commands::delete_thread, commands::remove_posted_message, commands::rename_text_channel,
            commands::set_channel_muted, commands::archive_project_channel, commands::worktree_plan, commands::create_branch_worktree, commands::rename_git_branch, commands::branch_deletion_info, commands::delete_git_branch, commands::create_git_branch, commands::worktree_removal_info, commands::remove_git_worktree,
            attachments::choose_attachments,
            attachments::import_attachments,
            attachments::attachment_preview,
            terminal::terminal_tabs, terminal::terminal_create, terminal::terminal_open, terminal::terminal_restart, terminal::terminal_write, terminal::terminal_resize, terminal::terminal_detach, terminal::terminal_close, terminal::terminal_kill,
            commands::get_worktree_requests,
            commands::decide_worktree_removal,
            commands::decide_approval,
            commands::cancel_turn,
            commands::set_panels,
            commands::set_member_list_visible,
            commands::toggle_branch,
            commands::mark_read,
            commands::mark_project_read,
            commands::mark_group_read,
            commands::confirm_resolve,
            commands::archive_confirm,
            commands::plugin_action,
            commands::context_menu,
            commands::run_action,
            commands::plugin_menu_action, commands::list_commands, commands::run_command,
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
            settings::agents::settings_agent_generate_personality,
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
    app.run(move |handle, event| {
        if let tauri::RunEvent::Exit = event { exit_terminals.close_all(); }
        // A click on the Dock icon brings the main window back when no app window shows.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { has_visible_windows, .. } = event && windows::reopen_shows_main(has_visible_windows, windows::visible_app_windows(handle)) {
            windows::show_main_window(handle);
        }
        if let tauri::RunEvent::ExitRequested { api, .. } = event && !exit_actions.exiting.load(Ordering::SeqCst) {
            // Tauri raises this event with code `None` when every window is destroyed, and with a code after `app.exit`.
            // The close of a window never gets here: the main window hides. So the Quit rule decides, with no other app
            // window and no background mode.
            if let plan::Close::Ask(ending) = plan::close_plan(plan::WindowKind::Main, 0, false, exit_core.any_running(), exit_actions.confirms()) {
                api.prevent_exit();
                windows::ask_visibly(handle, &exit_core, ending);
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

pub mod app_icon;

pub mod tray;

mod slash;

pub mod settings;

/// Open the channel of a clicked notification in the main window. This takes the steps of `switch_channel`: select off
/// the event thread, then show the view that changed. A channel that is gone still brings the main window up.
fn open_notified_channel(app: tauri::AppHandle, channels: Arc<channels::Channels>, runtime: tokio::runtime::Handle, channel_id: String) {
    runtime.spawn(async move {
        let id = channel_id.clone();
        let selected = tokio::task::spawn_blocking(move || channels.select(&view::ViewId::main(), &id).map(|changed| (changed, channels.in_project(&id)))).await;
        match selected {
            Ok(Ok((changed, in_project))) => {
                if !in_project {
                    use tauri::Emitter;
                    let _ = app.emit("discard://conversations", ());
                }
                windows::show_conversation(&app, &changed);
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, channel = %channel_id, "cannot open the channel of a notification");
                windows::show_main_window(&app);
            }
            Err(error) => {
                tracing::warn!(%error, channel = %channel_id, "cannot open the channel of a notification");
                windows::show_main_window(&app);
            }
        }
    });
}

/// Shows desktop notifications through the notification plugin.
struct TauriNotifier {
    app: tauri::AppHandle,
}
impl notify::Notifier for TauriNotifier {
    fn notify(&self, notification: &notify::Notification) {
        use tauri_plugin_notification::NotificationExt;
        // A denied permission is not an error of the app. Log it and go on.
        // The channel travels with the notification, so a click can open it (see `open_notified_channel`).
        let builder = self.app.notification().builder().title(&notification.title).body(&notification.body).extra("channelId", &notification.channel_id);
        if let Err(error) = builder.show() {
            tracing::warn!(%error, "cannot show a desktop notification");
        }
    }
}
