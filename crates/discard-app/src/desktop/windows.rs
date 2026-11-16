//! Persistence helpers for the main window.
use discard_core::window_state::{Bounds, StateStore};

/// Save bounds only when enough of the window is visible on a screen.
pub fn save_restorable_bounds(store: &mut StateStore, bounds: Bounds, screens: &[Bounds]) -> Result<(), String> {
    if bounds.is_restorable(screens) { store.set_bounds(bounds)?; }
    Ok(())
}

/// Choose the directory represented by the current channel.
pub fn branch_current_directory(mode: Option<discard_core::model::WorkspaceMode>, project: &std::path::Path, worktree: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    use discard_core::model::WorkspaceMode;
    match mode {
        Some(WorkspaceMode::MainCheckout) => Some(project.to_path_buf()),
        Some(WorkspaceMode::Worktree) => worktree.map(std::path::Path::to_path_buf),
        None => None,
    }
}

/// Carry out the part of a close decision that needs no window: the question. Returns true when the close request is
/// refused and the window stays open. `Hide` and `Quit` refuse it too: the caller hides the window or ends the app.
pub fn refuses_close(core: &super::core::AppCore, close: super::plan::Close) -> bool {
    use super::plan::Close;
    match close {
        Close::Allow => false,
        Close::Ask(ending) => { core.ask(ending); true }
        Close::Hide | Close::Quit => true,
    }
}

/// The channel unread indicator and pending approval count. A muted channel has no unread indicator. Its pending
/// approvals still count.
pub fn channel_indicators(messages: &[discard_core::model::Message], marker: Option<&str>, pending: usize, muted: bool) -> (bool, usize) {
    (!muted && discard_core::ui_state::first_unread_index(messages, marker).is_some(), pending)
}
/// A confirmation no longer applies after the turn ends.
pub fn should_clear_confirm(running: bool, pending: bool) -> bool { !running && pending }

