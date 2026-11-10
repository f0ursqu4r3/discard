use discard_core::worktree::{discover_repositories, list_branches, repository_branch_key};
use std::{path::Path, process::Command};

fn git(path: &Path, args: &[&str]) {
    let output = discard_core::process::spawn_std(Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
        .args(args).current_dir(path).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

fn repo(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["commit", "--allow-empty", "-qm", "initial"]);
}

#[test]
fn discovers_root_and_nested_repositories_but_skips_generated_directories() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    repo(&root);
    repo(&root.join("apps/frontend"));
    repo(&root.join("apps/frontend/nested"));
    repo(&root.join("services/backend"));
    repo(&root.join("node_modules/ignored"));
    repo(&root.join("target/ignored"));
    let repositories = discover_repositories(&root).unwrap();
    assert_eq!(repositories.len(), 4);
    assert_eq!(repositories[0].id, "");
    assert_eq!(repositories[0].path, root);
    assert_eq!(repositories[1].label, "frontend");
    assert_eq!(repositories[2].label, "nested");
    assert_eq!(repositories[3].label, "backend");
}

#[test]
fn deduplicates_linked_worktrees_and_lists_external_worktrees() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let main = root.join("z-repo");
    repo(&main);
    let linked = root.join("a-linked");
    git(&main, &["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    let external = dir.path().join("external");
    git(&main, &["worktree", "add", "-b", "external", external.to_str().unwrap()]);
    let repositories = discover_repositories(&root).unwrap();
    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0].path, main.canonicalize().unwrap());
    let branches = list_branches(&repositories[0].path).unwrap();
    assert_eq!(branches.len(), 3);
    assert!(branches.iter().any(|branch| branch.worktree.as_ref() == Some(&external.canonicalize().unwrap())));
}

#[test]
fn distinguishes_duplicate_names_and_does_not_discover_an_ancestor_repo() {
    let dir = tempfile::tempdir().unwrap();
    repo(dir.path());
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    assert!(discover_repositories(&project).unwrap().is_empty());
    repo(&project.join("apps/shared"));
    repo(&project.join("packages/shared"));
    let repositories = discover_repositories(&project).unwrap();
    assert_eq!(repositories.iter().map(|repo| repo.label.as_str()).collect::<Vec<_>>(), ["apps/shared", "packages/shared"]);
    assert_ne!(repository_branch_key(&repositories[0].id, "main"), repository_branch_key(&repositories[1].id, "main"));
    assert_eq!(repository_branch_key("", "main"), "main");
}

#[cfg(unix)]
#[test]
fn does_not_follow_directory_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    repo(external.path());
    std::os::unix::fs::symlink(external.path(), dir.path().join("external")).unwrap();
    std::os::unix::fs::symlink(dir.path(), dir.path().join("cycle")).unwrap();
    assert!(discover_repositories(dir.path()).unwrap().is_empty());
}
