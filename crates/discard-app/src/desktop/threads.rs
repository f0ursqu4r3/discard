//! Thread data for the webview: the summary of one thread, its chip and its channel-bar link.
use super::dto::{ThreadChipDto, ThreadLastMessageDto, ThreadLinkDto};
use discard_core::model::{Message, MessageAuthor, MessageId, Role, ThreadInfo, ThreadKind, ThreadState};
use discard_core::ui_state::first_unread_index;
use std::collections::HashMap;

/// Characters of the last message kept for the chip preview.
pub const PREVIEW_CHARS: usize = 80;

/// What the app keeps about one thread: enough for the chip, the channel bar and the unread flag, with no
/// message blocks. A loaded thread refreshes its record from its live transcript. A thread that is not loaded
/// keeps the record that was read from storage.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadRecord {
    pub id: String,
    pub parent: String,
    /// The anchor message ID. Empty for a sub-agent thread whose parent turn still runs.
    pub anchor: String,
    pub name: String,
    pub followed: bool,
    pub kind: ThreadKind,
    /// The run state of a sub-agent thread. `None` for a question thread.
    pub state: Option<ThreadState>,
    /// The thread's agent: the sub-agent, or the default responder of a question thread.
    pub agent_name: String,
    /// User and agent messages, oldest first, with their blocks removed. Unread is computed from these.
    rows: Vec<Message>,
    last: Option<ThreadLastMessageDto>,
}

fn preview(message: &Message) -> String {
    let text = discard_core::context_menu::message_text(&message.blocks);
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(PREVIEW_CHARS) {
        Some((end, _)) => format!("{}…", flat[..end].trim_end()),
        None => flat,
    }
}

impl ThreadRecord {
    /// Summarize a thread from its own messages (not the parent's) and their authors.
    pub fn build(info: &ThreadInfo, messages: &[Message], authors: &HashMap<MessageId, MessageAuthor>) -> Self {
        let mut record = Self {
            id: info.channel_id.0.clone(), parent: info.parent_channel_id.0.clone(), anchor: info.anchor_message_id.as_ref().map(|anchor| anchor.0.clone()).unwrap_or_default(),
            name: info.name.clone(), followed: info.followed, kind: info.kind, state: info.state, agent_name: info.agent_name.clone(), rows: Vec::new(), last: None,
        };
        record.refresh(messages, authors);
        record
    }

    /// Recompute the message-derived fields from the thread's current transcript.
    pub fn refresh(&mut self, messages: &[Message], authors: &HashMap<MessageId, MessageAuthor>) {
        self.rows = messages.iter().filter(|m| m.role != Role::System)
            .map(|m| Message { id: m.id.clone(), channel_id: m.channel_id.clone(), role: m.role, blocks: Vec::new() }).collect();
        self.last = messages.iter().rev().find(|m| m.role != Role::System).map(|message| ThreadLastMessageDto {
            role: message.role.as_str().into(),
            author_name: authors.get(&message.id).map(|author| author.name.clone()),
            preview: preview(message),
        });
    }

    /// User and agent messages in the thread.
    pub fn count(&self) -> usize { self.rows.len() }

    /// A question thread with no message is not shown anywhere. A sub-agent thread shows from its start, so the
    /// parent shows its chip while the brief is still being saved.
    pub fn visible(&self) -> bool { self.kind == ThreadKind::SubAgent || self.count() > 0 }

    /// Take the anchor from storage-shaped info. The state and the follow flag stay as they are: the parent engine
    /// reads the info when its turn ends, and a sub-agent may have ended or been followed since. Their own updates
    /// set those fields.
    pub fn set_anchor(&mut self, info: &ThreadInfo) {
        self.anchor = info.anchor_message_id.as_ref().map(|anchor| anchor.0.clone()).unwrap_or_default();
    }

    /// An agent message follows the read marker. No marker means nothing was read.
    pub fn unread(&self, marker: Option<&str>) -> bool { first_unread_index(&self.rows, marker).is_some() }

    pub fn chip(&self, marker: Option<&str>, pending_approvals: usize) -> ThreadChipDto {
        ThreadChipDto {
            channel_id: self.id.clone(), name: self.name.clone(), message_count: self.count(),
            last_message: self.last.clone(), followed: self.followed, unread: self.unread(marker),
            kind: self.kind.as_str().into(), state: self.state.map(|state| state.as_str().into()), agent_name: self.agent_name.clone(), pending_approvals,
        }
    }

    pub fn link(&self, marker: Option<&str>, running: bool, pending_approvals: usize) -> ThreadLinkDto {
        ThreadLinkDto {
            id: self.id.clone(), parent_channel_id: self.parent.clone(), name: self.name.clone(),
            unread: self.unread(marker), running, pending_approvals,
            kind: self.kind.as_str().into(), state: self.state.map(|state| state.as_str().into()),
        }
    }
}