/// Read the window and monitor bounds in logical points.
pub fn window_geometry<R: tauri::Runtime>(window: &tauri::Window<R>) -> Option<(Bounds, Vec<Bounds>)> {
    let scale = window.scale_factor().ok()? as f32;
    let pos = window.outer_position().ok()?;
    // The restore sets the inner size. On Linux the outer size adds the frame, so a saved outer size
    // makes the window grow at each start.
    let size = window.inner_size().ok()?;
    let bounds = Bounds { x: pos.x as f32 / scale, y: pos.y as f32 / scale, width: size.width as f32 / scale, height: size.height as f32 / scale };
    let screens = window.available_monitors().ok()?.iter().map(|m| {
        let scale = m.scale_factor() as f32;
        let pos = m.position();
        let size = m.size();
        Bounds { x: pos.x as f32 / scale, y: pos.y as f32 / scale, width: size.width as f32 / scale, height: size.height as f32 / scale }
    }).collect();
    Some((bounds, screens))
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::window_state::{Bounds, StateStore};

    #[test]
    fn branch_current_directory_follows_channel_workspace_mode() {
        use discard_core::model::WorkspaceMode;
        use std::path::{Path, PathBuf};
        assert_eq!(branch_current_directory(Some(WorkspaceMode::MainCheckout), Path::new("/repo/sub"), None), Some(PathBuf::from("/repo/sub")));
        assert_eq!(branch_current_directory(Some(WorkspaceMode::Worktree), Path::new("/repo"), Some(Path::new("/work"))), Some(PathBuf::from("/work")));
        assert_eq!(branch_current_directory(Some(WorkspaceMode::Worktree), Path::new("/repo"), None), None);
        assert_eq!(branch_current_directory(None, Path::new("/repo"), None), None);
    }

    #[test]
    fn close_request_prevents_close_and_shows_confirmation_during_a_turn() {
        use crate::desktop::{core::{AppCore, CoreInit}, plan::{Close, Confirm}, view::ViewId};
        use discard_core::{model::ChannelId, ui_state::UiState};
        let (commands, _rx) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId::new(), vec![]), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        core.send(&ViewId::main(), "hello".into());
        assert!(refuses_close(&core, Close::Ask(Confirm::Quit)));
        assert_eq!(core.snapshot(&ViewId::main()).banners.confirm.as_deref(), Some("The agent is working. Quit Discard and stop the turn?"));
    }

    #[test]
    fn only_a_close_that_is_allowed_goes_through() {
        use crate::desktop::{core::{AppCore, CoreInit}, plan::Close, view::ViewId};
        use discard_core::{model::ChannelId, ui_state::UiState};
        let (commands, _rx) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId::new(), vec![]), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        assert!(!refuses_close(&core, Close::Allow));
        // The main window hides and the app ends through their own paths. Neither one closes the window itself.
        assert!(refuses_close(&core, Close::Hide));
        assert!(refuses_close(&core, Close::Quit));
        assert!(core.snapshot(&ViewId::main()).banners.confirm.is_none(), "only an Ask shows the question");
    }

    #[test]
    fn a_view_that_a_command_changed_for_another_window_is_shown() {
        use crate::desktop::view::ViewId;
        let main = ViewId::main();
        let project = ViewId::project(&discard_core::model::ProjectId("p-1".into()));
        assert_eq!(view_to_reveal(&project, &main), Some(&main), "a project window that leads outside its project shows the main window");
        assert_eq!(view_to_reveal(&main, &project), Some(&project));
        assert_eq!(view_to_reveal(&main, &main), None);
        assert_eq!(view_to_reveal(&project, &project), None);
    }

    #[test]
    fn a_click_on_the_dock_icon_shows_the_main_window_when_no_app_window_is_visible() {
        assert!(reopen_shows_main(false, 0));
        assert!(reopen_shows_main(true, 0), "only Settings is visible");
        assert!(reopen_shows_main(false, 2));
        assert!(!reopen_shows_main(true, 1));
    }

    #[test]
    fn channel_indicators_report_unread_and_pending() {
        use discard_core::model::{ChannelId, Message, MessageId, Role};
        let channel = ChannelId::new();
        let make = |id: &str, role| Message { id: MessageId(id.into()), channel_id: channel.clone(), role, blocks: vec![] };
        let messages = vec![make("a", Role::User), make("b", Role::Agent), make("c", Role::User)];
        assert_eq!(channel_indicators(&[], Some("a"), 2, false), (false, 2));
        assert!(!channel_indicators(&messages[..1], Some("a"), 0, false).0);
        assert!(channel_indicators(&messages, None, 0, false).0);
        assert!(!channel_indicators(&messages, Some("missing"), 0, false).0);
        assert!(channel_indicators(&messages, Some("a"), 0, false).0);
        assert!(!channel_indicators(&messages, Some("b"), 0, false).0);
        assert!(!channel_indicators(&messages, Some("c"), 0, false).0);
        // A muted channel shows no unread marker. Its pending approvals still count.
        assert_eq!(channel_indicators(&messages, None, 3, true), (false, 3));
    }
    #[test]
    fn clears_pending_confirmation_when_turn_ends() {
        assert!(should_clear_confirm(false, true));
        assert!(!should_clear_confirm(true, true));
        assert!(!should_clear_confirm(false, false));
    }

    #[test]
    fn a_project_window_opens_32_points_from_the_main_window() {
        let main = Bounds { x: 100.0, y: 80.0, width: 1200.0, height: 800.0 };
        assert_eq!(project_window_position(Some(main)), Some((132.0, 112.0)));
        assert_eq!(project_window_position(None), None);
        // The offset does not depend on the size of the main window, and a negative position keeps its sign.
        let left = Bounds { x: -1500.0, y: -20.0, width: 640.0, height: 480.0 };
        assert_eq!(project_window_position(Some(left)), Some((-1468.0, 12.0)));
    }

    #[test]
    fn a_project_window_is_named_after_its_project() {
        assert_eq!(project_window_title("Mission Control"), "Mission Control");
        assert_eq!(project_window_title(""), "", "a title is the name as it is");
        let id = discard_core::model::ProjectId("0193a1b2-aaaa-7bbb-8ccc-0123456789ab".into());
        assert_eq!(project_window_url(&id), "index.html#/project/0193a1b2-aaaa-7bbb-8ccc-0123456789ab");
        assert_eq!(PROJECT_WINDOW_SIZE, (1100.0, 750.0));
    }

    #[test]
    fn the_project_id_in_the_window_url_is_percent_encoded() {
        let id = discard_core::model::ProjectId("my project#1/é".into());
        assert_eq!(project_window_url(&id), "index.html#/project/my%20project%231%2F%C3%A9");
        // Nothing of the ID can end the fragment path or start a new fragment or query.
        let url = project_window_url(&discard_core::model::ProjectId("a b#c?d".into()));
        let path = url.strip_prefix("index.html#/project/").unwrap();
        assert!(!path.contains([' ', '#', '?', '/']), "{path}");
        assert_eq!(path, "a%20b%23c%3Fd");
    }

    #[test]
    fn a_second_open_focuses_the_project_window() {
        assert_eq!(project_window_action(false), ProjectWindowAction::Create);
        assert_eq!(project_window_action(true), ProjectWindowAction::Focus);
    }

    #[test]
    fn app_window_labels_keep_the_labels_of_views() {
        let project = discard_core::model::ProjectId("p-1".into());
        let label = crate::desktop::view::ViewId::project(&project).label().to_owned();
        let labels = view_labels(["settings".to_owned(), label.clone(), "main".to_owned(), "project-a b".to_owned(), "other".to_owned()]);
        assert_eq!(labels, vec!["main".to_owned(), label]);
        assert!(view_labels(Vec::<String>::new()).is_empty());
    }

    #[test]
    fn a_late_destroyed_event_of_an_old_window_drops_no_view_of_a_new_window() {
        let mut windows = ProjectWindowRegistry::default();
        let old = windows.register("project-p");
        // The old window closes, the user opens the project again, and the Destroyed event of the old window comes last.
        assert!(windows.release("project-p", old), "the Destroyed event of the window that is open drops its view");
        let reopened = windows.register("project-p");
        assert_ne!(old, reopened);
        assert!(!windows.release("project-p", old), "the old window does not drop the view of the new window");
        assert!(windows.is_open("project-p"));
        assert!(windows.release("project-p", reopened));
        assert!(!windows.is_open("project-p"));
    }

    #[test]
    fn a_window_that_closes_by_command_leaves_nothing_for_its_destroyed_event() {
        let mut windows = ProjectWindowRegistry::default();
        let first = windows.register("project-p");
        let other = windows.register("project-q");
        windows.forget("project-p");
        assert!(!windows.release("project-p", first), "the command dropped the view already");
        assert!(windows.is_open("project-q"));
        assert!(windows.release("project-q", other));
    }

    #[test]
    fn a_project_window_does_not_open_while_a_project_change_runs() {
        assert_eq!(project_window_gate(false), Ok(()));
        assert!(project_window_gate(true).unwrap_err().contains("project change"));
    }

    #[test]
    fn project_windows_may_call_the_commands_of_the_main_window() {
        let capability: serde_json::Value = serde_json::from_str(include_str!("../../capabilities/default.json")).unwrap();
        assert_eq!(capability["windows"], serde_json::json!(["main", "settings", "project-*"]));
        // The pattern matches the label of every project window, and of no other window.
        let pattern = capability["windows"].as_array().unwrap().iter().filter_map(|window| window.as_str()?.strip_suffix('*')).next().unwrap();
        for id in ["p-1", "a b/c", "0193a1b2-aaaa-7bbb-8ccc-0123456789ab"] {
            let label = crate::desktop::view::ViewId::project(&discard_core::model::ProjectId(id.into()));
            assert!(label.label().starts_with(pattern), "{id}: {}", label.label());
        }
        assert!(!"settings".starts_with(pattern) && !"main".starts_with(pattern));
    }

    #[test]
    fn bounds_saved_only_when_restorable() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = StateStore::open(&dir.path().join("state.toml"));
        let screens = [Bounds { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 }];
        let outside = Bounds { x: 3000.0, y: 0.0, width: 800.0, height: 600.0 };
        save_restorable_bounds(&mut state, outside, &screens).unwrap();
        assert_eq!(state.state().bounds, None);
        let inside = Bounds { x: 10.0, ..outside };
        save_restorable_bounds(&mut state, inside, &screens).unwrap();
        assert_eq!(state.state().bounds, Some(inside));
    }
}

