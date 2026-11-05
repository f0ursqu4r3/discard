//! `MemoryRuntime`: the MCP server, the token registry, and per-session config files, behind the engine's `MemoryService`.

use crate::identity::{Identity, ScopeKindOrd, Surface, TokenRegistry};
use crate::server::McpServer;
use crate::tools::McpHandler;
use discard_core::engine::{MemoryService, PreparedMemory};
use discard_core::memory::{MemoryError, MemoryStore, Scope, compose_system_prompt};
use discard_lua::PluginService;
use discard_core::model::{AgentId, ProjectId};
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A session token lives at most this long, even if `finish` is never called.
pub const TOKEN_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// What to clean up when a session ends.
struct SessionFiles {
    path: PathBuf,
    /// Tokens other than the memory token (the plugin-tools token).
    extra_tokens: Vec<String>,
}

pub struct MemoryRuntime {
    store: Arc<MemoryStore>,
    registry: Arc<TokenRegistry>,
    url: String,
    runtime_dir: PathBuf,
    files: Mutex<HashMap<String, SessionFiles>>,
    server: tokio::sync::Mutex<Option<McpServer>>,
}

impl MemoryRuntime {
    /// Start the MCP server. Config files go in `runtime_dir`, which is created with mode 0700.
    pub async fn start(store: Arc<MemoryStore>, runtime_dir: &Path) -> std::io::Result<Arc<Self>> {
        Self::start_with_plugins(store, runtime_dir, None).await
    }

    /// Like `start`, and also serves the plugin-authoring tools to agents with `plugin.write`.
    pub async fn start_with_plugins(
        store: Arc<MemoryStore>,
        runtime_dir: &Path,
        plugins: Option<Arc<PluginService>>,
    ) -> std::io::Result<Arc<Self>> {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(runtime_dir)?;
        // `create` keeps an existing directory's mode, so set it again.
        std::fs::set_permissions(runtime_dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
        let registry = Arc::new(TokenRegistry::new());
        let mut handler = McpHandler::new(Arc::clone(&store));
        if let Some(plugins) = plugins {
            handler = handler.with_plugins(plugins);
        }
        let server = McpServer::start(Arc::new(handler), Arc::clone(&registry)).await?;
        Ok(Arc::new(Self {
            store,
            registry,
            url: server.url(),
            runtime_dir: runtime_dir.to_path_buf(),
            files: Mutex::new(HashMap::new()),
            server: tokio::sync::Mutex::new(Some(server)),
        }))
    }

    pub fn registry(&self) -> &Arc<TokenRegistry> {
        &self.registry
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub async fn shutdown(&self) {
        let server = self.server.lock().await.take();
        if let Some(server) = server {
            server.shutdown().await;
        }
    }
}

/// Keep the authoring guide in the agent's own core memory so it is part of every session prompt.
fn ensure_guide(store: &MemoryStore, agent: &AgentId) -> Result<(), String> {
    const PATH: &str = "core/plugin-guide.md";
    let scope = Scope::Agent(agent.clone());
    let guide = discard_lua::api_spec::guide();
    match store.read(&scope, PATH) {
        Ok(current) if current.text == guide => Ok(()),
        Ok(current) => store.replace(&scope, PATH, &guide, &current.version).map(|_| ()).map_err(|e| format!("could not update the plugin guide: {e}")),
        Err(MemoryError::NotFound(_)) => store.create(&scope, PATH, &guide).map(|_| ()).map_err(|e| format!("could not save the plugin guide: {e}")),
        Err(e) => Err(format!("could not read the plugin guide: {e}")),
    }
}

impl MemoryService for MemoryRuntime {
    fn prepare(&self, agent: &AgentId, project: Option<&ProjectId>, source: &discard_core::model::ChannelId, base_prompt: Option<&str>, plugin_write: bool) -> Result<PreparedMemory, String> {
        if plugin_write {
            ensure_guide(&self.store, agent)?;
        }
        let scopes = Scope::applicable(agent, project);
        let blocks = self.store.core_blocks(&scopes).map_err(|e| format!("could not read memory: {e}"))?;
        let system_prompt = compose_system_prompt(base_prompt, &blocks);

        let identity = |surface: Surface, scopes: &[ScopeKindOrd]| Identity {
            agent: agent.clone(),
            project: project.cloned(),
            source_conversation: Some(source.clone()),
            write_scopes: [if project.is_some() {ScopeKindOrd::AgentProject} else {ScopeKindOrd::Agent}].into(),
            surface,
            scopes: scopes.iter().copied().collect(),
            can_write: true,
        };
        let token = self.registry.issue(identity(Surface::Memory, &Scope::applicable(agent,project).iter().map(|s|s.kind().into()).collect::<Vec<_>>()), TOKEN_TTL)?;
        let mut extra_tokens = Vec::new();
        let mut servers = serde_json::Map::new();
        servers.insert(
            "discard-memory".into(),
            serde_json::json!({"type": "http", "url": self.url, "headers": {"Authorization": format!("Bearer {token}")}}),
        );
        if plugin_write {
            // A separate connection with its own token, so memory access never grants plugin access.
            let plugin_token = match self.registry.issue(identity(Surface::Plugins, &[]), TOKEN_TTL) {
                Ok(t) => t,
                Err(e) => {
                    self.registry.revoke(&token);
                    return Err(e);
                }
            };
            servers.insert(
                "discard-plugins".into(),
                serde_json::json!({"type": "http", "url": self.url, "headers": {"Authorization": format!("Bearer {plugin_token}")}}),
            );
            extra_tokens.push(plugin_token);
        }
        let config = serde_json::json!({"mcpServers": servers});
        let path = self.runtime_dir.join(format!("session-{}.json", uuid::Uuid::new_v4()));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut f| f.write_all(config.to_string().as_bytes()));
        if let Err(e) = written {
            self.registry.revoke(&token);
            for t in &extra_tokens {
                self.registry.revoke(t);
            }
            let _ = std::fs::remove_file(&path);
            return Err(format!("could not write the session config: {e}"));
        }
        self.files.lock().unwrap_or_else(|p| p.into_inner()).insert(token.clone(), SessionFiles { path: path.clone(), extra_tokens });
        Ok(PreparedMemory { system_prompt, mcp_config: path, token })
    }

    fn finish(&self, token: &str) {
        self.registry.revoke(token);
        let files = self.files.lock().unwrap_or_else(|p| p.into_inner()).remove(token);
        if let Some(files) = files {
            for t in &files.extra_tokens {
                self.registry.revoke(t);
            }
            let _ = std::fs::remove_file(files.path);
        }
    }
}
