//! Find a theme by name: the user's folder, Ghostty's folders, then the
//! bundled themes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::bundled;

/// Where a theme was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// `$DISCARD_HOME/themes/`.
    Yours,
    /// Ghostty's user folders or the Ghostty app.
    Ghostty,
    /// Embedded in Discard.
    Bundled,
}

/// A theme found by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundTheme {
    pub name: String,
    pub origin: Origin,
    /// The file the text came from. `None` for a bundled theme.
    pub path: Option<PathBuf>,
    pub text: String,
}

/// The folders to search, in order.
#[derive(Debug, Clone, Default)]
pub struct ThemeDirs {
    pub yours: Option<PathBuf>,
    /// Ghostty folders, searched in order. User folders come before the app.
    pub ghostty: Vec<PathBuf>,
}

impl ThemeDirs {
    /// The folders on this machine. `home` is the user's home directory.
    /// `xdg_config_home` is `$XDG_CONFIG_HOME` when set.
    pub fn standard(
        discard_home: &Path,
        home: &Path,
        xdg_config_home: Option<&Path>,
        ghostty_resources_dir: Option<&Path>,
    ) -> ThemeDirs {
        let config_home = xdg_config_home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".config"));
        let mut ghostty = vec![
            config_home.join("ghostty").join("themes"),
            home.join("Library/Application Support/com.mitchellh.ghostty/themes"),
        ];
        if let Some(resources) = ghostty_resources_dir {
            ghostty.push(resources.join("themes"));
        }
        ghostty.extend(GHOSTTY_SYSTEM_THEMES.iter().map(PathBuf::from));
        ThemeDirs {
            yours: Some(discard_home.join("themes")),
            ghostty,
        }
    }

    /// Find `name`. A name is a theme file name, or an absolute path.
    pub fn find(&self, name: &str) -> Option<FoundTheme> {
        let as_path = Path::new(name);
        if as_path.is_absolute() {
            let text = std::fs::read_to_string(as_path).ok()?;
            return Some(FoundTheme {
                name: name.to_string(),
                origin: Origin::Yours,
                path: Some(as_path.to_path_buf()),
                text,
            });
        }
        if !is_plain_name(name) {
            return None;
        }
        let searched = self
            .yours
            .iter()
            .map(|dir| (Origin::Yours, dir))
            .chain(self.ghostty.iter().map(|dir| (Origin::Ghostty, dir)));
        for (origin, dir) in searched {
            let path = dir.join(name);
            if let Ok(text) = std::fs::read_to_string(&path) {
                return Some(FoundTheme {
                    name: name.to_string(),
                    origin,
                    path: Some(path),
                    text,
                });
            }
        }
        bundled::text(name).map(|text| FoundTheme {
            name: name.to_string(),
            origin: Origin::Bundled,
            path: None,
            text: text.to_string(),
        })
    }

    /// Every theme found, sorted by name. A name in an earlier origin
    /// hides the same name in a later one.
    pub fn list(&self) -> Vec<(String, Origin)> {
        let mut found: BTreeMap<String, Origin> = BTreeMap::new();
        let searched = self
            .yours
            .iter()
            .map(|dir| (Origin::Yours, dir))
            .chain(self.ghostty.iter().map(|dir| (Origin::Ghostty, dir)));
        for (origin, dir) in searched {
            for name in file_names(dir) {
                found.entry(name).or_insert(origin);
            }
        }
        for name in bundled::names() {
            found.entry(name.to_string()).or_insert(Origin::Bundled);
        }
        let mut list: Vec<(String, Origin)> = found.into_iter().collect();
        list.sort_by_key(|(name, _)| name.to_lowercase());
        list
    }
}

/// Where a Ghostty install keeps its themes: the macOS app bundle, then the Linux package folders.
const GHOSTTY_SYSTEM_THEMES: &[&str] = &[
    "/Applications/Ghostty.app/Contents/Resources/ghostty/themes",
    "/usr/share/ghostty/themes",
    "/usr/local/share/ghostty/themes",
];

/// A name with no path separators, so it stays inside the theme folder.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

