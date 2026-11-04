//! The window theme: Ghostty colors, then plugin overrides on top. The pure parts
//! live here and are tested without a window. `apply` writes the result to GPUI.

use discard_core::ghostty::color::Rgb as CoreRgb;
use discard_core::ghostty::live::LiveTheme;
use discard_core::ghostty::map::UiColors;
use discard_core::ghostty::resolve::ThemeProblem;
use discard_core::ghostty::spec::Appearance;
use discard_core::settings::AppearanceSettings;
use discard_core::ghostty::lookup::ThemeDirs;
use discard_core::ghostty::resolve::ghostty_config_paths;
use discard_lua::theme::{Rgb, ThemeOverrides};
use gpui_kit::component::{Theme, ThemeConfig};
use gpui_kit::{App, WindowAppearance};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// gpui-component's radius when a theme sets none. `apply_config` keeps the old radius when
/// it gets none, so `apply` always passes one.
const DEFAULT_RADIUS: f32 = 6.0;

/// The 14 plugin theme tokens, as fields of `UiColors`.
const TOKENS: [&str; 14] = [
    "background", "foreground", "sidebar", "sidebar_foreground", "sidebar_accent", "primary",
    "primary_foreground", "muted", "muted_foreground", "border", "accent", "success", "warning", "danger",
];

fn to_core(c: Rgb) -> CoreRgb {
    CoreRgb { r: c.0, g: c.1, b: c.2 }
}

pub(crate) fn to_lua(c: CoreRgb) -> Rgb {
    Rgb(c.r, c.g, c.b)
}

fn token_mut<'a>(ui: &'a mut UiColors, token: &str) -> Option<&'a mut CoreRgb> {
    Some(match token {
        "background" => &mut ui.background,
        "foreground" => &mut ui.foreground,
        "sidebar" => &mut ui.sidebar,
        "sidebar_foreground" => &mut ui.sidebar_foreground,
        "sidebar_accent" => &mut ui.sidebar_accent,
        "primary" => &mut ui.primary,
        "primary_foreground" => &mut ui.primary_foreground,
        "muted" => &mut ui.muted,
        "muted_foreground" => &mut ui.muted_foreground,
        "border" => &mut ui.border,
        "accent" => &mut ui.accent,
        "success" => &mut ui.success,
        "warning" => &mut ui.warning,
        "danger" => &mut ui.danger,
        _ => return None,
    })
}

/// The Ghostty colors with the plugin color tokens on top. With `ignore_plugins`, the plugin
/// tokens have no effect. The diff colors, selection, and caret come from Ghostty only.
pub fn layer(ui: &UiColors, overrides: &ThemeOverrides, ignore_plugins: bool) -> UiColors {
    let mut layered = ui.clone();
    if ignore_plugins {
        return layered;
    }
    for token in TOKENS {
        if let (Some(color), Some(slot)) = (overrides.color(token), token_mut(&mut layered, token)) {
            *slot = to_core(color);
        }
    }
    layered
}

/// The 14 tokens of `ui` as theme overrides. Protected surfaces read their palette from this,
/// so they keep their contrast clamp against the colors the user actually sees.
pub fn as_overrides(ui: &UiColors) -> ThemeOverrides {
    let mut overrides = ThemeOverrides::default();
    let mut copy = ui.clone();
    for token in TOKENS {
        if let Some(color) = token_mut(&mut copy, token) {
            // `to_hex` always gives a valid `#rrggbb`, so this cannot fail.
            let _ = overrides.set_color(token, &color.to_hex());
        }
    }
    overrides
}

/// A gpui-component theme file for `ui`. gpui-component derives the colors this file leaves out
/// (hover states, tabs, tables, lists) from the ones it sets.
pub fn theme_config_json(ui: &UiColors, name: &str, radius: Option<f32>) -> serde_json::Value {
    let hex = |c: CoreRgb| c.to_hex();
    // The text color that reads best on a filled color.
    let on = |fill: CoreRgb| hex(fill.more_readable(ui.background, ui.foreground));
    let mut config = serde_json::json!({
        "name": name,
        "mode": if ui.background.is_dark() { "dark" } else { "light" },
        "colors": {
            "background": hex(ui.background),
            "foreground": hex(ui.foreground),
            "border": hex(ui.border),
            "input.border": hex(ui.border),
            "caret": hex(ui.caret),
            "selection.background": hex(ui.selection),
            "link": hex(ui.primary),
            "ring": hex(ui.primary),
            "popover.background": hex(ui.background),
            "popover.foreground": hex(ui.foreground),
            "sidebar.background": hex(ui.sidebar),
            "sidebar.foreground": hex(ui.sidebar_foreground),
            "sidebar.border": hex(ui.border),
            "sidebar.accent.background": hex(ui.sidebar_accent),
            "sidebar.accent.foreground": hex(ui.sidebar_foreground),
            "accent.background": hex(ui.accent),
            "accent.foreground": hex(ui.foreground),
            "muted.background": hex(ui.muted),
            "muted.foreground": hex(ui.muted_foreground),
            "secondary.background": hex(ui.muted),
            "secondary.foreground": hex(ui.foreground),
            "primary.background": hex(ui.primary),
            "primary.foreground": hex(ui.primary_foreground),
            "success.background": hex(ui.success),
            "success.foreground": on(ui.success),
            "warning.background": hex(ui.warning),
            "warning.foreground": on(ui.warning),
            "danger.background": hex(ui.danger),
            "danger.foreground": on(ui.danger),
        },
    });
    if let Some(radius) = radius {
        config["radius"] = serde_json::json!(radius.round() as usize);
    }
    config
}

