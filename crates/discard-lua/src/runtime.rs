//! One sandboxed Luau state per plugin, each on its own OS thread.
//! Limits: CPU budget per call, memory cap, bounded queue. A violation terminates the plugin.

use crate::element::{self, Element, ElementError};
use crate::manifest::{API_VERSION, Capabilities, Manifest};
use crate::queue::{BoundedQueue, Kind, PushOutcome};
use crate::snapshot::Snapshot;
use crate::theme::ThemeOverrides;
use mlua::{Function, Lua, LuaSerdeExt as _, Table, Value, VmState};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct PluginLimits {
    /// CPU time allowed for loading and for each call.
    pub call_budget: Duration,
    pub memory_bytes: usize,
    pub queue_capacity: usize,
}

impl Default for PluginLimits {
    fn default() -> Self {
        Self { call_budget: Duration::from_millis(250), memory_bytes: 32 * 1024 * 1024, queue_capacity: 64 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Violation {
    Cpu,
    Memory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Running,
    Terminated(String),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PluginError {
    #[error("plugin failed to load: {0}")]
    Load(String),
    #[error("plugin error: {0}")]
    Runtime(String),
    #[error("plugin exceeded its {0:?} limit and was terminated")]
    Violation(Violation),
    #[error("plugin is terminated: {0}")]
    Terminated(String),
    #[error("plugin queue is full")]
    QueueFull,
    #[error("plugin did not answer in time")]
    Timeout,
    #[error("no such {kind} `{name}`")]
    NotRegistered { kind: &'static str, name: String },
    #[error("invalid element: {0}")]
    Element(#[from] ElementError),
}

/// What a plugin registered while loading.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedInfo {
    pub manifest: Manifest,
    pub hash: String,
    pub theme: ThemeOverrides,
    pub slots: Vec<String>,
    pub commands: Vec<String>,
    pub actions: Vec<String>,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HookResult {
    pub block: bool,
    pub reason: Option<String>,
}

enum Request {
    Render { slot: String, state: serde_json::Value, reply: Sender<Result<Element, PluginError>> },
    Command { name: String, args: String, reply: Sender<Result<Option<String>, PluginError>> },
    Action { id: String, reply: Sender<Result<(), PluginError>> },
    Hook { event: String, payload: serde_json::Value, reply: Sender<Result<HookResult, PluginError>> },
    Event { event: String, payload: serde_json::Value },
}

pub struct PluginHandle {
    queue: Arc<BoundedQueue<Request>>,
    status: Arc<Mutex<Status>>,
    thread: Option<JoinHandle<()>>,
    call_budget: Duration,
}

impl std::fmt::Debug for PluginHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PluginHandle")
    }
}

fn set_status(status: &Mutex<Status>, new: Status) {
    *status.lock().unwrap_or_else(|p| p.into_inner()) = new;
}

impl PluginHandle {
    /// Load a plugin from a snapshot. Blocks until `init.lua` has run.
    pub fn spawn(snapshot: Snapshot, limits: PluginLimits) -> Result<(PluginHandle, LoadedInfo), PluginError> {
        let manifest = snapshot.manifest().map_err(|e| PluginError::Load(e.to_string()))?;
        let queue = Arc::new(BoundedQueue::new(limits.queue_capacity));
        let status = Arc::new(Mutex::new(Status::Running));
        let (init_tx, init_rx) = channel();
        let call_budget = limits.call_budget;
        let thread = {
            let (queue, status) = (Arc::clone(&queue), Arc::clone(&status));
            std::thread::Builder::new()
                .name(format!("plugin-{}", manifest.name))
                .spawn(move || {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        worker(snapshot, manifest, limits, &queue, &status, init_tx);
                    }));
                    if outcome.is_err() {
                        set_status(&status, Status::Terminated("plugin thread panicked".into()));
                    }
                    queue.close();
                })
                .map_err(|e| PluginError::Load(format!("could not start plugin thread: {e}")))?
        };
        let handle = PluginHandle { queue, status, thread: Some(thread), call_budget };
        match init_rx.recv_timeout(call_budget * 4 + Duration::from_secs(5)) {
            Ok(Ok(info)) => Ok((handle, info)),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(PluginError::Load("plugin did not finish loading".into())),
        }
    }

    fn terminated_error(&self) -> PluginError {
        match self.status() {
            Status::Terminated(reason) => PluginError::Terminated(reason),
            Status::Running => PluginError::Terminated("plugin is shutting down".into()),
        }
    }

    fn call<T>(&self, make: impl FnOnce(Sender<Result<T, PluginError>>) -> Request) -> Result<T, PluginError> {
        if let Status::Terminated(reason) = self.status() {
            return Err(PluginError::Terminated(reason));
        }
        let (tx, rx) = channel();
        match self.queue.push(Kind::Request, make(tx)) {
            PushOutcome::Queued | PushOutcome::QueuedDropped(_) => {}
            PushOutcome::Rejected => return Err(PluginError::QueueFull),
            PushOutcome::Closed => return Err(self.terminated_error()),
        }
        match rx.recv_timeout(self.call_budget * 4 + Duration::from_secs(2)) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(PluginError::Timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(self.terminated_error()),
        }
    }

    pub fn render(&self, slot: &str, state: &serde_json::Value) -> Result<Element, PluginError> {
        self.call(|reply| Request::Render { slot: slot.into(), state: state.clone(), reply })
    }

    pub fn run_command(&self, name: &str, args: &str) -> Result<Option<String>, PluginError> {
        self.call(|reply| Request::Command { name: name.into(), args: args.into(), reply })
    }

    pub fn run_action(&self, id: &str) -> Result<(), PluginError> {
        self.call(|reply| Request::Action { id: id.into(), reply })
    }

    /// Run `pre` style hooks: the first handler that returns `{ block = true }` wins.
    pub fn hook(&self, event: &str, payload: &serde_json::Value) -> Result<HookResult, PluginError> {
        self.call(|reply| Request::Hook { event: event.into(), payload: payload.clone(), reply })
    }

    /// Fire and forget. Never blocks. Drops the oldest queued event when the queue is full.
    pub fn emit(&self, event: &str, payload: &serde_json::Value) {
        let request = Request::Event { event: event.into(), payload: payload.clone() };
        if let PushOutcome::QueuedDropped(n) = self.queue.push(Kind::Event, request) {
            debug!(dropped = n, "plugin event queue full: dropped the oldest event");
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn shutdown(mut self) {
        self.queue.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PluginHandle {
    fn drop(&mut self) {
        self.queue.close();
    }
}

fn classify(e: &mlua::Error) -> PluginError {
    let text = e.to_string();
    if matches!(e, mlua::Error::MemoryError(_)) || text.contains("not enough memory") {
        PluginError::Violation(Violation::Memory)
    } else if text.contains("cpu budget exceeded") {
        PluginError::Violation(Violation::Cpu)
    } else {
        PluginError::Runtime(text)
    }
}

fn is_violation(e: &PluginError) -> bool {
    matches!(e, PluginError::Violation(_))
}

/// Everything the worker keeps after `init.lua` ran.
struct Vm {
    lua: Lua,
    slots: Table,
    commands: Table,
    actions: Table,
    events: Table,
    deadline: Arc<AtomicU64>,
    base: Instant,
    budget: Duration,
}

impl Vm {
    /// Run `f` with a CPU deadline. The interrupt callback fails the call once the deadline passes.
    fn budgeted<T>(&self, f: impl FnOnce() -> mlua::Result<T>) -> Result<T, PluginError> {
        let end = self.base.elapsed() + self.budget;
        self.deadline.store(end.as_nanos() as u64 + 1, Ordering::Relaxed);
        let result = f();
        self.deadline.store(0, Ordering::Relaxed);
        result.map_err(|e| classify(&e))
    }

    fn lookup(&self, table: &Table, kind: &'static str, name: &str) -> Result<Function, PluginError> {
        match table.raw_get::<Value>(name) {
            Ok(Value::Function(f)) => Ok(f),
            _ => Err(PluginError::NotRegistered { kind, name: name.to_string() }),
        }
    }

    fn render(&self, slot: &str, state: &serde_json::Value) -> Result<Element, PluginError> {
        let f = self.lookup(&self.slots, "slot", slot)?;
        let value: Value = self.budgeted(|| {
            let arg = self.lua.to_value(state)?;
            f.call(arg)
        })?;
        Ok(element::from_lua(&value)?)
    }

    fn command(&self, name: &str, args: &str) -> Result<Option<String>, PluginError> {
        let f = self.lookup(&self.commands, "command", name)?;
        let value: Value = self.budgeted(|| f.call(args.to_string()))?;
        Ok(match value {
            Value::String(s) => Some(s.to_string_lossy().to_string()),
            Value::Nil => None,
            Value::Integer(i) => Some(i.to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    }

    fn action(&self, id: &str) -> Result<(), PluginError> {
        let f = self.lookup(&self.actions, "action", id)?;
        self.budgeted(|| f.call::<()>(()))
    }

    fn handlers(&self, event: &str) -> Vec<Function> {
        match self.events.raw_get::<Value>(event) {
            Ok(Value::Table(list)) => (1..=list.raw_len()).filter_map(|i| list.raw_get::<Function>(i).ok()).collect(),
            _ => Vec::new(),
        }
    }

    fn hook(&self, event: &str, payload: &serde_json::Value) -> Result<HookResult, PluginError> {
        for handler in self.handlers(event) {
            let value: Value = self.budgeted(|| {
                let arg = self.lua.to_value(payload)?;
                handler.call(arg)
            })?;
            if let Value::Table(t) = value
                && t.raw_get::<bool>("block").unwrap_or(false)
            {
                let reason = t.raw_get::<Option<String>>("reason").ok().flatten();
                return Ok(HookResult { block: true, reason });
            }
        }
        Ok(HookResult::default())
    }

    /// Events never block. Errors are logged. Only a limit violation is returned.
    fn event(&self, event: &str, payload: &serde_json::Value, plugin: &str) -> Result<(), PluginError> {
        for handler in self.handlers(event) {
            let result: Result<(), PluginError> = self.budgeted(|| {
                let arg = self.lua.to_value(payload)?;
                handler.call::<()>(arg)
            });
            match result {
                Err(e) if is_violation(&e) => return Err(e),
                Err(e) => warn!(plugin, event, error = %e, "plugin event handler failed"),
                Ok(()) => {}
            }
        }
        Ok(())
    }
}

fn need(granted: bool, capability: &str, function: &str) -> mlua::Result<()> {
    if granted {
        Ok(())
    } else {
        Err(mlua::Error::runtime(format!("`{function}` needs the `{capability}` capability, which this plugin was not granted")))
    }
}

fn register_list(lua: &Lua, table: &Table, name: String, f: Function) -> mlua::Result<()> {
    let list: Table = match table.raw_get::<Value>(name.as_str())? {
        Value::Table(t) => t,
        _ => {
            let t = lua.create_table()?;
            table.raw_set(name.as_str(), t.clone())?;
            t
        }
    };
    list.raw_set(list.raw_len() + 1, f)
}

fn module_path(name: &str) -> mlua::Result<String> {
    let valid = !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !valid {
        return Err(mlua::Error::runtime(format!("invalid module name `{name}`: use names like `lib.util`")));
    }
    Ok(name.replace('.', "/"))
}

fn make_require(lua: &Lua, snapshot: Arc<Snapshot>) -> mlua::Result<Function> {
    let loaded = lua.create_table()?;
    let loading = lua.create_table()?;
    lua.create_function(move |lua, name: String| {
        let base = module_path(&name)?;
        if let Value::Table(_) | Value::Boolean(_) | Value::String(_) | Value::Number(_) | Value::Integer(_) =
            loaded.raw_get::<Value>(name.as_str())?
        {
            return loaded.raw_get::<Value>(name.as_str());
        }
        if loading.raw_get::<bool>(name.as_str()).unwrap_or(false) {
            return Err(mlua::Error::runtime(format!("cyclic require of `{name}`")));
        }
        let candidates = [format!("{base}.lua"), format!("{base}/init.lua")];
        let Some((path, bytes)) = candidates.iter().find_map(|p| snapshot.get(p).map(|b| (p.clone(), b))) else {
            return Err(mlua::Error::runtime(format!("module `{name}` not found in this plugin")));
        };
        let source = std::str::from_utf8(bytes)
            .map_err(|_| mlua::Error::runtime(format!("module `{name}` is not valid UTF-8")))?;
        loading.raw_set(name.as_str(), true)?;
        let result = lua.load(source).set_name(format!("={path}")).eval::<Value>();
        loading.raw_set(name.as_str(), Value::Nil)?;
        let value = match result? {
            Value::Nil => Value::Boolean(true),
            v => v,
        };
        loaded.raw_set(name.as_str(), value.clone())?;
        Ok(value)
    })
}

fn build(
    snapshot: &Arc<Snapshot>,
    manifest: &Manifest,
    limits: &PluginLimits,
) -> Result<(Vm, Arc<Mutex<ThemeOverrides>>), PluginError> {
    let load_err = |e: mlua::Error| match classify(&e) {
        PluginError::Runtime(msg) => PluginError::Load(msg),
        other => other,
    };
    let lua = Lua::new();
    lua.set_memory_limit(limits.memory_bytes).map_err(load_err)?;

    let base = Instant::now();
    let deadline = Arc::new(AtomicU64::new(0));
    {
        let deadline = Arc::clone(&deadline);
        lua.set_interrupt(move |_| {
            let end = deadline.load(Ordering::Relaxed);
            if end != 0 && base.elapsed().as_nanos() as u64 >= end {
                return Err(mlua::Error::runtime("cpu budget exceeded"));
            }
            Ok(VmState::Continue)
        });
    }

    let caps: Capabilities = manifest.capabilities.clone();
    let theme = Arc::new(Mutex::new(ThemeOverrides::default()));
    let (slots, commands, actions, events) =
        (lua.create_table().map_err(load_err)?, lua.create_table().map_err(load_err)?, lua.create_table().map_err(load_err)?, lua.create_table().map_err(load_err)?);

    let api = lua.create_table().map_err(load_err)?;
    let setup = || -> mlua::Result<()> {
        api.set("api_version", API_VERSION)?;
        let ui = caps.ui;

        let slots_t = slots.clone();
        api.set("slot", lua.create_function(move |_, (name, f): (String, Function)| {
            need(ui, "ui", "discard.slot")?;
            slots_t.raw_set(name, f)
        })?)?;

        let theme_t = Arc::clone(&theme);
        api.set("theme", lua.create_function(move |_, tokens: Table| {
            need(ui, "ui", "discard.theme")?;
            let mut theme = theme_t.lock().unwrap_or_else(|p| p.into_inner());
            for pair in tokens.pairs::<String, Value>() {
                let (key, value) = pair?;
                let result = match (key.as_str(), value) {
                    ("radius", Value::Number(n)) => theme.set_radius(n),
                    ("radius", Value::Integer(n)) => theme.set_radius(n as f64),
                    ("spacing_scale", Value::Number(n)) => theme.set_spacing_scale(n),
                    ("spacing_scale", Value::Integer(n)) => theme.set_spacing_scale(n as f64),
                    ("font_scale", Value::Number(n)) => theme.set_font_scale(n),
                    ("font_scale", Value::Integer(n)) => theme.set_font_scale(n as f64),
                    (_, Value::String(s)) => theme.set_color(&key, &s.to_string_lossy()),
                    (_, _) => {
                        return Err(mlua::Error::runtime(format!("theme token `{key}` has an unsupported value")));
                    }
                };
                result.map_err(|e| mlua::Error::runtime(e.to_string()))?;
            }
            Ok(())
        })?)?;

        let commands_t = commands.clone();
        api.set("command", lua.create_function(move |_, (name, f): (String, Function)| commands_t.raw_set(name, f))?)?;
        let actions_t = actions.clone();
        api.set("action", lua.create_function(move |_, (name, f): (String, Function)| actions_t.raw_set(name, f))?)?;
        let events_t = events.clone();
        api.set("on", lua.create_function(move |lua, (name, f): (String, Function)| register_list(lua, &events_t, name, f))?)?;

        let plugin_name = manifest.name.clone();
        api.set("log", lua.create_function(move |_, message: String| {
            info!(plugin = %plugin_name, "{message}");
            Ok(())
        })?)?;

        lua.globals().set("discard", api.clone())?;
        lua.globals().set("require", make_require(&lua, Arc::clone(snapshot))?)?;
        lua.sandbox(true)
    };
    setup().map_err(load_err)?;

    let vm = Vm { lua, slots, commands, actions, events, deadline, base, budget: limits.call_budget };
    let source = snapshot.entry_source().map_err(|e| PluginError::Load(e.to_string()))?;
    vm.budgeted(|| vm.lua.load(source).set_name("=init.lua").exec()).map_err(|e| match e {
        PluginError::Runtime(msg) => PluginError::Load(msg),
        other => other,
    })?;
    Ok((vm, theme))
}

fn names(table: &Table) -> Vec<String> {
    let set: BTreeSet<String> = table.pairs::<String, Value>().filter_map(Result::ok).map(|(k, _)| k).collect();
    set.into_iter().collect()
}

fn worker(
    snapshot: Snapshot,
    manifest: Manifest,
    limits: PluginLimits,
    queue: &BoundedQueue<Request>,
    status: &Mutex<Status>,
    init_tx: Sender<Result<LoadedInfo, PluginError>>,
) {
    let snapshot = Arc::new(snapshot);
    let (vm, theme) = match build(&snapshot, &manifest, &limits) {
        Ok(built) => built,
        Err(e) => {
            set_status(status, Status::Terminated(e.to_string()));
            let _ = init_tx.send(Err(e));
            return;
        }
    };
    let info = LoadedInfo {
        manifest: manifest.clone(),
        hash: snapshot.hash(),
        theme: theme.lock().unwrap_or_else(|p| p.into_inner()).clone(),
        slots: names(&vm.slots),
        commands: names(&vm.commands),
        actions: names(&vm.actions),
        events: names(&vm.events),
    };
    let _ = init_tx.send(Ok(info));

    loop {
        let Some(request) = queue.pop(Duration::from_millis(100)) else {
            if queue.is_closed() {
                break;
            }
            continue;
        };
        let violation = match request {
            Request::Render { slot, state, reply } => finish(&reply, vm.render(&slot, &state), status, queue),
            Request::Command { name, args, reply } => finish(&reply, vm.command(&name, &args), status, queue),
            Request::Action { id, reply } => finish(&reply, vm.action(&id), status, queue),
            Request::Hook { event, payload, reply } => finish(&reply, vm.hook(&event, &payload), status, queue),
            Request::Event { event, payload } => match vm.event(&event, &payload, &manifest.name) {
                Err(e) => {
                    terminate(&e, status, queue);
                    true
                }
                Ok(()) => false,
            },
        };
        if violation {
            break;
        }
    }
}

fn terminate(error: &PluginError, status: &Mutex<Status>, queue: &BoundedQueue<Request>) {
    set_status(status, Status::Terminated(error.to_string()));
    queue.close();
}

/// Send the reply. A limit violation terminates the plugin before the caller hears about it.
fn finish<T>(
    reply: &Sender<Result<T, PluginError>>,
    result: Result<T, PluginError>,
    status: &Mutex<Status>,
    queue: &BoundedQueue<Request>,
) -> bool {
    let violated = result.as_ref().err().is_some_and(is_violation);
    if violated && let Err(e) = &result {
        terminate(e, status, queue);
    }
    let _ = reply.send(result);
    violated
}