#[cfg(all(test, feature = "custom-protocol"))]
mod startup_tests {
    #[test]
    fn normal_cargo_run_uses_bundled_assets() {
        assert!(!tauri::is_dev(), "the default build must use the bundled web assets");
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn second_open_focuses_the_existing_window() {
        assert_eq!(settings_window_action(false), SettingsWindowAction::Create);
        assert_eq!(settings_window_action(true), SettingsWindowAction::Focus);
    }
}

/// Choose whether to create the settings window or focus it.
#[derive(Debug, PartialEq, Eq)]
pub enum SettingsWindowAction { Create, Focus }

pub fn settings_window_action(exists: bool) -> SettingsWindowAction {
    if exists { SettingsWindowAction::Focus } else { SettingsWindowAction::Create }
}

/// Last page requested by a settings entry point.
#[derive(Default)]
pub struct SettingsRoute(pub std::sync::Mutex<Option<String>>);

#[tauri::command]
pub fn settings_current_page(route: tauri::State<'_, SettingsRoute>) -> Option<String> {
    route.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Open the single settings window, or focus it and select the requested page.
pub fn open_settings(app: &tauri::AppHandle, page: discard_core::settings_page::SettingsPage) -> tauri::Result<()> {
    use tauri::{Emitter, Manager};
    let route = format!("/settings/{}", page.title().to_ascii_lowercase());
    *app.state::<SettingsRoute>().0.lock().unwrap_or_else(|e| e.into_inner()) = Some(route.clone());
    if let Some(window) = app.get_webview_window("settings") {
        window.emit("discard://settings-page", &route)?;
        window.show()?;
        window.set_focus()?;
    } else {
        tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App(format!("index.html#{route}").into()))
            .title("Settings").inner_size(900.0, 650.0).build()?;
    }
    Ok(())
}

/// Show a conversation that a command selected in the view `view`: tell its window, then show and focus it. A Settings
/// action that opens a conversation passes the main view.
pub(crate) fn show_conversation(app: &tauri::AppHandle, view: &super::view::ViewId) {
    use tauri::Emitter;
    let _ = app.emit_to(view.label(), "discard://conversation-opened", ());
    show_view_window(app, view);
}

/// The view to show after a command of the window of `caller` changed the view `changed`: the changed view, when it is
/// the view of another window. A project view that selects a target outside its project changes the main view, and the
/// user must see it (decision 17a).
pub fn view_to_reveal<'a>(caller: &super::view::ViewId, changed: &'a super::view::ViewId) -> Option<&'a super::view::ViewId> {
    (caller != changed).then_some(changed)
}
/// Show and focus the window of a view that a command changed for another window.
pub fn reveal_changed_view(app: &tauri::AppHandle, caller: &super::view::ViewId, changed: &super::view::ViewId) {
    if let Some(view) = view_to_reveal(caller, changed) { show_view_window(app, view); }
}

