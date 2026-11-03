//! Validated, comment-preserving `settings.toml` store.
//! A bad file never blocks startup: each bad key falls back to its default.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, Value, value};

use crate::bootstrap::PROVIDERS;
use crate::ghostty::bundled::DEFAULT_THEME;
use crate::ghostty::resolve::ThemeSource;
use crate::ghostty::spec::ThemeSpec;

/// The settings Discard understands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub general: GeneralSettings,
    pub appearance: AppearanceSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppearanceSettings {
    pub theme_source: ThemeSource,
    /// A Ghostty `theme` value. Used when `theme_source` is `custom`.
    pub theme: String,
    pub ignore_plugin_themes: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        AppearanceSettings {
            theme_source: ThemeSource::Default,
            theme: DEFAULT_THEME.to_string(),
            ignore_plugin_themes: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralSettings {
    pub default_provider: String,
    pub confirm_while_working: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        GeneralSettings {
            default_provider: "claude".to_string(),
            confirm_while_working: true,
        }
    }
}

/// A problem found while reading the file. `key` is a dotted key,
/// `"file"` for a parse failure, or `"version"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsIssue {
    pub key: String,
    pub message: String,
}

/// One change the Settings window can make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    DefaultProvider(String),
    ConfirmWhileWorking(bool),
    ThemeSource(ThemeSource),
    /// A Ghostty `theme` value: `Name` or `light:X,dark:Y`.
    Theme(String),
    IgnorePluginThemes(bool),
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("invalid setting: {0}")]
    Invalid(String),
    #[error("settings I/O error: {0}")]
    Io(String),
}

/// The settings file version this build writes.
const CURRENT_VERSION: i64 = 1;

/// The state of the file after one read.
struct Loaded {
    doc: DocumentMut,
    parse_failed: bool,
    bytes: Option<Vec<u8>>,
    settings: Settings,
    issues: Vec<SettingsIssue>,
}

/// A settings file on disk. Keeps the raw TOML document so a write keeps
/// comments, formatting, and keys Discard does not know.
pub struct SettingsStore {
    path: PathBuf,
    doc: DocumentMut,
    /// The file could not be parsed, so `doc` is empty. The first write
    /// saves the bad text to `settings.toml.invalid`.
    parse_failed: bool,
    /// The bytes last read or written. `None` means the file was missing.
    last_bytes: Option<Vec<u8>>,
    settings: Settings,
    issues: Vec<SettingsIssue>,
}