/// The system appearance, as the Ghostty `light:`/`dark:` choice.
pub fn system_appearance(appearance: WindowAppearance) -> Appearance {
    match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
    }
}

/// The theme folders and Ghostty config files on this machine.
pub fn theme_env(discard_home: &Path) -> (ThemeDirs, Vec<PathBuf>) {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    let resources = std::env::var_os("GHOSTTY_RESOURCES_DIR").map(PathBuf::from);
    let dirs = ThemeDirs::standard(discard_home, &home, xdg.as_deref(), resources.as_deref());
    (dirs, ghostty_config_paths(&home, xdg.as_deref()))
}

/// Write the state to the global GPUI theme and redraw every window.
pub fn apply(state: &AppearanceState, cx: &mut App) {
    let name = match state.theme_name() {
        "" => "Discard",
        name => name,
    };
    let json = theme_config_json(state.effective(), name, Some(state.radius().unwrap_or(DEFAULT_RADIUS)));
    match serde_json::from_value::<ThemeConfig>(json) {
        Ok(config) => {
            let config = Rc::new(config);
            Theme::update(cx, |theme| theme.apply_config(&config));
        }
        Err(e) => tracing::warn!(error = %e, "the theme config is not valid, so the theme is unchanged"),
    }
}

/// What the window should look like now. The timer in the main window calls `update`.
pub struct AppearanceState {
    live: LiveTheme,
    plugin_theme: ThemeOverrides,
    ignore_plugins: bool,
    effective: UiColors,
    radius: Option<f32>,
}

impl AppearanceState {
    pub fn new(live: LiveTheme, settings: &AppearanceSettings, plugin_theme: ThemeOverrides) -> Self {
        let (effective, radius) = compute(&live, &plugin_theme, settings.ignore_plugin_themes);
        AppearanceState { live, plugin_theme, ignore_plugins: settings.ignore_plugin_themes, effective, radius }
    }

    /// Check the settings, the system appearance, the theme files, and the plugin theme.
    /// Returns `true` when what the window shows is different now.
    pub fn update(&mut self, settings: &AppearanceSettings, system: Appearance, plugin_theme: &ThemeOverrides) -> bool {
        let live_changed = self.live.update(settings, system);
        let plugins_changed = *plugin_theme != self.plugin_theme
            || settings.ignore_plugin_themes != self.ignore_plugins;
        if !live_changed && !plugins_changed {
            return false;
        }
        let old_scales = self.scales();
        self.plugin_theme = plugin_theme.clone();
        self.ignore_plugins = settings.ignore_plugin_themes;
        let (effective, radius) = compute(&self.live, &self.plugin_theme, self.ignore_plugins);
        let changed = effective != self.effective || radius != self.radius || old_scales != self.scales();
        self.effective = effective;
        self.radius = radius;
        changed
    }

    pub fn scales(&self) -> (f32, f32) {
        scales(self.plugin_theme.font_scale(), self.plugin_theme.spacing_scale(), self.ignore_plugins)
    }

    pub fn effective(&self) -> &UiColors {
        &self.effective
    }

    /// The radius a plugin set, when plugin themes are on.
    pub fn radius(&self) -> Option<f32> {
        self.radius
    }

    /// The name of the Ghostty theme file in use. Empty when the Ghostty config names none.
    pub fn theme_name(&self) -> &str {
        &self.live.resolved().theme_name
    }

    /// The colors of the active theme file, before mapping.
    pub fn colors(&self) -> &discard_core::ghostty::file::ThemeColors {
        &self.live.resolved().colors
    }

    pub fn problems(&self) -> &[ThemeProblem] {
        &self.live.resolved().problems
    }

    /// The theme the protected surfaces use for their palette.
    pub fn protected_overrides(&self) -> ThemeOverrides {
        as_overrides(&self.effective)
    }
}

fn scales(font: Option<f32>, spacing: Option<f32>, ignore: bool) -> (f32, f32) {
    if ignore { (1.0, 1.0) } else {
        (font.unwrap_or(1.0).clamp(0.8, 1.5), spacing.unwrap_or(1.0).clamp(0.5, 2.0))
    }
}

