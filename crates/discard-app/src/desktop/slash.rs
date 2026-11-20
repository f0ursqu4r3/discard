//! What each built-in slash command does. Pure; the Tauri command executes the step.
use super::core::CommandChannel;
use discard_core::{action::Action, agent::AgentConfig, model::{AgentId, ChannelId}, provider::ModelInfo, slash::{Builtin, Entry, help_text, parse_model_args}, store::Store};

/// What a built-in command does. Pure; the Tauri command executes it.
#[derive(Debug, PartialEq)]
pub enum Step {
    Output(String),
    Dispatch(Action),
    Error(String),
    /// `/model @agent [model]`. The model is `None` when the command only asks for the current one.
    Model(Option<String>),
    /// `/clear`: start a new provider session with no earlier context.
    Clear,
}

pub fn builtin_step(builtin: Builtin, channel: &CommandChannel, entries: &[Entry], args: &str) -> Step {
    let name = builtin.name();
    if builtin.planned() { return Step::Error(format!("/{name} is not available yet.")); }
    match builtin {
        Builtin::Help => Step::Output(help_text(entries)),
        Builtin::Stop if !channel.turn_running => Step::Error("No turn is running.".into()),
        Builtin::Archive if channel.is_thread => Step::Error("/archive does not apply to a thread.".into()),
        Builtin::Archive if channel.turn_running => Step::Error("Stop the turn before archiving the channel".into()),
        // A thread has no provider session of its own, so there is nothing to clear in it.
        Builtin::Clear if channel.is_thread => Step::Error("/clear does not apply to a thread.".into()),
        Builtin::Clear if channel.turn_running => Step::Error("Stop the turn before clearing the session.".into()),
        Builtin::Clear if !args.is_empty() => Step::Error("/clear takes no arguments.".into()),
        Builtin::Clear => Step::Clear,
        Builtin::Model => match parse_model_args(args) { Ok(model) => Step::Model(model), Err(error) => Step::Error(error) },
        other => match other.action() {
            Some(action) => Step::Dispatch(action),
            None => Step::Error(format!("/{name} is not available yet.")),
        },
    }
}

/// The agent that `/model` changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTarget { pub id: AgentId, pub name: String, pub provider: String, pub model: Option<String> }

/// Resolve the mention in `args` with the mention rules of a message in `channel`.
pub fn model_target(store: &Store, channel: &ChannelId, args: &str) -> Result<ModelTarget, String> {
    let id = discard_core::mentions::resolve(store, channel, args)?.ok_or("Mention the agent: /model @name <model>")?;
    let AgentConfig { name, provider, model, .. } = store.load_agent(&id).map_err(|e| e.to_string())?;
    Ok(ModelTarget { id, name, provider, model })
}

fn model_ids(models: &[ModelInfo]) -> String { models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>().join(", ") }

/// The reply to `/model @agent` with no model: the current model and the models of the provider.
pub fn model_listing(target: &ModelTarget, models: &Result<Vec<ModelInfo>, String>) -> String {
    let mut text = format!("{} uses {}.", target.name, target.model.as_deref().unwrap_or("the provider default"));
    match models {
        Ok(models) if !models.is_empty() => {
            text.push_str(&format!("\nModels of {}:", target.provider));
            for model in models {
                text.push_str(&format!("\n- {}", model.id));
                if !model.name.is_empty() && model.name != model.id { text.push_str(&format!(" ({})", model.name)); }
            }
        }
        _ => text.push_str(&format!("\n{} does not list its models.", target.provider)),
    }
    text
}

