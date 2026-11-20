//! Merge review decisions. Rows are append-only history. Only `posted_at` changes after insert.

use super::{Result, Store, StoreError};
use crate::model::ChannelId;
use crate::review_state::{Decision, ReviewDecision};
use rusqlite::params;

pub(super) const REVIEW_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS review_decisions (
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL REFERENCES channels(id),
    decision TEXT NOT NULL,
    head TEXT NOT NULL,
    base TEXT NOT NULL,
    target TEXT NOT NULL,
    note TEXT,
    decided_at INTEGER NOT NULL,
    posted_at INTEGER
);
CREATE INDEX IF NOT EXISTS review_decisions_channel ON review_decisions(channel_id, decided_at);";

fn decision_text(decision: Decision) -> &'static str {
    match decision {
        Decision::Approved => "approved",
        Decision::ChangesRequested => "changes_requested",
    }
}

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(ReviewDecision, String)> {
    let text: String = r.get(2)?;
    let row = ReviewDecision {
        id: r.get(0)?,
        channel_id: r.get(1)?,
        decision: Decision::Approved,
        head: r.get(3)?,
        base: r.get(4)?,
        target: r.get(5)?,
        note: r.get(6)?,
        decided_at: r.get(7)?,
        posted_at: r.get(8)?,
    };
    Ok((row, text))
}

impl Store {
    /// Append a decision. An identical newest row (decision, head, note) is returned instead of a second row.
    pub fn add_review_decision(&self, row: &ReviewDecision) -> Result<ReviewDecision> {
        let channel = ChannelId(row.channel_id.clone());
        if let Some(newest) = self.review_decisions(&channel)?.into_iter().next()
            && newest.decision == row.decision
            && newest.head == row.head
            && newest.note == row.note
        {
            return Ok(newest);
        }
        self.conn.execute(
            "INSERT INTO review_decisions(id,channel_id,decision,head,base,target,note,decided_at,posted_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                row.id, row.channel_id, decision_text(row.decision), row.head, row.base, row.target,
                row.note, row.decided_at, row.posted_at
            ],
        )?;
        Ok(row.clone())
    }

    /// Newest first.
    pub fn review_decisions(&self, channel: &ChannelId) -> Result<Vec<ReviewDecision>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,channel_id,decision,head,base,target,note,decided_at,posted_at
             FROM review_decisions WHERE channel_id=?1 ORDER BY decided_at DESC, rowid DESC",
        )?;
        let raw: Vec<(ReviewDecision, String)> =
            stmt.query_map(params![channel.0], read_row)?.collect::<std::result::Result<_, _>>()?;
        raw.into_iter()
            .map(|(mut row, text)| {
                row.decision = match text.as_str() {
                    "approved" => Decision::Approved,
                    "changes_requested" => Decision::ChangesRequested,
                    other => return Err(StoreError::Corrupt(format!("review decision {other}"))),
                };
                Ok(row)
            })
            .collect()
    }

    /// Sets `posted_at` once. A later call keeps the first time.
    pub fn mark_review_posted(&self, id: &str, at: i64) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE review_decisions SET posted_at=?2 WHERE id=?1 AND posted_at IS NULL",
            params![id, at],
        )?;
        if changed == 0 {
            let exists: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM review_decisions WHERE id=?1", params![id], |r| r.get(0))?;
            if exists == 0 {
                return Err(StoreError::NotFound(format!("review decision {id}")));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AgentConfig, Channel, Project, ProjectId, Store};
    use super::*;
    use crate::capability::ApprovalPolicy;

    fn fixture() -> (Store, Channel, ProjectId) {
        let store = Store::open_in_memory().unwrap();
        let config = AgentConfig {
            workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false, reasoning_effort: None,
            name: "Main".into(), avatar: None, provider: "claude".into(), model: Some("model".into()),
            system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false,
        };
        let agent = store.create_agent(&config).unwrap();
        let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
        store.insert_project(&project).unwrap();
        let channel = store.ensure_project_general_channel(&project.id, &agent).unwrap();
        (store, channel, project.id)
    }

    fn row(channel: &ChannelId, decision: Decision, head: &str, note: Option<&str>, at: i64) -> ReviewDecision {
        ReviewDecision {
            id: uuid::Uuid::new_v4().to_string(), channel_id: channel.0.clone(), decision, head: head.into(),
            base: "base".into(), target: "main".into(), note: note.map(str::to_owned), decided_at: at, posted_at: None,
        }
    }

    #[test]
    fn decisions_append_and_list_newest_first() {
        let (store, ch, _) = fixture();
        let a = store.add_review_decision(&row(&ch.id, Decision::ChangesRequested, "h1", Some("fix"), 10)).unwrap();
        let b = store.add_review_decision(&row(&ch.id, Decision::Approved, "h2", None, 20)).unwrap();
        let list = store.review_decisions(&ch.id).unwrap();
        assert_eq!(list, vec![b, a]);
    }

    #[test]
    fn an_identical_newest_decision_is_returned_instead_of_a_second_row() {
        let (store, ch, _) = fixture();
        let first = store.add_review_decision(&row(&ch.id, Decision::Approved, "h1", Some("ok"), 10)).unwrap();
        let again = store.add_review_decision(&row(&ch.id, Decision::Approved, "h1", Some("ok"), 11)).unwrap();
        assert_eq!(again, first);
        assert_eq!(store.review_decisions(&ch.id).unwrap().len(), 1);
    }

    #[test]
    fn a_changed_note_or_head_or_decision_makes_a_new_row() {
        let (store, ch, _) = fixture();
        store.add_review_decision(&row(&ch.id, Decision::Approved, "h1", Some("ok"), 10)).unwrap();
        store.add_review_decision(&row(&ch.id, Decision::Approved, "h1", Some("ok!"), 11)).unwrap();
        store.add_review_decision(&row(&ch.id, Decision::Approved, "h2", Some("ok!"), 12)).unwrap();
        store.add_review_decision(&row(&ch.id, Decision::ChangesRequested, "h2", Some("ok!"), 13)).unwrap();
        assert_eq!(store.review_decisions(&ch.id).unwrap().len(), 4);
    }

    #[test]
    fn mark_review_posted_sets_posted_at_once_and_an_unknown_id_errors() {
        let (store, ch, _) = fixture();
        let r = store.add_review_decision(&row(&ch.id, Decision::ChangesRequested, "h1", Some("fix"), 10)).unwrap();
        store.mark_review_posted(&r.id, 50).unwrap();
        store.mark_review_posted(&r.id, 99).unwrap();
        assert_eq!(store.review_decisions(&ch.id).unwrap()[0].posted_at, Some(50));
        assert!(store.mark_review_posted("missing", 1).is_err());
    }

    #[test]
    fn the_table_survives_opening_the_store_twice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.db");
        Store::open(&path).unwrap();
        let store = Store::open(&path).unwrap();
        let count: i64 = store.conn.query_row("SELECT COUNT(*) FROM review_decisions", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn rows_of_one_channel_do_not_show_in_another() {
        let (store, ch, project) = fixture();
        let other = store.create_project_text_channel(&project, "other").unwrap().id;
        store.add_review_decision(&row(&ch.id, Decision::Approved, "h1", None, 10)).unwrap();
        assert!(store.review_decisions(&other).unwrap().is_empty());
    }
}
