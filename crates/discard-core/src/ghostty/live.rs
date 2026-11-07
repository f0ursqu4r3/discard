//! Keeps a resolved theme current. The UI calls [`LiveTheme::update`] on a
//! timer. It resolves again only when the settings, the system appearance,
//! or a theme file changed.

use std::path::PathBuf;
use std::time::SystemTime;

use super::lookup::ThemeDirs;
use super::resolve::{ResolvedTheme, ThemeRequest, resolve};
use super::spec::Appearance;
use crate::settings::AppearanceSettings;

/// The modified time and size of each watched file. `None` means the file is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint(Vec<(PathBuf, Option<(SystemTime, u64)>)>);

impl Fingerprint {
    pub fn of(paths: &[PathBuf]) -> Fingerprint {
        let stamp = |path: &PathBuf| {
            let meta = std::fs::metadata(path).ok()?;
            Some((meta.modified().ok()?, meta.len()))
        };
        Fingerprint(paths.iter().map(|p| (p.clone(), stamp(p))).collect())
    }
}

pub struct LiveTheme {
    dirs: ThemeDirs,
    ghostty_configs: Vec<PathBuf>,
    inputs: (AppearanceSettings, Appearance),
    fingerprint: Fingerprint,
    resolved: ResolvedTheme,
}

impl LiveTheme {
    /// Resolve the theme once.
    pub fn new(
        dirs: ThemeDirs,
        ghostty_configs: Vec<PathBuf>,
        settings: &AppearanceSettings,
        appearance: Appearance,
    ) -> LiveTheme {
        let resolved = resolve_with(&dirs, &ghostty_configs, settings, appearance);
        let fingerprint = Fingerprint::of(&resolved.watched);
        LiveTheme {
            dirs,
            ghostty_configs,
            inputs: (settings.clone(), appearance),
            fingerprint,
            resolved,
        }
    }

    pub fn resolved(&self) -> &ResolvedTheme {
        &self.resolved
    }

    /// Check the inputs and the files. Returns `true` when the resolved
    /// theme is different now.
    pub fn update(&mut self, settings: &AppearanceSettings, appearance: Appearance) -> bool {
        let (old_settings, old_appearance) = &self.inputs;
        let inputs_changed = settings.theme_source != old_settings.theme_source
            || settings.theme != old_settings.theme
            || appearance != *old_appearance;
        let files_changed = Fingerprint::of(&self.resolved.watched) != self.fingerprint;
        if !inputs_changed && !files_changed {
            return false;
        }
        let resolved = resolve_with(&self.dirs, &self.ghostty_configs, settings, appearance);
        self.fingerprint = Fingerprint::of(&resolved.watched);
        self.inputs = (settings.clone(), appearance);
        let changed = resolved != self.resolved;
        self.resolved = resolved;
        changed
    }
}

