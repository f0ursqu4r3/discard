//! Arguments for relaunching the app, for example when Safe Mode changes.

/// The original arguments without every `--safe-mode`, plus one `--safe-mode` when `safe_mode` is true.
pub fn args(original: &[String], safe_mode: bool) -> Vec<String> {
    let mut out: Vec<String> = original
        .iter()
        .filter(|a| a.as_str() != "--safe-mode")
        .cloned()
        .collect();
    if safe_mode {
        out.push("--safe-mode".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn relaunch_adds_safe_mode_once() {
        assert_eq!(
            args(&strings(&["--builder"]), true),
            strings(&["--builder", "--safe-mode"])
        );
        assert_eq!(args(&[], true), strings(&["--safe-mode"]));
    }

    #[test]
    fn relaunch_removes_safe_mode() {
        assert_eq!(
            args(&strings(&["--safe-mode", "--builder"]), false),
            strings(&["--builder"])
        );
    }

    #[test]
    fn relaunch_keeps_provider_flag_and_order() {
        assert_eq!(
            args(
                &strings(&["--provider", "codex", "--builder", "--worktree"]),
                true
            ),
            strings(&[
                "--provider",
                "codex",
                "--builder",
                "--worktree",
                "--safe-mode"
            ])
        );
        assert_eq!(
            args(
                &strings(&["--provider=codex", "--safe-mode", "--builder"]),
                false
            ),
            strings(&["--provider=codex", "--builder"])
        );
    }

    #[test]
    fn relaunch_input_with_two_safe_mode_flags_yields_one() {
        assert_eq!(
            args(&strings(&["--safe-mode", "--builder", "--safe-mode"]), true),
            strings(&["--builder", "--safe-mode"])
        );
        assert_eq!(
            args(&strings(&["--safe-mode", "--safe-mode"]), false),
            Vec::<String>::new()
        );
    }
}
