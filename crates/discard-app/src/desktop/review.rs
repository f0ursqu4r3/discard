//! Merge review commands, as plain functions over the store, the folder, and a send closure.
//! The Tauri commands in `commands.rs` only resolve the folder and the send path.

use discard_core::{
    merge_review::{snapshot, ReviewSnapshot},
    model::ChannelId,
    review_state::{review_state, Decision, ReviewDecision, ReviewState, NOTE_LIMIT},
    store::Store,
};
use serde::Serialize;
use std::path::Path;

pub const ARCHIVED: &str = "This channel is archived.";
pub const STALE: &str = "The branch changed. Reload the review.";
pub const SUMMARY_REQUEST: &str = "Summarize this branch against its target branch. Use project_git_compare and project_git_log, and read the changed files where you need to. Describe what the branch changes, and note anything that looks risky.";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDto {
    pub snapshot: ReviewSnapshot,
    pub state: ReviewState,
    /// Newest first.
    pub decisions: Vec<ReviewDecision>,
    /// The busy reason or the archived text. Actions are disabled when it is set.
    pub blocked: Option<String>,
    /// The id of a changes-requested row that never reached the conversation.
    pub unposted: Option<String>,
}

#[derive(Debug)]
/// The result of an action that can post. `post_error` is set when the row exists but the post failed.
pub struct Outcome {
    pub dto: ReviewDto,
    pub post_error: Option<String>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn archived(store: &Store, channel: &ChannelId) -> Result<bool, String> {
    store.channel_archived(channel).map_err(|e| e.to_string())
}

fn short(head: &str) -> String {
    head.chars().take(7).collect()
}

pub fn review(store: &Store, channel: &ChannelId, folder: &Path, busy: Option<&str>) -> Result<ReviewDto, String> {
    let rows = store.review_decisions(channel).map_err(|e| e.to_string())?;
    let branch = store.channel_branch(channel).map_err(|e| e.to_string())?;
    let snapshot = snapshot(folder, branch.as_deref(), rows.first().map(|row| row.head.as_str()));
    let state = review_state(&snapshot, rows.first());
    let blocked = if archived(store, channel)? { Some(ARCHIVED.to_owned()) } else { busy.map(str::to_owned) };
    let unposted = rows.iter().find(|row| row.decision == Decision::ChangesRequested && row.posted_at.is_none()).map(|row| row.id.clone());
    Ok(ReviewDto { snapshot, state, decisions: rows, blocked, unposted })
}

fn refuse_archived(store: &Store, channel: &ChannelId) -> Result<(), String> {
    if archived(store, channel)? { Err(ARCHIVED.into()) } else { Ok(()) }
}

fn refuse_busy(busy: Option<&str>) -> Result<(), String> {
    busy.map_or(Ok(()), |reason| Err(reason.to_owned()))
}

/// Posts the note of `row`, then sets `posted_at`. A send error leaves the row unposted.
fn post(store: &Store, row: &ReviewDecision, send: impl FnOnce(String) -> Result<(), String>) -> Result<(), String> {
    let note = row.note.as_deref().unwrap_or_default();
    send(format!("Review of {}: {note}", short(&row.head)))?;
    store.mark_review_posted(&row.id, now()).map_err(|e| e.to_string())
}

/// Records a decision. Request changes also posts the note through `send`.
#[expect(clippy::too_many_arguments, reason = "Each argument is one field of the decision.")]
pub fn decide(
    store: &Store,
    channel: &ChannelId,
    folder: &Path,
    busy: Option<&str>,
    decision: &str,
    head: &str,
    note: Option<&str>,
    send: impl FnOnce(String) -> Result<(), String>,
) -> Result<Outcome, String> {
    let decision = match decision {
        "approved" => Decision::Approved,
        "changes_requested" => Decision::ChangesRequested,
        other => return Err(format!("Unknown review decision: {other}.")),
    };
    let note = note.map(str::trim).filter(|note| !note.is_empty());
    if decision == Decision::ChangesRequested && note.is_none() {
        return Err("Write a note that says what to change.".into());
    }
    if note.is_some_and(|note| note.chars().count() > NOTE_LIMIT) {
        return Err(format!("The note is too long. Keep it under {NOTE_LIMIT} characters."));
    }
    refuse_archived(store, channel)?;
    if decision == Decision::ChangesRequested {
        refuse_busy(busy)?;
    }
    let newest = store.review_decisions(channel).map_err(|e| e.to_string())?.into_iter().next();
    let branch = store.channel_branch(channel).map_err(|e| e.to_string())?;
    let live = snapshot(folder, branch.as_deref(), newest.as_ref().map(|row| row.head.as_str()));
    if let Some(reason) = &live.reason {
        return Err(reason.clone());
    }
    if live.head.as_deref() != Some(head) {
        return Err(STALE.into());
    }
    if matches!(review_state(&live, newest.as_ref()), ReviewState::Merged | ReviewState::NothingToReview) {
        return Err("This branch has nothing to review.".into());
    }
    let (Some(base), Some(target)) = (live.base.clone(), live.target.clone()) else {
        return Err("The review has no target branch.".into());
    };
    let row = ReviewDecision {
        id: uuid::Uuid::new_v4().to_string(),
        channel_id: channel.0.clone(),
        decision,
        head: head.to_owned(),
        base,
        target,
        note: note.map(str::to_owned),
        decided_at: now(),
        posted_at: None,
    };
    // An identical newest row comes back instead of a second row. Its posted note is not posted again.
    let row = store.add_review_decision(&row).map_err(|e| e.to_string())?;
    let post_error = if decision == Decision::ChangesRequested && row.posted_at.is_none() { post(store, &row, send).err() } else { None };
    Ok(Outcome { dto: review(store, channel, folder, busy)?, post_error })
}

/// Posts the stored note of an unposted changes-requested row.
pub fn retry_post(
    store: &Store,
    channel: &ChannelId,
    folder: &Path,
    busy: Option<&str>,
    decision_id: &str,
    send: impl FnOnce(String) -> Result<(), String>,
) -> Result<Outcome, String> {
    refuse_archived(store, channel)?;
    refuse_busy(busy)?;
    let rows = store.review_decisions(channel).map_err(|e| e.to_string())?;
    let Some(row) = rows.iter().find(|row| row.id == decision_id) else { return Err("This review decision does not exist.".into()) };
    if row.decision != Decision::ChangesRequested {
        return Err("Only a request for changes has a note to post.".into());
    }
    if row.posted_at.is_some() {
        return Err("This note is already posted.".into());
    }
    let post_error = post(store, row, send).err();
    Ok(Outcome { dto: review(store, channel, folder, busy)?, post_error })
}

/// Posts the fixed summary request.
pub fn request_summary(store: &Store, channel: &ChannelId, busy: Option<&str>, send: impl FnOnce(String) -> Result<(), String>) -> Result<(), String> {
    refuse_archived(store, channel)?;
    refuse_busy(busy)?;
    send(SUMMARY_REQUEST.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::{
        agent::AgentConfig,
        capability::ApprovalPolicy,
        model::{Project, ProjectId},
    };
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn git(root: &Path, args: &[&str]) -> String {
        let mut all = vec!["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"];
        all.extend_from_slice(args);
        let output = discard_core::process::spawn_std(
            std::process::Command::new("git").args(&all).current_dir(root).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()),
        )
        .and_then(|child| child.wait_with_output())
        .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn commit_file(root: &Path, name: &str) {
        std::fs::write(root.join(name), name).unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", name]);
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        store: Store,
        channel: ChannelId,
    }

    /// A repository on `feature` with one commit over `main`, bound to a project channel.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        commit_file(&root, "first.txt");
        git(&root, &["checkout", "-q", "-b", "feature"]);
        commit_file(&root, "a.txt");
        let store = Store::open_in_memory().unwrap();
        let config = AgentConfig {
            workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false, reasoning_effort: None,
            name: "Main".into(), avatar: None, provider: "claude".into(), model: Some("model".into()),
            system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false,
        };
        let agent = store.create_agent(&config).unwrap();
        let project = Project { id: ProjectId::new(), name: "P".into(), path: root.to_string_lossy().into_owned() };
        store.insert_project(&project).unwrap();
        let channel = store.ensure_project_general_channel(&project.id, &agent).unwrap();
        store.bind_branch(&channel.id, "feature").unwrap();
        Fixture { _dir: dir, root, store, channel: channel.id }
    }

    fn head(f: &Fixture) -> String {
        git(&f.root, &["rev-parse", "refs/heads/feature"])
    }

    type Sent = RefCell<Vec<String>>;

    fn recorder(sent: &Sent) -> impl FnOnce(String) -> Result<(), String> + '_ {
        move |text| {
            sent.borrow_mut().push(text);
            Ok(())
        }
    }