/// The followed threads of a channel, from storage, oldest first. Unread follows each thread's read marker.
/// Question threads with no message are left out. A sub-agent thread that still runs shows as running.
pub fn stored_links(store: &discard_core::store::Store, channel: &discard_core::model::ChannelId, last_read: impl Fn(&str) -> Option<String>) -> Result<Vec<ThreadLinkDto>, String> {
    let mut links = Vec::new();
    for info in store.threads_for_channel(channel).map_err(|e| e.to_string())?.into_iter().filter(|info| info.followed) {
        let messages = store.messages_for_channel(&info.channel_id).map_err(|e| e.to_string())?;
        let record = ThreadRecord::build(&info, &messages, &HashMap::new());
        if record.visible() { links.push(record.link(last_read(&info.channel_id.0).as_deref(), info.state == Some(ThreadState::Running), 0)); }
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::model::{Block, ChannelId, ThreadKind};

    fn info() -> ThreadInfo {
        ThreadInfo { channel_id: ChannelId("t".into()), parent_channel_id: ChannelId("p".into()), anchor_message_id: Some(MessageId("a".into())), kind: ThreadKind::Question, followed: true, name: "Why?".into(), state: None, agent_id: discard_core::model::AgentId("g".into()), agent_name: "Gus".into() }
    }
    fn message(id: &str, role: Role, text: &str) -> Message {
        Message { id: MessageId(id.into()), channel_id: ChannelId("t".into()), role, blocks: vec![Block::Markdown { text: text.into() }] }
    }
    fn author(name: &str) -> MessageAuthor { MessageAuthor { agent_id: discard_core::model::AgentId("g".into()), name: name.into(), provider: "x".into(), avatar: None } }

    #[test]
    fn the_record_counts_user_and_agent_messages_and_names_the_last() {
        let messages = [message("1", Role::User, "why\n   is it   slow?"), message("2", Role::Agent, "Because."), message("3", Role::System, "marker")];
        let authors = HashMap::from([(MessageId("2".into()), author("Quinn"))]);
        let record = ThreadRecord::build(&info(), &messages, &authors);
        assert_eq!(record.count(), 2, "a system marker is not a thread message");
        let chip = record.chip(None, 0);
        assert_eq!(chip.last_message, Some(ThreadLastMessageDto { role: "agent".into(), author_name: Some("Quinn".into()), preview: "Because.".into() }));
        assert_eq!((chip.name.as_str(), chip.message_count, chip.followed), ("Why?", 2, true));
        let user_last = ThreadRecord::build(&info(), &messages[..1], &authors);
        assert_eq!(user_last.chip(None, 0).last_message.unwrap().preview, "why is it slow?", "whitespace collapses");
        assert_eq!(user_last.chip(None, 0).last_message.unwrap().author_name, None);
    }

    #[test]
    fn the_preview_is_cut_at_eighty_characters() {
        let record = ThreadRecord::build(&info(), &[message("1", Role::User, &"é".repeat(120))], &HashMap::new());
        let preview = record.chip(None, 0).last_message.unwrap().preview;
        assert_eq!(preview.chars().count(), PREVIEW_CHARS + 1);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn unread_means_an_agent_message_after_the_marker() {
        let messages = [message("1", Role::User, "q"), message("2", Role::Agent, "a"), message("3", Role::User, "q2")];
        let record = ThreadRecord::build(&info(), &messages, &HashMap::new());
        assert!(record.unread(None), "no marker: the reply is unread");
        assert!(record.unread(Some("1")));
        assert!(!record.unread(Some("2")));
        assert!(!record.unread(Some("3")));
        let waiting = ThreadRecord::build(&info(), &messages[..1], &HashMap::new());
        assert!(!waiting.unread(None), "a user message is never unread");
        assert_eq!(record.link(Some("2"), true, 1), ThreadLinkDto { id: "t".into(), parent_channel_id: "p".into(), name: "Why?".into(), unread: false, running: true, pending_approvals: 1, kind: "question".into(), state: None });
    }

    #[test]
    fn a_sub_agent_record_is_visible_from_its_start_and_takes_only_its_anchor_from_info() {
        let mut running = info();
        running.kind = ThreadKind::SubAgent;
        running.anchor_message_id = None;
        running.followed = false;
        running.state = Some(ThreadState::Running);
        running.agent_name = "Reviewer".into();
        let mut record = ThreadRecord::build(&running, &[], &HashMap::new());
        assert!(record.visible(), "a sub-agent chip shows before the brief is saved");
        assert!(!ThreadRecord::build(&info(), &[], &HashMap::new()).visible(), "an empty question thread does not");
        let chip = record.chip(None, 2);
        assert_eq!((chip.kind.as_str(), chip.state.as_deref(), chip.agent_name.as_str(), chip.pending_approvals, record.anchor.as_str()), ("sub_agent", Some("running"), "Reviewer", 2, ""));
        let mut done = running.clone();
        done.anchor_message_id = Some(MessageId("response".into()));
        done.state = Some(ThreadState::Done);
        record.set_anchor(&done);
        assert_eq!(record.anchor.as_str(), "response");
        assert_eq!((record.state, record.link(None, false, 0).state.as_deref()), (Some(ThreadState::Running), Some("running")), "the anchor does not change the state");
        // Info that the parent engine read before the sub-agent ended is older than the record: it must not undo the end.
        record.state = Some(ThreadState::Done);
        record.followed = true;
        let stale = running.clone();
        record.set_anchor(&stale);
        assert_eq!((record.state, record.followed), (Some(ThreadState::Done), true), "a final state and a follow survive older info");
    }
}
