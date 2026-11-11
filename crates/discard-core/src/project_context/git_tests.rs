use super::*;

fn git(root: &Path, args: &[&str]) {
    let output = crate::process::spawn_std(
        std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    )
    .and_then(|child| child.wait_with_output())
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn fixture() -> (tempfile::TempDir, ProjectContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    fs::write(root.join("first.txt"), "First\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "initial"]);
    let context = ProjectContext {
        name: "test".into(),
        channel: "test".into(),
        root,
        checkout: "test",
    };
    (dir, context)
}

#[test]
fn branches_report_merge_counts_and_dirty_linked_worktrees() {
    let (dir, context) = fixture();
    git(&context.root, &["branch", "merged"]);
    let linked = dir.path().join("linked");
    git(
        &context.root,
        &[
            "worktree",
            "add",
            "-qb",
            "feature",
            linked.to_str().unwrap(),
        ],
    );
    fs::write(linked.join("feature.txt"), "new\n").unwrap();
    git(&linked, &["add", "."]);
    git(&linked, &["commit", "-qm", "feature change"]);
    fs::write(linked.join("dirty.txt"), "dirty").unwrap();
    git(
        &context.root,
        &["commit", "--allow-empty", "-qm", "main change"],
    );
    let result = context.git_branches().unwrap();
    assert_eq!(result["default_branch"], "main");
    assert_eq!(result["current_branch"], "main");
    let branches = result["branches"].as_array().unwrap();
    let branch = |name: &str| branches.iter().find(|b| b["name"] == name).unwrap();
    assert_eq!(branch("merged")["merged_into_default"], true);
    assert_eq!(branch("feature")["merged_into_default"], false);
    assert_eq!(
        (
            branch("feature")["ahead"].as_u64(),
            branch("feature")["behind"].as_u64()
        ),
        (Some(1), Some(1))
    );
    assert_eq!(branch("feature")["worktree"]["path"], "linked");
    assert_eq!(branch("feature")["worktree"]["dirty_files"], 1);
    assert_eq!(branch("feature")["worktree"]["is_current"], false);
    assert_eq!(branch("main")["worktree"]["is_current"], true);
    assert!(!result.to_string().contains(dir.path().to_str().unwrap()));
    assert!(result["note"].as_str().unwrap().contains("untrusted"));
    let linked_context = ProjectContext {
        root: linked,
        ..context
    };
    assert_eq!(
        linked_context.git_branches().unwrap()["current_branch"],
        "feature"
    );
}

#[test]
fn log_limits_and_literal_paths_and_compare_three_dot() {
    let (_dir, context) = fixture();
    git(&context.root, &["branch", "base"]);
    fs::write(context.root.join("second.txt"), "second\n").unwrap();
    git(&context.root, &["add", "."]);
    git(&context.root, &["commit", "-qm", "second"]);
    git(
        &context.root,
        &[
            "commit",
            "--allow-empty",
            "-qm",
            "third",
            "-m",
            "BODY MUST NOT APPEAR",
        ],
    );
    let log = context.git_log("HEAD", None, 2).unwrap();
    assert_eq!(log["commits"].as_array().unwrap().len(), 2);
    assert_eq!(log["commits"][0]["subject"], "third");
    assert!(!log.to_string().contains("BODY MUST NOT APPEAR"));
    let log = context.git_log("HEAD", Some("first.txt"), 30).unwrap();
    assert_eq!(log["commits"].as_array().unwrap().len(), 1);
    assert_eq!(log["commits"][0]["subject"], "initial");
    let summary = context.git_compare("base", "HEAD", "summary").unwrap();
    assert_eq!(summary["ahead"], 2);
    assert_eq!(summary["behind"], 0);
    assert_eq!(summary["head_merged_into_base"], false);
    assert!(summary["merge_base"].is_string());
    assert_eq!(
        summary["files"][0],
        json!({"path":"second.txt", "status":"A", "insertions":1, "deletions":0})
    );
    let reverse = context.git_compare("HEAD", "base", "summary").unwrap();
    assert_eq!(reverse["head_merged_into_base"], true);
    assert_eq!(reverse["files"], json!([]));
    assert!(
        context.git_compare("base", "HEAD", "diff").unwrap()["diff"]
            .as_str()
            .unwrap()
            .contains("+second")
    );
}

#[test]
fn git_results_are_bounded_and_cut_unicode_safely() {
    let (_dir, context) = fixture();
    git(&context.root, &["branch", "base"]);
    fs::write(context.root.join("large.txt"), "🦀\\\"\n".repeat(4000)).unwrap();
    git(&context.root, &["add", "."]);
    git(&context.root, &["commit", "-qm", &"🦀".repeat(4000)]);
    let diff = context.git_compare("base", "HEAD", "diff").unwrap();
    assert_eq!(diff["truncated"], true);
    assert!(diff.to_string().len() <= RESULT_LIMIT);
    assert!(!diff["diff"].as_str().unwrap().contains('\u{fffd}'));
    for result in [
        context.git_log("HEAD", None, 10).unwrap(),
        context.git_branches().unwrap(),
    ] {
        assert!(result.to_string().len() <= RESULT_LIMIT);
    }
}

#[test]
fn git_ref_and_path_errors_do_not_expose_paths() {
    let (dir, context) = fixture();
    for name in [
        "-h",
        "--output=/tmp/unsafe",
        "HEAD\n",
        "HEAD\0",
        "",
        "missing",
        &"x".repeat(201),
    ] {
        for error in [
            context.git_log(name, None, 10).unwrap_err(),
            context.git_compare(name, "HEAD", "diff").unwrap_err(),
            context.git_compare("HEAD", name, "summary").unwrap_err(),
        ] {
            assert!(!error.contains(dir.path().to_str().unwrap()));
        }
    }
    assert_eq!(
        context.git_log("missing", None, 10).unwrap_err(),
        "Unknown ref."
    );
    fs::write(context.root.join(".gitignore"), "private.txt\n").unwrap();
    fs::write(context.root.join("private.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(dir.path(), context.root.join("escape")).unwrap();
    for path in [
        "../outside",
        "/etc/passwd",
        "escape",
        ".git/config",
        "private.txt",
    ] {
        assert!(context.git_log("HEAD", Some(path), 10).is_err(), "{path}");
    }
    for limit in [0, 31] {
        assert!(context.git_log("HEAD", None, limit).is_err());
    }
    assert!(context.git_compare("HEAD", "HEAD", "other").is_err());
    let plain = ProjectContext {
        root: dir.path().to_path_buf(),
        ..context
    };
    for result in [
        plain.git_branches(),
        plain.git_log("HEAD", None, 10),
        plain.git_compare("HEAD", "HEAD", "diff"),
    ] {
        assert_eq!(result.unwrap_err(), "This project is not a Git repository.");
    }
}

#[test]
fn search_options_preserve_literal_default_and_report_active_mode() {
    let (_dir, context) = fixture();
    fs::write(
        context.root.join("search.txt"),
        "Needle 42\nneedle [abc]\nNEEDLE 77\n",
    )
    .unwrap();
    for (query, ignore_case, regex, count) in [
        ("needle", false, false, 1),
        ("needle", true, false, 3),
        ("needle [0-9]+", false, true, 0),
        ("needle [0-9]+", true, true, 2),
        ("Needle [0-9]+", false, true, 1),
        ("[abc]", false, false, 1),
    ] {
        let result = context
            .search_options("", query, ignore_case, regex)
            .unwrap();
        assert_eq!(
            result["matches"].as_array().unwrap().len(),
            count,
            "{result}"
        );
        assert!(result["note"].as_str().unwrap().contains(if ignore_case {
            "case-insensitive"
        } else {
            "case-sensitive"
        }));
    }
    assert_eq!(
        context.search_options("", "[", false, true).unwrap_err(),
        "Invalid regular expression."
    );
}

#[test]
fn default_branch_uses_local_origin_then_main_master_and_main_checkout() {
    let (_dir, context) = fixture();
    git(&context.root, &["branch", "stable"]);
    git(
        &context.root,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/stable",
        ],
    );
    assert_eq!(context.git_branches().unwrap()["default_branch"], "stable");
    git(&context.root, &["branch", "-D", "stable"]);
    assert_eq!(context.git_branches().unwrap()["default_branch"], "main");
    git(&context.root, &["branch", "-m", "master"]);
    assert_eq!(context.git_branches().unwrap()["default_branch"], "master");
    git(&context.root, &["branch", "-m", "trunk"]);
    assert_eq!(context.git_branches().unwrap()["default_branch"], "trunk");
    git(&context.root, &["checkout", "--detach"]);
    let result = context.git_branches().unwrap();
    assert!(result["current_branch"].is_null());
    assert!(result["default_branch"].is_null());
    assert_eq!(result["branches"][0]["merged_into_default"], true);
}

#[test]
fn branch_and_file_lists_obey_item_and_byte_limits() {
    let (_dir, context) = fixture();
    for index in 0..65 {
        git(&context.root, &["branch", &format!("branch-{index:02}")]);
    }
    let branches = context.git_branches().unwrap();
    assert!(branches["branches"].as_array().unwrap().len() <= 60);
    assert_eq!(branches["truncated"], true);
    assert!(branches.to_string().len() <= RESULT_LIMIT);
    git(&context.root, &["branch", "base"]);
    for index in 0..110 {
        fs::write(context.root.join(format!("f{index:03}")), "new\n").unwrap();
    }
    git(&context.root, &["add", "."]);
    git(&context.root, &["commit", "-qm", "many files"]);
    let summary = context.git_compare("base", "HEAD", "summary").unwrap();
    assert!(summary["files"].as_array().unwrap().len() <= 100);
    assert_eq!(summary["truncated"], true);
    assert!(summary.to_string().len() <= RESULT_LIMIT);
}

#[test]
fn git_output_disables_external_helpers_and_preserves_unusual_file_names() {
    let (_dir, context) = fixture();
    git(&context.root, &["branch", "base"]);
    fs::write(context.root.join("line\nbreak\tfile.txt"), "new\n").unwrap();
    fs::write(context.root.join(".gitattributes"), "*.txt diff=custom\n").unwrap();
    git(&context.root, &["add", "."]);
    git(&context.root, &["commit", "-qm", "unusual paths"]);
    git(&context.root, &["config", "diff.external", "false"]);
    git(&context.root, &["config", "diff.custom.textconv", "false"]);
    let result = context.git_compare("base", "HEAD", "summary").unwrap();
    assert!(
        result["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "line\nbreak\tfile.txt")
    );
    assert!(
        context.git_compare("base", "HEAD", "diff").unwrap()["diff"]
            .as_str()
            .unwrap()
            .contains("+new")
    );
    assert_eq!(
        context
            .git_log("HEAD", Some("line\nbreak\tfile.txt"), 10)
            .unwrap()["commits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unrelated_histories_report_no_merge_base_and_no_three_dot_diff() {
    let (_dir, context) = fixture();
    git(&context.root, &["checkout", "--orphan", "unrelated"]);
    git(&context.root, &["commit", "-qm", "another root"]);
    for view in ["summary", "diff"] {
        let result = context.git_compare("main", "HEAD", view).unwrap();
        assert!(result["merge_base"].is_null());
        assert_eq!(result["ahead"], 1);
        assert_eq!(result["behind"], 1);
        assert_eq!(result["head_merged_into_base"], false);
        assert!(result["note"].as_str().unwrap().contains("No merge base"));
    }
}
