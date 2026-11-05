use discard_core::settings::{Change, SettingsStore};

/// Validate and save a default provider after loading outside edits.
pub fn set_default_provider(store: &mut SettingsStore, provider: String) -> Result<(), String> {
    apply_change(store, Change::DefaultProvider(provider))
}

/// Format settings file problems for display.
pub fn issue_lines(issues: &[discard_core::settings::SettingsIssue]) -> Vec<String> {
    issues.iter().map(|issue| format!("{}: {}", issue.key, issue.message)).collect()
}

/// Reload outside edits before saving.
pub fn apply_change(store: &mut SettingsStore, change: Change) -> Result<(), String> {
    store.reload_if_changed();
    store.apply(change).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::settings::SettingsStore;

    #[test]
    fn invalid_general_patch_returns_the_validation_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::open(&dir.path().join("settings.toml"));
        let error = set_default_provider(&mut store, "nope".into()).unwrap_err();
        assert_eq!(error, "invalid setting: unknown provider `nope`");
        assert_eq!(store.settings().general.default_provider, "claude");
    }
}

#[cfg(test)]
mod parity_tests {
    use super::*;
    use discard_core::settings::{Change, SettingsIssue};
    #[test]
    fn issue_lines_match_settings_issues() {
        assert!(issue_lines(&[]).is_empty());
        assert_eq!(issue_lines(&[SettingsIssue { key: "general.confirm_while_working".into(), message: "expected a boolean".into() }]), vec!["general.confirm_while_working: expected a boolean"]);
    }
    #[test]
    fn a_valid_change_is_written_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut store = SettingsStore::open(&path);
        apply_change(&mut store, Change::ConfirmWhileWorking(false)).unwrap();
        assert!(!store.settings().general.confirm_while_working);
        assert!(std::fs::read_to_string(path).unwrap().contains("confirm_while_working = false"));
    }
    #[test]
    fn an_outside_edit_is_read_before_the_change_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut store = SettingsStore::open(&path);
        std::fs::write(&path, "version = 1\n[general]\ndefault_provider = \"codex\"\n").unwrap();
        apply_change(&mut store, Change::ConfirmWhileWorking(false)).unwrap();
        assert_eq!(store.settings().general.default_provider, "codex");
        assert!(std::fs::read_to_string(path).unwrap().contains("codex"));
    }
}
