//! Explicit recipients in new user messages. History, topics and model output never call this parser.
use crate::{model::{AgentId, ChannelId}, store::Store};

pub fn names(text: &str) -> Vec<String> {
    let pattern = regex::Regex::new(r#"(?:^|[\s(])@(?:"([^"]+)"|([\p{L}\p{N}_-]+))"#).unwrap();
    let mut found = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        if line.starts_with("    ") || line.starts_with('\t') { continue; }
        let trimmed = line.trim_start();
        let first = trimmed.chars().next().unwrap_or(' ');
        let count = trimmed.chars().take_while(|c| *c == first).count();
        if matches!(first, '`' | '~') && count >= 3 {
            match fence {
                None => fence = Some((first, count)),
                Some((marker, length)) if marker == first && count >= length => fence = None,
                _ => {},
            }
            continue;
        }
        if fence.is_some() || trimmed.starts_with('>') { continue; }
        // Mask inline code and escaped @ signs while retaining whitespace boundaries.
        let mut plain = String::new();
        let mut ticks = 0;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' { plain.push('x'); if chars.next().is_some() { plain.push('x'); } continue; }
            if c == '`' {
                let mut width = 1;
                while chars.peek() == Some(&'`') { chars.next(); width += 1; }
                if ticks == 0 { ticks = width; } else if ticks == width { ticks = 0; }
                plain.push('x');
            } else { plain.push(if ticks == 0 { c } else { 'x' }); }
        }
        for captures in pattern.captures_iter(&plain) {
            let name = captures.get(1).or_else(|| captures.get(2)).unwrap().as_str().to_owned();
            if !found.iter().any(|candidate: &String| candidate.to_lowercase() == name.to_lowercase()) { found.push(name); }
        }
    }
    found
}

/// Resolve the current project default at send time; saved channel identity is a legacy fallback.
/// A thread answers as the author of its anchor message when that agent is active. Otherwise it uses the
/// default of its parent channel.
pub fn default_agent(store: &Store, channel: &crate::model::Channel) -> Result<AgentId, String> {
    if let Some(thread) = store.thread_info(&channel.id).map_err(|e| e.to_string())? {
        let parent = store.load_channel(&thread.parent_channel_id).map_err(|e| e.to_string())?;
        return anchor_agent(store, &parent, &thread.anchor_message_id);
    }
    let main = channel.project_id.as_ref().map(|project| store.project_main_agent(project)).transpose().map_err(|e| e.to_string())?.flatten();
    Ok(main.unwrap_or_else(|| channel.agent_id.clone()))
}

/// The default responder of a thread on `anchor` in `parent`: the anchor's author when that agent is active,
/// otherwise the default of the parent channel. Use this before the thread row exists.
pub fn anchor_agent(store: &Store, parent: &crate::model::Channel, anchor: &crate::model::MessageId) -> Result<AgentId, String> {
    let author = store.message_author(anchor).map_err(|e| e.to_string())?;
    if let Some(author) = author.filter(|a| store.agent_archived(&a.agent_id).is_ok_and(|archived| !archived)) { return Ok(author.agent_id); }
    default_agent(store, parent)
}

pub fn recipient(store: &Store, channel: &ChannelId, text: &str) -> Result<AgentId, String> {
    if let Some(agent) = resolve(store, channel, text)? { return Ok(agent); }
    default_agent(store, &store.load_channel(channel).map_err(|e| e.to_string())?)
}

