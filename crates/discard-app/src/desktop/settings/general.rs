use discard_core::settings::{Change, SettingsStore};

/// Validate and save a default provider after loading outside edits.
pub fn set_default_provider(store: &mut SettingsStore, provider: String) -> Result<(), String> {
    apply_change(store, Change::DefaultProvider(provider))
}

/// Format settings file problems for display.
pub fn issue_lines(issues: &[discard_core::settings::SettingsIssue]) -> Vec<String> {
    issues.iter().map(|issue| format!("{}: {}", issue.key, issue.message)).collect()
}

/// The `general` page of Settings. `background_unavailable` is the line that says why the tray icon is missing, if it is.
pub fn general_page(general: &discard_core::settings::GeneralSettings, issues: &[String], theme_problems: serde_json::Value, background_unavailable: Option<String>) -> serde_json::Value {
    serde_json::json!({
        "defaultProvider": general.default_provider,
        "confirmWhileWorking": general.confirm_while_working,
        "keepRunningInBackground": general.keep_running_in_background,
        "desktopNotifications": general.desktop_notifications,
        "backgroundUnavailable": background_unavailable,
        "providers": discard_core::bootstrap::PROVIDERS,
        "issues": issues,
        "themeProblems": theme_problems,
    })
}

/// The changes that a patch of the `general` page asks for, in the order of the page.
pub fn general_changes(patch: super::GeneralPatch) -> Vec<Change> {
    let mut changes = Vec::new();
    if let Some(value) = patch.default_provider { changes.push(Change::DefaultProvider(value)); }
    if let Some(value) = patch.confirm_while_working { changes.push(Change::ConfirmWhileWorking(value)); }
    if let Some(value) = patch.keep_running_in_background { changes.push(Change::KeepRunningInBackground(value)); }
    if let Some(value) = patch.desktop_notifications { changes.push(Change::DesktopNotifications(value)); }
    changes
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

#[cfg(test)]
mod background_tests {
    use super::*;
    use discard_core::settings::GeneralSettings;

    #[test]
    fn the_general_page_reports_the_background_setting_and_the_tray_problem() {
        let general = GeneralSettings { keep_running_in_background: true, ..GeneralSettings::default() };
        let page = general_page(&general, &[], serde_json::json!([]), None);
        assert_eq!(page["keepRunningInBackground"], serde_json::json!(true));
        assert_eq!(page["backgroundUnavailable"], serde_json::Value::Null);
        assert_eq!(page["confirmWhileWorking"], serde_json::json!(true));
        assert_eq!(page["defaultProvider"], serde_json::json!("claude"));
        let general = GeneralSettings { keep_running_in_background: false, ..GeneralSettings::default() };
        let line = crate::desktop::tray::unavailable_line("no appindicator");
        let page = general_page(&general, &["problem".into()], serde_json::json!(["theme"]), Some(line.clone()));
        assert_eq!(page["keepRunningInBackground"], serde_json::json!(false));
        assert_eq!(page["backgroundUnavailable"], serde_json::json!(line));
        assert_eq!(page["issues"], serde_json::json!(["problem"]));
        assert_eq!(page["themeProblems"], serde_json::json!(["theme"]));
    }

    #[test]
    fn a_general_patch_with_the_background_key_changes_that_setting_only() {
        let patch: crate::desktop::settings::GeneralPatch = serde_json::from_str(r#"{"keepRunningInBackground":false}"#).unwrap();
        assert_eq!(patch.keep_running_in_background, Some(false));
        assert_eq!(patch.confirm_while_working, None);
        assert_eq!(patch.default_provider, None);
        let changes = general_changes(patch);
        assert_eq!(changes, vec![Change::KeepRunningInBackground(false)]);
    }

    #[test]
    fn saving_the_background_change_writes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut store = SettingsStore::open(&path);
        apply_change(&mut store, Change::KeepRunningInBackground(false)).unwrap();
        assert!(!store.settings().general.keep_running_in_background);
        assert!(std::fs::read_to_string(path).unwrap().contains("keep_running_in_background = false"));
    }

    #[test]
    fn general_changes_maps_desktop_notifications() {
        let patch: crate::desktop::settings::GeneralPatch = serde_json::from_str(r#"{"desktopNotifications":false}"#).unwrap();
        assert_eq!(patch.desktop_notifications, Some(false));
        assert_eq!(general_changes(patch), vec![Change::DesktopNotifications(false)]);
        let page = general_page(&GeneralSettings::default(), &[], serde_json::json!([]), None);
        assert_eq!(page["desktopNotifications"], serde_json::json!(true));
    }
}