/// The names of the regular, non-hidden files in `dir`.
fn file_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn dirs_in(root: &Path) -> ThemeDirs {
        let yours = root.join("yours");
        let user = root.join("ghostty-user");
        let app = root.join("ghostty-app");
        for d in [&yours, &user, &app] {
            fs::create_dir_all(d).unwrap();
        }
        ThemeDirs {
            yours: Some(yours),
            ghostty: vec![user, app],
        }
    }

    #[test]
    fn your_folder_wins_over_ghostty_and_bundled() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        fs::write(dirs.yours.as_ref().unwrap().join("Nord"), "background = #010101\n").unwrap();
        fs::write(dirs.ghostty[0].join("Nord"), "background = #020202\n").unwrap();
        let found = dirs.find("Nord").unwrap();
        assert_eq!(found.origin, Origin::Yours);
        assert_eq!(found.text, "background = #010101\n");
        assert_eq!(found.path, Some(dirs.yours.unwrap().join("Nord")));
    }

    #[test]
    fn ghostty_user_folder_wins_over_the_app() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        fs::write(dirs.ghostty[0].join("Mine"), "background = #020202\n").unwrap();
        fs::write(dirs.ghostty[1].join("Mine"), "background = #030303\n").unwrap();
        fs::write(dirs.ghostty[1].join("AppOnly"), "background = #040404\n").unwrap();
        assert_eq!(dirs.find("Mine").unwrap().text, "background = #020202\n");
        let app_only = dirs.find("AppOnly").unwrap();
        assert_eq!((app_only.origin, app_only.text.as_str()), (Origin::Ghostty, "background = #040404\n"));
    }

    #[test]
    fn bundled_is_the_last_resort() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        let found = dirs.find("Nord").unwrap();
        assert_eq!(found.origin, Origin::Bundled);
        assert_eq!(found.path, None);
        assert_eq!(found.text, bundled::text("Nord").unwrap());
        assert_eq!(dirs.find("No Such Theme"), None);
    }

    #[test]
    fn an_absolute_path_is_read_directly() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        let file = root.path().join("custom-theme");
        fs::write(&file, "foreground = #ffffff\n").unwrap();
        let found = dirs.find(file.to_str().unwrap()).unwrap();
        assert_eq!(found.path, Some(file));
        assert_eq!(found.text, "foreground = #ffffff\n");
        assert_eq!(dirs.find(root.path().join("missing").to_str().unwrap()), None);
    }

    #[test]
    fn a_relative_name_cannot_leave_the_theme_folders() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        fs::write(root.path().join("secret"), "background = #000000\n").unwrap();
        for name in ["../secret", "sub/secret", "..", "."] {
            assert_eq!(dirs.find(name), None, "name {name:?}");
        }
    }

    #[test]
    fn list_merges_origins_hides_duplicates_and_sorts() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        fs::write(dirs.yours.as_ref().unwrap().join("Nord"), "x").unwrap();
        fs::write(dirs.yours.as_ref().unwrap().join("Zzz Mine"), "x").unwrap();
        fs::write(dirs.ghostty[1].join("Aardvark"), "x").unwrap();
        fs::write(dirs.ghostty[1].join(".hidden"), "x").unwrap();
        let list = dirs.list();
        let get = |name: &str| list.iter().find(|(n, _)| n == name).map(|(_, o)| *o);
        assert_eq!(get("Nord"), Some(Origin::Yours));
        assert_eq!(get("Zzz Mine"), Some(Origin::Yours));
        assert_eq!(get("Aardvark"), Some(Origin::Ghostty));
        assert_eq!(get("Dracula"), Some(Origin::Bundled));
        assert_eq!(get(".hidden"), None);
        assert_eq!(list.iter().filter(|(n, _)| n == "Nord").count(), 1);
        let names: Vec<&str> = list.iter().map(|(n, _)| n.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(names, sorted);
    }

    #[test]
    fn standard_dirs_follow_ghostty_conventions() {
        let discard = Path::new("/d");
        let home = Path::new("/h");
        let dirs = ThemeDirs::standard(discard, home, None, None);
        assert_eq!(dirs.yours, Some(PathBuf::from("/d/themes")));
        assert_eq!(
            dirs.ghostty,
            vec![
                PathBuf::from("/h/.config/ghostty/themes"),
                PathBuf::from("/h/Library/Application Support/com.mitchellh.ghostty/themes"),
                PathBuf::from("/Applications/Ghostty.app/Contents/Resources/ghostty/themes"),
                PathBuf::from("/usr/share/ghostty/themes"),
                PathBuf::from("/usr/local/share/ghostty/themes"),
            ]
        );
        let dirs = ThemeDirs::standard(discard, home, Some(Path::new("/x")), Some(Path::new("/res")));
        assert_eq!(
            dirs.ghostty,
            vec![
                PathBuf::from("/x/ghostty/themes"),
                PathBuf::from("/h/Library/Application Support/com.mitchellh.ghostty/themes"),
                PathBuf::from("/res/themes"),
                PathBuf::from("/Applications/Ghostty.app/Contents/Resources/ghostty/themes"),
                PathBuf::from("/usr/share/ghostty/themes"),
                PathBuf::from("/usr/local/share/ghostty/themes"),
            ]
        );
    }
}
