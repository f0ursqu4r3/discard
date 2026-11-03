//! Relaunching the app, for example when Safe Mode changes.

use std::path::Path;

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

/// The `sh` script for the waiter. It waits while the parent PID (`$1`) lives, then runs the rest of its arguments.
/// The executable and its arguments come in as positional arguments, so the script text never contains them.
pub const WAIT_SCRIPT: &str =
    r#"while kill -0 "$1" 2>/dev/null; do sleep 0.1; done; shift; exec "$@""#;

/// The program and arguments for a waiter that starts `exe` again after `parent_pid` exits.
///
/// GPUI on macOS ends the process with `exit()` inside `[NSApp terminate:]`, so code after the event loop never runs.
/// The waiter starts before the quit. It waits so the new process never shares the store, the memory runtime
/// socket, or the provider processes with this one.
pub fn wait_command(
    exe: &Path,
    argv: &[String],
    safe_mode: bool,
    parent_pid: u32,
) -> (String, Vec<String>) {
    let mut out = vec![
        "-c".to_string(),
        WAIT_SCRIPT.to_string(),
        "sh".to_string(),
        parent_pid.to_string(),
        exe.to_string_lossy().into_owned(),
    ];
    out.extend(args(argv, safe_mode));
    ("/bin/sh".to_string(), out)
}

/// Start a detached process that runs Discard again with `argv` after this process exits.
/// Unix only waits for the exit. Windows is out of scope for v1: there it starts the executable at once.
pub fn spawn(argv: &[String], safe_mode: bool) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe()?;
    #[cfg(unix)]
    let mut command = {
        use std::os::unix::process::CommandExt;
        let (program, wait_args) = wait_command(&exe, argv, safe_mode, std::process::id());
        let mut command = Command::new(program);
        // A new process group, so a signal to this app's group does not reach the waiter.
        command.args(wait_args).process_group(0);
        command
    };
    #[cfg(not(unix))]
    let mut command = {
        let mut command = Command::new(exe);
        command.args(args(argv, safe_mode));
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
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

    #[test]
    fn wait_command_runs_sh_with_the_fixed_script_and_the_parent_pid() {
        let (program, out) = wait_command(Path::new("/bin/discard"), &[], false, 4242);
        assert_eq!(program, "/bin/sh");
        assert_eq!(
            out,
            strings(&["-c", WAIT_SCRIPT, "sh", "4242", "/bin/discard"])
        );
    }

    #[test]
    fn wait_command_adds_or_removes_safe_mode_once_and_keeps_provider() {
        let argv = strings(&["--safe-mode", "--provider", "codex", "--safe-mode"]);
        let (_, on) = wait_command(Path::new("/bin/discard"), &argv, true, 1);
        assert_eq!(
            &on[5..],
            strings(&["--provider", "codex", "--safe-mode"]).as_slice()
        );
        let (_, off) = wait_command(Path::new("/bin/discard"), &argv, false, 1);
        assert_eq!(&off[5..], strings(&["--provider", "codex"]).as_slice());
        assert_eq!(on.iter().filter(|a| *a == "--safe-mode").count(), 1);
        assert_eq!(off.iter().filter(|a| *a == "--safe-mode").count(), 0);
    }

    #[test]
    fn spaces_and_quotes_stay_single_arguments_outside_the_script() {
        let exe = r#"/Applications/My "Odd" App's/discard"#;
        let arg = r#"--provider=co dex"; rm -rf ~ '"#;
        let (_, out) = wait_command(Path::new(exe), &strings(&[arg]), false, 7);
        assert_eq!(out[4], exe);
        assert_eq!(out[5], arg);
        assert_eq!(out.len(), 6);
        assert_eq!(out[1], WAIT_SCRIPT);
        assert!(!out[1].contains("Odd") && !out[1].contains("co dex"));
    }

    #[test]
    #[cfg(unix)]
    fn the_script_waits_for_the_pid_then_execs_the_arguments_verbatim() {
        // A PID that has exited, so the loop ends at once. The child then prints its arguments one per line.
        let mut done = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = done.id();
        done.wait().unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", WAIT_SCRIPT, "sh", &pid.to_string()])
            .args(["/usr/bin/printf", "%s\\n", "a b", r#"c"d'e"#])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "a b\nc\"d'e\n");
    }
}