impl SettingsStore {
    /// Read the file. A missing or bad file gives defaults. Never fails.
    pub fn open(path: &Path) -> SettingsStore {
        let loaded = match read_bytes(path) {
            Ok(bytes) => parse(bytes),
            Err(message) => file_problem(None, message),
        };
        SettingsStore {
            path: path.to_path_buf(),
            doc: loaded.doc,
            parse_failed: loaded.parse_failed,
            last_bytes: loaded.bytes,
            settings: loaded.settings,
            issues: loaded.issues,
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn issues(&self) -> &[SettingsIssue] {
        &self.issues
    }

    /// Validate and write one change. On error, the file and the
    /// in-memory state stay as they were.
    pub fn apply(&mut self, change: Change) -> Result<(), SettingsError> {
        if let Change::DefaultProvider(name) = &change
            && !PROVIDERS.contains(&name.as_str())
        {
            return Err(SettingsError::Invalid(format!(
                "unknown provider `{name}`"
            )));
        }
        if let Change::Theme(value) = &change
            && ThemeSpec::parse(value).is_none()
        {
            return Err(SettingsError::Invalid(format!(
                "`{value}` does not name a theme"
            )));
        }

        let mut doc = self.doc.clone();
        if doc.get("version").is_none() {
            doc.insert("version", value(CURRENT_VERSION));
        }
        let (table, key, new_value) = match change {
            Change::DefaultProvider(name) => ("general", "default_provider", Value::from(name)),
            Change::ConfirmWhileWorking(on) => {
                ("general", "confirm_while_working", Value::from(on))
            }
            Change::ThemeSource(source) => {
                ("appearance", "theme_source", Value::from(source.as_str()))
            }
            Change::Theme(value) => ("appearance", "theme", Value::from(value.trim())),
            Change::IgnorePluginThemes(on) => ("appearance", "ignore_plugin_themes", Value::from(on)),
        };
        set_in_table(&mut doc, table, key, new_value);

        if self.parse_failed {
            self.save_invalid_copy()?;
        }
        let bytes = doc.to_string().into_bytes();
        self.write_atomic(&bytes)?;

        let (settings, issues) = evaluate(&doc);
        self.doc = doc;
        self.parse_failed = false;
        self.last_bytes = Some(bytes);
        self.settings = settings;
        self.issues = issues;
        Ok(())
    }

    /// Re-read the file when its bytes differ from the last read or write.
    /// Returns `true` when the state changed.
    pub fn reload_if_changed(&mut self) -> bool {
        let loaded = match read_bytes(&self.path) {
            Ok(bytes) if bytes == self.last_bytes => return false,
            Ok(bytes) => parse(bytes),
            // An unreadable file keeps the current state.
            Err(_) => return false,
        };
        self.doc = loaded.doc;
        self.parse_failed = loaded.parse_failed;
        self.last_bytes = loaded.bytes;
        self.settings = loaded.settings;
        self.issues = loaded.issues;
        true
    }

    /// Copy the bad file to `<name>.invalid` before the first write.
    fn save_invalid_copy(&self) -> Result<(), SettingsError> {
        if !self.path.exists() {
            return Ok(());
        }
        let mut name = self.path.as_os_str().to_owned();
        name.push(".invalid");
        std::fs::copy(&self.path, PathBuf::from(name))
            .map(|_| ())
            .map_err(|e| SettingsError::Io(format!("cannot save the invalid file: {e}")))
    }

    /// Write a temp file next to the target, then rename it into place.
    fn write_atomic(&self, bytes: &[u8]) -> Result<(), SettingsError> {
        let io = |what: &str, e: std::io::Error| SettingsError::Io(format!("{what}: {e}"));
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| io("cannot create the directory", e))?;
        }
        let mut tmp_name = self.path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);
        let result = std::fs::write(&tmp, bytes)
            .map_err(|e| io("cannot write the file", e))
            .and_then(|()| {
                std::fs::rename(&tmp, &self.path).map_err(|e| io("cannot replace the file", e))
            });
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

/// Read the file. `Ok(None)` means it does not exist.
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read the settings file: {e}")),
    }
}

/// Defaults plus one `file` issue.
fn file_problem(bytes: Option<Vec<u8>>, message: String) -> Loaded {
    Loaded {
        doc: DocumentMut::new(),
        parse_failed: true,
        bytes,
        settings: Settings::default(),
        issues: vec![SettingsIssue {
            key: "file".to_string(),
            message,
        }],
    }
}

fn parse(bytes: Option<Vec<u8>>) -> Loaded {
    let Some(raw) = bytes else {
        return Loaded {
            doc: DocumentMut::new(),
            parse_failed: false,
            bytes: None,
            settings: Settings::default(),
            issues: Vec::new(),
        };
    };
    let doc = match std::str::from_utf8(&raw) {
        Ok(text) => text
            .parse::<DocumentMut>()
            .map_err(|e| format!("settings.toml is not valid TOML: {e}")),
        Err(_) => Err("settings.toml is not valid UTF-8".to_string()),
    };
    match doc {
        Ok(doc) => {
            let (settings, issues) = evaluate(&doc);
            Loaded {
                doc,
                parse_failed: false,
                bytes: Some(raw),
                settings,
                issues,
            }
        }
        Err(message) => file_problem(Some(raw), message),
    }
}

