//! The native menu bar, the GPUI action types, and the key bindings.
//! Rust builds the menu bar from `discard_core::action::menu_bar`. Plugins cannot change it.

use discard_core::action::{
    Action, MenuContext, MenuEntry, StandardItem, menu_bar,
};
use gpui_kit::component::input;
use gpui_kit::{App, Context, Global, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, Window};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use discard_core::context_menu::ContextTarget;

#[derive(Clone, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = discard, no_json)]
struct RunPluginItem { slot: usize }
struct PluginMenuTable(Vec<(String, usize)>);
impl Global for PluginMenuTable {}

fn plugin_table(ctx: &MenuContext) -> Vec<(String, usize)> {
    ctx.plugin_items.iter().filter(|item| item.target == discard_core::action::MenuTarget::App).take(32).map(|item| (item.plugin.clone(), item.index)).collect()
}

pub(crate) fn to_popup(entries: Vec<MenuEntry>, target: ContextTarget) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |mut menu, _, _| {
        for entry in entries.clone() {
            match entry {
                MenuEntry::Action { label, action, enabled, .. } => {
                    let target = target.clone();
                    menu = menu.item(PopupMenuItem::new(label).disabled(!enabled).on_click(move |_, _, cx| crate::dispatch::run_on(action, Some(target.clone()), cx)));
                }
                MenuEntry::Plugin { plugin, index, label } => {
                    let target = target.clone();
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| crate::dispatch::run_plugin_item(&plugin, index, Some(&target), cx)));
                }
                MenuEntry::Separator => menu = menu.separator(),
                _ => {}
            }
        }
        menu
    }
}

/// Declares one GPUI action type per core `Action`, plus `Soon`, and the one mapping between them.
/// The `match` in `boxed` is exhaustive, so a new core `Action` does not compile until it is listed here.
macro_rules! action_types {
    ($($name:ident),* $(,)?) => {
        gpui_kit::actions!(discard, [$($name,)* Soon]);

        /// The GPUI action for a core `Action`.
        pub(crate) fn boxed(action: Action) -> Box<dyn gpui_kit::Action> {
            match action {
                $(Action::$name => Box::new($name),)*
            }
        }

        /// Send each GPUI action to `dispatch::run`. `Soon` gets no handler, so the OS greys its items.
        pub(crate) fn register_handlers(cx: &mut App) {
            $(cx.on_action(|_: &$name, cx: &mut App| crate::dispatch::run(Action::$name, cx));)*
            cx.on_action(|action: &RunPluginItem, cx: &mut App| {
                let item = cx.try_global::<PluginMenuTable>().and_then(|table| table.0.get(action.slot)).cloned();
                if let Some((plugin, index)) = item { crate::dispatch::run_plugin_item(&plugin, index, None, cx); }
            });
        }
    };
}

action_types!(
    CopyDiff, CopyToolInput, CopyToolOutput, CopyAgentName, CopyProjectName, CopyProjectPath, OpenAgentSettings, OpenWorkspaceSettings,
    ArchiveChannel,
    CopyMessageText,
    CopyCode,
    CopyChannelName,
    OpenSettings,
    Quit,
    CancelTurn,
    ReviewPlugins,
    ShowPluginErrors,
    ToggleSafeMode,
    HideApp,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    CloseWindow,
);

/// Bind `default_keymap()`. System keys other than those in the keymap stay with the OS and the inputs.
pub(crate) fn bind_keys(cx: &mut App, map: &discard_core::keymap::Keymap) {
    let bindings: Vec<KeyBinding> = map.bindings()
        .iter()
        .filter_map(|(action, keys)| {
            KeyBinding::load(keys.as_str(), boxed(*action), None, false, None, cx)
                .map_err(|e| tracing::warn!(action = action.id(), keys = keys.as_str(), error = ?e, "invalid key binding"))
                .ok()
        })
        .collect();
    cx.bind_keys(bindings);
}

fn standard(item: StandardItem) -> MenuItem {
    match item {
        StandardItem::Undo => MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
        StandardItem::Redo => MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
        StandardItem::Cut => MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
        StandardItem::Copy => MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
        StandardItem::Paste => MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
        StandardItem::SelectAll => {
            MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll)
        }
    }
}

/// Convert one core menu entry to a GPUI menu item.
///
/// macOS enables a menu item when its action has a handler, and ignores the `disabled` flag.
/// So a disabled entry carries the handler-less `Soon` action, and macOS greys it. The flag is still set
/// for platforms that read it.
pub fn to_gpui(entry: &MenuEntry) -> MenuItem {
    to_gpui_with_slot(entry, None)
}

fn to_gpui_with_slot(entry: &MenuEntry, slot: Option<usize>) -> MenuItem {
    match entry {
        MenuEntry::Plugin { label, .. } => match slot { Some(slot) => MenuItem::action(label.clone(), RunPluginItem { slot }), None => MenuItem::action(label.clone(), Soon).disabled(true) },
        MenuEntry::Action {
            label,
            action,
            checked,
            enabled,
        } => {
            let item = if *enabled {
                MenuItem::Action {
                    name: label.clone().into(),
                    action: boxed(*action),
                    os_action: None,
                    checked: *checked,
                    disabled: false,
                }
            } else {
                MenuItem::action(label.clone(), Soon)
            };
            item.checked(*checked).disabled(!enabled)
        }
        MenuEntry::Standard(item) => standard(*item),
        // Native menus have no tooltip, so the label carries the "Soon" tag.
        MenuEntry::Future(item) => {
            MenuItem::action(format!("{} (Soon)", item.label), Soon).disabled(true)
        }
        MenuEntry::Separator => MenuItem::separator(),
        MenuEntry::Services => MenuItem::os_submenu("Services", SystemMenuType::Services),
    }
}

