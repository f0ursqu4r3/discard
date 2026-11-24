//! The integrated terminal: several shells per project channel, for the user only.
//!
//! No agent, MCP tool, plugin, or plugin event can reach this module. Its commands answer app windows
//! alone: the main window and the project windows. The module logs session start and end, never what the shell reads or writes.
pub mod cwd;
pub mod ring;
pub mod session;

use super::{channels::Channels, view::ViewId};
use base64::{Engine, engine::general_purpose::STANDARD};
use discard_core::model::ChannelId;
use serde::Serialize;
use session::{Event, Exit, Session, Sink, SpawnSpec};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tauri::{State, ipc::Channel};

/// Shown when the folder of a session is gone.
const FOLDER_GONE: &str = "The folder of this terminal was removed. The session ended.";
/// How long a closed shell has to leave after the hang-up before it is killed.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// The shorter wait at app exit.
const EXIT_GRACE: Duration = Duration::from_millis(500);

#[cfg(target_os = "macos")]
pub const FALLBACK_SHELL: &str = "/bin/zsh";
#[cfg(not(target_os = "macos"))]
pub const FALLBACK_SHELL: &str = "/bin/sh";

/// Variables the shell must not inherit. The first is an app-internal secret. The rest name the
/// terminal program that started the app, which is not this terminal.
pub const SCRUBBED_ENV: &[&str] = &["DISCARD_PLUGIN_PASSPHRASE", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "TERM_SESSION_ID"];

/// The user's login shell: `$SHELL` when it is an absolute path to a file, else the platform default.
pub fn shell_path(configured: Option<&OsStr>, is_file: impl Fn(&Path) -> bool) -> PathBuf {
    configured
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && is_file(path))
        .unwrap_or_else(|| PathBuf::from(FALLBACK_SHELL))
}

/// A login, interactive shell in `cwd` with a color terminal type and the app's environment.
pub fn login_spec(shell: PathBuf, cwd: PathBuf, cols: u16, rows: u16) -> SpawnSpec {
    SpawnSpec {
        shell,
        args: vec![OsString::from("-l"), OsString::from("-i")],
        cwd,
        cols,
        rows,
        env: vec![("TERM".into(), "xterm-256color".into()), ("COLORTERM".into(), "truecolor".into())],
        env_remove: SCRUBBED_ENV.iter().map(|name| (*name).to_owned()).collect(),
    }
}

/// The most terminals one channel can have.
pub const MAX_TABS: usize = 8;
/// Shown when a channel is at the limit.
pub const TAB_LIMIT: &str = "A channel can have 8 terminals.";

/// The number for a new tab: one more than the highest open number, or 1.
pub fn next_number(open: &[u32]) -> u32 {
    open.iter().max().map_or(1, |highest| highest + 1)
}

struct Entry {
    session: Arc<Session>,
    project: String,
    channel: String,
    /// Creation order. A restart keeps it.
    order: u64,
    number: u32,
}

#[derive(Default)]
struct Registry {
    entries: HashMap<String, Entry>,
    next_order: u64,
}

/// The sessions of the app, by session ID.
#[derive(Default)]
pub struct Terminals {
    sessions: Mutex<Registry>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// The path with symlinks resolved. A removed path resolves through its parent.
fn normalized(path: &Path) -> PathBuf {
    path.canonicalize().ok().or_else(|| Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?))).unwrap_or_else(|| path.to_path_buf())
}

impl Terminals {
    pub fn get(&self, session_id: &str) -> Option<Arc<Session>> {
        lock(&self.sessions).entries.get(session_id).map(|entry| entry.session.clone())
    }

    /// The channel and the project of a session.
    pub fn channel_of(&self, session_id: &str) -> Option<(String, String)> {
        lock(&self.sessions).entries.get(session_id).map(|entry| (entry.channel.clone(), entry.project.clone()))
    }

    /// The sessions of a channel with their numbers, in creation order.
    pub fn tabs(&self, channel: &str) -> Vec<(u32, Arc<Session>)> {
        let registry = lock(&self.sessions);
        let mut tabs: Vec<&Entry> = registry.entries.values().filter(|entry| entry.channel == channel).collect();
        tabs.sort_by_key(|entry| entry.order);
        tabs.into_iter().map(|entry| (entry.number, entry.session.clone())).collect()
    }

