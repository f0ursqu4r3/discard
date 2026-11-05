//! Versioned memory items. Each hidden markdown document is authoritative: one JSON
//! metadata line (including history) followed by readable current markdown. Only the
//! current live body enters retrieval. The SQLite index is disposable.
use crate::memory::{
    MAX_FILE_BYTES, MemoryError, MemoryStore, Result, Scope, io_err, lock, version_of,
};
use crate::model::{AgentId, ChannelId, ProjectId};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Fact,
    Preference,
    Instruction,
    Decision,
    Commitment,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationOrigin {
    Automatic,
    User,
    Approved,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRevision {
    pub version: u64,
    pub scope: Scope,
    pub content: String,
    pub deleted: bool,
    pub updated_at: u64,
    pub origin: MutationOrigin,
    #[serde(default)]
    pub change_conversation: Option<ChannelId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecaptureSuppression {
    pub source_conversation: ChannelId,
    pub content_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub scope: Scope,
    pub kind: ItemKind,
    pub content: String,
    /// Hash of the last committed body; external edits require reconcile before mutation.
    pub committed_body_hash: String,
    pub source_conversation: Option<ChannelId>,
    /// Conversation that requested the current mutation; source stays unchanged.
    #[serde(default)]
    pub change_conversation: Option<ChannelId>,
    pub created_at: u64,
    pub updated_at: u64,
    pub version: u64,
    pub deleted: bool,
    pub origin: MutationOrigin,
    pub history: Vec<ItemRevision>,
    /// Exact trimmed content hashes suppressed for named conversations.
    /// This does not detect paraphrases. Explicit user saves bypass suppression.
    pub recapture_suppression: Vec<RecaptureSuppression>,
    /// Authoritative conversation capture pauses caused by this item's Forget.
    #[serde(default)]
    pub paused_capture_conversations: Vec<ChannelId>,
    pub proposal_id: Option<String>,
}
impl MemoryItem {
    pub fn path(&self) -> String {
        format!("items/{}.md", self.id)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalAction {
    Forget {
        item_id: String,
        expected_version: u64,
    },
    Create {
        scope: Scope,
        kind: ItemKind,
        content: String,
    },
    Update {
        item_id: String,
        expected_version: u64,
        content: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProposal {
    pub id: String,
    pub agent: AgentId,
    pub source_conversation: ChannelId,
    pub created_at: u64,
    pub action: ProposalAction,
    pub resolved: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveOutcome {
    Saved(MemoryItem),
    Proposed(MemoryProposal),
}
/// Pollable saved/proposed records. Stable keys allow consumers to deduplicate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryChange {
    Saved { key: String, item: MemoryItem },
    Proposed(MemoryProposal),
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn document_err(e: impl std::fmt::Display) -> MemoryError {
    MemoryError::InvalidDocument(e.to_string())
}

impl MemoryStore {
    fn item_document_path(&self, dir: &str, id: &str) -> Result<PathBuf> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(MemoryError::BadScope);
        }
        let mut path = self.root.clone();
        for part in [dir, &format!("{id}.md")] {
            path.push(part);
            if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(MemoryError::Symlink(id.into()));
            }
        }
        if fs::symlink_metadata(&self.root).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(MemoryError::Symlink("memory root".into()));
        }
        Ok(path)
    }
    fn write_document<T: Serialize>(
        &self,
        dir: &str,
        id: &str,
        value: &T,
        body: &str,
    ) -> Result<()> {
        let path = self.item_document_path(dir, id)?;
        fs::create_dir_all(path.parent().unwrap()).map_err(io_err)?;
        let text = format!(
            "<!-- discard-memory {} -->\n\n{}",
            serde_json::to_string(value).map_err(document_err)?,
            body
        );
        let temp = path.with_file_name(format!(".tmp-{}", uuid::Uuid::new_v4()));
        let result = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            fs::rename(&temp, &path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map_err(io_err)
    }
    fn read_document<T: for<'de> Deserialize<'de>>(
        &self,
        dir: &str,
        id: &str,
    ) -> Result<(T, String)> {
        let path = self.item_document_path(dir, id)?;
        let text = fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MemoryError::NotFound(id.into())
            } else {
                io_err(e)
            }
        })?;
        let (meta, body) = text
            .split_once("\n\n")
            .ok_or_else(|| document_err("missing body"))?;
        let meta = meta
            .strip_prefix("<!-- discard-memory ")
            .and_then(|s| s.strip_suffix(" -->"))
            .ok_or_else(|| document_err("missing metadata"))?;
        Ok((
            serde_json::from_str(meta).map_err(document_err)?,
            body.into(),
        ))
    }
    fn document_ids(&self, dir: &str) -> Result<Vec<String>> {
        // Resolve even for discovery; never follow a substituted directory.
        self.item_document_path(dir, &uuid::Uuid::nil().to_string())?;
        let entries = match fs::read_dir(self.root.join(dir)) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(io_err(e)),
        };
        let mut ids = vec![];
        for e in entries {
            let e = e.map_err(io_err)?;
            if e.file_type().map_err(io_err)?.is_file() {
                if let Some(id) = e.file_name().to_str().and_then(|s| s.strip_suffix(".md")) {
                    if uuid::Uuid::parse_str(id).is_ok() {
                        ids.push(id.into());
                    }
                }
            }
        }
        ids.sort();
        Ok(ids)
    }
    pub fn read_item(&self, id: &str) -> Result<MemoryItem> {
        let (mut item, body): (MemoryItem, String) = self.read_document(".items", id)?;
        if item.id != id {
            return Err(document_err("item ID mismatch"));
        }
        self.scope_dir(&item.scope)?;
        // The readable markdown body, not a duplicate metadata field, is authoritative.
        item.content = body;
        Ok(item)
    }
    pub(crate) fn all_items(&self) -> Result<Vec<MemoryItem>> {
        self.document_ids(".items")?
            .iter()
            .map(|id| self.read_item(id))
            .collect()
    }
    pub fn list_items(&self, scopes: &[Scope]) -> Result<Vec<MemoryItem>> {
        for scope in scopes {
            self.resolve(scope, "check.md")?;
        }
        let items = self.all_items()?;
        Ok(scopes
            .iter()
            .flat_map(|scope| {
                items
                    .iter()
                    .filter(move |i| !i.deleted && &i.scope == scope)
                    .cloned()
            })
            .collect())
    }
    fn persist_item(&self, item: &mut MemoryItem) -> Result<()> {
        self.resolve(&item.scope, "check.md")?;
        if item.content.len() > MAX_FILE_BYTES {
            return Err(MemoryError::TooLarge);
        }
        item.committed_body_hash = version_of(&item.content);
        // The metadata body snapshot preserves the previous committed revision if
        // an external editor changes the authoritative readable body.
        self.write_document(".items", &item.id, item, &item.content)?;
        // Remove old scope entries after moves; history is never indexed.
        for scope in item
            .history
            .iter()
            .map(|h| &h.scope)
            .chain(std::iter::once(&item.scope))
        {
            self.unindex(&scope.label(), &item.path())?;
        }
        if !item.deleted {
            self.index_file(
                &item.scope.label(),
                &item.path(),
                &item.content,
                &version_of(&item.content),
            )?;
        }
        Ok(())
    }
    fn create_item_locked(
        &self,
        scope: &Scope,
        kind: ItemKind,
        content: &str,
        source: Option<&ChannelId>,
        origin: MutationOrigin,
        proposal_id: Option<String>,
    ) -> Result<MemoryItem> {
        self.resolve(scope, "check.md")?;
        if origin == MutationOrigin::Automatic {
            if let Some(source) = source {
                if self.capture_paused(source)? {
                    return Err(MemoryError::CapturePaused);
                }
            }
            let hash = version_of(content.trim());
            if self.all_items()?.iter().any(|i| {
                i.recapture_suppression
                    .iter()
                    .any(|r| Some(&r.source_conversation) == source && r.content_hash == hash)
            }) {
                return Err(MemoryError::Suppressed);
            }
            if matches!(scope, Scope::Global | Scope::Project(_)) {
                return Err(MemoryError::ApprovalRequired);
            }
        }
        if self.list_items(&[scope.clone()])?.len() >= crate::memory::MAX_FILES_PER_SCOPE {
            return Err(MemoryError::TooManyFiles);
        }
        let at = now();
        let mut item = MemoryItem {
            id: uuid::Uuid::new_v4().to_string(),
            scope: scope.clone(),
            kind,
            content: content.into(),
            committed_body_hash: version_of(content),
            source_conversation: source.cloned(),
            change_conversation: source.cloned(),
            created_at: at,
            updated_at: at,
            version: 1,
            deleted: false,
            origin,
            history: vec![],
            recapture_suppression: vec![],
            paused_capture_conversations: vec![],
            proposal_id,
        };
        self.persist_item(&mut item)?;
        Ok(item)
    }
    /// App-only direct creation. Agent entry points must use `capture_item`.
    pub fn create_item(
        &self,
        scope: &Scope,
        kind: ItemKind,
        content: &str,
        source: Option<&ChannelId>,
        origin: MutationOrigin,
    ) -> Result<MemoryItem> {
        let _guard = lock(&self.write_lock);
        self.create_item_locked(scope, kind, content, source, origin, None)
    }
    fn versioned_item(&self, id: &str, expected: u64) -> Result<MemoryItem> {
        let item = self.read_item(id)?;
        if item.version != expected {
            return Err(MemoryError::Conflict {
                path: id.into(),
                current: item.version.to_string(),
            });
        }
        Self::check_committed_body(&item)?;
        Ok(item)
    }
    fn check_committed_body(item: &MemoryItem) -> Result<()> {
        if version_of(&item.content) != item.committed_body_hash {
            return Err(MemoryError::Conflict {
                path: item.id.clone(),
                current: format!(
                    "{}; external markdown edit requires reconcile and reload",
                    item.version
                ),
            });
        }
        Ok(())
    }
    /// Import external readable-body edits as new user revisions. Caller holds write_lock.
    pub(crate) fn import_external_items_locked(&self) -> Result<usize> {
        let mut imported = 0;
        for id in self.document_ids(".items")? {
            let current = self.read_item(&id)?;
            if version_of(&current.content) == current.committed_body_hash {
                continue;
            }
            let (mut committed, _): (MemoryItem, String) = self.read_document(".items", &id)?;
            if version_of(&committed.content) != committed.committed_body_hash {
                return Err(document_err(
                    "committed body snapshot does not match its hash",
                ));
            }
            Self::revise(&mut committed, MutationOrigin::User);
            committed.content = current.content;
            self.persist_item(&mut committed)?;
            imported += 1;
        }
        Ok(imported)
    }
    fn revise(item: &mut MemoryItem, origin: MutationOrigin) {
        item.history.push(ItemRevision {
            version: item.version,
            scope: item.scope.clone(),
            content: item.content.clone(),
            deleted: item.deleted,
            updated_at: item.updated_at,
            origin: item.origin,
            change_conversation: item.change_conversation.clone(),
        });
        item.version += 1;
        item.updated_at = now();
        item.origin = origin;
        item.change_conversation = item.source_conversation.clone();
    }
    /// App-only explicit correction. Automatic inferred corrections need a proposal.
    pub fn update_item(
        &self,
        id: &str,
        expected: u64,
        content: &str,
        origin: MutationOrigin,
    ) -> Result<MemoryItem> {
        let _guard = lock(&self.write_lock);
        let mut item = self.versioned_item(id, expected)?;
        if origin == MutationOrigin::Automatic {
            return Err(MemoryError::ApprovalRequired);
        }
        if item.deleted {
            return Err(MemoryError::NotFound(id.into()));
        }
        Self::revise(&mut item, origin);
        item.content = content.into();
        self.persist_item(&mut item)?;
        Ok(item)
    }
    fn suppress(item: &mut MemoryItem, conversation: Option<&ChannelId>) {
        // Conversation-level pause prevents recapture without claiming semantic
        // equivalence detection. Hashes retain exact suppression after app resume.
        let sources = item
            .source_conversation
            .iter()
            .chain(conversation)
            .cloned()
            .collect::<Vec<_>>();
        let hashes = std::iter::once(&item.content)
            .chain(item.history.iter().map(|r| &r.content))
            .map(|s| version_of(s.trim()))
            .collect::<Vec<_>>();
        for source in sources {
            if !item.paused_capture_conversations.contains(&source) {
                item.paused_capture_conversations.push(source.clone());
            }
            for hash in &hashes {
                let suppression = RecaptureSuppression {
                    source_conversation: source.clone(),
                    content_hash: hash.clone(),
                };
                if !item.recapture_suppression.contains(&suppression) {
                    item.recapture_suppression.push(suppression);
                }
            }
        }
    }
    /// App/controller status. Pauses survive reopen in authoritative item documents.
    pub fn capture_paused(&self, conversation: &ChannelId) -> Result<bool> {
        Ok(self
            .all_items()?
            .iter()
            .any(|i| i.paused_capture_conversations.contains(conversation)))
    }
    /// App-only explicit user operation. Never expose this as an agent tool.
    /// Exact forgotten-version hashes stay suppressed after the pause is lifted.
    pub fn resume_capture(&self, conversation: &ChannelId) -> Result<()> {
        let _guard = lock(&self.write_lock);
        for mut item in self.all_items()? {
            if item.paused_capture_conversations.contains(conversation) {
                Self::check_committed_body(&item)?;
                item.paused_capture_conversations
                    .retain(|c| c != conversation);
                self.persist_item(&mut item)?;
            }
        }
        Ok(())
    }
    pub fn forget_item(&self, id: &str, expected: u64) -> Result<MemoryItem> {
        self.forget_item_in(id, expected, None)
    }
    /// App-only direct Forget. Pass the current conversation to pause it as well as the source.
    pub fn forget_item_in(
        &self,
        id: &str,
        expected: u64,
        requester: Option<&ChannelId>,
    ) -> Result<MemoryItem> {
        let _guard = lock(&self.write_lock);
        let mut item = self.versioned_item(id, expected)?;
        if item.deleted {
            return Err(MemoryError::NotFound(id.into()));
        }
        Self::suppress(&mut item, requester);
        Self::revise(&mut item, MutationOrigin::User);
        item.change_conversation = requester.cloned().or(item.source_conversation.clone());
        item.deleted = true;
        self.persist_item(&mut item)?;
        Ok(item)
    }
    pub fn move_item(&self, id: &str, expected: u64, scope: &Scope) -> Result<MemoryItem> {
        let _guard = lock(&self.write_lock);
        self.resolve(scope, "check.md")?;
        let mut item = self.versioned_item(id, expected)?;
        if item.deleted {
            return Err(MemoryError::NotFound(id.into()));
        }
        Self::revise(&mut item, MutationOrigin::User);
        item.scope = scope.clone();
        self.persist_item(&mut item)?;
        Ok(item)
    }
    /// Undo only the current change. Undo of creation forgets the item.
    pub fn undo_item(&self, id: &str, expected: u64) -> Result<MemoryItem> {
        let _guard = lock(&self.write_lock);
        let mut item = self.versioned_item(id, expected)?;
        let previous = item.history.last().cloned();
        Self::revise(&mut item, MutationOrigin::User);
        if let Some(p) = previous {
            item.scope = p.scope;
            item.content = p.content;
            item.deleted = p.deleted;
        } else {
            Self::suppress(&mut item, None);
            item.deleted = true;
        }
        self.persist_item(&mut item)?;
        Ok(item)
    }
    /// Agent capture: identity parameters must come from the runtime, never tool JSON.
    pub fn capture_item(
        &self,
        agent: &AgentId,
        project: Option<&ProjectId>,
        source: &ChannelId,
        scope: &Scope,
        kind: ItemKind,
        content: &str,
    ) -> Result<SaveOutcome> {
        let _guard = lock(&self.write_lock);
        if !Scope::applicable(agent, project).contains(scope) {
            return Err(MemoryError::Forbidden);
        }
        if self.capture_paused(source)? {
            return Err(MemoryError::CapturePaused);
        }
        let private = project.map_or_else(
            || Scope::Agent(agent.clone()),
            |p| Scope::AgentProject(agent.clone(), p.clone()),
        );
        if *scope == private {
            return self
                .create_item_locked(
                    scope,
                    kind,
                    content,
                    Some(source),
                    MutationOrigin::Automatic,
                    None,
                )
                .map(SaveOutcome::Saved);
        }
        if matches!(scope, Scope::Agent(_) | Scope::AgentProject(_, _)) {
            return Err(MemoryError::Forbidden);
        }
        if content.len() > MAX_FILE_BYTES {
            return Err(MemoryError::TooLarge);
        }
        self.proposal_locked(
            agent,
            source,
            ProposalAction::Create {
                scope: scope.clone(),
                kind,
                content: content.into(),
            },
        )
        .map(SaveOutcome::Proposed)
    }
    fn proposal_locked(
        &self,
        agent: &AgentId,
        source: &ChannelId,
        action: ProposalAction,
    ) -> Result<MemoryProposal> {
        if let ProposalAction::Update { content, .. } | ProposalAction::Create { content, .. } =
            &action
        {
            if content.len() > MAX_FILE_BYTES {
                return Err(MemoryError::TooLarge);
            }
        }
        let proposal = MemoryProposal {
            id: uuid::Uuid::new_v4().to_string(),
            agent: agent.clone(),
            source_conversation: source.clone(),
            created_at: now(),
            action,
            resolved: false,
        };
        self.write_document(
            ".proposals",
            &proposal.id,
            &proposal,
            "Pending user approval",
        )?;
        Ok(proposal)
    }
    pub fn propose_update(
        &self,
        agent: &AgentId,
        project: Option<&ProjectId>,
        source: &ChannelId,
        id: &str,
        expected: u64,
        content: &str,
    ) -> Result<MemoryProposal> {
        let _guard = lock(&self.write_lock);
        let item = self.versioned_item(id, expected)?;
        if item.deleted || !Scope::applicable(agent, project).contains(&item.scope) {
            return Err(MemoryError::Forbidden);
        }
        self.proposal_locked(
            agent,
            source,
            ProposalAction::Update {
                item_id: id.into(),
                expected_version: expected,
                content: content.into(),
            },
        )
    }
    pub fn propose_forget(
        &self,
        agent: &AgentId,
        project: Option<&ProjectId>,
        source: &ChannelId,
        id: &str,
        expected: u64,
    ) -> Result<MemoryProposal> {
        let _guard = lock(&self.write_lock);
        let item = self.versioned_item(id, expected)?;
        if item.deleted || !Scope::applicable(agent, project).contains(&item.scope) {
            return Err(MemoryError::Forbidden);
        }
        self.proposal_locked(
            agent,
            source,
            ProposalAction::Forget {
                item_id: id.into(),
                expected_version: expected,
            },
        )
    }
    pub fn pending_proposals(&self) -> Result<Vec<MemoryProposal>> {
        let mut out = vec![];
        for id in self.document_ids(".proposals")? {
            let (p, _): (MemoryProposal, String) = self.read_document(".proposals", &id)?;
            if !p.resolved
                && !self
                    .all_items()?
                    .iter()
                    .any(|i| i.proposal_id.as_ref() == Some(&p.id))
            {
                out.push(p);
            }
        }
        Ok(out)
    }
    /// Typed app-only operation. Never expose this method as an agent tool.
    pub fn approve_proposal(&self, id: &str) -> Result<MemoryItem> {
        let _guard = lock(&self.write_lock);
        let (mut p, _): (MemoryProposal, String) = self.read_document(".proposals", id)?;
        if p.resolved
            || self
                .all_items()?
                .iter()
                .any(|i| i.proposal_id.as_deref() == Some(id))
        {
            return Err(MemoryError::NotFound(id.into()));
        }
        let item = match &p.action {
            ProposalAction::Forget {
                item_id,
                expected_version,
            } => {
                let mut item = self.versioned_item(item_id, *expected_version)?;
                if item.deleted {
                    return Err(MemoryError::NotFound(item_id.clone()));
                }
                Self::suppress(&mut item, Some(&p.source_conversation));
                Self::revise(&mut item, MutationOrigin::Approved);
                item.deleted = true;
                item.proposal_id = Some(p.id.clone());
                item.change_conversation = Some(p.source_conversation.clone());
                self.persist_item(&mut item)?;
                item
            }
            ProposalAction::Create {
                scope,
                kind,
                content,
            } => self.create_item_locked(
                scope,
                *kind,
                content,
                Some(&p.source_conversation),
                MutationOrigin::Approved,
                Some(p.id.clone()),
            )?,
            ProposalAction::Update {
                item_id,
                expected_version,
                content,
            } => {
                let mut i = self.versioned_item(item_id, *expected_version)?;
                if i.deleted {
                    return Err(MemoryError::NotFound(item_id.clone()));
                }
                Self::revise(&mut i, MutationOrigin::Approved);
                i.content = content.clone();
                i.proposal_id = Some(p.id.clone());
                i.change_conversation = Some(p.source_conversation.clone());
                self.persist_item(&mut i)?;
                i
            }
        };
        p.resolved = true;
        self.write_document(".proposals", id, &p, "Approved")?;
        Ok(item)
    }
    pub fn reject_proposal(&self, id: &str) -> Result<()> {
        let _guard = lock(&self.write_lock);
        let (mut p, _): (MemoryProposal, String) = self.read_document(".proposals", id)?;
        p.resolved = true;
        self.write_document(".proposals", id, &p, "Rejected")
    }
    pub fn memory_changes(&self, source: &ChannelId) -> Result<Vec<MemoryChange>> {
        let mut changes = self
            .all_items()?
            .into_iter()
            .filter(|i| {
                i.change_conversation
                    .as_ref()
                    .or(i.source_conversation.as_ref())
                    == Some(source)
            })
            .map(|item| MemoryChange::Saved {
                key: format!("{}:{}", item.id, item.version),
                item,
            })
            .collect::<Vec<_>>();
        changes.extend(
            self.pending_proposals()?
                .into_iter()
                .filter(|p| &p.source_conversation == source)
                .map(MemoryChange::Proposed),
        );
        Ok(changes)
    }
    /// Finds private domains from disk and item metadata, independent of conversations.
    pub fn private_scopes(&self, agent: &AgentId) -> Result<Vec<Scope>> {
        self.scope_dir(&Scope::Agent(agent.clone()))?;
        self.all_items()?;
        Ok(self
            .scopes_on_disk()
            .into_iter()
            .filter(|s| matches!(s,Scope::Agent(a)|Scope::AgentProject(a,_) if a==agent))
            .collect())
    }
    pub fn delete_private_memory(&self, agent: &AgentId) -> Result<()> {
        let _guard = lock(&self.write_lock);
        for scope in self.private_scopes(agent)? {
            self.resolve(&scope, "check.md")?;
            let dir = self.scope_dir(&scope)?;
            if dir.exists() {
                fs::remove_dir_all(dir).map_err(io_err)?;
            }
            for item in self.all_items()?.into_iter().filter(|i| i.scope == scope) {
                fs::remove_file(self.item_document_path(".items", &item.id)?).map_err(io_err)?;
                self.unindex(&scope.label(), &item.path())?;
            }
        }
        for id in self.document_ids(".proposals")? {
            let (p, _): (MemoryProposal, String) = self.read_document(".proposals", &id)?;
            if &p.agent == agent {
                fs::remove_file(self.item_document_path(".proposals", &id)?).map_err(io_err)?;
            }
        }
        // Raw entries are removed by the next reconcile; remove them now without relocking.
        self.reconcile_locked().map(|_| ())
    }
}