/// Show and focus the main window. A hidden main window shows again, with its view and its place.
pub fn show_main_window(app: &tauri::AppHandle) { show_view_window(app, &super::view::ViewId::main()); }

/// Whether the app runs in the background: the last app window closes alone and the app goes on.
pub fn background_enabled() -> bool {
    // No tray icon and no setting exist yet. The close rule reads this one value, so the app ends with its last app window.
    false
}

/// Whether a click on the Dock icon shows the main window. `has_visible_windows` comes from the system and counts
/// Settings. `visible_app_windows` counts the app windows.
pub fn reopen_shows_main(has_visible_windows: bool, visible_app_windows: usize) -> bool { !has_visible_windows || visible_app_windows == 0 }

/// The start size of a project window, in logical points.
pub const PROJECT_WINDOW_SIZE: (f64, f64) = (1100.0, 750.0);
/// A project window opens this far from the main window on each axis, in logical points.
const PROJECT_WINDOW_OFFSET: f32 = 32.0;

/// Where a project window opens: the position of the main window plus the offset on each axis. `None` when the
/// bounds of the main window are unknown. The system then places the window.
pub fn project_window_position(main: Option<Bounds>) -> Option<(f32, f32)> {
    main.map(|bounds| (bounds.x + PROJECT_WINDOW_OFFSET, bounds.y + PROJECT_WINDOW_OFFSET))
}
/// The title of a project window is the name of its project.
pub fn project_window_title(name: &str) -> String { name.to_owned() }
/// The page of a project window. The ID is percent-encoded: the web reads it back with `decodeURIComponent`.
pub fn project_window_url(project: &discard_core::model::ProjectId) -> String { format!("index.html#/project/{}", percent_encode(&project.0)) }
/// Encode every byte except the unreserved characters of RFC 3986.
fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => encoded.push(char::from(byte)),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Choose whether to create the window of a project or focus it.
#[derive(Debug, PartialEq, Eq)]
pub enum ProjectWindowAction { Create, Focus }

