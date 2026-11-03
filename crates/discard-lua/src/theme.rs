//! Theme tokens that plugins may override. Values are validated and clamped.
//! Protected surfaces (approvals, review, errors) keep a minimum contrast no matter what a theme says.

use std::collections::BTreeMap;

pub const MIN_CONTRAST: f64 = 4.5;

/// Color token names a plugin may set.
pub const COLOR_TOKENS: &[&str] = &[
    "background", "foreground", "sidebar", "sidebar_foreground", "sidebar_accent", "primary",
    "primary_foreground", "muted", "muted_foreground", "border", "accent", "success", "warning", "danger",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ThemeError {
    #[error("`{0}` is not a color like #rrggbb")]
    BadColor(String),
    #[error("unknown theme token `{0}`")]
    UnknownToken(String),
    #[error("`{0}` must be a finite number")]
    NotFinite(&'static str),
}

impl Rgb {
    pub const WHITE: Rgb = Rgb(255, 255, 255);
    pub const BLACK: Rgb = Rgb(0, 0, 0);

    pub fn parse(s: &str) -> Result<Rgb, ThemeError> {
        let bad = || ThemeError::BadColor(s.to_string());
        let hex = s.strip_prefix('#').ok_or_else(bad)?;
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        Ok(Rgb(byte(0)?, byte(2)?, byte(4)?))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    fn luminance(self) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(self.0) + 0.7152 * channel(self.1) + 0.0722 * channel(self.2)
    }
}

/// WCAG contrast ratio, from 1.0 (identical) to 21.0 (black on white).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// `preferred` if it is readable on `background`. Otherwise black or white, whichever reads better.
pub fn readable_foreground(background: Rgb, preferred: Rgb) -> Rgb {
    if contrast_ratio(preferred, background) >= MIN_CONTRAST {
        return preferred;
    }
    if contrast_ratio(Rgb::WHITE, background) >= contrast_ratio(Rgb::BLACK, background) { Rgb::WHITE } else { Rgb::BLACK }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThemeOverrides {
    colors: BTreeMap<String, Rgb>,
    radius: Option<f32>,
    spacing_scale: Option<f32>,
    font_scale: Option<f32>,
}

impl ThemeOverrides {
    pub fn set_color(&mut self, token: &str, value: &str) -> Result<(), ThemeError> {
        if !COLOR_TOKENS.contains(&token) {
            return Err(ThemeError::UnknownToken(token.to_string()));
        }
        let color = Rgb::parse(value)?;
        self.colors.insert(token.to_string(), color);
        Ok(())
    }

    /// Clamped to 0..=24 pixels.
    pub fn set_radius(&mut self, px: f64) -> Result<(), ThemeError> {
        if !px.is_finite() {
            return Err(ThemeError::NotFinite("radius"));
        }
        self.radius = Some(px.clamp(0.0, 24.0) as f32);
        Ok(())
    }

    /// Clamped to 0.5..=2.0.
    pub fn set_spacing_scale(&mut self, scale: f64) -> Result<(), ThemeError> {
        if !scale.is_finite() {
            return Err(ThemeError::NotFinite("spacing_scale"));
        }
        self.spacing_scale = Some(scale.clamp(0.5, 2.0) as f32);
        Ok(())
    }

    /// Clamped to 0.8..=1.5.
    pub fn set_font_scale(&mut self, scale: f64) -> Result<(), ThemeError> {
        if !scale.is_finite() {
            return Err(ThemeError::NotFinite("font_scale"));
        }
        self.font_scale = Some(scale.clamp(0.8, 1.5) as f32);
        Ok(())
    }

    pub fn color(&self, token: &str) -> Option<Rgb> {
        self.colors.get(token).copied()
    }

    pub fn radius(&self) -> Option<f32> {
        self.radius
    }

    pub fn spacing_scale(&self) -> Option<f32> {
        self.spacing_scale
    }

    pub fn font_scale(&self) -> Option<f32> {
        self.font_scale
    }

    /// Later plugins win token by token.
    pub fn merge(&mut self, other: &ThemeOverrides) {
        for (k, v) in &other.colors {
            self.colors.insert(k.clone(), *v);
        }
        self.radius = other.radius.or(self.radius);
        self.spacing_scale = other.spacing_scale.or(self.spacing_scale);
        self.font_scale = other.font_scale.or(self.font_scale);
    }

    /// Names of the tokens this override sets, sorted.
    pub fn token_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.colors.keys().cloned().collect();
        for (set, name) in [(self.radius.is_some(), "radius"), (self.spacing_scale.is_some(), "spacing_scale"), (self.font_scale.is_some(), "font_scale")] {
            if set {
                names.push(name.to_string());
            }
        }
        names.sort();
        names
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty() && self.radius.is_none() && self.spacing_scale.is_none() && self.font_scale.is_none()
    }
}

/// Colors for a protected surface. Built from the theme, then forced to stay readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtectedPalette {
    pub background: Rgb,
    pub foreground: Rgb,
    pub border: Rgb,
}

pub fn protected_palette(theme: &ThemeOverrides, default: ProtectedPalette) -> ProtectedPalette {
    const MIN_BORDER_CONTRAST: f64 = 3.0;
    let background = theme.color("background").unwrap_or(default.background);
    let preferred = theme.color("foreground").unwrap_or(default.foreground);
    let foreground = readable_foreground(background, preferred);
    let border = match theme.color("warning") {
        Some(c) if contrast_ratio(c, background) >= MIN_BORDER_CONTRAST => c,
        _ if contrast_ratio(default.border, background) >= MIN_BORDER_CONTRAST => default.border,
        _ => readable_foreground(background, default.border),
    };
    ProtectedPalette { background, foreground, border }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors_case_insensitively() {
        assert_eq!(Rgb::parse("#FF8000"), Ok(Rgb(255, 128, 0)));
        assert_eq!(Rgb::parse("#0a0B0c"), Ok(Rgb(10, 11, 12)));
        assert_eq!(Rgb(1, 2, 255).to_hex(), "#0102ff");
    }

    #[test]
    fn rejects_malformed_colors() {
        for bad in ["", "red", "#fff", "ff8000", "#gg0000", "#ff80001", "rgb(1,2,3)", "#ff 000"] {
            assert_eq!(Rgb::parse(bad), Err(ThemeError::BadColor(bad.into())), "{bad}");
        }
    }

    #[test]
    fn contrast_matches_the_wcag_extremes() {
        assert!((contrast_ratio(Rgb::BLACK, Rgb::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(Rgb::WHITE, Rgb::WHITE) - 1.0).abs() < 0.0001);
        assert_eq!(contrast_ratio(Rgb(10, 20, 30), Rgb::WHITE), contrast_ratio(Rgb::WHITE, Rgb(10, 20, 30)));
    }

    #[test]
    fn readable_foreground_keeps_good_choices_and_replaces_bad_ones() {
        assert_eq!(readable_foreground(Rgb::BLACK, Rgb::WHITE), Rgb::WHITE);
        assert_eq!(readable_foreground(Rgb::WHITE, Rgb(250, 250, 250)), Rgb::BLACK);
        assert_eq!(readable_foreground(Rgb(20, 20, 20), Rgb(25, 25, 25)), Rgb::WHITE);
    }

    #[test]
    fn colors_are_validated_by_token_name() {
        let mut t = ThemeOverrides::default();
        t.set_color("primary", "#112233").unwrap();
        assert_eq!(t.color("primary"), Some(Rgb(0x11, 0x22, 0x33)));
        assert_eq!(t.set_color("sparkle", "#112233"), Err(ThemeError::UnknownToken("sparkle".into())));
        assert_eq!(t.set_color("primary", "blue"), Err(ThemeError::BadColor("blue".into())));
        assert_eq!(t.color("primary"), Some(Rgb(0x11, 0x22, 0x33)), "a failed set must not change the value");
    }

    #[test]
    fn numeric_tokens_are_clamped() {
        let mut t = ThemeOverrides::default();
        t.set_radius(500.0).unwrap();
        assert_eq!(t.radius(), Some(24.0));
        t.set_radius(-3.0).unwrap();
        assert_eq!(t.radius(), Some(0.0));
        t.set_spacing_scale(0.01).unwrap();
        assert_eq!(t.spacing_scale(), Some(0.5));
        t.set_font_scale(9.0).unwrap();
        assert_eq!(t.font_scale(), Some(1.5));
        assert_eq!(t.set_radius(f64::NAN), Err(ThemeError::NotFinite("radius")));
        assert_eq!(t.set_font_scale(f64::INFINITY), Err(ThemeError::NotFinite("font_scale")));
    }

    #[test]
    fn merge_lets_later_overrides_win_per_token() {
        let (mut a, mut b) = (ThemeOverrides::default(), ThemeOverrides::default());
        a.set_color("primary", "#111111").unwrap();
        a.set_color("border", "#222222").unwrap();
        a.set_radius(4.0).unwrap();
        b.set_color("primary", "#333333").unwrap();
        a.merge(&b);
        assert_eq!(a.color("primary"), Some(Rgb(0x33, 0x33, 0x33)));
        assert_eq!(a.color("border"), Some(Rgb(0x22, 0x22, 0x22)));
        assert_eq!(a.radius(), Some(4.0));
        assert!(!a.is_empty());
        assert!(ThemeOverrides::default().is_empty());
    }

    const DEFAULT: ProtectedPalette =
        ProtectedPalette { background: Rgb(30, 30, 30), foreground: Rgb(240, 240, 240), border: Rgb(255, 170, 0) };

    #[test]
    fn protected_surfaces_stay_readable_whatever_the_theme_says() {
        let mut t = ThemeOverrides::default();
        t.set_color("background", "#202020").unwrap();
        t.set_color("foreground", "#222222").unwrap(); // invisible text
        let p = protected_palette(&t, DEFAULT);
        assert!(contrast_ratio(p.foreground, p.background) >= MIN_CONTRAST);
    }

    #[test]
    fn protected_surfaces_use_theme_colors_when_they_are_readable() {
        let mut t = ThemeOverrides::default();
        t.set_color("background", "#101820").unwrap();
        t.set_color("foreground", "#f0f0f0").unwrap();
        let p = protected_palette(&t, DEFAULT);
        assert_eq!(p.background, Rgb(0x10, 0x18, 0x20));
        assert_eq!(p.foreground, Rgb(0xf0, 0xf0, 0xf0));
    }

    #[test]
    fn the_border_cannot_be_hidden_in_the_background() {
        let mut t = ThemeOverrides::default();
        t.set_color("background", "#202020").unwrap();
        t.set_color("warning", "#202020").unwrap();
        let p = protected_palette(&t, DEFAULT);
        assert!(contrast_ratio(p.border, p.background) >= 3.0, "{p:?}");
    }

    #[test]
    fn an_empty_theme_gives_the_defaults() {
        assert_eq!(protected_palette(&ThemeOverrides::default(), DEFAULT), DEFAULT);
    }
}
