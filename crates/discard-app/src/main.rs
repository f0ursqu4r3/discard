//! Discard: an agent harness with a Discord-style interface.

use discard_core::engine::Engine;
use discard_core::logging;
use discard_core::store::Store;
use discard_core::ui_state::UiState;
use discard_lua::{PluginLimits, PluginManager};
use discard_providers::claude::ClaudeProvider;
use discard_ui::ViewData;
use std::collections::HashSet;
use std::path::PathBuf;
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    logging::init();

    let data_dir = data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let project_dir = std::env::current_dir()?;
    let store = Store::open(&data_dir.join("discard.db"))?;

    // No provider process survives a restart. Fail orphaned turns and expire their approvals.
    let report = store.reconcile(&HashSet::new())?;
    if !report.turns_failed.is_empty() {
        warn!(
            turns = report.turns_failed.len(),
            approvals = report.approvals_staled.len(),
            "reconciled turns left over from the previous run"
        );
    }

    let workspace = discard_core::bootstrap::ensure_workspace(&store, &project_dir)?;
    let history = store.messages_for_channel(&workspace.channel.id)?;
    info!(project = %workspace.project.name, channel = %workspace.channel.name, "workspace ready");

    // The engine and provider processes run on tokio. GPUI owns the main thread.
    let runtime = tokio::runtime::Runtime::new()?;
    let _enter = runtime.enter();

    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (updates_tx, updates_rx) = mpsc::unbounded_channel();

    let engine = Engine {
        store: Arc::new(Mutex::new(store)),
        provider: Arc::new(ClaudeProvider::new("claude")),
        channel_id: workspace.channel.id.clone(),
        session_id: workspace.session_id.clone(),
        cwd: project_dir,
        model: workspace.agent.model.clone(),
        system_prompt: Some(workspace.agent.system_prompt.clone()).filter(|p| !p.is_empty()),
        policy: workspace.agent.policy,
    };
    runtime.spawn(engine.run(commands_rx, updates_tx));

    // Bundled plugins first. User plugins arrive with the approval flow in M5.
    let mut plugins = PluginManager::new(PluginLimits::default());
    plugins.load(discard_lua::bundled::default_ui())?;

    discard_ui::run(
        ViewData {
            project_name: workspace.project.name,
            channel_name: workspace.channel.name,
            agent_name: workspace.agent.name,
            provider_label: workspace.agent.provider,
        },
        plugins,
        UiState::new(workspace.channel.id, history),
        commands_tx,
        updates_rx,
    );
    Ok(())
}
