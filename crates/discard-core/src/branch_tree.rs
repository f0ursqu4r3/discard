use crate::worktree::BranchEntry;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind { Main, Worktree, Branch }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub name: String,
    pub kind: RowKind,
    pub worktree: Option<PathBuf>,
    pub detached: bool,
    pub is_discard: bool,
    pub parent: Option<String>,
}

impl TreeRow {
    pub fn depth(&self) -> u8 { u8::from(self.parent.is_some()) }

    /// Detached checkouts at the same commit need distinct, path-scoped keys.
    pub fn selection_key(&self, repository: &str) -> String {
        if self.detached {
            serde_json::to_string(&(repository, &self.name, &self.worktree)).expect("worktree key serializes")
        } else {
            crate::worktree::repository_branch_key(repository, &self.name)
        }
    }
}

pub fn build_tree(entries: &[BranchEntry], distance: &dyn Fn(&str, &str) -> Option<usize>) -> Vec<TreeRow> {
    let mut parents: Vec<_> = entries.iter().filter(|e| e.worktree.is_some()).collect();
    parents.sort_by(|a, b| b.is_main_checkout.cmp(&a.is_main_checkout).then_with(|| a.branch.cmp(&b.branch)));
    let main = parents.iter().find(|e| e.is_main_checkout).map(|e| e.branch.as_str());
    let mut children: Vec<_> = entries.iter().filter(|e| e.worktree.is_none()).map(|entry| {
        let parent = parents.iter().filter_map(|p| distance(&p.branch, &entry.branch).map(|d| (d, !p.is_main_checkout, p.branch.as_str())))
            .min().map(|(_, _, name)| name).or(main).map(str::to_owned);
        TreeRow { name: entry.branch.clone(), kind: RowKind::Branch, worktree: None, detached: entry.detached, is_discard: entry.is_discard, parent }
    }).collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));
    let mut rows = Vec::new();
    for entry in parents {
        rows.push(TreeRow { name: entry.branch.clone(), kind: if entry.is_main_checkout { RowKind::Main } else { RowKind::Worktree }, worktree: entry.worktree.clone(), detached: entry.detached, is_discard: entry.is_discard, parent: None });
        rows.extend(children.iter().filter(|child| child.parent.as_deref() == Some(&entry.branch)).cloned());
        // Detached worktrees can share a label; emit each child under only the first match.
        children.retain(|child| child.parent.as_deref() != Some(&entry.branch));
    }
    rows.extend(children.into_iter().filter(|child| child.parent.is_none()));
    rows
}

pub fn visible_rows<'a>(rows: &'a [TreeRow], collapsed: &HashSet<String>) -> Vec<&'a TreeRow> {
    rows.iter().filter(|row| row.parent.as_ref().is_none_or(|parent| !collapsed.contains(parent))).collect()
}