/// Accept `model` when the provider lists no models, or when the list holds its exact ID.
pub fn check_model(target: &ModelTarget, model: &str, models: &Result<Vec<ModelInfo>, String>) -> Result<(), String> {
    match models {
        Ok(models) if !models.is_empty() && !models.iter().any(|m| m.id == model) => Err(format!("{model} is not a model of {}. Available: {}.", target.provider, model_ids(models))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::{capability::ApprovalPolicy, model::{Project, ProjectId}};

    fn ch(turn_running: bool, is_thread: bool) -> CommandChannel { CommandChannel { turn_running, archived: false, is_thread } }
    fn entries() -> Vec<Entry> { discard_core::slash::registry(&[]).0 }
    fn step(builtin: Builtin, channel: CommandChannel, args: &str) -> Step { builtin_step(builtin, &channel, &entries(), args) }

    #[test] fn model_asks_for_a_mention_and_passes_the_model_on() {
        assert_eq!(step(Builtin::Model, ch(false, false), ""), Step::Error("Mention the agent: /model @name <model>".into()));
        assert_eq!(step(Builtin::Model, ch(false, false), "gpt-5"), Step::Error("Mention the agent: /model @name <model>".into()));
        assert_eq!(step(Builtin::Model, ch(false, false), "@Ann"), Step::Model(None));
        assert_eq!(step(Builtin::Model, ch(false, false), "@Ann gpt-5"), Step::Model(Some("gpt-5".into())));
        assert_eq!(step(Builtin::Model, ch(true, true), "@Ann gpt-5"), Step::Model(Some("gpt-5".into())), "the executor checks the agent, not the channel");
    }
    #[test] fn clear_is_refused_in_a_thread_during_a_turn_and_with_arguments() {
        assert_eq!(step(Builtin::Clear, ch(true, true), ""), Step::Error("/clear does not apply to a thread.".into()));
        assert_eq!(step(Builtin::Clear, ch(true, false), ""), Step::Error("Stop the turn before clearing the session.".into()));
        assert_eq!(step(Builtin::Clear, ch(false, false), "now"), Step::Error("/clear takes no arguments.".into()));
        assert_eq!(step(Builtin::Clear, ch(false, false), ""), Step::Clear);
    }
    #[test] fn stop_needs_a_running_turn() {
        assert_eq!(step(Builtin::Stop, ch(false, false), ""), Step::Error("No turn is running.".into()));
        assert_eq!(step(Builtin::Stop, ch(true, true), ""), Step::Dispatch(Action::CancelTurn));
    }
    #[test] fn archive_is_refused_in_a_thread_and_during_a_turn() {
        assert_eq!(step(Builtin::Archive, ch(false, true), ""), Step::Error("/archive does not apply to a thread.".into()));
        assert_eq!(step(Builtin::Archive, ch(true, false), ""), Step::Error("Stop the turn before archiving the channel".into()));
        assert_eq!(step(Builtin::Archive, ch(false, false), ""), Step::Dispatch(Action::ArchiveChannel));
    }
    #[test] fn help_outputs_the_registry_and_the_rest_dispatch() {
        assert!(matches!(step(Builtin::Help, ch(false, false), ""), Step::Output(text) if text.contains("/stop") && !text.contains("Soon")));
        assert_eq!(step(Builtin::Settings, ch(true, false), ""), Step::Dispatch(Action::OpenSettings));
        assert_eq!(step(Builtin::Members, ch(false, true), ""), Step::Dispatch(Action::ToggleMemberList));
        assert_eq!(step(Builtin::Plugins, ch(false, false), ""), Step::Dispatch(Action::ReviewPlugins));
    }

    fn info(id: &str, name: &str) -> ModelInfo { ModelInfo { id: id.into(), name: name.into(), description: String::new(), reasoning_efforts: vec![] } }
    fn target() -> ModelTarget { ModelTarget { id: AgentId("a".into()), name: "Ann".into(), provider: "claude".into(), model: Some("opus".into()) } }

    #[test] fn the_listing_shows_the_current_model_and_the_provider_models() {
        let models = Ok(vec![info("opus", "Opus"), info("sonnet", "Sonnet 4"), info("same", "same")]);
        assert_eq!(model_listing(&target(), &models), "Ann uses opus.\nModels of claude:\n- opus (Opus)\n- sonnet (Sonnet 4)\n- same");
        assert_eq!(model_listing(&target(), &Err("no list".into())), "Ann uses opus.\nclaude does not list its models.");
        assert_eq!(model_listing(&target(), &Ok(vec![])), "Ann uses opus.\nclaude does not list its models.");
    }
    #[test] fn a_model_is_checked_against_the_list_only_when_there_is_one() {
        let models = Ok(vec![info("opus", "Opus"), info("sonnet", "Sonnet")]);
        assert_eq!(check_model(&target(), "sonnet", &models), Ok(()));
        assert_eq!(check_model(&target(), "gpt", &models), Err("gpt is not a model of claude. Available: opus, sonnet.".into()));
        assert_eq!(check_model(&target(), "Sonnet", &models), Err("Sonnet is not a model of claude. Available: opus, sonnet.".into()), "an ID, not a display name");
        assert_eq!(check_model(&target(), "anything", &Err("no list".into())), Ok(()));
        assert_eq!(check_model(&target(), "anything", &Ok(vec![])), Ok(()));
    }
    #[test] fn the_mention_resolves_to_one_agent_by_the_message_rules() {
        let store = Store::open_in_memory().unwrap();
        let config = |name: &str| AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: name.into(), avatar: None, provider: "claude".into(), model: Some("opus".into()), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false };
        let (ann, bob) = (store.create_agent(&config("Ann")).unwrap(), store.create_agent(&config("Bob")).unwrap());
        let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
        store.insert_project(&project).unwrap();
        let channel = store.create_conversation(&ann, Some(&project.id), "work").unwrap();
        let found = model_target(&store, &channel.id, "@bob sonnet").unwrap();
        assert_eq!((found.id, found.name.as_str(), found.provider.as_str(), found.model.as_deref()), (bob, "Bob", "claude", Some("opus")));
        assert!(model_target(&store, &channel.id, "@Nobody x").unwrap_err().contains("No active configured agent is named Nobody"));
        store.set_agent_archived(&ann, true).unwrap();
        assert!(model_target(&store, &channel.id, "@Ann x").unwrap_err().contains("No active configured agent"), "an archived agent is not a target");
        assert_eq!(model_target(&store, &channel.id, "x").unwrap_err(), "Mention the agent: /model @name <model>");
    }
}
