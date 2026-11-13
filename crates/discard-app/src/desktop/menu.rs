//! Native menu built from the core menu model.
use discard_core::action::{Action, MenuContext, MenuEntry, MenuTarget, StandardItem, menu_bar};
use discard_core::keymap::{Chord, Keymap};
use tauri::{AppHandle, Runtime};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

pub fn action_id(action: Action) -> &'static str { action.id() }
/// Describe a non-native menu entry for tests and the webview.
pub struct EntryDescription { pub label: String, pub enabled: bool }
pub fn describe_entry(entry: &MenuEntry) -> EntryDescription {
    match entry {
        MenuEntry::Action { label, enabled, .. } => EntryDescription { label: label.clone(), enabled: *enabled },
        MenuEntry::Future(item) => EntryDescription { label: format!("{} (Soon)", item.label), enabled: false },
        MenuEntry::Plugin { label, .. } => EntryDescription { label: label.clone(), enabled: true },
        _ => EntryDescription { label: String::new(), enabled: true },
    }
}
pub fn plugin_items(ctx: &MenuContext) -> Vec<(String, usize)> {
    ctx.plugin_items.iter().filter(|item| item.target == MenuTarget::App).map(|item| (item.plugin.clone(), item.index)).collect()
}
/// Convert a core chord to a Tauri accelerator.
pub fn accelerator(chord: &Chord) -> String {
    let raw = chord.as_str();
    let (prefix, key) = if let Some(prefix) = raw.strip_suffix("--") { (prefix, "-") } else { raw.rsplit_once('-').unwrap_or(("", raw)) };
    let mut parts: Vec<String> = prefix.split('-').filter(|s| !s.is_empty()).map(|part| match part { "cmd" => "CmdOrCtrl", "ctrl" => "Ctrl", "alt" => "Alt", "shift" => "Shift", other => other }.to_string()).collect();
    parts.push(match key { "enter" => "Enter".into(), "escape" => "Escape".into(), "space" => "Space".into(), "up" => "ArrowUp".into(), "down" => "ArrowDown".into(), "left" => "ArrowLeft".into(), "right" => "ArrowRight".into(), _ => key.to_uppercase() });
    parts.join("+")
}
/// Bind actions that have no native menu item or no native accelerator representation.
pub fn web_shortcuts(keymap: &Keymap) -> Vec<(String, String)> {
    let actions: std::collections::HashSet<Action> = menu_bar(&MenuContext::default()).into_iter().flat_map(|menu| menu.entries).filter_map(|entry| match entry { MenuEntry::Action { action, .. } => Some(action), _ => None }).collect();
    keymap.bindings().into_iter().filter(|(action, chord)| {
        let raw = chord.as_str();
        let key = if raw.ends_with("--") { "-" } else { raw.rsplit('-').next().unwrap_or(raw) };
        let native = key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric() || "`,./;['\\]=-".contains(c)) || matches!(key, "enter" | "escape" | "tab" | "space" | "backspace" | "delete" | "up" | "down" | "left" | "right" | "home" | "end" | "pageup" | "pagedown" | "insert") || key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=12).contains(&n));
        !actions.contains(action) || !native
    }).map(|(action, chord)| (action.id().to_string(), chord.as_str().to_string())).collect()
}
/// Whether this platform can carry out `action`. Hide Others and Show All exist on macOS only.
/// Hide Discard is also left out elsewhere: with no Dock, nothing brings a hidden window back.
pub fn platform_action(action: Action, macos: bool) -> bool {
    macos || !matches!(action, Action::HideApp | Action::HideOthers | Action::ShowAll)
}
/// The entries this platform can carry out. Services is a macOS menu. The GTK menu has no Undo or
/// Redo item; the text inputs handle those keys themselves.
pub fn platform_entries(entries: Vec<MenuEntry>, macos: bool) -> Vec<MenuEntry> {
    if macos { return entries; }
    let mut kept: Vec<MenuEntry> = Vec::new();
    for entry in entries {
        match entry {
            MenuEntry::Services | MenuEntry::Standard(StandardItem::Undo | StandardItem::Redo) => {}
            MenuEntry::Action { action, .. } if !platform_action(action, macos) => {}
            MenuEntry::Separator if kept.last().is_none_or(|last| *last == MenuEntry::Separator) => {}
            entry => kept.push(entry),
        }
    }
    if kept.last() == Some(&MenuEntry::Separator) { kept.pop(); }
    kept
}
/// Build the native menu from the current core state and keymap.
pub fn build_menu<R: Runtime>(app: &AppHandle<R>, ctx: &MenuContext, keymap: &Keymap) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    for spec in menu_bar(ctx) {
        let submenu = Submenu::new(app, spec.title, true)?;
        for entry in platform_entries(spec.entries, cfg!(target_os = "macos")) {
            match entry {
                MenuEntry::Action { label, action, checked, enabled } => {
                    let shortcut = keymap.chord(action).map(accelerator);
                    match action {
                        Action::ToggleSafeMode => submenu.append(&CheckMenuItem::with_id(app, action_id(action), label, enabled, checked, shortcut.as_deref())?)?,
                        _ => submenu.append(&MenuItem::with_id(app, action_id(action), label, enabled, shortcut.as_deref())?)?,
                    }
                }
                MenuEntry::Future(item) => submenu.append(&MenuItem::new(app, format!("{} (Soon)", item.label), false, None::<&str>)?)?,
                MenuEntry::Plugin { plugin, index, label } => submenu.append(&MenuItem::with_id(app, format!("plugin:{index}:{plugin}"), label, true, None::<&str>)?)?,
                MenuEntry::Separator => submenu.append(&PredefinedMenuItem::separator(app)?)?,
                MenuEntry::Services => submenu.append(&PredefinedMenuItem::services(app, None)?)?,
                MenuEntry::Standard(item) => {
                    let item = match item {
                        StandardItem::Undo => PredefinedMenuItem::undo(app, None)?,
                        StandardItem::Redo => PredefinedMenuItem::redo(app, None)?,
                        StandardItem::Cut => PredefinedMenuItem::cut(app, None)?,
                        StandardItem::Copy => PredefinedMenuItem::copy(app, None)?,
                        StandardItem::Paste => PredefinedMenuItem::paste(app, None)?,
                        StandardItem::SelectAll => PredefinedMenuItem::select_all(app, None)?,
                    };
                    submenu.append(&item)?;
                }
            }
        }
        menu.append(&submenu)?;
    }
    Ok(menu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::action::{Action, MenuContext, MenuEntry, MenuTarget, PluginMenuItem, StandardItem, FUTURE_ITEMS};
    use discard_core::keymap::parse_chord;

    #[test]
    fn each_core_action_has_a_menu_item_id() {
        let ids: std::collections::HashSet<_> = Action::all().iter().map(|action| action_id(*action)).collect();
        assert_eq!(ids.len(), Action::all().len());
    }
    #[test]
    fn future_entry_becomes_a_disabled_item() {
        let item = describe_entry(&MenuEntry::Future(&FUTURE_ITEMS[0]));
        assert!(!item.enabled);
        assert!(item.label.contains("Soon"));
    }
    #[test]
    fn plugin_items_are_listed_once_in_menu_order() {
        let ctx = MenuContext { plugin_items: vec![PluginMenuItem { plugin: "p".into(), index: 2, target: MenuTarget::App, label: "Pin".into() }, PluginMenuItem { plugin: "q".into(), index: 1, target: MenuTarget::App, label: "Next".into() }], ..Default::default() };
        assert_eq!(plugin_items(&ctx), vec![("p".into(), 2), ("q".into(), 1)]);
    }
    #[test]
    fn web_shortcuts_cover_actions_without_native_menu_entries() {
        let (keymap, issues) = discard_core::keymap::resolve(&[("open_agent_settings".into(), "cmd-k".into())]);
        assert!(issues.is_empty());
        assert!(web_shortcuts(&keymap).contains(&("open_agent_settings".into(), "cmd-k".into())));
        assert!(!web_shortcuts(&keymap).iter().any(|(id, _)| id == "quit"));
    }
    #[test]
    fn other_platforms_drop_the_macos_only_app_entries() {
        let app = menu_bar(&MenuContext::default()).remove(0).entries;
        assert_eq!(platform_entries(app.clone(), true), app);
        let labels: Vec<String> = platform_entries(app, false).iter().map(|entry| match entry { MenuEntry::Separator => "-".into(), other => describe_entry(other).label }).collect();
        assert_eq!(labels, ["Settings…", "-", "Quit Discard"]);
        let edit = menu_bar(&MenuContext::default()).remove(2).entries;
        assert_eq!(platform_entries(edit, false), [StandardItem::Cut, StandardItem::Copy, StandardItem::Paste, StandardItem::SelectAll].map(MenuEntry::Standard));
    }
    #[test]
    fn chord_converts_to_a_tauri_accelerator() {
        assert_eq!(accelerator(&parse_chord("cmd-,").unwrap()), "CmdOrCtrl+,");
        assert_eq!(accelerator(&parse_chord("alt-cmd-h").unwrap()), "Alt+CmdOrCtrl+H");
    }
}