pub fn resolve(store: &Store, channel: &ChannelId, text: &str) -> Result<Option<AgentId>, String> {
    let names = names(text);
    if names.is_empty() { return Ok(None); }
    if names.len() > 1 { return Err("Mention one agent per message.".into()); }
    let agents = store.list_agents().map_err(|e| e.to_string())?;
    let matching: Vec<_> = agents.iter().filter(|(_, config)| config.name.to_lowercase() == names[0].to_lowercase()).collect();
    let (id, config) = match matching.as_slice() {
        [] => return Err(format!("No active configured agent is named {}. Choose an agent from the mention list.", names[0])),
        [agent] => *agent,
        _ => return Err(format!("More than one agent is named {}. Give them distinct names before mentioning one.", names[0])),
    };
    if config.model.as_ref().is_none_or(|model| model.trim().is_empty()) { return Err("Select a model for the mentioned agent first.".into()); }
    let channel = store.load_channel(channel).map_err(|e| e.to_string())?;
    if channel.project_id.is_none() && &channel.agent_id != id { return Err("Mention other agents in a project channel, or open their direct conversation.".into()); }
    Ok(Some(id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mentions_are_explicit_and_skip_quoted_code_and_email() {
        assert_eq!(names("@Jordan what do you think?"), vec!["Jordan"]);
        assert_eq!(names("Ask @\"Jordan Lee\" then @\"Jordan Lee\""), vec!["Jordan Lee"]);
        assert!(names("mail@Jordan.com\n> @Jordan\n`@Jordan`\n```sh\n@Jordan\n```\n    @Jordan\n\\@Jordan").is_empty());
        assert_eq!(names("`@Ignored` ask @Jordan"), vec!["Jordan"]);
        assert_eq!(names("    ```\n@Jordan"), vec!["Jordan"]);
    }
    fn agent(store: &Store, name: &str) -> AgentId {
        let id = AgentId::new();
        store.insert_agent(&id, &crate::agent::AgentConfig {
            workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: name.into(), avatar: None, provider: "fake".into(),
            model: Some("m".into()), system_prompt: String::new(), policy: crate::capability::ApprovalPolicy::Ask, plugin_write: false,
        }).unwrap();
        id
    }

    fn said(store: &Store, channel: &ChannelId, role: crate::model::Role, author: Option<&AgentId>) -> crate::model::MessageId {
        let message = crate::model::Message { id: crate::model::MessageId::new(), channel_id: channel.clone(), role, blocks: vec![crate::model::Block::Markdown { text: "x".into() }] };
        match author {
            Some(agent) => store.insert_authored_message(&message, &crate::model::MessageAuthor { agent_id: agent.clone(), name: "n".into(), provider: "fake".into(), avatar: None }).unwrap(),
            None => store.insert_message(&message).unwrap(),
        }
        message.id
    }

    #[test]
    fn a_thread_answers_as_the_anchor_author_or_else_as_the_parent_default() {
        let store = Store::open_in_memory().unwrap();
        let project = crate::model::Project { id: crate::model::ProjectId::new(), name: "p".into(), path: "/p".into() };
        store.insert_project(&project).unwrap();
        let (main, other) = (agent(&store, "Main"), agent(&store, "Other"));
        store.set_project_main_agent(&project.id, &main).unwrap();
        let parent = store.create_conversation(&other, Some(&project.id), "work").unwrap();
        let by_other = said(&store, &parent.id, crate::model::Role::Agent, Some(&other));
        let by_user = said(&store, &parent.id, crate::model::Role::User, None);
        let on_agent = store.create_thread(&parent.id, &by_other, "a", &other).unwrap();
        let on_user = store.create_thread(&parent.id, &by_user, "u", &main).unwrap();
        assert_eq!(default_agent(&store, &parent).unwrap(), main, "the parent still uses the project main agent");
        assert_eq!(default_agent(&store, &on_agent).unwrap(), other, "the anchor author answers in the thread");
        assert_eq!(default_agent(&store, &on_user).unwrap(), main, "a user anchor uses the main agent");
        assert_eq!(recipient(&store, &on_agent.id, "and then?").unwrap(), other);
        assert_eq!(recipient(&store, &on_agent.id, "@Main and then?").unwrap(), main, "a mention overrides for one response");
        assert_eq!(recipient(&store, &on_agent.id, "and again?").unwrap(), other);
        store.set_agent_archived(&other, true).unwrap();
        assert_eq!(default_agent(&store, &on_agent).unwrap(), main, "an archived anchor author falls back to the parent default");
    }
}