pub fn current_row(rows: &[TreeRow], cwd: &Path) -> Option<usize> {
    rows.iter().enumerate().filter_map(|(i, row)| row.worktree.as_ref().filter(|path| row.parent.is_none() && cwd.starts_with(path)).map(|path| (i, path.components().count())))
        .max_by_key(|(_, length)| *length).map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::BranchEntry;
    use std::collections::HashSet;
    use std::path::Path;

    fn entry(name: &str, path: Option<&str>, main: bool) -> BranchEntry {
        BranchEntry { branch: name.into(), worktree: path.map(Into::into), is_main_checkout: main, detached: false, is_discard: false }
    }

    #[test]
    fn main_row_comes_first_then_worktree_rows_each_followed_by_their_branches() {
        let entries = [entry("z", Some("/z"), false), entry("b", None, false), entry("main", Some("/repo"), true), entry("a", None, false), entry("w", Some("/w"), false)];
        let rows = build_tree(&entries, &|w, b| match (w, b) { ("w", "b") => Some(1), ("main", "a") => Some(1), _ => None });
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["main", "a", "w", "b", "z"]);
    }

    #[test]
    fn a_branch_nests_under_the_closest_worktree_branch() {
        let entries = [entry("main", Some("/repo"), true), entry("w1", Some("/1"), false), entry("w2", Some("/2"), false), entry("b", None, false)];
        let rows = build_tree(&entries, &|w, _| match w { "w1" => Some(3), "w2" => Some(1), _ => None });
        assert_eq!(rows.iter().find(|r| r.name == "b").unwrap().parent.as_deref(), Some("w2"));
    }

    #[test]
    fn a_branch_with_no_ancestor_worktree_goes_under_main() {
        let rows = build_tree(&[entry("main", Some("/repo"), true), entry("b", None, false)], &|_, _| None);
        assert_eq!(rows[1].parent.as_deref(), Some("main"));
    }

    #[test]
    fn ties_go_to_main_then_alphabetical() {
        let mut entries = vec![entry("main", Some("/repo"), true), entry("w2", Some("/2"), false), entry("w1", Some("/1"), false), entry("a", None, false), entry("b", None, false)];
        let distance = |w: &str, b: &str| if b == "a" || w != "main" { Some(1) } else { None };
        let rows = build_tree(&entries, &distance);
        entries.reverse();
        assert_eq!(rows, build_tree(&entries, &distance));
        assert_eq!(rows.iter().find(|r| r.name == "a").unwrap().parent.as_deref(), Some("main"));
        assert_eq!(rows.iter().find(|r| r.name == "b").unwrap().parent.as_deref(), Some("w1"));
    }

    #[test]
    fn a_branch_with_a_worktree_is_never_a_child() {
        let rows = build_tree(&[entry("main", Some("/repo"), true), entry("w", Some("/w"), false)], &|_, _| Some(0));
        assert!(rows.iter().all(|r| r.parent.is_none() && r.depth() == 0));
    }

    #[test]
    fn without_a_main_row_branches_have_no_parent() {
        let rows = build_tree(&[entry("b", None, false)], &|_, _| None);
        assert_eq!(rows[0].parent, None);
    }

    #[test]
    fn a_detached_worktree_row_keeps_its_short_sha_name_and_flag() {
        let mut e = entry("abcdef1", Some("/d"), false); e.detached = true;
        let rows = build_tree(&[e], &|_, _| None);
        assert_eq!(rows[0].name, "abcdef1");
        assert!(rows[0].detached);
        assert_eq!(rows[0].kind, RowKind::Worktree);
    }

    #[test]
    fn detached_worktrees_at_the_same_commit_keep_distinct_keys_and_do_not_repeat_children() {
        let mut first = entry("abcdef1", Some("/first"), false);
        first.detached = true;
        let mut second = first.clone();
        second.worktree = Some("/second".into());
        let rows = build_tree(&[first, second, entry("topic", None, false)], &|_, _| Some(0));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().filter(|row| row.name == "topic").count(), 1);
        assert_ne!(rows[0].selection_key(""), rows[2].selection_key(""));
    }

    #[test]
    fn visible_rows_hides_children_of_collapsed_parents() {
        let rows = build_tree(&[entry("main", Some("/repo"), true), entry("b", None, false)], &|_, _| None);
        assert_eq!(visible_rows(&rows, &HashSet::from(["main".into()])).len(), 1);
        assert_eq!(visible_rows(&rows, &HashSet::new()).len(), 2);
    }

    #[test]
    fn current_row_picks_the_longest_worktree_prefix() {
        let rows = build_tree(&[entry("main", Some("/repo"), true), entry("x", Some("/repo/.wt/x"), false)], &|_, _| None);
        assert_eq!(rows[current_row(&rows, Path::new("/repo/.wt/x/sub")).unwrap()].name, "x");
    }

    #[test]
    fn current_row_is_none_outside_every_worktree() {
        let rows = build_tree(&[entry("main", Some("/repo"), true)], &|_, _| None);
        assert_eq!(current_row(&rows, Path::new("/repository")), None);
    }
}
