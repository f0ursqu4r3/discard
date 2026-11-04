//! Advanced actions and their confirmation text.
use std::path::Path;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvancedAction {
    ResetSettings,
    RebuildMemoryIndex,
    ClearLogs,
}
impl AdvancedAction {
    pub fn title(self) -> &'static str {
        match self {
            Self::ResetSettings => "Reset settings",
            Self::RebuildMemoryIndex => "Rebuild memory index",
            Self::ClearLogs => "Clear logs",
        }
    }
    pub fn button_label(self) -> &'static str {
        self.title()
    }
}
pub fn confirm_lines(action: AdvancedAction, home: &Path) -> Vec<String> {
    match action {
        AdvancedAction::ResetSettings => vec![
            "Copies settings.toml to settings.toml.bak. Replaces an older settings.toml.bak."
                .into(),
            "Sets general, appearance, and keybindings back to the defaults.".into(),
            "Keeps your memory files, themes, plugins, and history.".into(),
            "Key changes apply at next launch.".into(),
        ],
        AdvancedAction::RebuildMemoryIndex => vec![
            "Rebuilds memory-index.db from the memory files.".into(),
            "Does not change any memory file.".into(),
        ],
        AdvancedAction::ClearLogs => vec![
            format!(
                "Deletes the older log files in `{}`.",
                home.join("logs").display()
            ),
            "Empties the current log file.".into(),
            "Does not change settings or history.".into(),
        ],
    }
}
pub fn clear_logs(home: &Path) -> Result<usize, String> {
    crate::log_file::clear(&home.join("logs")).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirm_lines_name_the_files_each_action_changes_and_none_say_delete_all() {
        for a in [
            AdvancedAction::ResetSettings,
            AdvancedAction::RebuildMemoryIndex,
            AdvancedAction::ClearLogs,
        ] {
            let lines = confirm_lines(a, std::path::Path::new("/tmp/home")).join(" ");
            assert!(!lines.contains("delete all"));
            assert!(lines.contains(match a {
                AdvancedAction::ResetSettings => "settings.toml.bak",
                AdvancedAction::RebuildMemoryIndex => "memory-index.db",
                AdvancedAction::ClearLogs => "logs",
            }));
        }
    }
    #[test]
    fn clear_logs_counts_changed_files_and_a_missing_folder_is_zero() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(clear_logs(d.path()).unwrap(), 0);
        let dir = d.path().join("logs");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("discard.log"), "active").unwrap();
        std::fs::write(dir.join("discard.log.1"), "old").unwrap();
        assert_eq!(clear_logs(d.path()).unwrap(), 2);
        assert!(std::fs::read(dir.join("discard.log")).unwrap().is_empty());
    }
}
