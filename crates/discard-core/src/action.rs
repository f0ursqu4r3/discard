//! Menu actions, the menu model, the default keymap, and confirm rules.
//! Pure logic. The GPUI layer renders `menu_bar` and binds `default_keymap`.

/// A user command that menus, keys, and banners all dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    OpenSettings,
    Quit,
    CancelTurn,
    ReviewPlugins,
    ShowPluginErrors,
    ToggleSafeMode,
    /// Hide Discard (Cmd+H).
    HideApp,
    /// Hide every other app.
    HideOthers,
    /// Show every hidden app.
    ShowAll,
    /// Minimize the focused window (Cmd+M).
    Minimize,
    /// Zoom the focused window.
    Zoom,
    /// Close the focused window (Cmd+W). Closing the main window ends the app, so it can need a confirm.
    CloseWindow,
}

impl Action {
    /// Stable identifier, safe to use in a keymap or a deep link.
    pub fn id(self) -> &'static str {
        match self {
            Action::OpenSettings => "open_settings",
            Action::Quit => "quit",
            Action::CancelTurn => "cancel_turn",
            Action::ReviewPlugins => "review_plugins",
            Action::ShowPluginErrors => "show_plugin_errors",
            Action::ToggleSafeMode => "toggle_safe_mode",
            Action::HideApp => "hide_app",
            Action::HideOthers => "hide_others",
            Action::ShowAll => "show_all",
            Action::Minimize => "minimize",
            Action::Zoom => "zoom",
            Action::CloseWindow => "close_window",
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        Action::all().iter().copied().find(|a| a.id() == id)
    }

    pub fn all() -> &'static [Action] {
        &[
            Action::OpenSettings,
            Action::Quit,
            Action::CancelTurn,
            Action::ReviewPlugins,
            Action::ShowPluginErrors,
            Action::ToggleSafeMode,
            Action::HideApp,
            Action::HideOthers,
            Action::ShowAll,
            Action::Minimize,
            Action::Zoom,
            Action::CloseWindow,
        ]
    }
}

/// A menu item with no backend yet. It shows greyed, tagged "Soon", with no handler.
#[derive(Debug, PartialEq, Eq)]
pub struct FutureItem {
    pub label: &'static str,
    pub tooltip: &'static str,
}

pub const FUTURE_ITEMS: &[FutureItem] = &[
    FutureItem { label: "New Channel…", tooltip: "Creating a channel from the menu is coming soon." },
    FutureItem { label: "Archive Channel…", tooltip: "Archiving a channel is coming soon." },
    FutureItem { label: "Switch Channel…", tooltip: "Switching channels from the menu is coming soon." },
    FutureItem { label: "Discard Documentation", tooltip: "The documentation is coming soon." },
];

/// An OS-standard Edit item. The platform handles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardItem {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Action { label: String, action: Action, checked: bool, enabled: bool },
    Standard(StandardItem),
    Future(&'static FutureItem),
    Separator,
    Services,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSpec {
    pub title: &'static str,
    pub entries: Vec<MenuEntry>,
}

/// App state that changes how menu entries look.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuContext {
    pub safe_mode: bool,
    pub turn_running: bool,
    pub pending_reviews: usize,
}

fn action(label: &str, action: Action) -> MenuEntry {
    MenuEntry::Action { label: label.to_string(), action, checked: false, enabled: true }
}

fn future(label: &str) -> MenuEntry {
    let item = FUTURE_ITEMS
        .iter()
        .find(|f| f.label == label)
        .expect("future item is declared in FUTURE_ITEMS");
    MenuEntry::Future(item)
}