pub fn project_window_action(exists: bool) -> ProjectWindowAction {
    if exists { ProjectWindowAction::Focus } else { ProjectWindowAction::Create }
}
/// The labels that name an app window (a view), in order. Other windows, such as Settings, are not in the list.
fn view_labels(labels: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut labels: Vec<String> = labels.into_iter().filter(|label| super::view::ViewId::from_label(label).is_some()).collect();
    labels.sort();
    labels
}
/// The labels of the app windows that are open.
pub fn app_window_labels(app: &tauri::AppHandle) -> Vec<String> {
    use tauri::Manager;
    view_labels(app.webview_windows().into_keys())
}
/// The labels of the app windows that are open and not hidden. A minimized window is open.
pub fn visible_app_window_labels(app: &tauri::AppHandle) -> Vec<String> {
    use tauri::Manager;
    view_labels(app.webview_windows().into_iter().filter(|(_, window)| window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false)).map(|(label, _)| label))
}
/// How many app windows are visible. A hidden main window is not an app window for the close rule (decision 7c).
pub fn visible_app_windows(app: &tauri::AppHandle) -> usize { visible_app_window_labels(app).len() }

/// Show and focus the window of a view.
pub fn show_view_window(app: &tauri::AppHandle, view: &super::view::ViewId) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window(view.label()) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The project windows that are open, each with the number of its open. The number tells a window from a window that
/// opened later with the same label.
#[derive(Default)]
pub struct ProjectWindowRegistry { next: u64, open: std::collections::HashMap<String, u64> }
impl ProjectWindowRegistry {
    /// Record a window that opened under `label`. Returns its number.
    pub fn register(&mut self, label: &str) -> u64 {
        self.next += 1;
        self.open.insert(label.to_owned(), self.next);
        self.next
    }
    /// The window with this number is destroyed. True when it is the window that the label names now: its view must
    /// drop. False when another window took the label since, or the close command did the work already.
    pub fn release(&mut self, label: &str, number: u64) -> bool {
        if self.open.get(label) == Some(&number) { self.open.remove(label); true } else { false }
    }
    /// A close command drops the view itself. The Destroyed event of that window must drop nothing.
    pub fn forget(&mut self, label: &str) { self.open.remove(label); }
    pub fn is_open(&self, label: &str) -> bool { self.open.contains_key(label) }
}
/// One lock for the open and the close of project windows. It covers the existence check, the view, the window build,
/// and the clean-up after a failure, so a second open of the same project finds the window and focuses it.
#[derive(Default)]
pub struct ProjectWindows(pub std::sync::Mutex<ProjectWindowRegistry>);

/// A project window does not open while a project change holds the app-wide guard: the project can be on its way out.
fn project_window_gate(project_change_running: bool) -> Result<(), String> {
    if project_change_running { Err("A project change is in progress. Try again when it ends.".into()) } else { Ok(()) }
}

/// Open the project window of `project`, or show and focus it when it is open. The view of a new window opens first.
/// A window that cannot be built leaves no view behind.
pub fn open_project_window(app: &tauri::AppHandle, channels: &std::sync::Arc<super::channels::Channels>, project: &discard_core::model::ProjectId) -> Result<(), String> {
    use tauri::Manager;
    let windows = app.state::<ProjectWindows>();
    // The lock is held until the window is built. The Destroyed hook of a window takes it on another thread, so the
    // event loop never waits for it.
    let mut registry = windows.0.lock().unwrap_or_else(|e| e.into_inner());
    project_window_gate(channels.core.project_change_running())?;
    let label = super::view::ViewId::project(project);
    if project_window_action(app.get_webview_window(label.label()).is_some()) == ProjectWindowAction::Focus {
        show_view_window(app, &label);
        return Ok(());
    }
    let view = channels.open_project_view(project)?;
    let name = channels.core.current_project(&view).name;
    // The live position of the main window, or its saved position when the window is not there.
    let main = app.get_webview_window("main").and_then(|window| window_geometry(&window.as_ref().window())).map(|(bounds, _)| bounds).or_else(|| channels.core.window_bounds());
    let mut builder = tauri::WebviewWindowBuilder::new(app, view.label(), tauri::WebviewUrl::App(project_window_url(project).into()))
        .title(project_window_title(&name)).inner_size(PROJECT_WINDOW_SIZE.0, PROJECT_WINDOW_SIZE.1).disable_drag_drop_handler();
    if let Some((x, y)) = project_window_position(main) { builder = builder.position(f64::from(x), f64::from(y)); }
    // The title bar options exist on macOS only.
    #[cfg(target_os = "macos")]
    { builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true).traffic_light_position(tauri::LogicalPosition::new(12.0, 20.0)); }
    let window = match builder.build() {
        Ok(window) => window,
        Err(error) => {
            // The lock keeps every other open away, so no window of this label exists and the view is ours.
            channels.core.drop_view(&view);
            return Err(error.to_string());
        }
    };
    let number = registry.register(view.label());
    let hook_app = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            // Another thread takes the lock: the thread of the event loop must not wait for an open that builds a window.
            let (app, view) = (hook_app.clone(), view.clone());
            tauri::async_runtime::spawn_blocking(move || {
                let drops = app.state::<ProjectWindows>().0.lock().unwrap_or_else(|e| e.into_inner()).release(view.label(), number);
                if drops {
                    let core = app.state::<std::sync::Arc<super::core::AppCore>>();
                    core.drop_view(&view);
                    refocus_after_close(&app, &core, view.label());
                }
            });
        }
    });
    Ok(())
}

