//! The tray icon, and the rule that says when the app keeps running with no app window (spec T1 to T8).
use super::{actions::DesktopActions, core::AppCore};
use discard_core::action::Action;
use std::sync::Arc;
use tauri::{AppHandle, Manager, menu::{MenuBuilder, MenuItemBuilder}, tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent}};

/// The ID of the tray icon.
const TRAY_ID: &str = "main";
const MAIN_WINDOW_ID: &str = "tray_main_window";
const QUIT_ID: &str = "tray_quit";
const MAIN_WINDOW_LABEL: &str = "Main Window";
const QUIT_LABEL: &str = "Quit Discard";

/// Whether the app stays alive with no app window (spec T1, T3, T7, T8). The setting must be on, and the user needs a
/// way back: the Dock on macOS, or a tray icon that exists.
pub fn background_enabled(setting: bool, macos: bool, tray_ok: bool) -> bool { setting && (macos || tray_ok) }

/// The line the General page shows when the tray icon failed.
pub fn unavailable_line(error: &str) -> String {
    format!("The tray icon is not available: {error}. Discard quits when the last window closes.")
}

/// Managed state: the result of the tray creation. `Ok(())` on macOS, which has no tray icon.
pub struct TrayState(pub Result<(), String>);
impl TrayState {
    /// The line for the General page, when the tray icon failed.
    pub fn unavailable(&self) -> Option<String> { self.0.as_ref().err().map(|error| unavailable_line(error)) }
}

/// What a tray menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuCommand { ShowMain, Quit }

fn menu_command(id: &str) -> Option<MenuCommand> {
    match id {
        MAIN_WINDOW_ID => Some(MenuCommand::ShowMain),
        QUIT_ID => Some(MenuCommand::Quit),
        _ => None,
    }
}

/// Whether a click on the icon shows the main window. Windows gives the click, and the menu opens on the right click.
/// Linux gives no click event, so the menu is the only path (spec T4).
fn click_shows_main(windows: bool, button: MouseButton, state: MouseButtonState) -> bool {
    windows && button == MouseButton::Left && state == MouseButtonState::Up
}

/// Create the tray icon, with the menu `Main Window`, a separator, and `Quit Discard` (spec T2, T4). The caller does
/// this on Windows and Linux only.
pub fn create(app: &AppHandle) -> Result<(), String> {
    let icon = app.state::<Arc<DesktopActions>>().settings.lock().unwrap_or_else(|e| e.into_inner()).settings().appearance.app_icon;
    let image = tauri::image::Image::from_bytes(super::app_icon::png(icon)).map_err(|error| error.to_string())?;
    let menu = MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id(MAIN_WINDOW_ID, MAIN_WINDOW_LABEL).build(app).map_err(|error| error.to_string())?)
        .separator()
        .item(&MenuItemBuilder::with_id(QUIT_ID, QUIT_LABEL).build(app).map_err(|error| error.to_string())?)
        .build()
        .map_err(|error| error.to_string())?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(image)
        .tooltip("Discard")
        .menu(&menu)
        // A left click shows the main window, so it must not open the menu.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match menu_command(event.id().as_ref()) {
            Some(MenuCommand::ShowMain) => super::windows::show_main_window(app),
            // The Quit rule decides, as for the Quit menu item (spec T6).
            Some(MenuCommand::Quit) => {
                let (core, actions) = (app.state::<Arc<AppCore>>(), app.state::<Arc<DesktopActions>>());
                super::actions::run(app, &core, &actions, &core.focused_view(), Action::Quit, None);
            }
            None => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button, button_state, .. } = event && click_shows_main(cfg!(windows), button, button_state) {
                super::windows::show_main_window(tray.app_handle());
            }
        })
        .build(app)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Show the app icon on the tray icon, when a tray icon exists (spec T5).
pub fn set_icon(app: &AppHandle, image: tauri::image::Image<'_>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) { let _ = tray.set_icon(Some(image)); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_needs_the_setting_and_a_way_back() {
        assert!(background_enabled(true, true, false)); // macOS: the Dock
        assert!(background_enabled(true, false, true)); // tray icon
        assert!(!background_enabled(true, false, false)); // no tray icon
        assert!(!background_enabled(false, true, true)); // setting off
        assert!(!background_enabled(false, false, false));
        assert!(!background_enabled(false, false, true));
    }

    #[test]
    fn the_unavailable_line_names_the_error() {
        assert_eq!(unavailable_line("no appindicator"), "The tray icon is not available: no appindicator. Discard quits when the last window closes.");
    }

    #[test]
    fn the_settings_show_a_line_only_when_the_tray_icon_failed() {
        assert_eq!(TrayState(Ok(())).unavailable(), None);
        assert_eq!(TrayState(Err("no appindicator".into())).unavailable(), Some(unavailable_line("no appindicator")));
    }

    #[test]
    fn the_tray_menu_items_run_the_main_window_and_quit_commands() {
        assert_eq!(MAIN_WINDOW_ID, "tray_main_window");
        assert_eq!(QUIT_ID, "tray_quit");
        assert_eq!(MAIN_WINDOW_LABEL, "Main Window");
        assert_eq!(QUIT_LABEL, "Quit Discard");
        assert_eq!(menu_command(MAIN_WINDOW_ID), Some(MenuCommand::ShowMain));
        assert_eq!(menu_command(QUIT_ID), Some(MenuCommand::Quit));
        assert_eq!(menu_command("quit"), None);
        assert_eq!(menu_command(""), None);
    }

    #[test]
    fn a_left_click_shows_the_main_window_on_windows_only() {
        use tauri::tray::{MouseButton, MouseButtonState};
        assert!(click_shows_main(true, MouseButton::Left, MouseButtonState::Up));
        assert!(!click_shows_main(true, MouseButton::Left, MouseButtonState::Down), "one click is one show");
        assert!(!click_shows_main(true, MouseButton::Right, MouseButtonState::Up), "the right click opens the menu");
        assert!(!click_shows_main(false, MouseButton::Left, MouseButtonState::Up), "on Linux the menu is the only path");
    }
}
