//! Map Ghostty colors to the colors Discard's UI uses.

use super::color::Rgb;
use super::file::ThemeColors;

/// How far each derived color moves from the background toward the
/// foreground. Tune the look here.
pub const SIDEBAR_MIX: f64 = 0.06;
pub const MUTED_MIX: f64 = 0.08;
pub const ACCENT_MIX: f64 = 0.12;
pub const BORDER_MIX: f64 = 0.18;
pub const SELECTION_MIX: f64 = 0.25;
/// When palette 8 has too little contrast, `muted_foreground` is the
/// foreground moved this far toward the background.
pub const MUTED_FOREGROUND_MIX: f64 = 0.40;
/// Minimum contrast of the foreground against the sidebar.
pub const SIDEBAR_MIN_CONTRAST: f64 = 4.5;
/// Minimum contrast for `muted_foreground` to use palette 8.
pub const MUTED_FOREGROUND_MIN_CONTRAST: f64 = 4.5;

/// Ghostty's own defaults, used for a color a theme does not set.
pub const DEFAULT_BACKGROUND: Rgb = Rgb { r: 0x28, g: 0x2c, b: 0x34 };
pub const DEFAULT_FOREGROUND: Rgb = Rgb { r: 0xff, g: 0xff, b: 0xff };
const DEFAULT_PALETTE: [u32; 16] = [
    0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, 0x666666,
    0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
];

/// The UI colors for one theme. The first 14 fields have the names of the
/// plugin theme tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiColors {
    pub background: Rgb,
    pub foreground: Rgb,
    pub sidebar: Rgb,
    pub sidebar_foreground: Rgb,
    pub sidebar_accent: Rgb,
    pub primary: Rgb,
    pub primary_foreground: Rgb,
    pub muted: Rgb,
    pub muted_foreground: Rgb,
    pub border: Rgb,
    pub accent: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub danger: Rgb,
    /// Palette colors 1 through 6 for identity colors.
    pub ansi: [Rgb; 6],
    /// Text selection background.
    pub selection: Rgb,
    /// Text caret.
    pub caret: Rgb,
    /// Added lines in a diff.
    pub diff_added: Rgb,
    /// Removed lines in a diff.
    pub diff_removed: Rgb,
}

/// Map `colors` to UI colors. A color the theme does not set uses
/// Ghostty's default.
pub fn map(colors: &ThemeColors) -> UiColors {
    let background = colors.background.unwrap_or(DEFAULT_BACKGROUND);
    let foreground = colors.foreground.unwrap_or(DEFAULT_FOREGROUND);
    let palette = |n: usize| colors.palette[n].unwrap_or_else(|| default_palette(n));
    let toward_foreground = |fraction: f64| background.mix(foreground, fraction);

    let muted_foreground = Some(palette(8))
        .filter(|c| c.contrast(background) >= MUTED_FOREGROUND_MIN_CONTRAST)
        .unwrap_or_else(|| foreground.mix(background, MUTED_FOREGROUND_MIX));
    let primary = palette(4);
    // The sidebar carries text. Keep it on the background when a lighter
    // or darker shade would drop the text below the minimum contrast.
    let sidebar = Some(toward_foreground(SIDEBAR_MIX))
        .filter(|c| foreground.contrast(*c) >= SIDEBAR_MIN_CONTRAST)
        .unwrap_or(background);

    UiColors {
        background,
        foreground,
        sidebar,
        sidebar_foreground: foreground,
        sidebar_accent: toward_foreground(ACCENT_MIX),
        primary,
        primary_foreground: primary.more_readable(background, foreground),
        muted: toward_foreground(MUTED_MIX),
        muted_foreground,
        border: colors
            .split_divider_color
            .unwrap_or_else(|| toward_foreground(BORDER_MIX)),
        accent: toward_foreground(ACCENT_MIX),
        success: palette(2),
        warning: palette(3),
        danger: palette(1),
        ansi: std::array::from_fn(|n| palette(n + 1)),
        selection: colors
            .selection_background
            .unwrap_or_else(|| toward_foreground(SELECTION_MIX)),
        caret: colors.cursor_color.unwrap_or(foreground),
        diff_added: palette(2),
        diff_removed: palette(1),
    }
}