/// The whole menu bar for the given app state.
pub fn menu_bar(ctx: &MenuContext) -> Vec<MenuSpec> {
    let review_label = if ctx.pending_reviews == 0 {
        "Review Plugins…".to_string()
    } else {
        format!("Review Plugins ({})…", ctx.pending_reviews)
    };
    vec![
        MenuSpec {
            title: "Discard",
            entries: vec![
                action("Settings…", Action::OpenSettings),
                MenuEntry::Separator,
                MenuEntry::Services,
                MenuEntry::Separator,
                action("Hide Discard", Action::HideApp),
                action("Hide Others", Action::HideOthers),
                action("Show All", Action::ShowAll),
                MenuEntry::Separator,
                action("Quit Discard", Action::Quit),
            ],
        },
        MenuSpec {
            title: "File",
            entries: vec![
                future("New Channel…"),
                future("Archive Channel…"),
                MenuEntry::Separator,
                action("Close Window", Action::CloseWindow),
                MenuEntry::Separator,
                MenuEntry::Action {
                    label: "Cancel Turn".to_string(),
                    action: Action::CancelTurn,
                    checked: false,
                    enabled: ctx.turn_running,
                },
            ],
        },
        MenuSpec {
            title: "Edit",
            entries: vec![
                MenuEntry::Standard(StandardItem::Undo),
                MenuEntry::Standard(StandardItem::Redo),
                MenuEntry::Separator,
                MenuEntry::Standard(StandardItem::Cut),
                MenuEntry::Standard(StandardItem::Copy),
                MenuEntry::Standard(StandardItem::Paste),
                MenuEntry::Standard(StandardItem::SelectAll),
            ],
        },
        MenuSpec { title: "View", entries: vec![future("Switch Channel…")] },
        MenuSpec {
            title: "Plugins",
            entries: vec![
                MenuEntry::Action {
                    label: review_label,
                    action: Action::ReviewPlugins,
                    checked: false,
                    enabled: true,
                },
                action("Plugin Errors…", Action::ShowPluginErrors),
                MenuEntry::Separator,
                MenuEntry::Action {
                    label: "Safe Mode".to_string(),
                    action: Action::ToggleSafeMode,
                    checked: ctx.safe_mode,
                    enabled: true,
                },
            ],
        },
        MenuSpec {
            title: "Window",
            entries: vec![action("Minimize", Action::Minimize), action("Zoom", Action::Zoom)],
        },
        MenuSpec { title: "Help", entries: vec![future("Discard Documentation")] },
    ]
}

/// Default key bindings. GPUI adds no window or app keys of its own, so Cmd+W, Cmd+H, and Cmd+M are listed here.
/// The edit keys (Cmd+C and so on) stay with the OS and the text inputs and are not listed.
pub fn default_keymap() -> &'static [(Action, &'static str)] {
    &[
        (Action::OpenSettings, "cmd-,"),
        (Action::Quit, "cmd-q"),
        (Action::CancelTurn, "cmd-."),
        (Action::HideApp, "cmd-h"),
        (Action::HideOthers, "alt-cmd-h"),
        (Action::Minimize, "cmd-m"),
        (Action::CloseWindow, "cmd-w"),
    ]
}

