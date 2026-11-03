//! The pages of the Settings window and the sidebar search over them.
//! Pure logic. The GPUI layer renders the sidebar from `SettingsPage::all`.

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
        matches!(self, SettingsPage::General | SettingsPage::Plugins)
    }

    /// Field labels the search also matches. Pages with no body have none.
    fn field_labels(self) -> &'static [&'static str] {
        match self {
            SettingsPage::General => &[
                "Default provider",
                "Confirm while working",
                "Reopen last channel",
            ],
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
    fn all_has_ten_pages_and_two_are_available() {
        assert_eq!(SettingsPage::all().len(), 10);
        let available: Vec<_> = SettingsPage::all()
            .iter()
            .copied()
            .filter(|p| p.is_available())
            .collect();
        assert_eq!(
            available,
            vec![SettingsPage::General, SettingsPage::Plugins]
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
    fn no_match_returns_nothing() {
        assert_eq!(search("zzz"), vec![]);
    }
}
