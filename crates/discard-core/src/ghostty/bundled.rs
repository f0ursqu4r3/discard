//! Themes embedded in the binary. Used when no Ghostty install has the
//! theme. The files come from iTerm2-Color-Schemes (see THIRD-PARTY-NOTICES).

/// The theme pair used when the user chooses nothing.
pub const DEFAULT_THEME: &str = "light:Catppuccin Latte,dark:Catppuccin Mocha";

/// Sorted by name.
const THEMES: &[(&str, &str)] = &[
    ("Atom One Dark", include_str!("../../themes/Atom One Dark")),
    ("Catppuccin Latte", include_str!("../../themes/Catppuccin Latte")),
    ("Catppuccin Mocha", include_str!("../../themes/Catppuccin Mocha")),
    ("Dracula", include_str!("../../themes/Dracula")),
    ("Gruvbox Dark", include_str!("../../themes/Gruvbox Dark")),
    ("Gruvbox Light", include_str!("../../themes/Gruvbox Light")),
    ("Nord", include_str!("../../themes/Nord")),
    ("TokyoNight", include_str!("../../themes/TokyoNight")),
    ("iTerm2 Solarized Dark", include_str!("../../themes/iTerm2 Solarized Dark")),
    ("iTerm2 Solarized Light", include_str!("../../themes/iTerm2 Solarized Light")),
];

/// The text of the bundled theme `name`. Names match exactly.
pub fn text(name: &str) -> Option<&'static str> {
    THEMES.iter().find(|(n, _)| *n == name).map(|(_, text)| *text)
}

/// The names of all bundled themes, sorted.
pub fn names() -> Vec<&'static str> {
    THEMES.iter().map(|(name, _)| *name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ghostty::file::parse;
    use crate::ghostty::spec::ThemeSpec;

    #[test]
    fn ten_themes_are_bundled_in_sorted_order() {
        let names = names();
        assert_eq!(names.len(), 10, "{names:?}");
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn every_bundled_theme_is_complete_and_clean() {
        for name in names() {
            let parsed = parse(text(name).unwrap());
            assert!(parsed.issues.is_empty(), "{name}: {:?}", parsed.issues);
            let c = parsed.colors;
            assert!(c.palette.iter().all(Option::is_some), "{name}: palette");
            assert!(c.background.is_some(), "{name}: background");
            assert!(c.foreground.is_some(), "{name}: foreground");
        }
    }

    #[test]
    fn the_default_theme_names_bundled_themes() {
        let ThemeSpec::Pair { light, dark } = ThemeSpec::parse(DEFAULT_THEME).unwrap() else {
            panic!("the default must be a light/dark pair");
        };
        assert!(text(&light).is_some(), "{light}");
        assert!(text(&dark).is_some(), "{dark}");
    }

    #[test]
    fn an_unknown_name_is_none() {
        assert_eq!(text("No Such Theme"), None);
    }
}