fn resolve_with(
    dirs: &ThemeDirs,
    ghostty_configs: &[PathBuf],
    settings: &AppearanceSettings,
    appearance: Appearance,
) -> ResolvedTheme {
    resolve(&ThemeRequest {
        source: settings.theme_source,
        custom_theme: &settings.theme,
        appearance,
        dirs,
        ghostty_configs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ghostty::color::Rgb;
    use crate::ghostty::resolve::ThemeSource;
    use std::fs;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    fn custom(theme: &str) -> AppearanceSettings {
        AppearanceSettings {
            theme_source: ThemeSource::Custom,
            theme: theme.to_string(),
            ..AppearanceSettings::default()
        }
    }

    struct Fixture {
        _root: tempfile::TempDir,
        yours: PathBuf,
        config: PathBuf,
    }

    impl Fixture {
        fn new() -> Fixture {
            let root = tempfile::tempdir().unwrap();
            let yours = root.path().join("yours");
            fs::create_dir_all(&yours).unwrap();
            let config = root.path().join("ghostty-config");
            Fixture { _root: root, yours, config }
        }

        fn live(&self, settings: &AppearanceSettings, appearance: Appearance) -> LiveTheme {
            let dirs = ThemeDirs { yours: Some(self.yours.clone()), ghostty: Vec::new() };
            LiveTheme::new(dirs, vec![self.config.clone()], settings, appearance)
        }
    }

    #[test]
    fn the_first_resolve_uses_the_settings_and_appearance() {
        let fx = Fixture::new();
        let live = fx.live(&AppearanceSettings::default(), Appearance::Dark);
        assert_eq!(live.resolved().theme_name, "Catppuccin Mocha");
    }

    #[test]
    fn nothing_changed_means_no_update() {
        let fx = Fixture::new();
        let settings = AppearanceSettings::default();
        let mut live = fx.live(&settings, Appearance::Dark);
        assert!(!live.update(&settings, Appearance::Dark));
        assert!(!live.update(&settings, Appearance::Dark));
    }

    #[test]
    fn a_new_appearance_switches_the_theme() {
        let fx = Fixture::new();
        let settings = AppearanceSettings::default();
        let mut live = fx.live(&settings, Appearance::Dark);
        assert!(live.update(&settings, Appearance::Light));
        assert_eq!(live.resolved().theme_name, "Catppuccin Latte");
        assert!(!live.update(&settings, Appearance::Light));
    }

    #[test]
    fn new_settings_switch_the_theme() {
        let fx = Fixture::new();
        let mut live = fx.live(&AppearanceSettings::default(), Appearance::Dark);
        assert!(live.update(&custom("Nord"), Appearance::Dark));
        assert_eq!(live.resolved().theme_name, "Nord");
    }

    #[test]
    fn a_settings_change_that_gives_the_same_theme_is_not_an_update() {
        let fx = Fixture::new();
        let mut live = fx.live(&custom("light:Nord,dark:Nord"), Appearance::Dark);
        assert!(!live.update(&custom("Nord"), Appearance::Dark));
    }

    #[test]
    fn ignoring_plugin_themes_does_not_resolve_again() {
        let fx = Fixture::new();
        let settings = AppearanceSettings::default();
        let mut live = fx.live(&settings, Appearance::Dark);
        let toggled = AppearanceSettings { ignore_plugin_themes: true, ..settings };
        assert!(!live.update(&toggled, Appearance::Dark), "the colors are the same");
    }

    #[test]
    fn editing_a_theme_file_updates_the_colors() {
        let fx = Fixture::new();
        fs::write(fx.yours.join("Mine"), "background = #111111\nforeground = #eeeeee\n").unwrap();
        let settings = custom("Mine");
        let mut live = fx.live(&settings, Appearance::Dark);
        assert_eq!(live.resolved().ui.background, rgb(0x11, 0x11, 0x11));
        assert!(!live.update(&settings, Appearance::Dark));

        fs::write(fx.yours.join("Mine"), "background = #222222\nforeground = #eeeeee\n# edited\n").unwrap();
        assert!(live.update(&settings, Appearance::Dark));
        assert_eq!(live.resolved().ui.background, rgb(0x22, 0x22, 0x22));
    }

    #[test]
    fn a_same_size_edit_is_seen_through_the_modified_time() {
        let fx = Fixture::new();
        let file = fx.yours.join("Mine");
        fs::write(&file, "background = #111111\n").unwrap();
        let settings = custom("Mine");
        let mut live = fx.live(&settings, Appearance::Dark);

        fs::write(&file, "background = #222222\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        fs::File::options().write(true).open(&file).unwrap().set_modified(later).unwrap();
        assert!(live.update(&settings, Appearance::Dark));
        assert_eq!(live.resolved().ui.background, rgb(0x22, 0x22, 0x22));
    }

    #[test]
    fn a_missing_theme_that_appears_later_is_picked_up() {
        let fx = Fixture::new();
        let settings = custom("Later");
        let mut live = fx.live(&settings, Appearance::Dark);
        assert_eq!(live.resolved().problems.len(), 1);
        assert_eq!(live.resolved().theme_name, "Catppuccin Mocha");

        fs::write(fx.yours.join("Later"), "background = #333333\n").unwrap();
        assert!(live.update(&settings, Appearance::Dark));
        assert_eq!(live.resolved().theme_name, "Later");
        assert!(live.resolved().problems.is_empty());
    }

    #[test]
    fn a_ghostty_config_edit_is_picked_up_and_a_deleted_one_falls_back() {
        let fx = Fixture::new();
        let settings = AppearanceSettings { theme_source: ThemeSource::Ghostty, ..Default::default() };
        fs::write(&fx.config, "background = #444444\n").unwrap();
        let mut live = fx.live(&settings, Appearance::Dark);
        assert_eq!(live.resolved().ui.background, rgb(0x44, 0x44, 0x44));

        fs::write(&fx.config, "background = #555555\nforeground = #ffffff\n").unwrap();
        assert!(live.update(&settings, Appearance::Dark));
        assert_eq!(live.resolved().ui.background, rgb(0x55, 0x55, 0x55));

        fs::remove_file(&fx.config).unwrap();
        assert!(live.update(&settings, Appearance::Dark));
        assert_eq!(live.resolved().theme_name, "Catppuccin Mocha");
        assert_eq!(live.resolved().problems.len(), 1);
    }

    #[test]
    fn a_fingerprint_notes_missing_files() {
        let fx = Fixture::new();
        let missing = fx.yours.join("nope");
        let before = Fingerprint::of(std::slice::from_ref(&missing));
        fs::write(&missing, "x").unwrap();
        assert_ne!(before, Fingerprint::of(std::slice::from_ref(&missing)));
    }
}
