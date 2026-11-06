//! Discard: an agent harness with a Discord-style interface.

mod args;
mod keys;
mod plugin_events;
use discard_app::plugin_memory;
mod relaunch;

use discard_core::bootstrap::{Role, WorkspaceChoice};
use discard_core::engine::Engine;
use discard_core::logging;
use discard_core::store::Store;
use discard_core::workspace::GitWorkspace;
use discard_core::worktree::WorktreeManager;
use discard_core::ui_state::UiState;
use discard_core::memory::MemoryStore;
use discard_lua::ledger::Ledger;
use discard_lua::{Host, PluginLimits, PluginManager, PluginService};
use discard_mcp::MemoryRuntime;
use discard_core::provider::Provider;
use discard_providers::claude::ClaudeProvider;
use discard_providers::codex::CodexProvider;
use discard_core::settings::SettingsStore;
use discard_app::desktop::actions::{ProviderInfo, Recheck, Relaunch};
use discard_app::desktop::{self, core::CoreInit};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// `$DISCARD_HOME`, or `~/.discard`.
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DISCARD_HOME") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".discard")
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The provider implementation for an agent's `provider` name.
fn provider_for(name: &str) -> Result<Arc<dyn Provider>, String> {
    match name {
        "claude" => Ok(Arc::new(ClaudeProvider::new("claude"))),
        "codex" => Ok(Arc::new(CodexProvider::new("codex"))),
        other => Err(format!("unknown provider `{other}`")),
    }
}

