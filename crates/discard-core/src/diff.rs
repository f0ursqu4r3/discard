//! Unified diff text: tell a diff from other text, and tell what each line of one is.

/// What a line of a unified diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// `+text`.
    Added,
    /// `-text`.
    Removed,
    /// `@@ -1,2 +1,2 @@`.
    Hunk,
    /// Everything else: context lines, `---` and `+++` file headers, blank lines.
    Context,
}

/// The kind of one diff line. The `---` and `+++` file headers count as context, so they
/// do not look like a removed and an added line.
pub fn classify_line(line: &str) -> LineKind {
    if line.starts_with("+++") || line.starts_with("---") {
        LineKind::Context
    } else if line.starts_with('+') {
        LineKind::Added
    } else if line.starts_with('-') {
        LineKind::Removed
    } else if line.starts_with("@@") {
        LineKind::Hunk
    } else {
        LineKind::Context
    }
}

/// True when `text` has a hunk header (`@@ ... @@`), or a `--- ` line right before a `+++ ` line.
/// A plain list that starts its lines with `-` is not a diff.
pub fn looks_like_unified_diff(text: &str) -> bool {
    let mut after_old_header = false;
    for line in text.lines() {
        let is_hunk = line
            .strip_prefix("@@ ")
            .is_some_and(|rest| rest.contains(" @@"));
        if is_hunk || (after_old_header && line.starts_with("+++ ")) {
            return true;
        }
        after_old_header = line.starts_with("--- ");
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_removed_and_hunk_lines_are_told_apart() {
        assert_eq!(classify_line("+added"), LineKind::Added);
        assert_eq!(classify_line("-removed"), LineKind::Removed);
        assert_eq!(classify_line("@@ -1 +1 @@"), LineKind::Hunk);
        assert_eq!(classify_line("+"), LineKind::Added);
        assert_eq!(classify_line("-"), LineKind::Removed);
    }

    #[test]
    fn file_headers_blank_and_plain_lines_are_context() {
        for line in ["+++ b/file", "--- a/file", "", " context", "context"] {
            assert_eq!(classify_line(line), LineKind::Context, "line {line:?}");
        }
    }

    #[test]
    fn a_hunk_header_makes_a_diff() {
        assert!(looks_like_unified_diff("@@ -1 +1 @@\n-old\n+new"));
        assert!(looks_like_unified_diff("text first\n@@ -3,2 +3,2 @@ fn x()\n"));
    }

    #[test]
    fn old_and_new_file_headers_next_to_each_other_make_a_diff() {
        assert!(looks_like_unified_diff("--- a/file\n+++ b/file\n"));
    }

    #[test]
    fn bullets_and_split_headers_are_not_a_diff() {
        assert!(!looks_like_unified_diff("- item\n- another"));
        assert!(!looks_like_unified_diff("--- a/file\ncontext\n+++ b/file"));
        assert!(!looks_like_unified_diff("@@ not a hunk"));
        assert!(!looks_like_unified_diff("+++ b/file\n--- a/file"));
        assert!(!looks_like_unified_diff(""));
    }
}
