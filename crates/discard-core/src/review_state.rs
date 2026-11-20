//! Merge review decisions and the state derived from them.

use crate::merge_review::ReviewSnapshot;
use serde::{Deserialize, Serialize};

/// Caps a review note, in characters. The composer has no cap.
pub const NOTE_LIMIT: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approved,
    ChangesRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDecision {
    pub id: String,
    pub channel_id: String,
    pub decision: Decision,
    pub head: String,
    pub base: String,
    pub target: String,
    pub note: Option<String>,
    pub decided_at: i64,
    pub posted_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    NotReviewed,
    Approved,
    ApprovalOutOfDate,
    ChangesRequested,
    ChangesRequestedNewCommits,
    Merged,
    NothingToReview,
}

/// Merged and NothingToReview win. Then the newest row against the live head.
pub fn review_state(snapshot: &ReviewSnapshot, newest: Option<&ReviewDecision>) -> ReviewState {
    if snapshot.merged {
        return ReviewState::Merged;
    }
    if snapshot.nothing_to_review {
        return ReviewState::NothingToReview;
    }
    let Some(row) = newest else { return ReviewState::NotReviewed };
    let current = snapshot.head.as_deref() == Some(row.head.as_str());
    match (row.decision, current) {
        (Decision::Approved, true) => ReviewState::Approved,
        (Decision::Approved, false) => ReviewState::ApprovalOutOfDate,
        (Decision::ChangesRequested, true) => ReviewState::ChangesRequested,
        (Decision::ChangesRequested, false) => ReviewState::ChangesRequestedNewCommits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(head: &str) -> ReviewSnapshot {
        ReviewSnapshot {
            source: Some("feature".into()), target: Some("main".into()), head: Some(head.into()),
            head_short: None, base: Some("b".into()), base_short: None, ahead: 1, behind: 0,
            nothing_to_review: false, merged: false, dirty: 0, commits: Vec::new(), files: Vec::new(),
            truncated: false, reason: None,
        }
    }

    fn row(decision: Decision, head: &str) -> ReviewDecision {
        ReviewDecision {
            id: "1".into(), channel_id: "c".into(), decision, head: head.into(), base: "b".into(),
            target: "main".into(), note: None, decided_at: 1, posted_at: None,
        }
    }

    #[test]
    fn no_row_is_not_reviewed() {
        assert_eq!(review_state(&snapshot("a"), None), ReviewState::NotReviewed);
    }

    #[test]
    fn an_approval_at_the_live_head_is_approved_and_at_an_older_head_is_out_of_date() {
        let r = row(Decision::Approved, "a");
        assert_eq!(review_state(&snapshot("a"), Some(&r)), ReviewState::Approved);
        assert_eq!(review_state(&snapshot("b"), Some(&r)), ReviewState::ApprovalOutOfDate);
    }

    #[test]
    fn changes_requested_at_the_live_head_and_at_an_older_head_differ() {
        let r = row(Decision::ChangesRequested, "a");
        assert_eq!(review_state(&snapshot("a"), Some(&r)), ReviewState::ChangesRequested);
        assert_eq!(review_state(&snapshot("b"), Some(&r)), ReviewState::ChangesRequestedNewCommits);
    }

    #[test]
    fn merged_and_nothing_to_review_win_over_any_row() {
        let r = row(Decision::Approved, "a");
        let mut merged = snapshot("a");
        merged.merged = true;
        assert_eq!(review_state(&merged, Some(&r)), ReviewState::Merged);
        let mut nothing = snapshot("a");
        nothing.nothing_to_review = true;
        assert_eq!(review_state(&nothing, Some(&r)), ReviewState::NothingToReview);
        assert_eq!(review_state(&nothing, None), ReviewState::NothingToReview);
    }
}
