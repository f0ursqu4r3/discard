//! Read the primary Ghostty font family for the UI.

use std::path::PathBuf;

/// Read the first family in each config. A later config replaces or clears it.
pub fn mono_font_family(configs: &[PathBuf]) -> Option<String> {
    let mut family = None;
    for path in configs {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            if key.trim() != "font-family" { continue; }
            let value = value.trim();
            if value.is_empty() { continue; }
            let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
            family = Some(value.to_string());
            break;
        }
    }
    family.filter(|value| !value.is_empty() && !value.chars().any(|c| c.is_control() || matches!(c, '"' | '\\' | ';' | '{' | '}')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn read(files: &[&str]) -> Option<String> {
        let root = tempfile::tempdir().unwrap();
        let paths: Vec<_> = files.iter().enumerate().map(|(i, text)| {
            let path = root.path().join(i.to_string());
            std::fs::write(&path, text).unwrap();
            path
        }).collect();
        mono_font_family(&paths)
    }

    #[test]
    fn reads_the_first_family_in_a_file() {
        assert_eq!(read(&["font-family = \"SF Mono\"\nfont-family = Menlo\n"]), Some("SF Mono".into()));
    }
    #[test]
    fn a_later_file_wins() {
        assert_eq!(read(&["font-family = Iosevka\n", "font-family = \"SF Mono\"\n"]), Some("SF Mono".into()));
    }
    #[test]
    fn an_empty_value_in_a_later_file_clears_it() {
        assert_eq!(read(&["font-family = Iosevka\n", "font-family = \"\"\n"]), None);
    }
    #[test]
    fn ignores_comments() {
        assert_eq!(read(&["# font-family = Iosevka\nfont-family = SF Mono\n"]), Some("SF Mono".into()));
    }
    #[test]
    fn rejects_css_breaking_values() {
        for value in ["Foo\"", "Foo\\", "Foo;color:red", "Foo{", "Foo}", "Foo\u{7}", "\"Foo\t\""] {
            assert_eq!(read(&[&format!("font-family = {value}\n")]), None, "{value:?}");
        }
    }
    #[test]
    fn missing_files_give_none() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(mono_font_family(&[PathBuf::from(root.path()).join("missing")]), None);
    }
}
