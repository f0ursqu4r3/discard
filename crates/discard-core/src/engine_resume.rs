//! Native provider resume, engine side: which turns may resume, the session fingerprint, the input of a resumed
//! turn, and the session record after a completed turn. The pure rules live in `native_resume`.

use super::{Engine, PreparedMemory, ResumableParts, send};
use crate::conversation_context::{self as context, ContextError};
use crate::model::{Block, Message, MessageId, Role};
use crate::native_resume::{self as resume, DropReason, ResumeDecision, SessionFingerprint, MARKER_HANDLE_REJECTED, MARKER_PROVIDER_COMPACTED};
use crate::provider::SessionMode;
use crate::store::{AgentSession, ChannelHistory};
use crate::ui_state::{Command, Update};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::warn;

/// An eligible turn before its decision. Clone it to build the input again, as a replay after a rejected handle.
#[derive(Clone)]
pub(super) struct ResumeSetup {
    pub session_dir: PathBuf,
    pub parts: ResumableParts,
    /// Sorted MCP server names of this turn's config, plus the tools a Workspace session adds.
    pub tools: Vec<String>,
}

/// An eligible turn after its decision. Saved as the session record when the turn completes.
pub(super) struct ResumeTurn {
    pub decision: ResumeDecision,
    pub fingerprint: SessionFingerprint,
    pub session_dir: PathBuf,
    /// Message IDs that the session has seen, in this turn's input or before it.
    pub delivered_ids: HashSet<String>,
    /// Memory items the session holds after this turn's input.
    pub delivered_memory: BTreeMap<String, u64>,
    pub topic_version: u64,
    pub capture_paused: bool,
    /// Bytes the session held before this turn's input.
    pub prev_bytes: u64,
}

/// The parts of a turn that a replay after a rejected handle replaces.
pub(super) struct Rejected<'a> {
    pub spec: &'a mut crate::provider::SessionSpec,
    pub input: &'a mut String,
    pub images: &'a mut Vec<crate::attachments::Attachment>,
    pub turn: &'a mut Option<ResumeTurn>,
}

impl ResumeTurn {
    pub fn handle(&self) -> Option<String> {
        match &self.decision { ResumeDecision::Resume(row) => Some(row.handle.clone()), _ => None }
    }
}

/// How an eligible turn ended without a saved reply. Each drops the handle: the provider session may hold a turn
/// that the transcript lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TurnFailure { Cancelled, Failed, NotSaved }

const UNSAFE_CANCELLED: &str = "the previous response was cancelled";
const UNSAFE_FAILED: &str = "the previous response failed";
const UNSAFE_NOT_SAVED: &str = "the previous response was not saved";
const UNSAFE_UNFINISHED: &str = "the previous response did not finish";
/// The resume state of a turn that falls back to a read-only Conversation session.
const READ_ONLY_FALLBACK: &str = "the workspace sandbox is unavailable, so this response is read-only";

/// True when a provider error says the input passed the model's context limit. A heuristic on the error text: the
/// adapters do not classify this error.
fn is_context_limit(error: &str) -> bool {
    let error = error.to_lowercase();
    // "context deadline exceeded" is a timeout, not a full context.
    if error.contains("deadline") { return false; }
    error.contains("prompt is too long")
        || (error.contains("context") && ["limit", "window", "too long", "exceed"].iter().any(|word| error.contains(word)))
}

/// The drop reason of a failed eligible turn. A context-limit error on a resumed session means the session is full.
fn failure_reason(failure: TurnFailure, resumed: bool, errors: &str) -> DropReason {
    match failure {
        TurnFailure::Failed if resumed && is_context_limit(errors) => DropReason::OverBudget,
        TurnFailure::Failed => DropReason::PreviousTurnUnsafe(UNSAFE_FAILED.into()),
        TurnFailure::Cancelled => DropReason::PreviousTurnUnsafe(UNSAFE_CANCELLED.into()),
        TurnFailure::NotSaved => DropReason::PreviousTurnUnsafe(UNSAFE_NOT_SAVED.into()),
    }
}

