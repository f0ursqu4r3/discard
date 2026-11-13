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

/// Decide whether to prevent a close request and show a confirmation.
pub fn request_main_close(core: &super::core::AppCore, is_main: bool, confirm_while_working: bool) -> bool {
    if let super::plan::Close::Ask(ending) = super::plan::close_plan(is_main, core.any_running(), confirm_while_working) {
        core.ask(ending);
        true
    } else { false }
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
        use crate::desktop::core::{AppCore, CoreInit};
        use discard_core::{model::ChannelId, ui_state::UiState};
        let (commands, _rx) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId::new(), vec![]), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        core.send("hello".into());
        assert!(request_main_close(&core, true, true));
        assert_eq!(core.snapshot().banners.confirm.as_deref(), Some("The agent is working. Quit Discard and stop the turn?"));
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

/// Show a conversation selected by a source link or a fresh-conversation action.
pub(crate) fn show_conversation(app: &tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    let _ = app.emit("discard://conversation-opened", ());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
