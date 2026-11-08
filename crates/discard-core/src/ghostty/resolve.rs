//! Turn the user's theme choice into colors. A failure never blocks the
//! UI: the result falls back to the default theme and lists the problems.

use std::path::{Path, PathBuf};

use super::bundled;
use super::file::{FileIssue, ThemeColors, parse};
use super::lookup::ThemeDirs;
use super::map::{UiColors, map};
use super::spec::{Appearance, ThemeSpec};

/// Where the theme comes from. Matches `appearance.theme_source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeSource {
    /// The bundled default pair.
    #[default]
    Default,
    /// The user's Ghostty config.
    Ghostty,
    /// The `theme` setting in `settings.toml`.
    Custom,
}

impl ThemeSource {
    /// The value in `settings.toml`.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeSource::Default => "default",
            ThemeSource::Ghostty => "ghostty",
            ThemeSource::Custom => "custom",
        }
    }

    pub fn from_setting(value: &str) -> Option<ThemeSource> {
        [ThemeSource::Default, ThemeSource::Ghostty, ThemeSource::Custom]
            .into_iter()
            .find(|source| source.as_str() == value)
    }
}

/// What to resolve.
#[derive(Debug, Clone)]
pub struct ThemeRequest<'a> {
    pub source: ThemeSource,
    /// The `appearance.theme` setting. Used when `source` is `Custom`.
    pub custom_theme: &'a str,
    pub appearance: Appearance,
    pub dirs: &'a ThemeDirs,
    /// Ghostty config files, in load order. A later file wins.
    pub ghostty_configs: &'a [PathBuf],
}

/// One thing that went wrong while resolving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeProblem {
    /// The file the problem is in, when there is one.
    pub file: Option<PathBuf>,
    /// The line in `file`, starting at 1.
    pub line: Option<usize>,
    pub message: String,
}

/// The colors to use, and what went wrong on the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTheme {
    /// The name of the theme file in use.
    pub theme_name: String,
    pub colors: ThemeColors,
    pub mono_font: Option<String>,
    pub ui: UiColors,
    pub problems: Vec<ThemeProblem>,
    /// Every file that can change the result, found or not. Poll these.
    pub watched: Vec<PathBuf>,
}

/// The Ghostty config files on this machine, in load order.
pub fn ghostty_config_paths(home: &Path, xdg_config_home: Option<&Path>) -> Vec<PathBuf> {
    let config_home = xdg_config_home
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".config"));
    let folders = [
        config_home.join("ghostty"),
        home.join("Library/Application Support/com.mitchellh.ghostty"),
    ];
    folders
        .into_iter()
        .flat_map(|folder| [folder.join("config.ghostty"), folder.join("config")])
        .collect()
}

/// Resolve `request`. Never fails: see [`ResolvedTheme::problems`].
pub fn resolve(request: &ThemeRequest<'_>) -> ResolvedTheme {
    let mut run = Run::default();
    for path in request.ghostty_configs { run.watch(path.clone()); }
    let default_spec = ThemeSpec::parse(bundled::DEFAULT_THEME);

    let (spec, inline) = match request.source {
        ThemeSource::Default => (default_spec.clone(), ThemeColors::default()),
        ThemeSource::Custom => match ThemeSpec::parse(request.custom_theme) {
            Some(spec) => (Some(spec), ThemeColors::default()),
            None => {
                run.problem(
                    None,
                    None,
                    format!(
                        "the `theme` setting `{}` does not name a theme",
                        request.custom_theme
                    ),
                );
                (default_spec.clone(), ThemeColors::default())
            }
        },
        ThemeSource::Ghostty => read_ghostty_configs(request, &mut run, default_spec.as_ref()),
    };

    let (theme_name, base) = match spec {
        Some(spec) => load_base(request, &mut run, &spec, default_spec.as_ref()),
        None => (String::new(), ThemeColors::default()),
    };
    let colors = base.overlaid_with(&inline);
    ResolvedTheme {
        theme_name,
        mono_font: super::font::mono_font_family(request.ghostty_configs),
        ui: map(&colors),
        colors,
        problems: run.problems,
        watched: run.watched,
    }
}