fn mode_name(mode: SessionMode) -> &'static str {
    match mode { SessionMode::Workspace => "workspace", SessionMode::Conversation => "conversation", SessionMode::Coding => "coding" }
}

fn hex_sha256(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The MCP server names of a session config file, sorted. `None` if the file cannot be read.
fn mcp_servers(path: &std::path::Path) -> Option<Vec<String>> {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let mut names: Vec<String> = config.get("mcpServers")?.as_object()?.keys().cloned().collect();
    names.sort();
    Some(names)
}

/// The message ID of a replay chunk ID `<message-id>:part:<index>`.
fn message_of(chunk: &MessageId) -> &str {
    chunk.0.split(":part:").next().unwrap_or(&chunk.0)
}

/// Images of the messages that these replay chunks carry.
fn images_of(history: &ChannelHistory, chunks: &[(MessageId, String)]) -> Vec<crate::attachments::Attachment> {
    let retained: HashSet<&str> = chunks.iter().map(|(id, _)| message_of(id)).collect();
    history.messages.iter().filter(|message| retained.contains(message.id.0.as_str()))
        .flat_map(|message| &message.blocks)
        .filter_map(|block| match block { Block::Attachment { attachment } if attachment.is_image() => Some(attachment.clone()), _ => None })
        .collect()
}

/// The context block of a turn goes before its request: the context update, then the session state.
fn with_context(update: Option<&str>, session_state: &str, current: &str) -> String {
    let mut text = String::new();
    for part in [update.unwrap_or(""), session_state] {
        if part.is_empty() { continue; }
        text.push_str(part.trim_end());
        text.push_str("\n\n");
    }
    text.push_str(current);
    text
}

/// The last chunk of the contiguous prefix of `messages` whose message the session has seen.
fn delivered_through(messages: &[(MessageId, String)], seen: &HashSet<String>) -> Option<MessageId> {
    messages.iter().take_while(|(id, _)| seen.contains(message_of(id))).last().map(|(id, _)| id.clone())
}

impl Engine {
    /// True when this turn may continue a provider session: a Conversation or Workspace response, not in a thread,
    /// of an agent with `native_resume` on. Summary sessions never get here.
    pub(super) fn native_resume_enabled(&self, thread_turn: bool) -> bool {
        self.mode == SessionMode::Conversation && !thread_turn
            && crate::driver::lock(&self.store).load_agent(&self.agent_id).is_ok_and(|config| config.native_resume)
    }

    /// The setup of an eligible turn, or `None` for today's replay. An agent with the setting off loses its saved
    /// session here, at the start of the turn.
    pub(super) fn resume_setup(&self, session_mode: SessionMode, thread_turn: bool, prepared: Option<&PreparedMemory>) -> Option<ResumeSetup> {
        if !self.native_resume_enabled(thread_turn) {
            let off = crate::driver::lock(&self.store).load_agent(&self.agent_id).is_ok_and(|config| !config.native_resume);
            if off && let Err(error) = crate::driver::lock(&self.store).delete_agent_session(&self.channel_id, &self.agent_id) {
                warn!(%error, "could not delete the native session record");
            }
            return None;
        }
        let prepared = prepared?;
        let parts = prepared.parts.clone()?;
        let session_dir = match session_mode {
            SessionMode::Workspace => self.write_access.as_ref().ok()?.root.clone(),
            SessionMode::Conversation => self.memory.as_ref()?.resume_dir(&self.channel_id, &self.agent_id)?,
            SessionMode::Coding => return None,
        };
        let mut tools = mcp_servers(&prepared.mcp_config)?;
        if session_mode == SessionMode::Workspace { tools.push("worktree_commit".into()); }
        Some(ResumeSetup { session_dir, parts, tools })
    }

    /// The fingerprint of this turn's session, without the summary boundary and the history digest:
    /// `resumable_input` reads those with the saved record.
    pub(super) fn session_fingerprint(&self, setup: &ResumeSetup, session_mode: SessionMode, system: &str, delegation: bool, autorun: bool) -> SessionFingerprint {
        SessionFingerprint {
            provider: self.provider.id().into(),
            model: self.model.clone().unwrap_or_default(),
            effort: self.reasoning_effort.clone(),
            mode: mode_name(session_mode).into(),
            session_dir: setup.session_dir.to_string_lossy().into(),
            fixed_prompt_hash: hex_sha256(&[system, &setup.parts.core_hash]),
            tools: setup.tools.clone(),
            delegation,
            sandbox: session_mode == SessionMode::Workspace,
            autorun,
            cli_version: self.provider.resume_epoch(),
            summary_through: None,
            history_digest: String::new(),
        }
    }

    /// Build the input of an eligible turn: the delta on a resume, else the full replay with the context block in
    /// front of the request. A replay that follows a drop shows its reason.
    #[expect(clippy::too_many_arguments, reason = "The turn's inputs, as `conversation_input` takes them.")]
    pub(super) async fn resumable_input(&self, setup: ResumeSetup, mut fingerprint: SessionFingerprint, request: &str, history: &ChannelHistory, system: &str, commands: &mut mpsc::UnboundedReceiver<Command>, updates: &mpsc::UnboundedSender<Update>) -> Result<(String, Vec<crate::attachments::Attachment>, ResumeTurn), ContextError> {
        let (topic, saved) = {
            let store = crate::driver::lock(&self.store);
            fingerprint.summary_through = store.conversation_context(&self.channel_id).map_err(|e| e.to_string())?.through.map(|id| id.0);
            (store.channel_topic(&self.channel_id).map_err(|e| e.to_string())?, store.agent_session(&self.channel_id, &self.agent_id).map_err(|e| e.to_string())?)
        };
        let messages = context::replay_with_posted(&history.messages, &history.authors, &history.posted);
        fingerprint.history_digest = resume::history_digest(&messages, saved.as_ref().and_then(|row| row.delivered_through.as_ref()));
        // The snapshots and the session state go in every turn, labelled so that they replace earlier ones.
        let snapshots = resume::snapshots_block(&self.turn_snapshots().await?);
        let parts = &setup.parts;
        let session_state = resume::session_state_block(&parts.session_state);
        // The delta of a resume. Topic and capture state go in only when they changed. A cleared topic is sent as
        // an empty topic element.
        let delta = saved.as_ref().map(|row| {
            let undelivered = resume::undelivered(&messages, row.delivered_through.as_ref());
            let topic_text = (row.delivered_topic_version != Some(topic.version)).then_some(topic.text.as_str());
            let capture = (row.delivered_capture_paused != parts.capture_paused).then_some(parts.capture_paused);
            let update = resume::context_update(&row.delivered_memory, &parts.selected, &parts.live_versions, topic_text, capture);
            (resume::compose_resume(update.as_deref(), &session_state, undelivered, &format!("{snapshots}{request}")), undelivered)
        });
        let budget = context::input_budget(self.provider.model_context_window(self.model.as_deref().unwrap_or("")));
        let decision = resume::decide(saved.as_ref(), &fingerprint, delta.as_ref().map_or(0, |(input, _)| input.len()), budget);
        let delivered_ids = history.messages.iter().filter(|m| m.role != Role::System).map(|m| m.id.0.clone()).collect();
        let (base_memory, prev_bytes, input, images) = match (&decision, delta) {
            (ResumeDecision::Resume(row), Some((input, undelivered))) => {
                send(updates, Update::ResumeState(crate::ui_state::ResumeState::Resumed));
                (row.delivered_memory.clone(), row.bytes_in_session, input, images_of(history, undelivered))
            }
            _ => {
                if let ResumeDecision::Replay(reason) = &decision { self.announce_replay(reason, updates)?; }
                // A fresh session gets every selected item, and the capture state only when capture is paused.
                let update = resume::context_update(&BTreeMap::new(), &parts.selected, &parts.live_versions, None, parts.capture_paused.then_some(true));
                let current = with_context(update.as_deref(), &session_state, &format!("{snapshots}{}", super::topic_request(&topic.text, request)));
                let (input, images) = self.conversation_input(&current, history, system, true, commands, updates).await?;
                (BTreeMap::new(), system.len() as u64, input, images)
            }
        };
        let delivered_memory = resume::delivered_after_turn(base_memory, &parts.selected, &parts.live_versions);
        let turn = ResumeTurn { decision, fingerprint, session_dir: setup.session_dir, delivered_ids, delivered_memory, topic_version: topic.version, capture_paused: parts.capture_paused, prev_bytes };
        Ok((input, images, turn))
    }

    fn announce_replay(&self, reason: &DropReason, updates: &mpsc::UnboundedSender<Update>) -> Result<(), String> {
        send(updates, Update::Notice(reason.notice()));
        send(updates, Update::ResumeState(crate::ui_state::ResumeState::Replay { reason: reason.notice().trim_start_matches("Fresh replay: ").into() }));
        self.marker(reason.notice(), updates)
    }

    /// A resumed turn is about to continue its provider session. Mark the record as unsafe first: if the app exits,
    /// crashes, or stops the engine during the turn, the provider session holds a partial reply that the transcript
    /// lacks, and the next response replays. The record of a completed turn clears the mark.
    pub(super) fn mark_turn_started(&self, turn: &ResumeTurn) {
        if turn.handle().is_none() { return; }
        let reason = DropReason::PreviousTurnUnsafe(UNSAFE_UNFINISHED.into()).marker();
        if let Err(error) = crate::driver::lock(&self.store).drop_agent_session(&self.channel_id, &self.agent_id, &reason) {
            warn!(%error, "could not mark the native session record");
        }
    }

    /// A steer stopped this turn, and the steer turn continues the same provider session. Put the record back as it
    /// was before `mark_turn_started`.
    pub(super) fn restore_session(&self, turn: &ResumeTurn) {
        if let ResumeDecision::Resume(row) = &turn.decision && let Err(error) = crate::driver::lock(&self.store).put_agent_session(row) {
            warn!(%error, "could not restore the native session record");
        }
    }

    /// The sandbox is unavailable, so this eligible turn runs as a read-only replay that saves no session. Delete
    /// the record, so later responses do not repeat an old drop notice, and correct the resume state.
    pub(super) fn forget_session_for_read_only(&self, updates: &mpsc::UnboundedSender<Update>) {
        if let Err(error) = crate::driver::lock(&self.store).delete_agent_session(&self.channel_id, &self.agent_id) {
            warn!(%error, "could not delete the native session record");
        }
        send(updates, Update::ResumeState(crate::ui_state::ResumeState::Replay { reason: READ_ONLY_FALLBACK.into() }));
    }

    /// Mark the saved session of a failed eligible turn as dropped, so the next response replays with a notice.
    /// `errors` holds the provider's error text of the turn.
    pub(super) fn drop_failed_session(&self, turn: &ResumeTurn, failure: TurnFailure, errors: &str) {
        let reason = failure_reason(failure, turn.handle().is_some(), errors);
        if let Err(error) = crate::driver::lock(&self.store).drop_agent_session(&self.channel_id, &self.agent_id, &reason.marker()) {
            warn!(%error, "could not drop the native session record");
        }
    }

    /// The provider rejected the saved handle. Drop the record and build the full replay input again, with the
    /// rejection notice. The caller starts one new session without a handle.
    #[expect(clippy::too_many_arguments, reason = "The turn's inputs, as `resumable_input` takes them.")]
    pub(super) async fn replay_rejected_handle(&self, (setup, fingerprint): (ResumeSetup, SessionFingerprint), request: &str, history: &ChannelHistory, system: &str, attachments: &[crate::attachments::Attachment], commands: &mut mpsc::UnboundedReceiver<Command>, updates: &mpsc::UnboundedSender<Update>) -> Result<(String, Vec<crate::attachments::Attachment>, ResumeTurn), ContextError> {
        crate::driver::lock(&self.store).drop_agent_session(&self.channel_id, &self.agent_id, MARKER_HANDLE_REJECTED).map_err(|e| e.to_string())?;
        let (input, previous, turn) = self.resumable_input(setup, fingerprint, request, history, system, commands, updates).await?;
        let images = super::turn_images(previous, attachments).ok_or(super::IMAGES_TOO_LARGE)?;
        Ok((input, images, turn))
    }

    /// Replay after a rejected handle: rebuild the input and clear the handle in `rejected`. On a failure, report it,
    /// end the turn, and return false.
    #[expect(clippy::too_many_arguments, reason = "The turn's state that the replay replaces or reports.")]
    pub(super) async fn replay_after_rejection(&self, setup: (ResumeSetup, SessionFingerprint), rejected: Rejected<'_>, request: &str, history: &crate::store::Result<ChannelHistory>, attachments: &[crate::attachments::Attachment], start: Option<&Result<crate::workspace_changes::StartSnapshot, String>>, author: &crate::model::MessageAuthor, commands: &mut mpsc::UnboundedReceiver<Command>, updates: &mpsc::UnboundedSender<Update>) -> bool {
        let system = rejected.spec.system_prompt.clone().unwrap_or_default();
        let built = match history {
            Ok(history) => self.replay_rejected_handle(setup, request, history, &system, attachments, commands, updates).await,
            Err(error) => Err(ContextError::Failed(error.to_string())),
        };
        let (state, outcome) = match built {
            Ok((input, images, turn)) => {
                debug_assert!(turn.handle().is_none(), "a replay after a rejected handle never resumes");
                (*rejected.input, *rejected.images, *rejected.turn) = (input, images, Some(turn));
                rejected.spec.resume_handle = None;
                return true;
            }
            Err(ContextError::Cancelled) => {
                send(updates, Update::Notice("History summary cancelled. The saved summary is unchanged.".into()));
                (crate::workspace_changes::ChangesState::Stopped, crate::driver::TurnOutcome::Cancelled)
            }
            Err(error) => {
                send(updates, Update::Error(error.to_string()));
                (crate::workspace_changes::ChangesState::Failed, crate::driver::TurnOutcome::Failed)
            }
        };
        self.end_without_response(start, author, state, outcome, updates);
        false
    }

    /// The user stopped the turn while the provider rejected the handle. Mark the record, so the next response
    /// replays with the rejection notice, and end the turn as cancelled.
    pub(super) fn cancel_after_rejection(&self, start: Option<&Result<crate::workspace_changes::StartSnapshot, String>>, author: &crate::model::MessageAuthor, updates: &mpsc::UnboundedSender<Update>) {
        if let Err(error) = crate::driver::lock(&self.store).drop_agent_session(&self.channel_id, &self.agent_id, MARKER_HANDLE_REJECTED) {
            warn!(%error, "could not drop the native session record");
        }
        self.end_without_response(start, author, crate::workspace_changes::ChangesState::Stopped, crate::driver::TurnOutcome::Cancelled, updates);
    }

    /// End a turn that has no provider response. A Workspace turn still saves its changes block.
    fn end_without_response(&self, start: Option<&Result<crate::workspace_changes::StartSnapshot, String>>, author: &crate::model::MessageAuthor, state: crate::workspace_changes::ChangesState, outcome: crate::driver::TurnOutcome, updates: &mpsc::UnboundedSender<Update>) {
        if let Some(start) = start {
            match self.store_message(Role::Agent, vec![self.changes_block(start, state)], Some(author)) {
                Ok(response) => send(updates, Update::ResponseCompleted(response)),
                Err(error) => send(updates, Update::ResponseSaveFailed(error)),
            }
        }
        self.finish(updates, outcome);
    }

    /// Save the session record after a completed turn whose reply is saved. Without a handle there is nothing to
    /// resume, so the record goes. A session that the provider compacted no longer matches the transcript: its
    /// record is saved as dropped, so the next response replays.
    pub(super) fn save_agent_session(&self, turn: ResumeTurn, handle: Option<String>, compacted: bool, input_len: usize, response: &Message, user_message: Option<&MessageId>) {
        let store = crate::driver::lock(&self.store);
        let result = (|| -> crate::store::Result<()> {
            let Some(handle) = handle else { return store.delete_agent_session(&self.channel_id, &self.agent_id) };
            let history = store.channel_history(&self.channel_id)?;
            let messages = context::replay_with_posted(&history.messages, &history.authors, &history.posted);
            let mut seen = turn.delivered_ids;
            seen.extend(user_message.map(|id| id.0.clone()));
            seen.insert(response.id.0.clone());
            let through = delivered_through(&messages, &seen);
            let mut fingerprint = turn.fingerprint;
            fingerprint.summary_through = store.conversation_context(&self.channel_id)?.through.map(|id| id.0);
            fingerprint.history_digest = resume::history_digest(&messages, through.as_ref());
            // The provider session holds the full reply, tool outputs included, not the compacted replay form.
            let reply_len = serde_json::to_string(&response.blocks).map_or(0, |text| text.len());
            store.put_agent_session(&AgentSession {
                channel_id: self.channel_id.clone(),
                agent_id: self.agent_id.clone(),
                handle,
                fingerprint: fingerprint.canonical(),
                delivered_through: through,
                delivered_memory: turn.delivered_memory,
                delivered_topic_version: Some(turn.topic_version),
                delivered_capture_paused: turn.capture_paused,
                bytes_in_session: resume::bytes_after_turn(turn.prev_bytes, input_len, reply_len),
                cwd: turn.session_dir.to_string_lossy().into(),
                dropped_reason: compacted.then(|| MARKER_PROVIDER_COMPACTED.into()),
            })
        })();
        if let Err(error) = result { warn!(%error, "could not save the native session record"); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(ids: &[&str]) -> Vec<(MessageId, String)> {
        ids.iter().map(|id| (MessageId((*id).into()), String::new())).collect()
    }

    #[test]
    fn delivered_through_stops_at_the_first_unseen_message() {
        let messages = chunks(&["a:part:0", "b:part:0", "b:part:1", "c:part:0", "d:part:0"]);
        let seen: HashSet<String> = ["a", "b", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(delivered_through(&messages, &seen), Some(MessageId("b:part:1".into())));
        assert_eq!(delivered_through(&messages, &HashSet::new()), None);
    }

    #[test]
    fn only_a_failed_resumed_turn_with_a_context_limit_error_is_over_budget() {
        let limit = "Error: prompt is too long: 210000 tokens > 200000 maximum";
        assert_eq!(failure_reason(TurnFailure::Failed, true, limit), DropReason::OverBudget);
        assert_eq!(failure_reason(TurnFailure::Failed, true, "The context window was exceeded"), DropReason::OverBudget);
        assert_eq!(failure_reason(TurnFailure::Failed, false, limit), DropReason::PreviousTurnUnsafe(UNSAFE_FAILED.into()));
        assert_eq!(failure_reason(TurnFailure::Failed, true, "network unreachable"), DropReason::PreviousTurnUnsafe(UNSAFE_FAILED.into()));
        assert_eq!(failure_reason(TurnFailure::Failed, true, "rpc error: context deadline exceeded"), DropReason::PreviousTurnUnsafe(UNSAFE_FAILED.into()));
        assert_eq!(failure_reason(TurnFailure::Failed, true, "missing context file"), DropReason::PreviousTurnUnsafe(UNSAFE_FAILED.into()));
        assert_eq!(failure_reason(TurnFailure::Cancelled, true, limit), DropReason::PreviousTurnUnsafe(UNSAFE_CANCELLED.into()));
        assert_eq!(failure_reason(TurnFailure::NotSaved, true, ""), DropReason::PreviousTurnUnsafe(UNSAFE_NOT_SAVED.into()));
    }

    #[test]
    fn the_context_block_goes_before_the_request() {
        assert_eq!(with_context(Some("UPDATE\n"), "STATE", "REQUEST"), "UPDATE\n\nSTATE\n\nREQUEST");
        assert_eq!(with_context(None, "", "REQUEST"), "REQUEST");
    }
}