/// Read the settings from the data directory. This creates nothing: `SettingsStore::open` only reads.
/// A bad or missing file gives defaults, so it never stops startup.
fn open_settings(data_dir: &Path) -> SettingsStore {
    SettingsStore::open(&data_dir.join("settings.toml"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // Opening the store reads only. `--help` and bad arguments must not create the data directory.
    let data_dir = data_dir();
    let settings = open_settings(&data_dir);
    // Precedence: the `--provider` flag (handled in `parse`), then `$DISCARD_PROVIDER`, then settings.
    let default_provider =
        args::default_provider(std::env::var("DISCARD_PROVIDER").ok().as_deref(), &settings.settings().general.default_provider);
    let options = match args::parse(&argv, Some(&default_provider)) {
        Ok(args::Parsed::Run(options)) => options,
        Ok(args::Parsed::Help) => {
            print!("{}", args::USAGE);
            return Ok(());
        }
        Err(message) => {
            eprintln!("discard: {message}\n\n{}", args::USAGE);
            std::process::exit(2);
        }
    };
    logging::init(Some(&data_dir.join("logs")));
    for issue in settings.issues() {
        warn!(key = %issue.key, message = %issue.message, "settings issue");
    }
    std::fs::create_dir_all(&data_dir)?;

    // `run_app` owns the store, the memory runtime, the provider processes and the tokio runtime.
    // They are all dropped when it returns. A relaunch does not depend on that: on macOS the quit ends the process
    // inside the event loop. The relaunch waiter starts before the quit and waits for this process to exit.
    let relaunch: Relaunch = Arc::new(move |safe_mode| relaunch::spawn(&argv, safe_mode).map_err(|e| e.to_string()));
    run_app(&options, &data_dir, settings, relaunch)
}

/// Only the deliberate Builder path creates a Coding workspace.
fn startup_workspace(store: &Store, project_dir: &Path, options: &args::Options) -> discard_core::store::Result<Option<discard_core::bootstrap::Workspace>> {
    if options.builder { discard_core::bootstrap::ensure_workspace(store, project_dir, &options.provider, Role::Builder, options.workspace).map(Some) }
    else { Ok(None) }
}

fn run_app(
    options: &args::Options,
    data_dir: &Path,
    settings: SettingsStore,
    relaunch: Relaunch,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = Arc::new(Mutex::new(settings));
    let store = Arc::new(Mutex::new(Store::open(&data_dir.join("discard.db"))?));
    let selected_project = lock(&store).selected_project()?;
    let project_dir = if options.builder { std::env::current_dir()? } else { selected_project.as_ref().map(|p| PathBuf::from(&p.path)).unwrap_or_default() };

    // No provider process survives a restart. Fail orphaned turns and expire their approvals.
    let report = lock(&store).reconcile(&HashSet::new())?;
    if !report.turns_failed.is_empty() {
        warn!(
            turns = report.turns_failed.len(),
            approvals = report.approvals_staled.len(),
            "reconciled turns left over from the previous run"
        );
    }

    if options.builder && options.workspace == WorkspaceChoice::Worktree && !WorktreeManager::is_repo(&project_dir) {
        eprintln!("discard: --worktree needs a git repository, and {} is not inside one", project_dir.display());
        std::process::exit(2);
    }
    // Normal startup creates no project, agent, or conversation. Builder is an explicit Coding path.
    let workspace = startup_workspace(&lock(&store), &project_dir, options)?;
    let startup_channel = workspace.as_ref().map(|w| w.channel.clone());
    let sidebar_project = workspace.as_ref().map(|w| w.project.clone()).or(selected_project);

    // The engine and provider processes run on tokio. Tauri owns the main thread.
    let runtime = tokio::runtime::Runtime::new()?;
    let _enter = runtime.enter();
    let mut provider_checks = Vec::new();
    let mut providers = Vec::new();
    for id in discard_core::bootstrap::PROVIDERS {
        let provider = provider_for(id)?;
        let setup = Arc::new(Mutex::new(None));
        providers.push(ProviderInfo { provider: provider.clone(), id: (*id).into(), matrix: provider.capabilities().clone(), setup: setup.clone() });
        provider_checks.push(((*id).to_string(), provider, setup));
    }
    let handle = runtime.handle().clone();
    let recheck: Recheck = Arc::new(move |id| {
        if let Some((_, provider, setup)) = provider_checks.iter().find(|(name, _, _)| name == id) {
            *lock(setup) = None;
            let provider = provider.clone();
            let setup = setup.clone();
            handle.spawn(async move { let status = provider.check_setup().await; *lock(&setup) = Some(status); });
        }
    });
    for id in discard_core::bootstrap::PROVIDERS { recheck(id); }

    // Memory: markdown files are the source of truth. The FTS index can always be rebuilt from them.
    let memory_store = Arc::new(MemoryStore::open(&data_dir.join("memory"), &data_dir.join("memory-index.db"))?);
    match memory_store.reconcile() {
        Ok(stats) if stats.indexed + stats.removed > 0 => info!(indexed = stats.indexed, removed = stats.removed, "memory index reconciled"),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "could not reconcile the memory index"),
    }
    // Plugins. The bundled UI loads first. User plugins load only if their bytes match a signed approval.
    let keys = keys::choose(std::env::var("DISCARD_PLUGIN_PASSPHRASE").ok().as_deref(), data_dir, keys::system_store());
    let ledger = Ledger::open(&data_dir.join("plugins.db"), keys)?;
    // Approved plugins get the files and memory their manifests ask for. Drafts are tested in a manager with none.
    let plugin_memory = Arc::new(plugin_memory::PluginMemory::new(
        Arc::clone(&memory_store),
        startup_channel.as_ref().map(|c| c.agent_id.clone()).unwrap_or_else(|| discard_core::model::AgentId(String::new())),
        startup_channel.as_ref().and_then(|c| c.project_id.clone()),
    ));
    let manager = Arc::new(Mutex::new(PluginManager::with_host(PluginLimits::default(), Host::live(Some(plugin_memory.clone())))));
    manager.lock().unwrap_or_else(|p| p.into_inner()).load(discard_lua::bundled::default_ui())?;
    let plugins = Arc::new(PluginService::new(data_dir, ledger, Arc::clone(&manager), options.safe_mode)?);
    let load_report = plugins.load_all();
    info!(
        loaded = ?load_report.loaded,
        modified_outside_review = ?load_report.modified_outside_review,
        trusted_updates = ?load_report.trusted_updates,
        reverted = ?load_report.reverted,
        safe_mode = load_report.safe_mode,
        key_problem = ?load_report.key_problem,
        "user plugins loaded"
    );

    let memory = runtime.block_on(MemoryRuntime::start_with_plugins(
        Arc::clone(&memory_store),
        &data_dir.join("run"),
        Some(Arc::clone(&plugins)),
    ))?;

    let git_workspace = Arc::new(GitWorkspace::new(&data_dir.join("worktrees"), Arc::clone(&store)));
    let launcher: desktop::channels::Launcher = {
        let store = store.clone(); let git_workspace = git_workspace.clone(); let memory = memory.clone();
        let manager = manager.clone(); let handle = runtime.handle().clone();
        let conversation_dir = data_dir.join("conversations");
        let session_mode = if options.builder { discard_core::provider::SessionMode::Coding } else { discard_core::provider::SessionMode::Conversation };
        Arc::new(move |channel| {
            let project = channel.project_id.as_ref().map(|id| {
                lock(&store).list_projects().map_err(|e| e.to_string())?.into_iter().find(|p| &p.id == id).ok_or_else(|| "Project unavailable".to_string())
            }).transpose()?;
            let project_dir = project.as_ref().map(|p| PathBuf::from(&p.path)).unwrap_or_else(|| conversation_dir.clone());
            let (agent, session_id, history) = {
                let store = lock(&store);
                (store.load_agent(&channel.agent_id).map_err(|e| e.to_string())?,
                 store.latest_session(&channel.id).map_err(|e| e.to_string())?.ok_or("This channel has no session")?,
                 store.messages_for_channel(&channel.id).map_err(|e| e.to_string())?)
            };
            let read_only = {
                let store = lock(&store);
                store.agent_archived(&channel.agent_id).unwrap_or(true) || store.channel_archived(&channel.id).map_err(|e| e.to_string())?
            };
            if read_only {
                let (commands, _) = mpsc::unbounded_channel();
                let (_, updates) = mpsc::unbounded_channel();
                return Ok((CoreInit { state: UiState::new(channel.id.clone(), history), commands, project: project.as_ref().map(|p| p.name.clone()).unwrap_or_default(), channel: channel.name.clone(), agent: agent.name, provider: agent.provider }, updates));
            }
            let provider = provider_for(&agent.provider)?;
            let (commands_tx, commands_rx) = mpsc::unbounded_channel();
            let (updates_tx, updates_rx) = mpsc::unbounded_channel();
            let engine = Engine {
        reasoning_effort: agent.reasoning_effort.clone(),
        mode: session_mode, providers: discard_core::bootstrap::PROVIDERS.iter().filter_map(|id| provider_for(id).ok()).collect(),
                store: store.clone(), agent_id: channel.agent_id.clone(), project_id: channel.project_id.clone(),
                memory: Some(memory.clone()), plugin_write: agent.plugin_write,
                workspace_mode: channel.workspace_mode, workspace: Some(git_workspace.clone()),
                observer: Some(Arc::new(plugin_events::PluginEvents::with_context(manager.clone(), Some((channel.agent_id.clone(), channel.project_id.clone()))))),
                channel_name: channel.name.clone(), provider, channel_id: channel.id.clone(), session_id,
                cwd: project_dir.clone(), model: agent.model.clone(),
                system_prompt: Some(agent.runtime_system_prompt()), policy: agent.policy,
            };
            handle.spawn(engine.run(commands_rx, updates_tx));
            Ok((CoreInit { state: UiState::new(channel.id.clone(), history), commands: commands_tx,
                project: project.as_ref().map(|p|p.name.clone()).unwrap_or_default(), channel: channel.name.clone(), agent: agent.name, provider: agent.provider }, updates_rx))
        })
    };
    let on_ready: desktop::channels::OnReady = Arc::new(move |core| {
        let project_core = Arc::downgrade(core);
        plugin_memory.set_project_source(Arc::new(move || project_core.upgrade().and_then(|core| core.active_context().1)));
        let core = Arc::downgrade(core);
        plugin_memory.set_active_source(Arc::new(move || discard_core::model::AgentId(core.upgrade().map(|core| core.active_agent_id()).unwrap_or_default())));
    });
    let (init, updates_rx) = if let Some(channel) = &startup_channel { launcher(channel)? } else {
        let (commands, _) = mpsc::unbounded_channel();
        let (_, updates) = mpsc::unbounded_channel();
        (CoreInit { state: UiState::new(discard_core::model::ChannelId(String::new()), vec![]), commands, project: String::new(), channel: String::new(), agent: String::new(), provider: String::new() }, updates)
    };
    desktop::run(init, updates_rx, runtime.handle().clone(), settings, plugins, data_dir.to_path_buf(), desktop::DesktopConfig { startup_channel, sidebar_project, load_report, workspace: git_workspace, store, relaunch, providers, recheck, memory: memory_store, launcher, on_ready })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_startup_creates_no_project_agent_or_conversation() {
        let store = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let options = args::Options { provider: "claude".into(), safe_mode: false, builder: false, workspace: WorkspaceChoice::Auto };
        assert!(startup_workspace(&store, dir.path(), &options).unwrap().is_none());
        assert!(store.list_projects().unwrap().is_empty());
        assert!(store.all_agents().unwrap().is_empty());
        assert!(store.all_conversations().unwrap().is_empty());
    }
    #[test]
    fn opening_settings_in_a_missing_directory_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("missing").join(".discard");
        let store = open_settings(&data_dir);
        assert_eq!(store.settings().general.default_provider, "claude");
        assert!(!tmp.path().join("missing").exists(), "opening the settings must not create the data directory");
    }
}