/// Read typed values key by key. A bad key falls back to its default.
fn evaluate(doc: &DocumentMut) -> (Settings, Vec<SettingsIssue>) {
    let mut settings = Settings::default();
    let mut issues = Vec::new();
    let mut issue = |key: &str, message: String| {
        issues.push(SettingsIssue {
            key: key.to_string(),
            message,
        });
    };

    match doc.get("version") {
        None => {}
        Some(item) => match item.as_integer() {
            Some(CURRENT_VERSION) => {}
            Some(other) => issue(
                "version",
                format!("version {other} is newer than this build understands ({CURRENT_VERSION})"),
            ),
            None => issue("version", "`version` must be an integer".to_string()),
        },
    }

    match doc.get("general") {
        None => {}
        Some(item) => match item.as_table_like() {
            None => issue("general", "`general` must be a table".to_string()),
            Some(general) => {
                if let Some(item) = general.get("default_provider") {
                    match item.as_str() {
                        Some(name) if PROVIDERS.contains(&name) => {
                            settings.general.default_provider = name.to_string();
                        }
                        Some(name) => issue(
                            "general.default_provider",
                            format!("unknown provider `{name}`"),
                        ),
                        None => issue(
                            "general.default_provider",
                            "must be a string".to_string(),
                        ),
                    }
                }
                if let Some(item) = general.get("confirm_while_working") {
                    match item.as_bool() {
                        Some(on) => settings.general.confirm_while_working = on,
                        None => issue(
                            "general.confirm_while_working",
                            "must be true or false".to_string(),
                        ),
                    }
                }
            }
        },
    }

    match doc.get("appearance") {
        None => {}
        Some(item) => match item.as_table_like() {
            None => issue("appearance", "`appearance` must be a table".to_string()),
            Some(appearance) => {
                if let Some(item) = appearance.get("theme_source") {
                    match item.as_str().and_then(ThemeSource::from_setting) {
                        Some(source) => settings.appearance.theme_source = source,
                        None => issue(
                            "appearance.theme_source",
                            "must be \"default\", \"ghostty\", or \"custom\"".to_string(),
                        ),
                    }
                }
                if let Some(item) = appearance.get("theme") {
                    match item.as_str() {
                        Some(value) if ThemeSpec::parse(value).is_some() => {
                            settings.appearance.theme = value.trim().to_string();
                        }
                        Some(value) => issue(
                            "appearance.theme",
                            format!("`{value}` does not name a theme"),
                        ),
                        None => issue("appearance.theme", "must be a string".to_string()),
                    }
                }
                if let Some(item) = appearance.get("ignore_plugin_themes") {
                    match item.as_bool() {
                        Some(on) => settings.appearance.ignore_plugin_themes = on,
                        None => issue(
                            "appearance.ignore_plugin_themes",
                            "must be true or false".to_string(),
                        ),
                    }
                }
            }
        },
    }
    (settings, issues)
}