#[derive(Default)]
struct Run {
    problems: Vec<ThemeProblem>,
    watched: Vec<PathBuf>,
}

impl Run {
    fn problem(&mut self, file: Option<&Path>, line: Option<usize>, message: String) {
        self.problems.push(ThemeProblem {
            file: file.map(Path::to_path_buf),
            line,
            message,
        });
    }

    fn watch(&mut self, path: PathBuf) {
        if !self.watched.contains(&path) {
            self.watched.push(path);
        }
    }

    fn file_issues(&mut self, file: Option<&Path>, issues: Vec<FileIssue>) {
        for issue in issues {
            self.problem(file, Some(issue.line), issue.message);
        }
    }
}

/// Read every Ghostty config that exists. A later file wins. Returns the
/// theme the config names and its inline colors.
fn read_ghostty_configs(
    request: &ThemeRequest<'_>,
    run: &mut Run,
    default_spec: Option<&ThemeSpec>,
) -> (Option<ThemeSpec>, ThemeColors) {
    let mut colors = ThemeColors::default();
    let mut theme: Option<String> = None;
    let mut found_any = false;
    for path in request.ghostty_configs {
        run.watch(path.clone());
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        found_any = true;
        let parsed = parse(&text);
        run.file_issues(Some(path), parsed.issues);
        colors = colors.overlaid_with(&parsed.colors);
        if parsed.theme.is_some() {
            theme = parsed.theme;
        }
    }
    if !found_any {
        run.problem(
            None,
            None,
            "no Ghostty config found, so the default theme is in use".to_string(),
        );
        return (default_spec.cloned(), ThemeColors::default());
    }
    let Some(raw) = theme else {
        return (None, colors);
    };
    match ThemeSpec::parse(&raw) {
        Some(spec) => (Some(spec), colors),
        None => {
            run.problem(
                None,
                None,
                format!("the Ghostty `theme` value `{raw}` does not name a theme"),
            );
            (default_spec.cloned(), colors)
        }
    }
}

/// Load the theme file for `spec`. A missing theme loads the default.
fn load_base(
    request: &ThemeRequest<'_>,
    run: &mut Run,
    spec: &ThemeSpec,
    default_spec: Option<&ThemeSpec>,
) -> (String, ThemeColors) {
    let name = spec.name_for(request.appearance);
    for candidate in candidate_paths(request.dirs, name) {
        run.watch(candidate);
    }
    if let Some(found) = request.dirs.find(name) {
        let parsed = parse(&found.text);
        run.file_issues(found.path.as_deref(), parsed.issues);
        return (name.to_string(), parsed.colors);
    }
    run.problem(None, None, format!("theme `{name}` was not found"));
    let Some(fallback) = default_spec.map(|s| s.name_for(request.appearance)) else {
        return (String::new(), ThemeColors::default());
    };
    let colors = bundled::text(fallback)
        .map(|text| parse(text).colors)
        .unwrap_or_default();
    (fallback.to_string(), colors)
}

