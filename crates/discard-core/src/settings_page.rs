//! The pages of the Settings window and the sidebar search over them.
//! Pure logic. The GPUI layer renders the sidebar from `SettingsPage::all`.

/// Field labels of the General page. The search index and the page share them.
pub const LABEL_DEFAULT_PROVIDER: &str = "Default provider";
pub const LABEL_CONFIRM_WHILE_WORKING: &str = "Confirm while working";
pub const LABEL_REOPEN_LAST_CHANNEL: &str = "Reopen last channel";

pub const LABEL_THEME_SOURCE: &str = "Theme source";
pub const LABEL_LIGHT_THEME: &str = "Light theme";
pub const LABEL_DARK_THEME: &str = "Dark theme";
pub const LABEL_ONE_THEME: &str = "Use one theme for both";
pub const LABEL_EDIT_COPY: &str = "Edit a copy";
pub const LABEL_ACTIVE_THEME_FILE: &str = "Active theme file";
pub const LABEL_GHOSTTY_CONFIG: &str = "Ghostty config";
pub const LABEL_IGNORE_PLUGIN_THEMES: &str = "Ignore plugin themes";
pub const LABEL_PLUGIN_OVERRIDES: &str = "Plugin overrides";
pub const LABEL_SWATCHES: &str = "Swatches";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Appearance,
    Agents,
    Providers,
    Plugins,
    Memory,
    Workspaces,
    Keybindings,
    Logs,
    Advanced,
}

impl SettingsPage {
    pub fn all() -> &'static [SettingsPage] {
        &[
            SettingsPage::General,
            SettingsPage::Appearance,
            SettingsPage::Agents,
            SettingsPage::Providers,
            SettingsPage::Plugins,
            SettingsPage::Memory,
            SettingsPage::Workspaces,
            SettingsPage::Keybindings,
            SettingsPage::Logs,
            SettingsPage::Advanced,
        ]
    }

    pub fn title(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Appearance => "Appearance",
            SettingsPage::Agents => "Agents",
            SettingsPage::Providers => "Providers",
            SettingsPage::Plugins => "Plugins",
            SettingsPage::Memory => "Memory",
            SettingsPage::Workspaces => "Workspaces",
            SettingsPage::Keybindings => "Keybindings",
            SettingsPage::Logs => "Logs",
            SettingsPage::Advanced => "Advanced",
        }
    }

    /// Only available pages have a body. The others show as "Soon".
    pub fn is_available(self) -> bool {
        matches!(self, SettingsPage::General | SettingsPage::Appearance | SettingsPage::Plugins)
    }

    /// Field labels the search also matches. Pages with no body have none.
    fn field_labels(self) -> &'static [&'static str] {
        match self {
            SettingsPage::General => &[
                LABEL_DEFAULT_PROVIDER,
                LABEL_CONFIRM_WHILE_WORKING,
                LABEL_REOPEN_LAST_CHANNEL,
            ],
            SettingsPage::Appearance => &[LABEL_THEME_SOURCE, LABEL_LIGHT_THEME, LABEL_DARK_THEME, LABEL_ONE_THEME, LABEL_EDIT_COPY, LABEL_ACTIVE_THEME_FILE, LABEL_GHOSTTY_CONFIG, LABEL_IGNORE_PLUGIN_THEMES, LABEL_PLUGIN_OVERRIDES, LABEL_SWATCHES],
            SettingsPage::Plugins => &[
                "Safe mode",
                "Review",
                "Roll back",
                "Trust future edits",
                "Plugin problems",
            ],
            _ => &[],
        }
    }
}

/// Lowercase text with all whitespace removed, so "Roll back" matches "rollback".
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Pages whose title or field labels contain `query`, in `all()` order.
/// Case and whitespace are ignored.
pub fn search(query: &str) -> Vec<SettingsPage> {
    let needle = normalize(query);
    SettingsPage::all()
        .iter()
        .copied()
        .filter(|page| {
            normalize(page.title()).contains(&needle)
                || page
                    .field_labels()
                    .iter()
                    .any(|label| normalize(label).contains(&needle))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_has_ten_pages_and_three_are_available() {
        assert_eq!(SettingsPage::all().len(), 10);
        let available: Vec<_> = SettingsPage::all()
            .iter()
            .copied()
            .filter(|p| p.is_available())
            .collect();
        assert_eq!(
            available,
            vec![SettingsPage::General, SettingsPage::Appearance, SettingsPage::Plugins]
        );
    }

    #[test]
    fn empty_query_returns_every_page() {
        assert_eq!(search(""), SettingsPage::all().to_vec());
    }

    #[test]
    fn provider_matches_general_and_providers() {
        assert_eq!(
            search("provider"),
            vec![SettingsPage::General, SettingsPage::Providers]
        );
    }

    #[test]
    fn rollback_matches_plugins_only() {
        assert_eq!(search("rollback"), vec![SettingsPage::Plugins]);
        assert_eq!(search("roll back"), vec![SettingsPage::Plugins]);
    }

    #[test]
    fn surrounding_spaces_are_ignored() {
        assert_eq!(
            search("  provider "),
            vec![SettingsPage::General, SettingsPage::Providers]
        );
    }

    #[test]
    fn search_is_case_insensitive() {
        assert_eq!(search("PROVIDER"), search("provider"));
        assert_eq!(search("SaFe MoDe"), vec![SettingsPage::Plugins]);
    }

    #[test]
    fn every_general_label_matches_the_general_page() {
        for label in [
            LABEL_DEFAULT_PROVIDER,
            LABEL_CONFIRM_WHILE_WORKING,
            LABEL_REOPEN_LAST_CHANNEL,
        ] {
            assert!(search(label).contains(&SettingsPage::General), "{label}");
        }
    }

    #[test]
    fn appearance_fields_are_searchable() {
        for label in ["Theme source", "Light theme", "Dark theme", "Use one theme for both", "Edit a copy", "Active theme file", "Ghostty config", "Ignore plugin themes", "Plugin overrides", "Swatches"] {
            assert!(search(label).contains(&SettingsPage::Appearance), "{label}");
        }
    }

    #[test]
    fn no_match_returns_nothing() {
        assert_eq!(search("zzz"), vec![]);
    }
}