/// Set `[<table>].<key>`, keeping the old value's comments and spacing.
fn set_in_table(doc: &mut DocumentMut, table: &str, key: &str, mut new_value: Value) {
    if doc
        .get_mut(table)
        .and_then(Item::as_table_like_mut)
        .is_none()
    {
        doc.insert(table, Item::Table(Table::new()));
    }
    let Some(section) = doc.get_mut(table).and_then(Item::as_table_like_mut) else {
        return;
    };
    if let Some(old) = section.get(key).and_then(Item::as_value) {
        *new_value.decor_mut() = old.decor().clone();
    }
    section.insert(key, Item::Value(new_value));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn path_in(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("settings.toml")
    }

    #[test]
    fn missing_file_gives_defaults_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let store = SettingsStore::open(&path);
        assert_eq!(store.settings(), &Settings::default());
        assert_eq!(store.settings().general.default_provider, "claude");
        assert!(store.settings().general.confirm_while_working);
        assert!(store.issues().is_empty());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn valid_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(
            &path,
            "version = 1\n[general]\ndefault_provider = \"codex\"\nconfirm_while_working = false\n",
        )
        .unwrap();
        let store = SettingsStore::open(&path);
        assert_eq!(store.settings().general.default_provider, "codex");
        assert!(!store.settings().general.confirm_while_working);
        assert!(store.issues().is_empty());
    }

    #[test]
    fn wrong_type_falls_back_for_that_key_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(
            &path,
            "version = 1\n[general]\ndefault_provider = \"codex\"\nconfirm_while_working = \"yes\"\n",
        )
        .unwrap();
        let store = SettingsStore::open(&path);
        assert_eq!(store.settings().general.default_provider, "codex");
        assert!(store.settings().general.confirm_while_working);
        assert_eq!(store.issues().len(), 1);
        assert_eq!(store.issues()[0].key, "general.confirm_while_working");
    }

    #[test]
    fn unknown_provider_falls_back_with_issue() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(
            &path,
            "version = 1\n[general]\ndefault_provider = \"gemini\"\nconfirm_while_working = false\n",
        )
        .unwrap();
        let store = SettingsStore::open(&path);
        assert_eq!(store.settings().general.default_provider, "claude");
        assert!(!store.settings().general.confirm_while_working);
        assert_eq!(store.issues().len(), 1);
        assert_eq!(store.issues()[0].key, "general.default_provider");
    }

    #[test]
    fn unparseable_file_gives_defaults_and_a_file_issue() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(&path, "this is = [not toml").unwrap();
        let store = SettingsStore::open(&path);
        assert_eq!(store.settings(), &Settings::default());
        assert_eq!(store.issues().len(), 1);
        assert_eq!(store.issues()[0].key, "file");
    }

    #[test]
    fn apply_writes_the_value_and_keeps_unknown_keys_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(
            &path,
            "# note\nversion = 1\n\n[general]\nconfirm_while_working = true # keep me\n\n[plugins.demo]\nextra = 1\n",
        )
        .unwrap();
        let mut store = SettingsStore::open(&path);
        store.apply(Change::DefaultProvider("codex".into())).unwrap();
        store.apply(Change::ConfirmWhileWorking(false)).unwrap();
        assert_eq!(store.settings().general.default_provider, "codex");
        assert!(!store.settings().general.confirm_while_working);

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# note"), "{text}");
        assert!(text.contains("# keep me"), "{text}");
        assert!(text.contains("[plugins.demo]"), "{text}");
        assert!(text.contains("extra = 1"), "{text}");
        assert!(text.contains("default_provider = \"codex\""), "{text}");
        assert!(text.contains("confirm_while_working = false"), "{text}");

        let reopened = SettingsStore::open(&path);
        assert_eq!(reopened.settings(), store.settings());
        assert!(reopened.issues().is_empty());
    }

    #[test]
    fn apply_on_a_bad_file_saves_it_to_settings_toml_invalid_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let bad = "this is = [not toml";
        fs::write(&path, bad).unwrap();
        let mut store = SettingsStore::open(&path);
        store.apply(Change::ConfirmWhileWorking(false)).unwrap();

        let invalid = dir.path().join("settings.toml.invalid");
        assert_eq!(fs::read_to_string(&invalid).unwrap(), bad);
        let reopened = SettingsStore::open(&path);
        assert!(!reopened.settings().general.confirm_while_working);
        assert!(reopened.issues().is_empty());
        assert!(store.issues().is_empty());
    }

    #[test]
    fn apply_rejects_an_unknown_provider_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = SettingsStore::open(&path);
        let err = store.apply(Change::DefaultProvider("gemini".into())).unwrap_err();
        assert!(matches!(err, SettingsError::Invalid(_)));
        assert!(!path.exists());
        assert_eq!(store.settings(), &Settings::default());
    }

    #[test]
    fn apply_fails_when_the_directory_cannot_be_created() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "i am a file").unwrap();
        let path = blocker.join("settings.toml");
        let mut store = SettingsStore::open(&path);
        let err = store.apply(Change::ConfirmWhileWorking(false)).unwrap_err();
        assert!(matches!(err, SettingsError::Io(_)));
        assert!(store.settings().general.confirm_while_working);
        assert_eq!(store.settings(), &Settings::default());
    }

    #[test]
    fn missing_version_is_written_as_1() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = SettingsStore::open(&path);
        store.apply(Change::ConfirmWhileWorking(false)).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = text.parse().unwrap();
        assert_eq!(doc["version"].as_integer(), Some(1));
        assert_eq!(doc["general"]["confirm_while_working"].as_bool(), Some(false));
    }

    #[test]
    fn newer_version_gives_an_issue_still_reads_and_keeps_the_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(
            &path,
            "version = 2\n[general]\ndefault_provider = \"codex\"\n",
        )
        .unwrap();
        let mut store = SettingsStore::open(&path);
        assert_eq!(store.settings().general.default_provider, "codex");
        assert_eq!(store.issues().len(), 1);
        assert_eq!(store.issues()[0].key, "version");

        store.apply(Change::ConfirmWhileWorking(false)).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = text.parse().unwrap();
        assert_eq!(doc["version"].as_integer(), Some(2));
    }

    #[test]
    fn reload_ignores_our_own_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = SettingsStore::open(&path);
        assert!(!store.reload_if_changed());
        store.apply(Change::ConfirmWhileWorking(false)).unwrap();
        assert!(!store.reload_if_changed());
    }

    #[test]
    fn reload_picks_up_a_hand_edit_and_clears_the_issue() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(
            &path,
            "version = 1\n[general]\nconfirm_while_working = \"yes\"\n",
        )
        .unwrap();
        let mut store = SettingsStore::open(&path);
        assert_eq!(store.issues().len(), 1);
        assert!(!store.reload_if_changed());

        fs::write(
            &path,
            "version = 1\n[general]\nconfirm_while_working = false\n",
        )
        .unwrap();
        assert!(store.reload_if_changed());
        assert!(!store.settings().general.confirm_while_working);
        assert!(store.issues().is_empty());
        assert!(!store.reload_if_changed());
    }

    #[test]
    fn appearance_defaults_to_the_bundled_pair() {
        let store = SettingsStore::open(&path_in(&tempfile::tempdir().unwrap()));
        let a = &store.settings().appearance;
        assert_eq!(a.theme_source, ThemeSource::Default);
        assert_eq!(a.theme, DEFAULT_THEME);
        assert!(!a.ignore_plugin_themes);
    }

    #[test]
    fn appearance_keys_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(
            &path,
            "version = 1\n[appearance]\ntheme_source = \"custom\"\ntheme = \"light:Nord,dark:Dracula\"\nignore_plugin_themes = true\n",
        )
        .unwrap();
        let store = SettingsStore::open(&path);
        let a = &store.settings().appearance;
        assert_eq!(a.theme_source, ThemeSource::Custom);
        assert_eq!(a.theme, "light:Nord,dark:Dracula");
        assert!(a.ignore_plugin_themes);
        assert!(store.issues().is_empty(), "{:?}", store.issues());
    }

    #[test]
    fn bad_appearance_keys_fall_back_one_by_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(
            &path,
            "[appearance]\ntheme_source = \"neon\"\ntheme = 5\nignore_plugin_themes = true\n",
        )
        .unwrap();
        let store = SettingsStore::open(&path);
        let a = &store.settings().appearance;
        assert_eq!(a.theme_source, ThemeSource::Default);
        assert_eq!(a.theme, DEFAULT_THEME);
        assert!(a.ignore_plugin_themes, "a good key still applies");
        let keys: Vec<&str> = store.issues().iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["appearance.theme_source", "appearance.theme"]);
    }

    #[test]
    fn an_empty_theme_value_is_an_issue() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(&path, "[appearance]\ntheme = \"light:Nord\"\n").unwrap();
        let store = SettingsStore::open(&path);
        assert_eq!(store.settings().appearance.theme, DEFAULT_THEME);
        assert_eq!(store.issues().len(), 1);
        assert_eq!(store.issues()[0].key, "appearance.theme");
    }

    #[test]
    fn apply_writes_appearance_keys_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(&path, "version = 1\n[general]\nconfirm_while_working = false # keep\n").unwrap();
        let mut store = SettingsStore::open(&path);
        store.apply(Change::ThemeSource(ThemeSource::Ghostty)).unwrap();
        store.apply(Change::Theme("Nord".to_string())).unwrap();
        store.apply(Change::IgnorePluginThemes(true)).unwrap();
        let a = &store.settings().appearance;
        assert_eq!(a.theme_source, ThemeSource::Ghostty);
        assert_eq!(a.theme, "Nord");
        assert!(a.ignore_plugin_themes);

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep"), "{text}");
        assert!(text.contains("theme_source = \"ghostty\""), "{text}");
        assert!(text.contains("theme = \"Nord\""), "{text}");
        assert!(text.contains("ignore_plugin_themes = true"), "{text}");
        let reopened = SettingsStore::open(&path);
        assert_eq!(reopened.settings(), store.settings());
        assert!(reopened.issues().is_empty());
    }

    #[test]
    fn apply_rejects_a_theme_that_names_nothing_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = SettingsStore::open(&path);
        for bad in ["", "  ", "light:Nord"] {
            let err = store.apply(Change::Theme(bad.to_string())).unwrap_err();
            assert!(matches!(err, SettingsError::Invalid(_)), "{bad:?}");
        }
        assert!(!path.exists());
        assert_eq!(store.settings().appearance.theme, DEFAULT_THEME);
    }
}