/// Every path a theme named `name` could be read from.
fn candidate_paths(dirs: &ThemeDirs, name: &str) -> Vec<PathBuf> {
    let as_path = Path::new(name);
    if as_path.is_absolute() {
        return vec![as_path.to_path_buf()];
    }
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Vec::new();
    }
    dirs.yours
        .iter()
        .chain(dirs.ghostty.iter())
        .map(|dir| dir.join(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ghostty::bundled;
    use crate::ghostty::color::Rgb;
    use crate::ghostty::file::parse;
    use std::fs;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    struct Fixture {
        _root: tempfile::TempDir,
        dirs: ThemeDirs,
        configs: Vec<PathBuf>,
    }

    impl Fixture {
        fn new() -> Fixture {
            let root = tempfile::tempdir().unwrap();
            let yours = root.path().join("yours");
            let app = root.path().join("app");
            fs::create_dir_all(&yours).unwrap();
            fs::create_dir_all(&app).unwrap();
            let configs = vec![root.path().join("config-a"), root.path().join("config-b")];
            Fixture {
                dirs: ThemeDirs {
                    yours: Some(yours),
                    ghostty: vec![app],
                },
                configs,
                _root: root,
            }
        }

        fn theme(&self, name: &str, text: &str) {
            fs::write(self.dirs.yours.as_ref().unwrap().join(name), text).unwrap();
        }

        fn request<'a>(
            &'a self,
            source: ThemeSource,
            custom: &'a str,
            appearance: Appearance,
        ) -> ThemeRequest<'a> {
            ThemeRequest {
                source,
                custom_theme: custom,
                appearance,
                dirs: &self.dirs,
                ghostty_configs: &self.configs,
            }
        }
    }

    #[test]
    fn custom_source_still_reads_the_ghostty_font() {
        let fx = Fixture::new();
        fs::write(&fx.configs[0], "font-family = SF Mono\n").unwrap();
        for source in [ThemeSource::Custom, ThemeSource::Default, ThemeSource::Ghostty] {
            let resolved = resolve(&fx.request(source, "Nord", Appearance::Dark));
            assert_eq!(resolved.mono_font.as_deref(), Some("SF Mono"));
            assert!(fx.configs.iter().all(|path| resolved.watched.contains(path)));
        }
    }

    #[test]
    fn default_source_follows_the_appearance() {
        let fx = Fixture::new();
        let light = resolve(&fx.request(ThemeSource::Default, "", Appearance::Light));
        let dark = resolve(&fx.request(ThemeSource::Default, "", Appearance::Dark));
        assert_eq!(light.theme_name, "Catppuccin Latte");
        assert_eq!(dark.theme_name, "Catppuccin Mocha");
        assert!(light.problems.is_empty() && dark.problems.is_empty());
        assert_eq!(dark.colors, parse(bundled::text("Catppuccin Mocha").unwrap()).colors);
        assert_ne!(light.ui.background, dark.ui.background);
    }

    #[test]
    fn custom_source_uses_the_theme_setting() {
        let fx = Fixture::new();
        let r = resolve(&fx.request(
            ThemeSource::Custom,
            "light:Nord,dark:Dracula",
            Appearance::Dark,
        ));
        assert_eq!(r.theme_name, "Dracula");
        let single = resolve(&fx.request(ThemeSource::Custom, "Nord", Appearance::Light));
        assert_eq!(single.theme_name, "Nord");
    }

    #[test]
    fn your_theme_file_overrides_a_bundled_name() {
        let fx = Fixture::new();
        fx.theme("Nord", "background = #010203\nforeground = #fdfdfd\n");
        let r = resolve(&fx.request(ThemeSource::Custom, "Nord", Appearance::Dark));
        assert_eq!(r.colors.background, Some(rgb(1, 2, 3)));
        assert_eq!(r.ui.background, rgb(1, 2, 3));
    }

    #[test]
    fn a_missing_custom_theme_falls_back_and_reports() {
        let fx = Fixture::new();
        let r = resolve(&fx.request(ThemeSource::Custom, "No Such Theme", Appearance::Dark));
        assert_eq!(r.theme_name, "Catppuccin Mocha");
        assert_eq!(r.problems.len(), 1);
        assert!(r.problems[0].message.contains("No Such Theme"), "{:?}", r.problems);
    }

    #[test]
    fn an_empty_custom_theme_falls_back_and_reports() {
        let fx = Fixture::new();
        let r = resolve(&fx.request(ThemeSource::Custom, "light:Nord", Appearance::Dark));
        assert_eq!(r.theme_name, "Catppuccin Mocha");
        assert_eq!(r.problems.len(), 1);
    }

    #[test]
    fn bad_lines_in_a_theme_file_are_reported_with_file_and_line() {
        let fx = Fixture::new();
        fx.theme("Broken", "background = #000000\nforeground = oops\n");
        let r = resolve(&fx.request(ThemeSource::Custom, "Broken", Appearance::Dark));
        assert_eq!(r.theme_name, "Broken");
        assert_eq!(r.colors.background, Some(rgb(0, 0, 0)));
        assert_eq!(r.problems.len(), 1);
        assert_eq!(r.problems[0].line, Some(2));
        assert_eq!(
            r.problems[0].file,
            Some(fx.dirs.yours.as_ref().unwrap().join("Broken"))
        );
    }

    #[test]
    fn ghostty_source_reads_the_config_theme_and_inline_overrides() {
        let fx = Fixture::new();
        fx.theme("Mine", "background = #111111\nforeground = #eeeeee\npalette = 1=#aa0000\n");
        fs::write(
            &fx.configs[0],
            "theme = light:Nord,dark:Mine\nbackground = #222222\n",
        )
        .unwrap();
        let r = resolve(&fx.request(ThemeSource::Ghostty, "ignored", Appearance::Dark));
        assert_eq!(r.theme_name, "Mine");
        assert_eq!(r.colors.background, Some(rgb(0x22, 0x22, 0x22)), "inline wins");
        assert_eq!(r.colors.foreground, Some(rgb(0xee, 0xee, 0xee)));
        assert_eq!(r.colors.palette[1], Some(rgb(0xaa, 0, 0)));
        assert!(r.problems.is_empty(), "{:?}", r.problems);
    }

    #[test]
    fn a_later_ghostty_config_wins() {
        let fx = Fixture::new();
        fs::write(&fx.configs[0], "theme = Nord\nbackground = #111111\n").unwrap();
        fs::write(&fx.configs[1], "theme = Dracula\n").unwrap();
        let r = resolve(&fx.request(ThemeSource::Ghostty, "", Appearance::Dark));
        assert_eq!(r.theme_name, "Dracula");
        assert_eq!(r.colors.background, Some(rgb(0x11, 0x11, 0x11)), "inline colors merge");
    }

    #[test]
    fn ghostty_source_without_a_theme_line_uses_only_inline_colors() {
        let fx = Fixture::new();
        fs::write(&fx.configs[0], "background = #123456\n").unwrap();
        let r = resolve(&fx.request(ThemeSource::Ghostty, "", Appearance::Dark));
        assert_eq!(r.theme_name, "");
        assert_eq!(r.ui.background, rgb(0x12, 0x34, 0x56));
        assert!(r.problems.is_empty());
    }

    #[test]
    fn ghostty_source_without_a_config_falls_back_and_reports() {
        let fx = Fixture::new();
        let r = resolve(&fx.request(ThemeSource::Ghostty, "", Appearance::Light));
        assert_eq!(r.theme_name, "Catppuccin Latte");
        assert_eq!(r.problems.len(), 1);
        assert!(r.problems[0].message.contains("Ghostty config"), "{:?}", r.problems);
    }

    #[test]
    fn a_missing_ghostty_theme_falls_back_for_the_base_and_keeps_inline_colors() {
        let fx = Fixture::new();
        fs::write(&fx.configs[0], "theme = Gone\nforeground = #abcdef\n").unwrap();
        let r = resolve(&fx.request(ThemeSource::Ghostty, "", Appearance::Dark));
        assert_eq!(r.theme_name, "Catppuccin Mocha");
        assert_eq!(r.colors.foreground, Some(rgb(0xab, 0xcd, 0xef)));
        assert_eq!(r.problems.len(), 1);
        assert!(r.problems[0].message.contains("Gone"));
    }

    #[test]
    fn watched_lists_every_candidate_file() {
        let fx = Fixture::new();
        fs::write(&fx.configs[0], "theme = Dracula\n").unwrap();
        fx.theme("Dracula", "background = #000000\n");
        let r = resolve(&fx.request(ThemeSource::Ghostty, "", Appearance::Dark));
        assert!(r.watched.contains(&fx.configs[0]));
        assert!(r.watched.contains(&fx.configs[1]), "a missing config can appear later");
        assert!(r.watched.contains(&fx.dirs.yours.as_ref().unwrap().join("Dracula")));
    }

    #[test]
    fn config_paths_follow_ghostty_conventions() {
        let home = Path::new("/h");
        assert_eq!(
            ghostty_config_paths(home, None),
            vec![
                PathBuf::from("/h/.config/ghostty/config.ghostty"),
                PathBuf::from("/h/.config/ghostty/config"),
                PathBuf::from("/h/Library/Application Support/com.mitchellh.ghostty/config.ghostty"),
                PathBuf::from("/h/Library/Application Support/com.mitchellh.ghostty/config"),
            ]
        );
        assert_eq!(
            ghostty_config_paths(home, Some(Path::new("/x")))[0],
            PathBuf::from("/x/ghostty/config.ghostty")
        );
    }
}