/// Whether to ask the user before running `action`.
pub fn needs_confirm(action: Action, turn_running: bool, confirm_while_working: bool) -> bool {
    turn_running
        && confirm_while_working
        && matches!(action, Action::Quit | Action::ToggleSafeMode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn all_entries(ctx: &MenuContext) -> Vec<MenuEntry> {
        menu_bar(ctx).into_iter().flat_map(|m| m.entries).collect()
    }

    fn find_action(ctx: &MenuContext, action: Action) -> (String, bool, bool) {
        all_entries(ctx)
            .into_iter()
            .find_map(|e| match e {
                MenuEntry::Action { label, action: a, checked, enabled } if a == action => {
                    Some((label, checked, enabled))
                }
                _ => None,
            })
            .expect("action in menu bar")
    }

    #[test]
    fn titles_are_in_order() {
        let titles: Vec<_> = menu_bar(&MenuContext::default()).iter().map(|m| m.title).collect();
        assert_eq!(titles, ["Discard", "File", "Edit", "View", "Plugins", "Window", "Help"]);
    }

    #[test]
    fn every_action_appears_in_the_menu_bar() {
        let entries = all_entries(&MenuContext::default());
        for action in Action::all() {
            assert!(
                entries.iter().any(|e| matches!(e, MenuEntry::Action { action: a, .. } if a == action)),
                "{action:?} missing from the menu bar"
            );
        }
    }

    #[test]
    fn ids_round_trip() {
        for action in Action::all() {
            assert_eq!(Action::from_id(action.id()), Some(*action));
        }
        assert_eq!(Action::from_id("nope"), None);
    }

    #[test]
    fn cancel_turn_enabled_only_while_a_turn_runs() {
        let idle = MenuContext::default();
        let running = MenuContext { turn_running: true, ..Default::default() };
        assert!(!find_action(&idle, Action::CancelTurn).2);
        assert!(find_action(&running, Action::CancelTurn).2);
    }

    #[test]
    fn safe_mode_entry_is_checked_when_safe_mode_is_on() {
        let off = MenuContext::default();
        let on = MenuContext { safe_mode: true, ..Default::default() };
        assert!(!find_action(&off, Action::ToggleSafeMode).1);
        assert!(find_action(&on, Action::ToggleSafeMode).1);
    }

    #[test]
    fn review_label_counts_pending() {
        let none = MenuContext::default();
        let two = MenuContext { pending_reviews: 2, ..Default::default() };
        assert_eq!(find_action(&none, Action::ReviewPlugins).0, "Review Plugins…");
        assert_eq!(find_action(&two, Action::ReviewPlugins).0, "Review Plugins (2)…");
    }

    #[test]
    fn future_items_are_in_the_menu_as_future_entries() {
        let entries = all_entries(&MenuContext::default());
        assert_eq!(FUTURE_ITEMS.len(), 4);
        for item in FUTURE_ITEMS {
            assert!(
                entries.iter().any(|e| matches!(e, MenuEntry::Future(f) if f.label == item.label)),
                "{} missing as a Future entry",
                item.label
            );
        }
    }

    #[test]
    fn keymap_has_no_duplicate_keys() {
        let keys: HashSet<_> = default_keymap().iter().map(|(_, k)| *k).collect();
        assert_eq!(keys.len(), default_keymap().len());
        assert!(default_keymap().contains(&(Action::OpenSettings, "cmd-,")));
        assert!(default_keymap().contains(&(Action::Quit, "cmd-q")));
        assert!(default_keymap().contains(&(Action::CancelTurn, "cmd-.")));
        assert!(default_keymap().contains(&(Action::HideApp, "cmd-h")));
        assert!(default_keymap().contains(&(Action::HideOthers, "alt-cmd-h")));
        assert!(default_keymap().contains(&(Action::Minimize, "cmd-m")));
        assert!(default_keymap().contains(&(Action::CloseWindow, "cmd-w")));
    }

    fn actions_in(title: &str) -> Vec<(String, Action)> {
        menu_bar(&MenuContext::default())
            .into_iter()
            .find(|m| m.title == title)
            .expect("menu exists")
            .entries
            .into_iter()
            .filter_map(|e| match e {
                MenuEntry::Action { label, action, .. } => Some((label, action)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn discard_menu_has_the_standard_app_items_in_order() {
        let menu = &menu_bar(&MenuContext::default())[0];
        assert_eq!(menu.title, "Discard");
        let services = menu.entries.iter().position(|e| *e == MenuEntry::Services).expect("Services");
        assert_eq!(
            actions_in("Discard"),
            [
                ("Settings…".to_string(), Action::OpenSettings),
                ("Hide Discard".to_string(), Action::HideApp),
                ("Hide Others".to_string(), Action::HideOthers),
                ("Show All".to_string(), Action::ShowAll),
                ("Quit Discard".to_string(), Action::Quit),
            ]
        );
        let hide = menu
            .entries
            .iter()
            .position(|e| matches!(e, MenuEntry::Action { action: Action::HideApp, .. }))
            .expect("Hide Discard");
        assert!(services < hide, "Services comes before Hide Discard");
    }

    #[test]
    fn file_menu_has_close_window_and_window_menu_has_minimize_and_zoom() {
        assert!(actions_in("File").contains(&("Close Window".to_string(), Action::CloseWindow)));
        assert_eq!(
            actions_in("Window"),
            [("Minimize".to_string(), Action::Minimize), ("Zoom".to_string(), Action::Zoom)]
        );
    }

    #[test]
    fn confirm_only_for_quit_and_safe_mode_while_working() {
        for action in Action::all() {
            for turn_running in [false, true] {
                for confirm in [false, true] {
                    let expected = matches!(action, Action::Quit | Action::ToggleSafeMode)
                        && turn_running
                        && confirm;
                    assert_eq!(
                        needs_confirm(*action, turn_running, confirm),
                        expected,
                        "{action:?} turn_running={turn_running} confirm={confirm}"
                    );
                }
            }
        }
    }
}