/// Build the menu bar for `ctx` and give it to the OS. Calling it again replaces the menu bar.
pub fn install(ctx: &MenuContext, cx: &mut App) {
    let table = plugin_table(ctx);
    cx.set_global(PluginMenuTable(table));
    let mut slot = 0usize;
    let menus = menu_bar(ctx)
        .into_iter()
        .map(|spec| Menu::new(spec.title).items(spec.entries.iter().filter_map(|entry| {
            if matches!(entry, MenuEntry::Plugin { .. }) {
                let current = slot;
                slot += 1;
                (current < 32).then(|| to_gpui_with_slot(entry, Some(current)))
            } else { Some(to_gpui(entry)) }
        })));
    cx.set_menus(menus);
}

#[cfg(test)]
mod tests {
    use super::{Soon, boxed, to_gpui};
    use discard_core::action::{Action, FUTURE_ITEMS, MenuEntry, StandardItem};
    use gpui_kit::{MenuItem, OsAction, SystemMenuType};

    #[test]
    fn plugin_items_for_the_new_targets_are_kept_out_of_the_app_table() {
        use discard_core::action::{MenuContext,MenuTarget,PluginMenuItem};
        let items=[MenuTarget::Diff,MenuTarget::ToolCall,MenuTarget::Agent,MenuTarget::Server].into_iter().map(|target|PluginMenuItem {plugin:"p".into(),index:0,target,label:"Run".into()}).collect();
        assert!(super::plugin_table(&MenuContext {plugin_items:items,..Default::default()}).is_empty());
    }
    #[test]
    fn plugin_table_lists_each_plugin_item_once_in_menu_order() {
        use discard_core::action::{MenuContext, MenuTarget, PluginMenuItem};
        let ctx = MenuContext { plugin_items: vec![PluginMenuItem { plugin: "notes".into(), index: 2, target: MenuTarget::App, label: "Pin".into() }], ..Default::default() };
        let table = super::plugin_table(&ctx);
        assert_eq!(table, vec![("notes".into(), 2)]);
    }

    #[test]
    fn future_entry_becomes_a_disabled_item() {
        let item = to_gpui(&MenuEntry::Future(&FUTURE_ITEMS[0]));
        match item {
            MenuItem::Action {
                name,
                action,
                checked,
                disabled,
                os_action,
            } => {
                assert_eq!(name.as_ref(), format!("{} (Soon)", FUTURE_ITEMS[0].label));
                assert!(disabled);
                assert!(!checked);
                assert!(os_action.is_none());
                assert!(
                    action.partial_eq(&Soon),
                    "a Future entry must carry the handler-less Soon action"
                );
            }
            _ => panic!("a Future entry must be an action item"),
        }
    }

    #[test]
    fn action_entry_keeps_label_checked_and_disabled_flags() {
        let entry = MenuEntry::Action {
            label: "Safe Mode".to_string(),
            action: Action::ToggleSafeMode,
            checked: true,
            enabled: true,
        };
        match to_gpui(&entry) {
            MenuItem::Action {
                name,
                action,
                checked,
                disabled,
                ..
            } => {
                assert_eq!(name.as_ref(), "Safe Mode");
                assert!(checked);
                assert!(!disabled);
                assert!(action.partial_eq(boxed(Action::ToggleSafeMode).as_ref()));
            }
            _ => panic!("an Action entry must be an action item"),
        }

        let entry = MenuEntry::Action {
            label: "Cancel Turn".to_string(),
            action: Action::CancelTurn,
            checked: false,
            enabled: false,
        };
        match to_gpui(&entry) {
            MenuItem::Action {
                name,
                action,
                checked,
                disabled,
                ..
            } => {
                assert_eq!(name.as_ref(), "Cancel Turn");
                assert!(!checked);
                assert!(disabled);
                assert!(
                    action.partial_eq(&Soon),
                    "a disabled entry must carry Soon so macOS greys it"
                );
            }
            _ => panic!("an Action entry must be an action item"),
        }
    }

    #[test]
    fn separator_and_services_convert() {
        assert!(matches!(
            to_gpui(&MenuEntry::Separator),
            MenuItem::Separator
        ));
        match to_gpui(&MenuEntry::Services) {
            MenuItem::SystemMenu(menu) => {
                assert!(menu.menu_type == SystemMenuType::Services);
                assert_eq!(menu.name.as_ref(), "Services");
            }
            _ => panic!("Services must be a system menu"),
        }
    }

    #[test]
    fn standard_entries_carry_their_os_action() {
        let cases = [
            (StandardItem::Undo, OsAction::Undo),
            (StandardItem::Redo, OsAction::Redo),
            (StandardItem::Cut, OsAction::Cut),
            (StandardItem::Copy, OsAction::Copy),
            (StandardItem::Paste, OsAction::Paste),
            (StandardItem::SelectAll, OsAction::SelectAll),
        ];
        for (standard, expected) in cases {
            match to_gpui(&MenuEntry::Standard(standard)) {
                MenuItem::Action {
                    os_action,
                    disabled,
                    ..
                } => {
                    assert!(os_action == Some(expected), "{standard:?}");
                    assert!(!disabled, "{standard:?}");
                }
                _ => panic!("{standard:?} must be an action item"),
            }
        }
    }

    #[test]
    fn each_core_action_maps_to_its_own_gpui_action() {
        let names: Vec<&str> = Action::all().iter().map(|a| boxed(*a).name()).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), Action::all().len(), "{names:?}");
        assert!(
            names.iter().all(|n| n.starts_with("discard::")),
            "{names:?}"
        );
        assert!(!names.contains(&"discard::Soon"));
    }
}