fn compute(live: &LiveTheme, plugin_theme: &ThemeOverrides, ignore_plugins: bool) -> (UiColors, Option<f32>) {
    let effective = layer(&live.resolved().ui, plugin_theme, ignore_plugins);
    let radius = if ignore_plugins { None } else { plugin_theme.radius() };
    (effective, radius)
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::ghostty::file::parse;
    use discard_core::ghostty::map::map;
    use gpui_kit::component::ThemeConfig;

    fn ui(text: &str) -> UiColors {
        map(&parse(text).colors)
    }

    const DARK: &str = "background = #101010\nforeground = #f0f0f0\npalette = 1=#ff0000\npalette = 2=#00ff00\npalette = 3=#ffff00\npalette = 4=#0000ff\n";

    #[test]
    fn plugin_tokens_replace_the_ghostty_tokens() {
        let base = ui(DARK);
        let mut plugin = ThemeOverrides::default();
        plugin.set_color("primary", "#123456").unwrap();
        plugin.set_color("danger", "#abcdef").unwrap();
        let layered = layer(&base, &plugin, false);
        assert_eq!(layered.primary, CoreRgb { r: 0x12, g: 0x34, b: 0x56 });
        assert_eq!(layered.danger, CoreRgb { r: 0xab, g: 0xcd, b: 0xef });
        assert_eq!(layered.background, base.background, "an untouched token keeps Ghostty's color");
        assert_eq!(layered.success, base.success);
    }

    #[test]
    fn ignoring_plugin_themes_keeps_ghostty_alone() {
        let base = ui(DARK);
        let mut plugin = ThemeOverrides::default();
        plugin.set_color("primary", "#123456").unwrap();
        assert_eq!(layer(&base, &plugin, true), base);
    }

    #[test]
    fn plugins_cannot_change_selection_caret_or_diff_colors() {
        let base = ui(DARK);
        let mut plugin = ThemeOverrides::default();
        for token in TOKENS {
            plugin.set_color(token, "#010203").unwrap();
        }
        let layered = layer(&base, &plugin, false);
        assert_eq!(layered.selection, base.selection);
        assert_eq!(layered.caret, base.caret);
        assert_eq!(layered.diff_added, base.diff_added);
        assert_eq!(layered.diff_removed, base.diff_removed);
    }

    #[test]
    fn as_overrides_holds_all_14_tokens() {
        let base = ui(DARK);
        let overrides = as_overrides(&base);
        assert_eq!(overrides.color("background"), Some(Rgb(0x10, 0x10, 0x10)));
        assert_eq!(overrides.color("warning"), Some(to_lua(base.warning)));
        for token in TOKENS {
            assert!(overrides.color(token).is_some(), "{token}");
        }
    }

    #[test]
    fn protected_surfaces_follow_the_visible_theme() {
        use discard_lua::theme::{ProtectedPalette, protected_palette};
        let fallback = ProtectedPalette { background: Rgb(0, 0, 0), foreground: Rgb(1, 1, 1), border: Rgb(2, 2, 2) };
        let palette = protected_palette(&as_overrides(&ui(DARK)), fallback);
        assert_eq!(palette.background, Rgb(0x10, 0x10, 0x10));
        assert_eq!(palette.foreground, Rgb(0xf0, 0xf0, 0xf0));
    }

    fn config(ui: &UiColors, radius: Option<f32>) -> ThemeConfig {
        serde_json::from_value(theme_config_json(ui, "Test", radius)).expect("the JSON is a valid theme config")
    }

    fn color(value: &Option<gpui_kit::SharedString>) -> &str {
        value.as_deref().expect("the color is set")
    }

    #[test]
    fn the_theme_config_carries_the_colors_and_the_mode() {
        let dark = ui(DARK);
        let cfg = config(&dark, None);
        assert_eq!(cfg.name.as_ref(), "Test");
        assert!(cfg.mode.is_dark());
        assert_eq!(color(&cfg.colors.background), "#101010");
        assert_eq!(color(&cfg.colors.foreground), "#f0f0f0");
        assert_eq!(color(&cfg.colors.primary), "#0000ff");
        assert_eq!(color(&cfg.colors.danger), "#ff0000");
        assert_eq!(color(&cfg.colors.success), "#00ff00");
        assert_eq!(color(&cfg.colors.sidebar), &dark.sidebar.to_hex());
        assert_eq!(color(&cfg.colors.selection), &dark.selection.to_hex());
        assert_eq!(color(&cfg.colors.caret), &dark.caret.to_hex());
        assert_eq!(color(&cfg.colors.sidebar_border), &dark.border.to_hex());
    }

    #[test]
    fn a_light_background_gives_light_mode() {
        let light = ui("background = #fafafa\nforeground = #202020\n");
        assert!(!config(&light, None).mode.is_dark());
    }

    #[test]
    fn the_radius_is_set_only_when_given() {
        let base = ui(DARK);
        assert_eq!(config(&base, None).radius, None);
        assert_eq!(config(&base, Some(4.0)).radius, Some(4));
    }

    fn state(settings: &AppearanceSettings, system: Appearance, plugin: ThemeOverrides) -> AppearanceState {
        use discard_core::ghostty::lookup::ThemeDirs;
        let live = LiveTheme::new(ThemeDirs::default(), Vec::new(), settings, system);
        AppearanceState::new(live, settings, plugin)
    }

    fn plugin(token: &str, value: &str) -> ThemeOverrides {
        let mut overrides = ThemeOverrides::default();
        overrides.set_color(token, value).unwrap();
        overrides
    }

    #[test]
    fn scale_defaults_ignore_and_clamps() {
        assert_eq!(scales(None, None, false), (1.0, 1.0));
        assert_eq!(scales(Some(1.5), Some(2.0), true), (1.0, 1.0));
        assert_eq!(scales(Some(9.0), Some(0.1), false), (1.5, 0.5));
    }

    #[test]
    fn a_new_state_layers_the_plugin_theme_on_ghostty() {
        let settings = AppearanceSettings::default();
        let s = state(&settings, Appearance::Dark, plugin("primary", "#123456"));
        assert_eq!(s.theme_name(), "Catppuccin Mocha");
        assert_eq!(s.effective().primary, CoreRgb { r: 0x12, g: 0x34, b: 0x56 });
        assert!(s.problems().is_empty());
    }

    #[test]
    fn no_change_means_no_update() {
        let settings = AppearanceSettings::default();
        let mut s = state(&settings, Appearance::Dark, ThemeOverrides::default());
        assert!(!s.update(&settings, Appearance::Dark, &ThemeOverrides::default()));
    }

    #[test]
    fn a_new_system_appearance_is_an_update() {
        let settings = AppearanceSettings::default();
        let mut s = state(&settings, Appearance::Dark, ThemeOverrides::default());
        let dark_background = s.effective().background;
        assert!(s.update(&settings, Appearance::Light, &ThemeOverrides::default()));
        assert_eq!(s.theme_name(), "Catppuccin Latte");
        assert_ne!(s.effective().background, dark_background);
    }

    #[test]
    fn a_new_plugin_theme_is_an_update() {
        let settings = AppearanceSettings::default();
        let mut s = state(&settings, Appearance::Dark, ThemeOverrides::default());
        let theme = plugin("danger", "#fedcba");
        assert!(s.update(&settings, Appearance::Dark, &theme));
        assert_eq!(s.effective().danger, CoreRgb { r: 0xfe, g: 0xdc, b: 0xba });
        assert!(!s.update(&settings, Appearance::Dark, &theme));
        assert!(s.update(&settings, Appearance::Dark, &ThemeOverrides::default()), "removing a plugin restores Ghostty");
    }

    #[test]
    fn the_ignore_switch_matters_only_when_a_plugin_sets_something() {
        let off = AppearanceSettings::default();
        let on = AppearanceSettings { ignore_plugin_themes: true, ..AppearanceSettings::default() };
        let theme = plugin("primary", "#123456");

        let mut s = state(&off, Appearance::Dark, theme.clone());
        let with_plugin = s.effective().clone();
        assert!(s.update(&on, Appearance::Dark, &theme));
        assert_ne!(*s.effective(), with_plugin);
        assert!(s.update(&off, Appearance::Dark, &theme));
        assert_eq!(*s.effective(), with_plugin);

        let mut empty = state(&off, Appearance::Dark, ThemeOverrides::default());
        assert!(!empty.update(&on, Appearance::Dark, &ThemeOverrides::default()));
    }

    #[test]
    fn a_plugin_radius_applies_unless_plugin_themes_are_ignored() {
        let off = AppearanceSettings::default();
        let on = AppearanceSettings { ignore_plugin_themes: true, ..AppearanceSettings::default() };
        let mut theme = ThemeOverrides::default();
        theme.set_radius(4.0).unwrap();
        let mut s = state(&off, Appearance::Dark, theme.clone());
        assert_eq!(s.radius(), Some(4.0));
        assert!(s.update(&on, Appearance::Dark, &theme));
        assert_eq!(s.radius(), None);
    }

    #[test]
    fn a_missing_theme_shows_up_as_a_problem() {
        let settings = AppearanceSettings {
            theme_source: discard_core::ghostty::resolve::ThemeSource::Custom,
            theme: "Nope".to_string(),
            ignore_plugin_themes: false,
        };
        let s = state(&settings, Appearance::Dark, ThemeOverrides::default());
        assert_eq!(s.problems().len(), 1);
    }
}
