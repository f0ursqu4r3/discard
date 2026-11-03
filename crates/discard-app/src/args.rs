//! Command-line options.

use discard_core::bootstrap::WorkspaceChoice;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub provider: String,
    /// Start with every user plugin disabled.
    pub safe_mode: bool,
    /// Talk to the Builder agent, which may write plugins.
    pub builder: bool,
    pub workspace: WorkspaceChoice,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Help,
}

pub const USAGE: &str = "Usage: discard [--provider claude|codex] [--builder] [--worktree | --main-checkout] [--safe-mode]\n\n\
  --provider NAME   Which agent to talk to in this project (default: $DISCARD_PROVIDER or claude)\n\
  --builder         Use the Builder agent, which can write plugins for you to review\n\
  --worktree        Work in a git worktree for this channel (the default for new channels in a git repo)\n\
  --main-checkout   Work in the main checkout of the project. Changing an existing channel starts a new session.\n\
  --safe-mode       Start with all user plugins disabled\n\
  --help            Show this help\n";

/// Parse arguments (without the program name). `env_provider` is the value of `$DISCARD_PROVIDER`.
pub fn parse(args: &[String], env_provider: Option<&str>) -> Result<Parsed, String> {
    let valid = discard_core::bootstrap::PROVIDERS;
    let check = |name: &str| -> Result<String, String> {
        if valid.contains(&name) {
            Ok(name.to_string())
        } else {
            Err(format!("unknown provider `{name}`. Choose one of: {}", valid.join(", ")))
        }
    };
    let mut provider = match env_provider.filter(|p| !p.is_empty()) {
        Some(name) => check(name)?,
        None => "claude".to_string(),
    };
    let mut safe_mode = false;
    let mut builder = false;
    let mut workspace = WorkspaceChoice::Auto;
    let mut help = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => help = true,
            "--safe-mode" => safe_mode = true,
            "--builder" => builder = true,
            "--worktree" | "--main-checkout" => {
                let choice = if arg == "--worktree" { WorkspaceChoice::Worktree } else { WorkspaceChoice::Main };
                if workspace != WorkspaceChoice::Auto && workspace != choice {
                    return Err("--worktree and --main-checkout cannot be used together".to_string());
                }
                workspace = choice;
            }
            "--provider" => {
                let value = iter.next().ok_or("--provider needs a value")?;
                provider = check(value)?;
            }
            other if other.starts_with("--provider=") => provider = check(&other["--provider=".len()..])?,
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    if help {
        return Ok(Parsed::Help);
    }
    Ok(Parsed::Run(Options { provider, safe_mode, builder, workspace }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], env: Option<&str>) -> Result<Parsed, String> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse(&args, env)
    }

    fn opts(provider: &str, safe_mode: bool) -> Result<Parsed, String> {
        Ok(Parsed::Run(Options { provider: provider.into(), safe_mode, builder: false, workspace: WorkspaceChoice::Auto }))
    }

    #[test]
    fn defaults_to_claude_without_safe_mode() {
        assert_eq!(run(&[], None), opts("claude", false));
    }

    #[test]
    fn the_environment_sets_the_default_provider_and_a_flag_overrides_it() {
        assert_eq!(run(&[], Some("codex")), opts("codex", false));
        assert_eq!(run(&["--provider", "claude"], Some("codex")), opts("claude", false));
    }

    #[test]
    fn provider_accepts_both_spellings() {
        assert_eq!(run(&["--provider", "codex"], None), opts("codex", false));
        assert_eq!(run(&["--provider=codex"], None), opts("codex", false));
    }

    #[test]
    fn unknown_providers_are_rejected_with_the_valid_choices() {
        for bad in [run(&["--provider", "gemini"], None), run(&[], Some("gemini"))] {
            let err = bad.unwrap_err();
            assert!(err.contains("gemini") && err.contains("claude") && err.contains("codex"), "{err}");
        }
    }

    #[test]
    fn an_empty_environment_value_is_ignored() {
        assert_eq!(run(&[], Some("")), opts("claude", false));
    }

    #[test]
    fn safe_mode_is_a_flag() {
        assert_eq!(run(&["--safe-mode"], None), opts("claude", true));
        assert_eq!(run(&["--safe-mode", "--provider", "codex"], None), opts("codex", true));
    }

    #[test]
    fn builder_is_a_flag_that_combines_with_the_others() {
        let Ok(Parsed::Run(o)) = run(&["--builder", "--provider", "codex", "--safe-mode"], None) else { panic!() };
        assert_eq!((o.builder, o.provider.as_str(), o.safe_mode), (true, "codex", true));
        let Ok(Parsed::Run(o)) = run(&[], None) else { panic!() };
        assert!(!o.builder, "the builder is opt-in");
    }

    #[test]
    fn the_workspace_defaults_to_auto_and_flags_choose_explicitly() {
        let ws = |args: &[&str]| match run(args, None) {
            Ok(Parsed::Run(o)) => o.workspace,
            other => panic!("{other:?}"),
        };
        assert_eq!(ws(&[]), WorkspaceChoice::Auto);
        assert_eq!(ws(&["--worktree"]), WorkspaceChoice::Worktree);
        assert_eq!(ws(&["--main-checkout"]), WorkspaceChoice::Main);
        assert_eq!(ws(&["--worktree", "--worktree"]), WorkspaceChoice::Worktree);
    }

    #[test]
    fn worktree_and_main_checkout_conflict() {
        let err = run(&["--worktree", "--main-checkout"], None).unwrap_err();
        assert!(err.contains("--worktree") && err.contains("--main-checkout"), "{err}");
        assert!(run(&["--main-checkout", "--worktree"], None).is_err());
    }

    #[test]
    fn a_missing_value_and_unknown_flags_are_errors() {
        assert!(run(&["--provider"], None).unwrap_err().contains("--provider"));
        assert!(run(&["--bogus"], None).unwrap_err().contains("--bogus"));
        assert!(run(&["stray"], None).unwrap_err().contains("stray"));
    }

    #[test]
    fn help_wins() {
        assert_eq!(run(&["--help"], None), Ok(Parsed::Help));
        assert_eq!(run(&["--provider", "codex", "--help"], None), Ok(Parsed::Help));
    }
}
