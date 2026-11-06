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
pub fn default_agent(store: &Store, channel: &crate::model::Channel) -> Result<AgentId, String> {
    let main = channel.project_id.as_ref().map(|project| store.project_main_agent(project)).transpose().map_err(|e| e.to_string())?.flatten();
    Ok(main.unwrap_or_else(|| channel.agent_id.clone()))
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
}
