//! Ghostty color syntax and the color math the theme mapping needs.

use super::x11::X11_COLORS;

/// An opaque sRGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    /// Parse a color the way Ghostty does: `#rrggbb`, `rrggbb`, `#rgb`,
    /// `rgb`, or an X11 color name (case-insensitive, spaces ignored).
    pub fn parse_ghostty(text: &str) -> Option<Rgb> {
        let text = text.trim();
        let digits = text.strip_prefix('#').unwrap_or(text);
        if let Some(rgb) = parse_hex(digits) {
            return Some(rgb);
        }
        if text.starts_with('#') {
            return None;
        }
        let name: String = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        let index = X11_COLORS
            .binary_search_by(|(known, _)| (*known).cmp(name.as_str()))
            .ok()?;
        let [r, g, b] = X11_COLORS[index].1;
        Some(Rgb { r, g, b })
    }
}

/// Three or six hex digits, no `#`.
fn parse_hex(digits: &str) -> Option<Rgb> {
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = |s: &str| u8::from_str_radix(s, 16).ok();
    match digits.len() {
        6 => Some(Rgb {
            r: value(&digits[0..2])?,
            g: value(&digits[2..4])?,
            b: value(&digits[4..6])?,
        }),
        3 => {
            let double = |i: usize| value(&digits[i..=i]).map(|n| n * 17);
            Some(Rgb {
                r: double(0)?,
                g: double(1)?,
                b: double(2)?,
            })
        }
        _ => None,
    }
}

impl Rgb {
    /// `#rrggbb`, lowercase.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Move `fraction` (0.0..=1.0) of the way from `self` toward `other`.
    pub fn mix(self, other: Rgb, fraction: f64) -> Rgb {
        let t = fraction.clamp(0.0, 1.0);
        let channel = |from: u8, to: u8| {
            (f64::from(from) + (f64::from(to) - f64::from(from)) * t).round() as u8
        };
        Rgb {
            r: channel(self.r, other.r),
            g: channel(self.g, other.g),
            b: channel(self.b, other.b),
        }
    }

    /// WCAG contrast ratio, 1.0..=21.0.
    pub fn contrast(self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (light, dark) = if a >= b { (a, b) } else { (b, a) };
        (light + 0.05) / (dark + 0.05)
    }

    /// The color is closer to black than to white.
    pub fn is_dark(self) -> bool {
        let black = Rgb { r: 0, g: 0, b: 0 };
        let white = Rgb { r: 255, g: 255, b: 255 };
        self.contrast(white) > self.contrast(black)
    }

    /// Whichever of `a` and `b` has more contrast against `self`.
    pub fn more_readable(self, a: Rgb, b: Rgb) -> Rgb {
        if self.contrast(b) > self.contrast(a) { b } else { a }
    }

    /// WCAG relative luminance.
    fn luminance(self) -> f64 {
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    #[test]
    fn parses_hash_and_six_digits() {
        assert_eq!(Rgb::parse_ghostty("#1b2123"), Some(rgb(0x1b, 0x21, 0x23)));
    }

    #[test]
    fn parses_six_digits_without_hash() {
        assert_eq!(Rgb::parse_ghostty("fefbdc"), Some(rgb(0xfe, 0xfb, 0xdc)));
    }

    #[test]
    fn parses_three_digits_by_doubling_each() {
        assert_eq!(Rgb::parse_ghostty("#fa0"), Some(rgb(0xff, 0xaa, 0x00)));
        assert_eq!(Rgb::parse_ghostty("fa0"), Some(rgb(0xff, 0xaa, 0x00)));
    }

    #[test]
    fn hex_is_case_insensitive_and_trimmed() {
        assert_eq!(Rgb::parse_ghostty("  #C4BBA1 "), Some(rgb(0xc4, 0xbb, 0xa1)));
    }

    #[test]
    fn parses_x11_names_ignoring_case_and_spaces() {
        assert_eq!(Rgb::parse_ghostty("black"), Some(rgb(0, 0, 0)));
        assert_eq!(
            Rgb::parse_ghostty("CornflowerBlue"),
            Some(rgb(100, 149, 237))
        );
        assert_eq!(
            Rgb::parse_ghostty("cornflower blue"),
            Some(rgb(100, 149, 237))
        );
    }

    #[test]
    fn x11_gray_is_not_css_gray() {
        assert_eq!(Rgb::parse_ghostty("gray"), Some(rgb(190, 190, 190)));
    }

    #[test]
    fn rejects_bad_input() {
        for bad in ["", "#", "#12", "#12345", "#1234567", "#ggg", "nocolor", "#12 34 56"] {
            assert_eq!(Rgb::parse_ghostty(bad), None, "input {bad:?}");
        }
    }

    #[test]
    fn to_hex_is_lowercase_and_padded() {
        assert_eq!(rgb(0x0a, 0xff, 0x00).to_hex(), "#0aff00");
    }

    #[test]
    fn mix_moves_part_of_the_way() {
        let black = rgb(0, 0, 0);
        let white = rgb(255, 255, 255);
        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(black.mix(white, 1.0), white);
        assert_eq!(black.mix(white, 0.5), rgb(128, 128, 128));
        assert_eq!(black.mix(white, 2.0), white, "fraction clamps");
    }

    #[test]
    fn contrast_matches_wcag_extremes() {
        let black = rgb(0, 0, 0);
        let white = rgb(255, 255, 255);
        assert!((black.contrast(white) - 21.0).abs() < 0.01);
        assert!((white.contrast(black) - 21.0).abs() < 0.01);
        assert!((white.contrast(white) - 1.0).abs() < 0.01);
    }

    #[test]
    fn is_dark_splits_by_contrast_with_black_and_white() {
        assert!(rgb(0, 0, 0).is_dark());
        assert!(rgb(0x1e, 0x1e, 0x2e).is_dark());
        assert!(rgb(0x00, 0x2b, 0x36).is_dark(), "solarized dark");
        assert!(!rgb(255, 255, 255).is_dark());
        assert!(!rgb(0xfd, 0xf6, 0xe3).is_dark(), "solarized light");
    }

    #[test]
    fn more_readable_picks_the_higher_contrast_color() {
        let dark = rgb(10, 10, 10);
        let light = rgb(240, 240, 240);
        assert_eq!(rgb(0, 0, 0).more_readable(dark, light), light);
        assert_eq!(rgb(255, 255, 255).more_readable(dark, light), dark);
    }
}
