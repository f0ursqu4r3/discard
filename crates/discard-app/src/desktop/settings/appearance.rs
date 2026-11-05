use discard_core::ghostty::spec::{Appearance, ThemeSpec};

/// Update one part of the stored theme choice.
pub fn theme_choice(value: &str, choice: &str, name: Option<&str>, light_system: bool) -> Result<String, String> {
    let spec = ThemeSpec::parse(value).ok_or_else(|| "Invalid theme".to_string())?;
    let next = match choice {
        "single" => spec.as_single(if light_system { Appearance::Light } else { Appearance::Dark }),
        "pair" => spec.as_pair(),
        "light" | "dark" => {
            let name = name.filter(|n| !n.is_empty()).ok_or_else(|| "Invalid theme".to_string())?;
            match (&spec, choice) {
                (ThemeSpec::Single(_), _) => ThemeSpec::Single(name.to_string()),
                (_, "light") => spec.with_light(name),
                _ => spec.with_dark(name),
            }
        },
        _ => return Err("Invalid theme choice".into()),
    };
    Ok(next.to_value())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_light_preserves_dark() {
        assert_eq!(theme_choice("light:Day,dark:Night", "light", Some("New"), false).unwrap(), "light:New,dark:Night");
    }
    #[test]
    fn single_mode_uses_current_system_theme() {
        assert_eq!(theme_choice("light:Day,dark:Night", "single", None, true).unwrap(), "Day");
    }
}