    /// Start a new session in a channel. Returns it with its tab number.
    /// Fails with `TAB_LIMIT` when the channel has `MAX_TABS` sessions.
    pub fn create(&self, channel: &str, project: &str, spec: SpawnSpec) -> Result<(Arc<Session>, u32), String> {
        let mut registry = lock(&self.sessions);
        let open: Vec<u32> = registry.entries.values().filter(|entry| entry.channel == channel).map(|entry| entry.number).collect();
        if open.len() >= MAX_TABS {
            return Err(TAB_LIMIT.to_owned());
        }
        let session = Session::spawn(spec, None)?;
        let number = next_number(&open);
        let order = registry.next_order;
        registry.next_order += 1;
        registry.entries.insert(session.id().to_owned(), Entry { session: session.clone(), project: project.to_owned(), channel: channel.to_owned(), order, number });
        Ok((session, number))
    }

    /// Start a new shell in the place of a session. The new session keeps the order, the number, the channel,
    /// and the project. A failed start leaves the old session as it is.
    pub fn restart(&self, session_id: &str, spec: SpawnSpec) -> Result<(Arc<Session>, u32), String> {
        let mut registry = lock(&self.sessions);
        if !registry.entries.contains_key(session_id) {
            return Err(format!("Terminal session {session_id} not found."));
        }
        let session = Session::spawn(spec, None)?;
        let Some(old) = registry.entries.remove(session_id) else {
            return Err(format!("Terminal session {session_id} not found."));
        };
        let number = old.number;
        registry.entries.insert(session.id().to_owned(), Entry { session: session.clone(), project: old.project, channel: old.channel, order: old.order, number });
        drop(registry);
        Self::end(vec![old.session], None);
        Ok((session, number))
    }

    /// Take the sessions that `pick` selects out of the map.
    fn take(&self, pick: impl Fn(&Entry) -> bool) -> Vec<Arc<Session>> {
        let mut registry = lock(&self.sessions);
        let keys: Vec<String> = registry.entries.iter().filter(|(_, entry)| pick(entry)).map(|(key, _)| key.clone()).collect();
        keys.into_iter().filter_map(|key| registry.entries.remove(&key)).map(|entry| entry.session).collect()
    }

    /// End sessions off the calling thread. The listener hears `reason` first when there is one.
    fn end(sessions: Vec<Arc<Session>>, reason: Option<&'static str>) {
        if sessions.is_empty() {
            return;
        }
        for session in &sessions {
            if let Some(reason) = reason {
                session.announce_close(reason);
            }
        }
        let _ = std::thread::Builder::new().name("terminal-close".into()).spawn(move || {
            for session in sessions {
                session.terminate(CLOSE_GRACE);
            }
        });
    }

    /// Close one session and drop its tab. An unknown session is fine.
    pub fn close_session(&self, session_id: &str, reason: Option<&'static str>) {
        let removed = lock(&self.sessions).entries.remove(session_id).map(|entry| entry.session);
        Self::end(removed.into_iter().collect(), reason);
    }

    /// End the shell of a session and keep its tab, so the user can restart it. The listener hears `reason` first.
    pub fn kill(&self, session_id: &str, reason: &'static str) {
        if let Some(session) = self.get(session_id) {
            Self::end(vec![session], Some(reason));
        }
    }

    /// Close every session of a channel. A channel without a session is fine.
    pub fn close_channel(&self, channel: &str, reason: Option<&'static str>) {
        Self::end(self.take(|entry| entry.channel == channel), reason);
    }

    /// Close every session of a project.
    pub fn close_project(&self, project: &str, reason: Option<&'static str>) {
        Self::end(self.take(|entry| entry.project == project), reason);
    }

    /// Close every session that works inside `folder`. Call it after the folder went away.
    pub fn close_inside(&self, folder: &Path, reason: Option<&'static str>) {
        let folder = normalized(folder);
        Self::end(self.take(|entry| normalized(entry.session.cwd()).starts_with(&folder)), reason);
    }

