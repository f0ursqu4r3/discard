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
pub const LABEL_SETUP_STATUS: &str = "Setup status";
pub const LABEL_CAPABILITIES: &str = "Capabilities";
pub const LABEL_HOOK_ENFORCEMENT: &str = "Hook enforcement";
pub const LABEL_RECHECK: &str = "Re-check";
pub const LABEL_AGENT: &str = "Agent";
pub const LABEL_MODEL: &str = "Model";
pub const LABEL_SYSTEM_PROMPT: &str = "System prompt";
pub const LABEL_APPROVAL_POLICY: &str = "Approval policy";
pub const LABEL_PLUGIN_WRITE: &str = "plugin.write";

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
        matches!(self, SettingsPage::General | SettingsPage::Appearance | SettingsPage::Plugins | SettingsPage::Providers | SettingsPage::Agents)
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
            SettingsPage::Providers => &[LABEL_SETUP_STATUS, LABEL_CAPABILITIES, LABEL_HOOK_ENFORCEMENT, LABEL_RECHECK],
            SettingsPage::Agents => &[LABEL_AGENT, LABEL_MODEL, LABEL_SYSTEM_PROMPT, LABEL_APPROVAL_POLICY, LABEL_PLUGIN_WRITE],
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
            vec![SettingsPage::General, SettingsPage::Appearance, SettingsPage::Agents, SettingsPage::Providers, SettingsPage::Plugins]
        );
    }

    #[test]
    fn empty_query_returns_every_page() {
        assert_eq!(search(""), SettingsPage::all().to_vec());
    }

    #[test]
    fn provider_labels_are_searchable() {
        for label in ["Setup status", "Capabilities", "Hook enforcement", "Re-check"] {
            assert!(search(label).contains(&SettingsPage::Providers));
        }
        assert!(SettingsPage::Providers.is_available());
    }

    #[test]
    fn agent_labels_are_searchable() {
        for label in ["Agent", "Model", "System prompt", "Approval policy", "plugin.write"] { assert!(search(label).contains(&SettingsPage::Agents)); }
        assert!(SettingsPage::Agents.is_available());
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
