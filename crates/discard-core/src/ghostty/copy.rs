//! Edit a copy: put a theme in the user's own theme folder.

use super::lookup::ThemeDirs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum CopyError {
    Missing(String),
    NoYours,
    Io(std::io::Error),
}
impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(name) => write!(f, "Theme not found: {name}"),
            Self::NoYours => write!(f, "Theme folder is not configured"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for CopyError {}
impl From<std::io::Error> for CopyError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Copy theme `name` into `$DISCARD_HOME/themes/` and return the copy's path. An existing copy is
/// returned as it is, never overwritten. An absolute path is returned as it is.
pub fn copy_to_yours(dirs: &ThemeDirs, name: &str) -> Result<PathBuf, CopyError> {
    if Path::new(name).is_absolute() {
        return Ok(PathBuf::from(name));
    }
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(CopyError::Missing(name.into()));
    }
    let dir = dirs.yours.as_ref().ok_or(CopyError::NoYours)?;
    let path = dir.join(name);
    if path.symlink_metadata().is_ok() {
        return Ok(path);
    }
    let found = dirs
        .find(name)
        .ok_or_else(|| CopyError::Missing(name.into()))?;
    std::fs::create_dir_all(dir)?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file.write_all(found.text.as_bytes())?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Ghostty folder holding `Mine`, and a "yours" folder that does not exist yet.
    fn dirs(root: &Path) -> ThemeDirs {
        let ghostty = root.join("ghostty");
        std::fs::create_dir(&ghostty).unwrap();
        std::fs::write(ghostty.join("Mine"), "original").unwrap();
        ThemeDirs {
            yours: Some(root.join("themes")),
            ghostty: vec![ghostty],
        }
    }

    #[test]
    fn a_ghostty_theme_is_copied_into_yours() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let path = copy_to_yours(&dirs, "Mine").unwrap();
        assert_eq!(path, root.path().join("themes").join("Mine"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
    }

    #[test]
    fn a_bundled_theme_is_written_from_the_embedded_text() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let path = copy_to_yours(&dirs, "Nord").unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            crate::ghostty::bundled::text("Nord").unwrap()
        );
    }

    #[test]
    fn an_existing_copy_is_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let path = copy_to_yours(&dirs, "Mine").unwrap();
        std::fs::write(&path, "changed").unwrap();
        assert_eq!(copy_to_yours(&dirs, "Mine").unwrap(), path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed");
    }

    #[test]
    fn an_existing_copy_that_is_not_text_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let themes = root.path().join("themes");
        std::fs::create_dir(&themes).unwrap();
        let binary = themes.join("Mine");
        std::fs::write(&binary, [0xff]).unwrap();
        assert_eq!(copy_to_yours(&dirs, "Mine").unwrap(), binary);
        assert_eq!(std::fs::read(&binary).unwrap(), [0xff]);
    }

    #[test]
    fn an_absolute_path_is_returned_as_it_is() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let file = root.path().join("elsewhere");
        let name = file.to_str().unwrap();
        assert_eq!(copy_to_yours(&dirs, name).unwrap(), file);
        assert!(!root.path().join("themes").exists());
    }

    #[test]
    fn an_unknown_theme_is_an_error_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        assert!(matches!(
            copy_to_yours(&dirs, "unknown theme 123"),
            Err(CopyError::Missing(_))
        ));
        assert!(!root.path().join("themes").exists());
    }

    #[test]
    fn a_name_that_leaves_the_folder_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        for bad in ["", ".", "..", "../Mine", "a/b", "a\\b"] {
            assert!(
                matches!(copy_to_yours(&dirs, bad), Err(CopyError::Missing(_))),
                "name {bad:?}"
            );
        }
    }

    #[test]
    fn no_yours_folder_is_an_error() {
        let dirs = ThemeDirs::default();
        assert!(matches!(
            copy_to_yours(&dirs, "Nord"),
            Err(CopyError::NoYours)
        ));
    }
}
