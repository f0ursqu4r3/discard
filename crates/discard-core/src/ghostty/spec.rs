//! Ghostty's `theme` value: one name, or `light:X,dark:Y`.

/// The system appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

/// The theme names a `theme` value selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeSpec {
    /// One theme for both appearances.
    Single(String),
    Pair { light: String, dark: String },
}

impl ThemeSpec {
    /// Read a `theme` value. Returns `None` when it names no theme.
    pub fn parse(value: &str) -> Option<ThemeSpec> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        if !value.starts_with("light:") && !value.starts_with("dark:") {
            return Some(ThemeSpec::Single(value.to_string()));
        }
        let (mut light, mut dark) = (None, None);
        for part in value.split(',') {
            let part = part.trim();
            if let Some(name) = part.strip_prefix("light:") {
                light = Some(name.trim()).filter(|n| !n.is_empty());
            } else if let Some(name) = part.strip_prefix("dark:") {
                dark = Some(name.trim()).filter(|n| !n.is_empty());
            }
        }
        Some(ThemeSpec::Pair {
            light: light?.to_string(),
            dark: dark?.to_string(),
        })
    }

    /// The theme name for `appearance`.
    pub fn name_for(&self, appearance: Appearance) -> &str {
        match (self, appearance) {
            (ThemeSpec::Single(name), _) => name,
            (ThemeSpec::Pair { light, .. }, Appearance::Light) => light,
            (ThemeSpec::Pair { dark, .. }, Appearance::Dark) => dark,
        }
    }

    /// The value as Ghostty writes it.
    pub fn to_value(&self) -> String {
        match self {
            ThemeSpec::Single(name) => name.clone(),
            ThemeSpec::Pair { light, dark } => format!("light:{light},dark:{dark}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(light: &str, dark: &str) -> ThemeSpec {
        ThemeSpec::Pair {
            light: light.to_string(),
            dark: dark.to_string(),
        }
    }

    #[test]
    fn a_plain_name_is_one_theme_for_both() {
        let spec = ThemeSpec::parse("Nord").unwrap();
        assert_eq!(spec, ThemeSpec::Single("Nord".to_string()));
        assert_eq!(spec.name_for(Appearance::Light), "Nord");
        assert_eq!(spec.name_for(Appearance::Dark), "Nord");
    }

    #[test]
    fn a_pair_picks_by_appearance() {
        let spec = ThemeSpec::parse("light:TokyoNight Day,dark:TokyoNight").unwrap();
        assert_eq!(spec, pair("TokyoNight Day", "TokyoNight"));
        assert_eq!(spec.name_for(Appearance::Light), "TokyoNight Day");
        assert_eq!(spec.name_for(Appearance::Dark), "TokyoNight");
    }

    #[test]
    fn the_pair_order_and_spacing_do_not_matter() {
        assert_eq!(
            ThemeSpec::parse(" dark: Nord , light: Dracula "),
            Some(pair("Dracula", "Nord"))
        );
    }

    #[test]
    fn names_may_contain_spaces_and_a_colon_free_path() {
        assert_eq!(
            ThemeSpec::parse("/Users/me/my theme"),
            Some(ThemeSpec::Single("/Users/me/my theme".to_string()))
        );
    }

    #[test]
    fn empty_or_incomplete_values_name_no_theme() {
        for bad in ["", "   ", "light:Nord", "dark:Nord", "light:,dark:Nord", "light:A,dark:"] {
            assert_eq!(ThemeSpec::parse(bad), None, "input {bad:?}");
        }
    }

    #[test]
    fn to_value_round_trips() {
        for value in ["Nord", "light:Dracula,dark:Nord"] {
            assert_eq!(ThemeSpec::parse(value).unwrap().to_value(), value);
        }
    }
}