    /// Close every session whose folder no longer exists. Returns how many closed.
    pub fn close_missing(&self) -> usize {
        let gone = self.take(|entry| !entry.session.cwd().is_dir());
        let count = gone.len();
        Self::end(gone, Some(FOLDER_GONE));
        count
    }

    /// End every shell and wait for each for a short time. For app exit.
    pub fn close_all(&self) {
        let all = self.take(|_| true);
        let handles: Vec<_> = all
            .into_iter()
            .filter_map(|session| std::thread::Builder::new().name("terminal-exit".into()).spawn(move || session.terminate(EXIT_GRACE)).ok())
            .collect();
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// What the webview receives on the stream of a session.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TerminalEvent {
    /// Output bytes, base64. A chunk can end inside a UTF-8 sequence, so the webview decodes them as a stream.
    Output { data: String },
    Exit { exit_code: Option<i32>, signal: Option<String> },
    /// Another window took the session. This window stops hearing it.
    Moved,
    /// The app ended the session, for example because its folder is gone.
    Closed { reason: String },
}

impl From<Event> for TerminalEvent {
    fn from(event: Event) -> Self {
        match event {
            Event::Output(bytes) => TerminalEvent::Output { data: STANDARD.encode(bytes) },
            Event::Exit(Exit { code, signal }) => TerminalEvent::Exit { exit_code: code, signal },
            Event::Moved => TerminalEvent::Moved,
            Event::Closed(reason) => TerminalEvent::Closed { reason },
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpenedTerminal {
    pub session_id: String,
    pub number: u32,
    pub cwd: String,
    /// Output kept so far, base64.
    pub scrollback: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

/// One tab of a channel.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TerminalTab {
    pub session_id: String,
    pub number: u32,
    pub cwd: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

/// Pass for the label of an app window: the main window or a project window.
fn app_label(label: &str) -> Result<(), String> {
    if ViewId::from_label(label).is_some() { Ok(()) } else { Err("The terminal is available in app windows only.".into()) }
}

fn app_window_only(window: &tauri::WebviewWindow) -> Result<(), String> {
    app_label(window.label())
}

/// The session when this window may use it. An unknown session is `Ok(None)`.
fn owned_session(terminals: &Terminals, session_id: &str, window: &str) -> Result<Option<Arc<Session>>, String> {
    let session = terminals.get(session_id);
    if let Some(session) = &session {
        session.require_owner(window)?;
    }
    Ok(session)
}

fn missing(session_id: &str) -> String {
    format!("Terminal session {session_id} not found.")
}

/// Attach the window to the session and describe the session for the webview.
fn attach_window(session: &Session, number: u32, owner: &str, on_event: Channel<TerminalEvent>) -> OpenedTerminal {
    let sink: Sink = Box::new(move |event| on_event.send(event.into()).is_ok());
    let attached = session.attach(owner, sink);
    OpenedTerminal {
        session_id: session.id().to_owned(),
        number,
        cwd: session.cwd().to_string_lossy().into_owned(),
        scrollback: STANDARD.encode(attached.scrollback),
        running: attached.exit.is_none(),
        exit_code: attached.exit.as_ref().and_then(|exit| exit.code),
        signal: attached.exit.and_then(|exit| exit.signal),
    }
}

/// The spec of a new shell in the working folder of a channel. Returns it with the project.
fn spec_for_channel(channels: &Channels, channel: &str, cols: u16, rows: u16) -> Result<(SpawnSpec, String), String> {
    let (cwd, project) = cwd::for_channel(&channels.store, &ChannelId(channel.to_owned()))?;
    let shell = shell_path(std::env::var_os("SHELL").as_deref(), |path| path.is_file());
    Ok((login_spec(shell, cwd, cols, rows), project))
}

/// The tabs of a channel in creation order. Any app window can ask.
#[tauri::command]
pub async fn terminal_tabs(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, channel_id: String) -> Result<Vec<TerminalTab>, String> {
    app_window_only(&window)?;
    Ok(terminals
        .tabs(&channel_id)
        .into_iter()
        .map(|(number, session)| {
            let exit = session.exit();
            TerminalTab {
                session_id: session.id().to_owned(),
                number,
                cwd: session.cwd().to_string_lossy().into_owned(),
                running: exit.is_none(),
                exit_code: exit.as_ref().and_then(|exit| exit.code),
                signal: exit.and_then(|exit| exit.signal),
            }
        })
        .collect())
}

/// Start a new tab in a channel, in the working folder of the channel. The window gets its stream.
#[tauri::command]
pub async fn terminal_create(
    window: tauri::WebviewWindow,
    channels: State<'_, Arc<Channels>>,
    terminals: State<'_, Arc<Terminals>>,
    channel_id: String,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> Result<OpenedTerminal, String> {
    app_window_only(&window)?;
    let owner = window.label().to_owned();
    let (channels, terminals) = (channels.inner().clone(), terminals.inner().clone());
    tauri::async_runtime::spawn_blocking(move || {
        let (spec, project) = spec_for_channel(&channels, &channel_id, cols, rows)?;
        let (session, number) = terminals.create(&channel_id, &project, spec)?;
        Ok(attach_window(&session, number, &owner, on_event))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Re-attach to a session. `on_event` replaces any earlier listener, also one of a window that is gone.
/// A listener of another window hears `moved` first. The result holds the output so far, and the stream continues from there.
#[tauri::command]
pub async fn terminal_open(
    window: tauri::WebviewWindow,
    terminals: State<'_, Arc<Terminals>>,
    session_id: String,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> Result<OpenedTerminal, String> {
    app_window_only(&window)?;
    let owner = window.label().to_owned();
    let terminals = terminals.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let session = terminals.get(&session_id).ok_or_else(|| missing(&session_id))?;
        let (channel, _) = terminals.channel_of(&session_id).ok_or_else(|| missing(&session_id))?;
        let number = terminals.tabs(&channel).into_iter().find(|(_, tab)| tab.id() == session_id).map(|(number, _)| number).ok_or_else(|| missing(&session_id))?;
        session.resize(cols, rows)?;
        Ok(attach_window(&session, number, &owner, on_event))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Replace a session with a new shell in the same tab. A failed start keeps the old tab.
/// Only the owner can do it.
#[tauri::command]
pub async fn terminal_restart(
    window: tauri::WebviewWindow,
    channels: State<'_, Arc<Channels>>,
    terminals: State<'_, Arc<Terminals>>,
    session_id: String,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> Result<OpenedTerminal, String> {
    app_window_only(&window)?;
    let owner = window.label().to_owned();
    let (channels, terminals) = (channels.inner().clone(), terminals.inner().clone());
    tauri::async_runtime::spawn_blocking(move || {
        owned_session(&terminals, &session_id, &owner)?.ok_or_else(|| missing(&session_id))?;
        let (channel, _) = terminals.channel_of(&session_id).ok_or_else(|| missing(&session_id))?;
        let (spec, _) = spec_for_channel(&channels, &channel, cols, rows)?;
        let (session, number) = terminals.restart(&session_id, spec)?;
        Ok(attach_window(&session, number, &owner, on_event))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Send keystrokes to the shell. The webview sends one call at a time, so the order holds.
#[tauri::command]
pub async fn terminal_write(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, session_id: String, data: String) -> Result<(), String> {
    app_window_only(&window)?;
    owned_session(&terminals, &session_id, window.label())?.ok_or_else(|| missing(&session_id))?.write(data.into_bytes())
}

#[tauri::command]
pub async fn terminal_resize(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, session_id: String, cols: u16, rows: u16) -> Result<(), String> {
    app_window_only(&window)?;
    owned_session(&terminals, &session_id, window.label())?.ok_or_else(|| missing(&session_id))?.resize(cols, rows)
}

/// Stop streaming to the webview. The session keeps running and keeps its output. Only the owner can do it.
#[tauri::command]
pub async fn terminal_detach(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, session_id: String) -> Result<(), String> {
    app_window_only(&window)?;
    match terminals.get(&session_id) {
        Some(session) => session.detach(window.label()),
        None => Ok(()),
    }
}

/// Kill the shell and its process group, then drop the tab. An unknown session is fine.
/// Only the owner can do it.
#[tauri::command]
pub async fn terminal_close(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, session_id: String) -> Result<(), String> {
    app_window_only(&window)?;
    owned_session(&terminals, &session_id, window.label())?;
    terminals.close_session(&session_id, None);
    Ok(())
}

/// End the shell and keep the tab, so the user can restart it. Only the owner can do it.
#[tauri::command]
pub async fn terminal_kill(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, session_id: String) -> Result<(), String> {
    app_window_only(&window)?;
    owned_session(&terminals, &session_id, window.label())?.ok_or_else(|| missing(&session_id))?;
    terminals.kill(&session_id, "The terminal was stopped.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_comes_from_the_environment_when_it_is_a_file() {
        let shell = shell_path(Some(OsStr::new("/usr/bin/fish")), |path| path == Path::new("/usr/bin/fish"));
        assert_eq!(shell, Path::new("/usr/bin/fish"));
    }

    #[test]
    fn a_bad_shell_falls_back_to_the_platform_default() {
        for configured in [None, Some(OsStr::new("")), Some(OsStr::new("zsh")), Some(OsStr::new("/nope/zsh"))] {
            assert_eq!(shell_path(configured, |_| false), Path::new(FALLBACK_SHELL));
        }
    }

    #[test]
    fn the_spec_starts_a_login_interactive_shell_without_secrets() {
        let spec = login_spec("/bin/sh".into(), "/tmp".into(), 80, 24);
        assert_eq!(spec.args, ["-l", "-i"]);
        assert!(spec.env.contains(&("TERM".into(), "xterm-256color".into())));
        assert!(spec.env.contains(&("COLORTERM".into(), "truecolor".into())));
        assert!(spec.env_remove.iter().any(|name| name == "DISCARD_PLUGIN_PASSPHRASE"));
    }

    #[test]
    fn events_serialize_with_camel_case_fields_and_base64_output() {
        let output = serde_json::to_value(TerminalEvent::from(Event::Output(vec![0xC3, 0xA9]))).unwrap();
        assert_eq!(output, serde_json::json!({ "kind": "output", "data": "w6k=" }));
        let exit = serde_json::to_value(TerminalEvent::from(Event::Exit(Exit { code: Some(3), signal: None }))).unwrap();
        assert_eq!(exit, serde_json::json!({ "kind": "exit", "exitCode": 3, "signal": null }));
        let closed = serde_json::to_value(TerminalEvent::from(Event::Closed("gone".into()))).unwrap();
        assert_eq!(closed, serde_json::json!({ "kind": "closed", "reason": "gone" }));
        let moved = serde_json::to_value(TerminalEvent::from(Event::Moved)).unwrap();
        assert_eq!(moved, serde_json::json!({ "kind": "moved" }));
    }

    #[test]
    fn the_main_window_and_project_windows_are_app_windows() {
        assert_eq!(app_label("main"), Ok(()));
        assert_eq!(app_label(ViewId::project(&discard_core::model::ProjectId("p1".into())).label()), Ok(()));
        for label in ["settings", "project-", "Main", ""] {
            assert_eq!(app_label(label), Err("The terminal is available in app windows only.".to_string()), "{label:?}");
        }
    }

    #[test]
    fn an_unknown_session_is_missing_without_an_error_for_any_owner() {
        let terminals = Terminals::default();
        assert!(owned_session(&terminals, "session-1", "main").unwrap().is_none());
    }

    #[test]
    fn next_number_is_one_more_than_the_highest() {
        assert_eq!(next_number(&[]), 1);
        assert_eq!(next_number(&[1, 2]), 3);
        assert_eq!(next_number(&[2, 5]), 6);
    }

    #[test]
    fn a_removed_folder_normalizes_through_its_parent() {
        let root = tempfile::tempdir().unwrap();
        let inside = root.path().join("tree");
        std::fs::create_dir(&inside).unwrap();
        let before = normalized(&inside);
        std::fs::remove_dir(&inside).unwrap();
        assert_eq!(normalized(&inside), before);
    }
}
