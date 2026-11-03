//! The plugin problem lines. The main window counts them and the Plugins page lists them.

use discard_core::ghostty::resolve::ThemeProblem;
use discard_lua::service::PluginStatus;
use discard_lua::{LoadReport, PluginState};
use std::collections::BTreeMap;

/// One line for each theme problem. A problem in a file names the file and the line.
pub fn theme_problem_lines(problems: &[ThemeProblem]) -> Vec<String> {
    problems
        .iter()
        .map(|p| match (&p.file, p.line) {
            (Some(file), Some(line)) => format!("{}:{line}: {}", file.display(), p.message),
            (Some(file), None) => format!("{}: {}", file.display(), p.message),
            (None, _) => p.message.clone(),
        })
        .collect()
}

/// One line for each plugin problem. An empty list means no problem.
pub fn problem_lines(
    errors: &BTreeMap<String, String>,
    statuses: &[PluginStatus],
    load_report: &LoadReport,
) -> Vec<String> {
    let mut lines: Vec<String> = errors
        .iter()
        .map(|(name, msg)| format!("{name}: {}", msg.lines().next().unwrap_or("")))
        .collect();
    for status in statuses {
        match &status.state {
            PluginState::ModifiedOutsideReview => lines.push(format!(
                "{}: disabled because its files changed outside review",
                status.name
            )),
            PluginState::Failed(reason) if !errors.contains_key(&status.name) => {
                lines.push(format!(
                    "{}: {}",
                    status.name,
                    reason.lines().next().unwrap_or("")
                ));
            }
            _ => {}
        }
    }
    if let Some(problem) = &load_report.key_problem {
        lines.push(format!("user plugins are off: {problem}"));
    }
    if load_report.safe_mode {
        lines.push("safe mode: user plugins are disabled".to_string());
    }
    lines
}

/// How many problems the main window counts: one for each problem line, and one for a notice.
pub fn problem_count(lines: usize, has_notice: bool) -> usize {
    lines + usize::from(has_notice)
}

/// The one line the main window shows: "N plugin problems".
pub fn problem_summary(count: usize) -> String {
    match count {
        1 => "1 plugin problem".to_string(),
        n => format!("{n} plugin problems"),
    }
}

#[cfg(test)]
mod tests {
    use super::{problem_count, problem_lines, problem_summary};
    use discard_lua::service::PluginStatus;
    use discard_lua::{LoadReport, PluginState};
    use std::collections::BTreeMap;

    fn status(name: &str, state: PluginState) -> PluginStatus {
        PluginStatus {
            name: name.to_string(),
            state,
            trusted_edits: false,
        }
    }

    #[test]
    fn no_problems_gives_no_lines() {
        let lines = problem_lines(&BTreeMap::new(), &[], &LoadReport::default());
        assert!(lines.is_empty());
    }

    #[test]
    fn safe_mode_adds_a_line() {
        let report = LoadReport {
            safe_mode: true,
            ..LoadReport::default()
        };
        assert_eq!(
            problem_lines(&BTreeMap::new(), &[], &report),
            vec!["safe mode: user plugins are disabled".to_string()]
        );
    }

    #[test]
    fn key_problem_adds_a_line() {
        let report = LoadReport {
            key_problem: Some("no signing key".to_string()),
            ..LoadReport::default()
        };
        assert_eq!(
            problem_lines(&BTreeMap::new(), &[], &report),
            vec!["user plugins are off: no signing key".to_string()]
        );
    }

    #[test]
    fn modified_outside_review_adds_a_line() {
        let statuses = [status("clock", PluginState::ModifiedOutsideReview)];
        assert_eq!(
            problem_lines(&BTreeMap::new(), &statuses, &LoadReport::default()),
            vec!["clock: disabled because its files changed outside review".to_string()]
        );
    }

    #[test]
    fn failed_status_is_not_listed_twice_when_it_is_also_in_errors() {
        let mut errors = BTreeMap::new();
        errors.insert("clock".to_string(), "boom\nsecond line".to_string());
        let statuses = [
            status("clock", PluginState::Failed("boom".to_string())),
            status("other", PluginState::Failed("bad\nmore".to_string())),
        ];
        assert_eq!(
            problem_lines(&errors, &statuses, &LoadReport::default()),
            vec!["clock: boom".to_string(), "other: bad".to_string()]
        );
    }

    #[test]
    fn a_notice_counts_as_one_problem() {
        assert_eq!(problem_count(0, true), 1);
        assert_eq!(problem_count(2, true), 3);
    }

    #[test]
    fn no_lines_and_no_notice_is_zero_problems() {
        assert_eq!(problem_count(0, false), 0);
        assert_eq!(problem_count(2, false), 2);
    }

    #[test]
    fn the_summary_uses_the_singular_for_one() {
        assert_eq!(problem_summary(1), "1 plugin problem");
        assert_eq!(problem_summary(3), "3 plugin problems");
    }
}

#[cfg(test)]
mod theme_tests {
    use super::*;
    use std::path::PathBuf;

    fn problem(file: Option<&str>, line: Option<usize>, message: &str) -> ThemeProblem {
        ThemeProblem { file: file.map(PathBuf::from), line, message: message.to_string() }
    }

    #[test]
    fn a_file_problem_names_the_file_and_line() {
        let lines = theme_problem_lines(&[problem(Some("/t/Mine"), Some(3), "`foreground`: `x` is not a color")]);
        assert_eq!(lines, vec!["/t/Mine:3: `foreground`: `x` is not a color"]);
    }

    #[test]
    fn a_file_problem_without_a_line_names_only_the_file() {
        let lines = theme_problem_lines(&[problem(Some("/t/Mine"), None, "cannot read")]);
        assert_eq!(lines, vec!["/t/Mine: cannot read"]);
    }

    #[test]
    fn a_problem_with_no_file_is_just_the_message() {
        let lines = theme_problem_lines(&[problem(None, None, "theme `Nope` was not found")]);
        assert_eq!(lines, vec!["theme `Nope` was not found"]);
    }

    #[test]
    fn no_problems_gives_no_lines() {
        assert!(theme_problem_lines(&[]).is_empty());
    }
}