/// Keep the focused view on a window that the user can see. `visible` lists the visible app windows. With none, the
/// main window shows again: the menu, the shortcuts, and a quit question need a window.
fn apply_refocus(app: &tauri::AppHandle, core: &super::core::AppCore, visible: &[String]) {
    use super::plan::Refocus;
    match super::plan::refocus(visible, &core.focused_view()) {
        None => {}
        Some(Refocus::Focus(view)) => core.set_focused_view(&view),
        Some(Refocus::ShowMain) => {
            show_main_window(app);
            core.set_focused_view(&super::view::ViewId::main());
        }
    }
}
/// A window closed that no close request handled: the window of a project, by command or by its own close. Its view is
/// dropped already. The focus moves to a window that stays, and the main window shows when none stays. The closing
/// window is still in the list of windows when this runs, so it is left out.
fn refocus_after_close(app: &tauri::AppHandle, core: &super::core::AppCore, closing: &str) {
    apply_refocus(app, core, &super::plan::other_app_labels(&visible_app_window_labels(app), closing));
}
/// Ask a question about ending the app in a window that the user can see. It shows in the focused window, so the focus
/// goes to a visible window first, and the main window shows when no app window is visible.
pub fn ask_visibly(app: &tauri::AppHandle, core: &super::core::AppCore, ending: super::plan::Confirm) {
    apply_refocus(app, core, &visible_app_window_labels(app));
    core.ask(ending);
}

/// Close the project window of `project` and drop its view. The window closes with no confirmation. A turn goes on,
/// and the channels stay loaded. This skips the close rule, so it moves the focus and shows the main window when no
/// other app window is visible.
pub fn close_project_window(app: &tauri::AppHandle, core: &super::core::AppCore, project: &discard_core::model::ProjectId) {
    use tauri::Manager;
    let view = super::view::ViewId::project(project);
    let windows = app.state::<ProjectWindows>();
    let mut registry = windows.0.lock().unwrap_or_else(|e| e.into_inner());
    registry.forget(view.label());
    core.drop_view(&view);
    if let Some(window) = app.get_webview_window(view.label()) { let _ = window.destroy(); }
    refocus_after_close(app, core, view.label());
}

/// Handle the close request of a window. Returns true when the window must stay open. `Hide` hides the main window
/// here, and `Quit` ends the app by the exit path of the Quit action.
pub fn on_close_requested(window: &tauri::Window, core: &super::core::AppCore, actions: &super::actions::DesktopActions) -> bool {
    use super::plan::{self, Close, Confirm};
    use tauri::Manager;
    let app = window.app_handle();
    let visible = visible_app_window_labels(app);
    let stay = plan::other_app_labels(&visible, window.label());
    let close = plan::close_plan(plan::window_kind(window.label()), stay.len(), background_enabled(), core.any_running(), actions.confirms());
    // The question shows in the focused window: that window must be visible.
    if matches!(close, Close::Ask(_)) { apply_refocus(app, core, &visible); }
    let refused = refuses_close(core, close);
    match close {
        Close::Hide => {
            let _ = window.hide();
            // A window stays visible (no background mode): the focus goes to it, not to the hidden window.
            if !stay.is_empty() { apply_refocus(app, core, &stay); }
        }
        Close::Quit => super::actions::perform(app, core, actions, Confirm::Quit),
        Close::Allow | Close::Ask(_) => {}
    }
    refused
}

/// Show the new name of a project in the title of its project window.
pub fn set_project_window_title(app: &tauri::AppHandle, project: &discard_core::model::ProjectId, name: &str) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window(super::view::ViewId::project(project).label()) { let _ = window.set_title(&project_window_title(name)); }
}
