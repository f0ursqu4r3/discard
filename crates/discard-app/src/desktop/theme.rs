//! Theme resolution without GUI dependencies.
use super::dto::ThemeDto;
use discard_core::ghostty::color::Rgb as CoreRgb;
use discard_core::ghostty::live::LiveTheme;
use discard_core::ghostty::lookup::ThemeDirs;
use discard_core::ghostty::map::UiColors;
use discard_core::ghostty::resolve::{ThemeProblem, ghostty_config_paths};
use discard_core::ghostty::spec::Appearance;
use discard_core::settings::AppearanceSettings;
use discard_lua::theme::{
    ProtectedPalette, Rgb, ThemeOverrides, contrast_ratio, protected_palette,
};
use std::path::{Path, PathBuf};
const TOKENS: [&str; 14] = [
    "background",
    "foreground",
    "sidebar",
    "sidebar_foreground",
    "sidebar_accent",
    "primary",
    "primary_foreground",
    "muted",
    "muted_foreground",
    "border",
    "accent",
    "success",
    "warning",
    "danger",
];
const PROTECTED_DEFAULT: ProtectedPalette = ProtectedPalette {
    background: Rgb(0x1e, 0x1e, 0x1e),
    foreground: Rgb(0xf0, 0xf0, 0xf0),
    border: Rgb(0xff, 0xaa, 0x00),
};
fn to_core(c: Rgb) -> CoreRgb {
    CoreRgb {
        r: c.0,
        g: c.1,
        b: c.2,
    }
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
        if let (Some(color), Some(slot)) = (overrides.color(token), token_mut(&mut layered, token))
        {
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

/// The theme folders and Ghostty config files on this machine.
pub fn theme_env(discard_home: &Path) -> (ThemeDirs, Vec<PathBuf>) {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    let resources = std::env::var_os("GHOSTTY_RESOURCES_DIR").map(PathBuf::from);
    let dirs = ThemeDirs::standard(discard_home, &home, xdg.as_deref(), resources.as_deref());
    (dirs, ghostty_config_paths(&home, xdg.as_deref()))
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
    pub fn new(
        live: LiveTheme,
        settings: &AppearanceSettings,
        plugin_theme: ThemeOverrides,
    ) -> Self {
        let (effective, radius) = compute(&live, &plugin_theme, settings.ignore_plugin_themes);
        AppearanceState {
            live,
            plugin_theme,
            ignore_plugins: settings.ignore_plugin_themes,
            effective,
            radius,
        }
    }

    /// Check the settings, the system appearance, the theme files, and the plugin theme.
    /// Returns `true` when what the window shows is different now.
    pub fn update(
        &mut self,
        settings: &AppearanceSettings,
        system: Appearance,
        plugin_theme: &ThemeOverrides,
    ) -> bool {
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
        let changed =
            effective != self.effective || radius != self.radius || old_scales != self.scales();
        self.effective = effective;
        self.radius = radius;
        changed
    }

    pub fn scales(&self) -> (f32, f32) {
        scales(
            self.plugin_theme.font_scale(),
            self.plugin_theme.spacing_scale(),
            self.ignore_plugins,
        )
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
    if ignore {
        (1.0, 1.0)
    } else {
        (
            font.unwrap_or(1.0).clamp(0.8, 1.5),
            spacing.unwrap_or(1.0).clamp(0.5, 2.0),
        )
    }
}

fn compute(
    live: &LiveTheme,
    plugin_theme: &ThemeOverrides,
    ignore_plugins: bool,
) -> (UiColors, Option<f32>) {
    let effective = layer(&live.resolved().ui, plugin_theme, ignore_plugins);
    let radius = if ignore_plugins {
        None
    } else {
        plugin_theme.radius()
    };
    (effective, radius)
}

pub fn contrast_ok(fg: Rgb, bg: Rgb) -> bool {
    contrast_ratio(fg, bg) >= 4.5
}
pub fn theme_dto(state: &AppearanceState, _mode: Appearance) -> ThemeDto {
    let ui = state.effective();
    let overrides = as_overrides(ui);
    let tokens = TOKENS
        .iter()
        .filter_map(|name| {
            overrides
                .color(name)
                .map(|color| ((*name).to_string(), color.to_hex()))
        })
        .collect();
    let palette = if contrast_ok(to_lua(ui.foreground), to_lua(ui.background)) {
        protected_palette(&overrides, PROTECTED_DEFAULT)
    } else {
        PROTECTED_DEFAULT
    };
    let (font_scale, spacing_scale) = state.scales();
    ThemeDto {
        mode: if ui.background.is_dark() {
            "dark"
        } else {
            "light"
        }
        .into(),
        tokens,
        radius: state.radius(),
        font_scale,
        spacing_scale,
        protected_bg: palette.background.to_hex(),
        protected_fg: palette.foreground.to_hex(),
        protected_border: palette.border.to_hex(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::ghostty::file::parse;
    use discard_core::ghostty::map::map;
    use discard_core::ghostty::{live::LiveTheme, lookup::ThemeDirs, spec::Appearance};
    use discard_core::settings::AppearanceSettings;
    use discard_lua::theme::{Rgb, ThemeOverrides};
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
        assert_eq!(
            layered.primary,
            CoreRgb {
                r: 0x12,
                g: 0x34,
                b: 0x56
            }
        );
        assert_eq!(
            layered.danger,
            CoreRgb {
                r: 0xab,
                g: 0xcd,
                b: 0xef
            }
        );
        assert_eq!(
            layered.background, base.background,
            "an untouched token keeps Ghostty's color"
        );
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
        let fallback = ProtectedPalette {
            background: Rgb(0, 0, 0),
            foreground: Rgb(1, 1, 1),
            border: Rgb(2, 2, 2),
        };
        let palette = protected_palette(&as_overrides(&ui(DARK)), fallback);
        assert_eq!(palette.background, Rgb(0x10, 0x10, 0x10));
        assert_eq!(palette.foreground, Rgb(0xf0, 0xf0, 0xf0));
    }

    fn state(
        settings: &AppearanceSettings,
        system: Appearance,
        plugin: ThemeOverrides,
    ) -> AppearanceState {
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
        assert_eq!(
            s.effective().primary,
            CoreRgb {
                r: 0x12,
                g: 0x34,
                b: 0x56
            }
        );
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
        assert_eq!(
            s.effective().danger,
            CoreRgb {
                r: 0xfe,
                g: 0xdc,
                b: 0xba
            }
        );
        assert!(!s.update(&settings, Appearance::Dark, &theme));
        assert!(
            s.update(&settings, Appearance::Dark, &ThemeOverrides::default()),
            "removing a plugin restores Ghostty"
        );
    }

    #[test]
    fn the_ignore_switch_matters_only_when_a_plugin_sets_something() {
        let off = AppearanceSettings::default();
        let on = AppearanceSettings {
            ignore_plugin_themes: true,
            ..AppearanceSettings::default()
        };
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
        let on = AppearanceSettings {
            ignore_plugin_themes: true,
            ..AppearanceSettings::default()
        };
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
    fn sample_state(theme: ThemeOverrides) -> AppearanceState {
        let settings = AppearanceSettings::default();
        AppearanceState::new(
            LiveTheme::new(ThemeDirs::default(), vec![], &settings, Appearance::Dark),
            &settings,
            theme,
        )
    }
    #[test]
    fn theme_dto_carries_all_14_tokens() {
        assert_eq!(
            theme_dto(&sample_state(ThemeOverrides::default()), Appearance::Dark)
                .tokens
                .len(),
            14
        );
    }
    #[test]
    fn a_light_background_gives_light_mode() {
        let mut t = ThemeOverrides::default();
        t.set_color("background", "#fafafa").unwrap();
        assert_eq!(theme_dto(&sample_state(t), Appearance::Dark).mode, "light");
    }
    #[test]
    fn protected_palette_falls_back_when_contrast_is_low() {
        let mut t = ThemeOverrides::default();
        t.set_color("background", "#101010").unwrap();
        t.set_color("foreground", "#101010").unwrap();
        let dto = theme_dto(&sample_state(t), Appearance::Dark);
        assert!(contrast_ok(
            parse_rgb(&dto.protected_fg),
            parse_rgb(&dto.protected_bg)
        ));
    }
    fn parse_rgb(s: &str) -> Rgb {
        Rgb(
            u8::from_str_radix(&s[1..3], 16).unwrap(),
            u8::from_str_radix(&s[3..5], 16).unwrap(),
            u8::from_str_radix(&s[5..7], 16).unwrap(),
        )
    }
}