    fn request(f: &Fixture, busy: Option<&str>, head: &str, note: &str, sent: &Sent) -> Result<Outcome, String> {
        decide(&f.store, &f.channel, &f.root, busy, "changes_requested", head, Some(note), recorder(sent))
    }

    fn approve(f: &Fixture, busy: Option<&str>, head: &str, sent: &Sent) -> Result<Outcome, String> {
        decide(&f.store, &f.channel, &f.root, busy, "approved", head, None, recorder(sent))
    }

    fn rows(f: &Fixture) -> Vec<ReviewDecision> {
        f.store.review_decisions(&f.channel).unwrap()
    }

    #[test]
    fn review_dto_joins_snapshot_rows_state_and_busy_reason() {
        let f = fixture();
        let sent = Sent::default();
        let dto = review(&f.store, &f.channel, &f.root, Some("A turn runs.")).unwrap();
        assert_eq!(dto.snapshot.reason, None);
        assert_eq!(dto.snapshot.source.as_deref(), Some("feature"));
        assert_eq!(dto.state, ReviewState::NotReviewed);
        assert!(dto.decisions.is_empty());
        assert_eq!(dto.blocked.as_deref(), Some("A turn runs."));
        assert_eq!(dto.unposted, None);
        approve(&f, None, &head(&f), &sent).unwrap();
        let dto = review(&f.store, &f.channel, &f.root, None).unwrap();
        assert_eq!(dto.state, ReviewState::Approved);
        assert_eq!(dto.decisions.len(), 1);
        assert_eq!(dto.blocked, None);
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["state"], "approved");
        assert!(json["decisions"][0]["channelId"].is_string());
    }

    #[test]
    fn approve_writes_one_row_posts_nothing_and_is_allowed_while_busy() {
        let f = fixture();
        let sent = Sent::default();
        let out = approve(&f, Some("A turn runs."), &head(&f), &sent).unwrap();
        assert_eq!(out.dto.state, ReviewState::Approved);
        assert_eq!(out.post_error, None);
        assert!(sent.borrow().is_empty());
        let rows = rows(&f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].decision, Decision::Approved);
        assert_eq!(rows[0].head, head(&f));
        assert_eq!(rows[0].target, "main");
        assert_eq!(rows[0].note, None);
    }

    #[test]
    fn request_changes_needs_a_note_and_a_free_channel_and_writes_no_row_when_refused() {
        let f = fixture();
        let sent = Sent::default();
        let h = head(&f);
        assert!(request(&f, None, &h, "   \n", &sent).is_err());
        let over = "x".repeat(NOTE_LIMIT + 1);
        assert!(request(&f, None, &h, &over, &sent).is_err());
        let ok = "é".repeat(NOTE_LIMIT);
        assert!(request(&f, Some("A turn runs."), &h, &ok, &sent).unwrap_err().contains("A turn runs."));
        assert!(decide(&f.store, &f.channel, &f.root, None, "changes_requested", &h, None, recorder(&sent)).is_err());
        assert!(decide(&f.store, &f.channel, &f.root, None, "maybe", &h, None, recorder(&sent)).is_err());
        assert!(rows(&f).is_empty());
        assert!(sent.borrow().is_empty());
        request(&f, None, &h, &ok, &sent).unwrap();
        assert_eq!(rows(&f).len(), 1, "a note of exactly NOTE_LIMIT characters passes");
    }

    #[test]
    fn request_changes_posts_the_prefixed_note_once_and_sets_posted_at() {
        let f = fixture();
        let sent = Sent::default();
        let h = head(&f);
        let out = request(&f, None, &h, "  Fix the name.  ", &sent).unwrap();
        assert_eq!(*sent.borrow(), vec![format!("Review of {}: Fix the name.", &h[..7])]);
        assert_eq!(out.post_error, None);
        assert_eq!(out.dto.unposted, None);
        assert_eq!(out.dto.state, ReviewState::ChangesRequested);
        let rows = rows(&f);
        assert_eq!(rows[0].note.as_deref(), Some("Fix the name."));
        assert_eq!(rows[0].head, h, "the row keeps the full head");
        assert!(rows[0].posted_at.is_some());
    }

    #[test]
    fn a_failed_post_keeps_the_row_unposted_and_retry_posts_the_stored_text_after_the_head_moved() {
        let f = fixture();
        let h = head(&f);
        let out = decide(&f.store, &f.channel, &f.root, None, "changes_requested", &h, Some("Fix it."), |_| Err("offline".into())).unwrap();
        assert_eq!(out.post_error.as_deref(), Some("offline"));
        let id = rows(&f)[0].id.clone();
        assert_eq!(out.dto.unposted.as_deref(), Some(id.as_str()));
        assert_eq!(rows(&f)[0].posted_at, None);
        commit_file(&f.root, "later.txt");
        let sent = Sent::default();
        assert!(retry_post(&f.store, &f.channel, &f.root, Some("A turn runs."), &id, recorder(&sent)).is_err());
        assert!(sent.borrow().is_empty());
        let out = retry_post(&f.store, &f.channel, &f.root, None, &id, recorder(&sent)).unwrap();
        assert_eq!(*sent.borrow(), vec![format!("Review of {}: Fix it.", &h[..7])]);
        assert_eq!(out.dto.unposted, None);
        assert_eq!(out.dto.state, ReviewState::ChangesRequestedNewCommits);
        assert!(rows(&f)[0].posted_at.is_some());
        assert_eq!(rows(&f).len(), 1, "no second row");
        // A posted row, an approval, and an unknown id are refused.
        assert!(retry_post(&f.store, &f.channel, &f.root, None, &id, recorder(&sent)).is_err());
        let new_head = head(&f);
        approve(&f, None, &new_head, &sent).unwrap();
        let approval = rows(&f)[0].id.clone();
        assert!(retry_post(&f.store, &f.channel, &f.root, None, &approval, recorder(&sent)).is_err());
        assert!(retry_post(&f.store, &f.channel, &f.root, None, "missing", recorder(&sent)).is_err());
        assert_eq!(sent.borrow().len(), 1);
    }

    #[test]
    fn a_stale_head_is_refused_with_the_exact_copy_and_writes_no_row() {
        let f = fixture();
        let sent = Sent::default();
        let old = head(&f);
        commit_file(&f.root, "later.txt");
        assert_eq!(approve(&f, None, &old, &sent).err().as_deref(), Some("The branch changed. Reload the review."));
        assert_eq!(request(&f, None, &old, "note", &sent).err().as_deref(), Some(STALE));
        assert!(rows(&f).is_empty());
        assert!(sent.borrow().is_empty());
    }

    #[test]
    fn merged_nothing_to_review_and_reason_states_refuse_decisions() {
        let sent = Sent::default();
        // Merged.
        let f = fixture();
        let h = head(&f);
        approve(&f, None, &h, &sent).unwrap();
        git(&f.root, &["checkout", "-q", "main"]);
        git(&f.root, &["merge", "-q", "--no-ff", "-m", "merge", "feature"]);
        assert_eq!(review(&f.store, &f.channel, &f.root, None).unwrap().state, ReviewState::Merged);
        assert!(approve(&f, None, &h, &sent).is_err());
        assert!(request(&f, None, &h, "note", &sent).is_err());
        assert_eq!(rows(&f).len(), 1, "only the approval before the merge");
        // Nothing to review.
        let f = fixture();
        git(&f.root, &["checkout", "-q", "main"]);
        git(&f.root, &["branch", "-f", "feature", "main"]);
        let h = head(&f);
        assert_eq!(review(&f.store, &f.channel, &f.root, None).unwrap().state, ReviewState::NothingToReview);
        assert!(approve(&f, None, &h, &sent).is_err());
        assert!(request(&f, None, &h, "note", &sent).is_err());
        // A reason: the folder is not a repository.
        let f = fixture();
        let h = head(&f);
        let plain = tempfile::tempdir().unwrap();
        let err = decide(&f.store, &f.channel, plain.path(), None, "approved", &h, None, recorder(&sent)).unwrap_err();
        assert_eq!(Some(err), review(&f.store, &f.channel, plain.path(), None).unwrap().snapshot.reason);
        assert!(rows(&f).is_empty());
        assert!(sent.borrow().is_empty());
    }

    #[test]
    fn an_archived_channel_refuses_every_action_with_its_reason() {
        let f = fixture();
        let sent = Sent::default();
        let h = head(&f);
        let mut id = None;
        let out = decide(&f.store, &f.channel, &f.root, None, "changes_requested", &h, Some("n"), |_| Err("offline".into())).unwrap();
        id.get_or_insert(out.dto.unposted.unwrap());
        f.store.archive_channel(&f.channel).unwrap();
        let dto = review(&f.store, &f.channel, &f.root, Some("A turn runs.")).unwrap();
        assert_eq!(dto.blocked.as_deref(), Some("This channel is archived."));
        assert_eq!(approve(&f, None, &h, &sent).err().as_deref(), Some(ARCHIVED));
        assert_eq!(request(&f, None, &h, "other", &sent).err().as_deref(), Some(ARCHIVED));
        assert_eq!(retry_post(&f.store, &f.channel, &f.root, None, &id.unwrap(), recorder(&sent)).err().as_deref(), Some(ARCHIVED));
        assert_eq!(request_summary(&f.store, &f.channel, None, recorder(&sent)).err().as_deref(), Some(ARCHIVED));
        assert_eq!(rows(&f).len(), 1);
        assert!(sent.borrow().is_empty());
    }

    #[test]
    fn two_identical_requests_leave_one_row() {
        let f = fixture();
        let sent = Sent::default();
        let h = head(&f);
        request(&f, None, &h, "same", &sent).unwrap();
        let out = request(&f, None, &h, "same", &sent).unwrap();
        assert_eq!(rows(&f).len(), 1);
        assert_eq!(out.dto.unposted, None);
        assert_eq!(sent.borrow().len(), 1, "the repeat posts nothing");
    }

    #[test]
    fn the_summary_request_posts_once_and_obeys_the_busy_rule() {
        let f = fixture();
        let sent = Sent::default();
        assert_eq!(request_summary(&f.store, &f.channel, Some("A turn runs."), recorder(&sent)).err().as_deref(), Some("A turn runs."));
        assert!(sent.borrow().is_empty());
        request_summary(&f.store, &f.channel, None, recorder(&sent)).unwrap();
        assert_eq!(*sent.borrow(), vec![SUMMARY_REQUEST.to_owned()]);
        assert!(rows(&f).is_empty());
    }
}
