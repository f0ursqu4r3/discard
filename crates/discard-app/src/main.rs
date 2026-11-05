//! Discard: an agent harness with a Discord-style interface.

mod args;
mod keys;
mod plugin_events;
mod plugin_memory;
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
use discard_ui::Relaunch;
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

fn run_app(
    options: &args::Options,
    data_dir: &Path,
    settings: SettingsStore,
    _relaunch: Relaunch,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = Arc::new(Mutex::new(settings));
    let project_dir = std::env::current_dir()?;
    let store = Arc::new(Mutex::new(Store::open(&data_dir.join("discard.db"))?));

    // No provider process survives a restart. Fail orphaned turns and expire their approvals.
    let report = lock(&store).reconcile(&HashSet::new())?;
    if !report.turns_failed.is_empty() {
        warn!(
            turns = report.turns_failed.len(),
            approvals = report.approvals_staled.len(),
            "reconciled turns left over from the previous run"
        );
    }

    if options.workspace == WorkspaceChoice::Worktree && !WorktreeManager::is_repo(&project_dir) {
        eprintln!("discard: --worktree needs a git repository, and {} is not inside one", project_dir.display());
        std::process::exit(2);
    }
    let workspace = discard_core::bootstrap::ensure_workspace(
        &lock(&store),
        &project_dir,
        &options.provider,
        if options.builder { Role::Builder } else { Role::Default },
        options.workspace,
    )?;
    let history = lock(&store).messages_for_channel(&workspace.channel.id)?;
    info!(project = %workspace.project.name, channel = %workspace.channel.name, "workspace ready");

    // The engine and provider processes run on tokio. GPUI owns the main thread.
    let runtime = tokio::runtime::Runtime::new()?;
    let _enter = runtime.enter();
    let mut provider_checks = Vec::new();
    let mut providers = Vec::new();
    for id in discard_core::bootstrap::PROVIDERS {
        let provider = provider_for(id)?;
        let setup = Arc::new(Mutex::new(None));
        providers.push(discard_ui::ProviderInfo { id: (*id).into(), matrix: provider.capabilities().clone(), setup: setup.clone() });
        provider_checks.push(((*id).to_string(), provider, setup));
    }
    let handle = runtime.handle().clone();
    let recheck: discard_ui::Recheck = Arc::new(move |id| {
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
        workspace.channel.agent_id.clone(),
        workspace.project.id.clone(),
    ));
    let manager = Arc::new(Mutex::new(PluginManager::with_host(PluginLimits::default(), Host::live(Some(plugin_memory)))));
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

    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (updates_tx, updates_rx) = mpsc::unbounded_channel();

    let git_workspace = Arc::new(GitWorkspace::new(&data_dir.join("worktrees"), Arc::clone(&store)));
    let engine = Engine {
        store: Arc::clone(&store),
        agent_id: workspace.channel.agent_id.clone(),
        project_id: workspace.project.id.clone(),
        memory: Some(memory),
        plugin_write: workspace.agent.plugin_write,
        workspace_mode: workspace.channel.workspace_mode,
        workspace: Some(git_workspace.clone()),
        observer: Some(Arc::new(plugin_events::PluginEvents::new(Arc::clone(&manager)))),
        channel_name: workspace.channel.name.clone(),
        provider: provider_for(&workspace.agent.provider)?,
        channel_id: workspace.channel.id.clone(),
        session_id: workspace.session_id.clone(),
        cwd: project_dir,
        model: workspace.agent.model.clone(),
        system_prompt: Some(workspace.agent.system_prompt.clone()).filter(|p| !p.is_empty()),
        policy: workspace.agent.policy,
    };
    runtime.spawn(engine.run(commands_rx, updates_tx));

    desktop::run(CoreInit {
        state: UiState::new(workspace.channel.id.clone(), history),
        commands: commands_tx,
        project: workspace.project.name,
        channel: workspace.channel.name,
        agent: workspace.agent.name,
        provider: workspace.agent.provider,
    }, updates_rx, runtime.handle().clone(), settings, plugins, data_dir.to_path_buf())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_settings_in_a_missing_directory_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("missing").join(".discard");
        let store = open_settings(&data_dir);
        assert_eq!(store.settings().general.default_provider, "claude");
        assert!(!tmp.path().join("missing").exists(), "opening the settings must not create the data directory");
    }
}