/// The 16 palette colors, then background, foreground, and selection, each with a label.
/// Unset palette colors use Ghostty's defaults.
pub fn swatches(colors: &ThemeColors) -> Vec<(String, Rgb)> {
    let mut result: Vec<_> = (0..16).map(|n| (n.to_string(), colors.palette[n].unwrap_or_else(|| default_palette(n)))).collect();
    let ui = map(colors);
    result.extend([("background".into(), ui.background), ("foreground".into(), ui.foreground), ("selection".into(), ui.selection)]);
    result
}

fn default_palette(n: usize) -> Rgb {
    let value = DEFAULT_PALETTE[n];
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ghostty::{bundled, file::parse};

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    fn colors(text: &str) -> ThemeColors {
        parse(text).colors
    }

    const DARK: &str = "\
background = #000000
foreground = #ffffff
palette = 1=#ff0000
palette = 2=#00ff00
palette = 3=#ffff00
palette = 4=#0000ff
palette = 8=#999999
";

    #[test]
    fn swatches_include_defaults_and_theme_colors() {
        let c = colors("palette = 1=#123456\nbackground = #010203\n");
        let s = swatches(&c);
        assert_eq!(s.len(), 19);
        assert_eq!(s[0], ("0".into(), rgb(0x1d, 0x1f, 0x21)));
        assert_eq!(s[1], ("1".into(), rgb(0x12, 0x34, 0x56)));
        assert_eq!(s[16], ("background".into(), rgb(1, 2, 3)));
        assert_eq!(s[18].1, map(&c).selection);
    }

    #[test]
    fn ansi_tokens_come_from_palette_1_to_6() {
        let ui = map(&colors(bundled::text("Atom One Dark").unwrap()));
        assert_eq!(ui.ansi[0], rgb(0xe0, 0x6c, 0x75));
        assert_eq!(ui.ansi[5], rgb(0x56, 0xb6, 0xc2));
    }

    #[test]
    fn plain_tokens_copy_the_theme() {
        let ui = map(&colors(DARK));
        assert_eq!(ui.background, rgb(0, 0, 0));
        assert_eq!(ui.foreground, rgb(255, 255, 255));
        assert_eq!(ui.sidebar_foreground, rgb(255, 255, 255));
        assert_eq!(ui.primary, rgb(0, 0, 255));
        assert_eq!(ui.success, rgb(0, 255, 0));
        assert_eq!(ui.warning, rgb(255, 255, 0));
        assert_eq!(ui.danger, rgb(255, 0, 0));
        assert_eq!(ui.diff_added, rgb(0, 255, 0));
        assert_eq!(ui.diff_removed, rgb(255, 0, 0));
    }

    #[test]
    fn derived_tokens_mix_the_background_toward_the_foreground() {
        let ui = map(&colors(DARK));
        let (bg, fg) = (rgb(0, 0, 0), rgb(255, 255, 255));
        assert_eq!(ui.sidebar, bg.mix(fg, SIDEBAR_MIX));
        assert_eq!(ui.muted, bg.mix(fg, MUTED_MIX));
        assert_eq!(ui.sidebar_accent, bg.mix(fg, ACCENT_MIX));
        assert_eq!(ui.accent, bg.mix(fg, ACCENT_MIX));
        assert_eq!(ui.border, bg.mix(fg, BORDER_MIX));
    }

    #[test]
    fn the_sidebar_stays_on_the_background_when_mixing_would_hurt_the_text() {
        // Gray text on black clears 4.5 by a small margin. A lighter sidebar would not.
        let ui = map(&colors("background = #000000\nforeground = #777777\n"));
        assert!(rgb(0x77, 0x77, 0x77).contrast(rgb(0, 0, 0)) >= 4.5);
        assert_eq!(ui.sidebar, rgb(0, 0, 0));
        let plenty = map(&colors(DARK));
        assert_ne!(plenty.sidebar, rgb(0, 0, 0), "white text has room to spare");
    }

    #[test]
    fn split_divider_color_replaces_the_derived_border() {
        let ui = map(&colors(&format!("{DARK}split-divider-color = #123456\n")));
        assert_eq!(ui.border, rgb(0x12, 0x34, 0x56));
    }

    #[test]
    fn muted_foreground_uses_palette_8_when_it_is_readable() {
        let ui = map(&colors(DARK));
        assert_eq!(ui.muted_foreground, rgb(0x99, 0x99, 0x99));
    }

    #[test]
    fn muted_foreground_falls_back_when_palette_8_is_hard_to_read() {
        let ui = map(&colors("background = #000000\nforeground = #ffffff\npalette = 8=#111111\n"));
        assert_eq!(
            ui.muted_foreground,
            rgb(255, 255, 255).mix(rgb(0, 0, 0), MUTED_FOREGROUND_MIX)
        );
        let none = map(&colors("background = #000000\nforeground = #ffffff\n"));
        assert!(
            none.muted_foreground.contrast(rgb(0, 0, 0)) >= 1.0,
            "an unset palette 8 uses Ghostty's default gray"
        );
    }

    #[test]
    fn primary_foreground_is_the_more_readable_of_background_and_foreground() {
        let ui = map(&colors(DARK));
        assert_eq!(ui.primary_foreground, rgb(255, 255, 255), "white on blue");
        let light = map(&colors(
            "background = #ffffff\nforeground = #000000\npalette = 4=#ffee00\n",
        ));
        assert_eq!(light.primary_foreground, rgb(0, 0, 0), "black on yellow");
    }

    #[test]
    fn selection_and_caret_use_their_keys_or_derive() {
        let set = map(&colors(&format!(
            "{DARK}selection-background = #445566\ncursor-color = #778899\n"
        )));
        assert_eq!(set.selection, rgb(0x44, 0x55, 0x66));
        assert_eq!(set.caret, rgb(0x77, 0x88, 0x99));
        let unset = map(&colors(DARK));
        assert_eq!(unset.selection, rgb(0, 0, 0).mix(rgb(255, 255, 255), SELECTION_MIX));
        assert_eq!(unset.caret, rgb(255, 255, 255));
    }

    #[test]
    fn unset_colors_use_ghostty_defaults() {
        let ui = map(&ThemeColors::default());
        assert_eq!(ui.background, DEFAULT_BACKGROUND);
        assert_eq!(ui.foreground, DEFAULT_FOREGROUND);
        assert_eq!(ui.danger, rgb(0xcc, 0x66, 0x66));
        assert_eq!(ui.success, rgb(0xb5, 0xbd, 0x68));
        assert_eq!(ui.warning, rgb(0xf0, 0xc6, 0x74));
        assert_eq!(ui.primary, rgb(0x81, 0xa2, 0xbe));
    }

    #[test]
    fn every_bundled_theme_keeps_its_text_as_readable_as_the_theme_intends() {
        for name in bundled::names() {
            let ui = map(&colors(bundled::text(name).unwrap()));
            let own = ui.foreground.contrast(ui.background);
            assert!(own >= 4.0, "{name}: the theme's own text contrast is {own}");
            let floor = own.min(SIDEBAR_MIN_CONTRAST) - 0.001;
            assert!(
                ui.sidebar_foreground.contrast(ui.sidebar) >= floor,
                "{name}: sidebar text"
            );
            assert!(
                ui.primary_foreground.contrast(ui.primary) >= 3.0,
                "{name}: primary text"
            );
        }
    }
}
