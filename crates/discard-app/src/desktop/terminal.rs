//! The integrated terminal: one shell per project channel, for the user only.
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

struct Entry {
    session: Arc<Session>,
    project: String,
}

/// The sessions of the app, by channel ID.
#[derive(Default)]
pub struct Terminals {
    sessions: Mutex<HashMap<String, Entry>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// The path with symlinks resolved. A removed path resolves through its parent.
fn normalized(path: &Path) -> PathBuf {
    path.canonicalize().ok().or_else(|| Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?))).unwrap_or_else(|| path.to_path_buf())
}

impl Terminals {
    pub fn get(&self, channel: &str) -> Option<Arc<Session>> {
        lock(&self.sessions).get(channel).map(|entry| entry.session.clone())
    }

    /// The session of a channel. Starts one with `spec` when the channel has none.
    /// Returns `true` when this call started it.
    pub fn open(&self, channel: &str, project: &str, spec: SpawnSpec) -> Result<(Arc<Session>, bool), String> {
        let mut sessions = lock(&self.sessions);
        if let Some(entry) = sessions.get(channel) {
            return Ok((entry.session.clone(), false));
        }
        let session = Session::spawn(spec, None)?;
        sessions.insert(channel.to_owned(), Entry { session: session.clone(), project: project.to_owned() });
        Ok((session, true))
    }

    /// Take the sessions that `pick` selects out of the map.
    fn take(&self, pick: impl Fn(&str, &Entry) -> bool) -> Vec<Arc<Session>> {
        let mut sessions = lock(&self.sessions);
        let keys: Vec<String> = sessions.iter().filter(|(key, entry)| pick(key, entry)).map(|(key, _)| key.clone()).collect();
        keys.into_iter().filter_map(|key| sessions.remove(&key)).map(|entry| entry.session).collect()
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

    /// Close the session of a channel. A channel without a session is fine.
    pub fn close_channel(&self, channel: &str, reason: Option<&'static str>) {
        Self::end(self.take(|key, _| key == channel), reason);
    }

    /// Close every session of a project.
    pub fn close_project(&self, project: &str, reason: Option<&'static str>) {
        Self::end(self.take(|_, entry| entry.project == project), reason);
    }

    /// Close every session that works inside `folder`. Call it after the folder went away.
    pub fn close_inside(&self, folder: &Path, reason: Option<&'static str>) {
        let folder = normalized(folder);
        Self::end(self.take(|_, entry| normalized(entry.session.cwd()).starts_with(&folder)), reason);
    }

    /// Close every session whose folder no longer exists. Returns how many closed.
    pub fn close_missing(&self) -> usize {
        let gone = self.take(|_, entry| !entry.session.cwd().is_dir());
        let count = gone.len();
        Self::end(gone, Some(FOLDER_GONE));
        count
    }

    /// End every shell and wait for each for a short time. For app exit.
    pub fn close_all(&self) {
        let all = self.take(|_, _| true);
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
    pub cwd: String,
    /// Output kept so far, base64.
    pub scrollback: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStatus {
    pub session_id: String,
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

/// The session of a channel when this window may use it. A channel without a session is `Ok(None)`.
fn owned_session(terminals: &Terminals, channel: &str, window: &str) -> Result<Option<Arc<Session>>, String> {
    let session = terminals.get(channel);
    if let Some(session) = &session {
        session.require_owner(window)?;
    }
    Ok(session)
}

fn missing(channel: &str) -> String {
    format!("Channel {channel} has no terminal session.")
}

/// Open or re-attach the terminal of a channel. A new session starts in the working folder of the channel.
/// `on_event` replaces any earlier listener, also one of a window that is gone. A listener of another window
/// hears `moved` first. The result holds the output so far, and the stream continues from there.
#[tauri::command]
pub async fn terminal_open(
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
        let session = match terminals.get(&channel_id) {
            Some(session) => {
                session.resize(cols, rows)?;
                session
            }
            None => {
                let (cwd, project) = cwd::for_channel(&channels.store, &ChannelId(channel_id.clone()))?;
                let shell = shell_path(std::env::var_os("SHELL").as_deref(), |path| path.is_file());
                terminals.open(&channel_id, &project, login_spec(shell, cwd, cols, rows))?.0
            }
        };
        let sink: Sink = Box::new(move |event| on_event.send(event.into()).is_ok());
        let attached = session.attach(&owner, sink);
        Ok(OpenedTerminal {
            session_id: session.id().to_owned(),
            cwd: session.cwd().to_string_lossy().into_owned(),
            scrollback: STANDARD.encode(attached.scrollback),
            running: attached.exit.is_none(),
            exit_code: attached.exit.as_ref().and_then(|exit| exit.code),
            signal: attached.exit.and_then(|exit| exit.signal),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Send keystrokes to the shell. The webview sends one call at a time, so the order holds.
#[tauri::command]
pub async fn terminal_write(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, channel_id: String, data: String) -> Result<(), String> {
    app_window_only(&window)?;
    owned_session(&terminals, &channel_id, window.label())?.ok_or_else(|| missing(&channel_id))?.write(data.into_bytes())
}

#[tauri::command]
pub async fn terminal_resize(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, channel_id: String, cols: u16, rows: u16) -> Result<(), String> {
    app_window_only(&window)?;
    owned_session(&terminals, &channel_id, window.label())?.ok_or_else(|| missing(&channel_id))?.resize(cols, rows)
}

/// Stop streaming to the webview. The session keeps running and keeps its output. Only the owner can do it.
#[tauri::command]
pub async fn terminal_detach(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, channel_id: String) -> Result<(), String> {
    app_window_only(&window)?;
    match terminals.get(&channel_id) {
        Some(session) => session.detach(window.label()),
        None => Ok(()),
    }
}

/// Kill the shell and its process group, then drop the session. A channel without a session is fine.
/// Only the owner can do it.
#[tauri::command]
pub async fn terminal_close(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, channel_id: String) -> Result<(), String> {
    app_window_only(&window)?;
    owned_session(&terminals, &channel_id, window.label())?;
    terminals.close_channel(&channel_id, None);
    Ok(())
}

/// The state of a channel session, or null when it has none. Any app window can ask.
#[tauri::command]
pub async fn terminal_status(window: tauri::WebviewWindow, terminals: State<'_, Arc<Terminals>>, channel_id: String) -> Result<Option<TerminalStatus>, String> {
    app_window_only(&window)?;
    Ok(terminals.get(&channel_id).map(|session| {
        let exit = session.exit();
        TerminalStatus {
            session_id: session.id().to_owned(),
            cwd: session.cwd().to_string_lossy().into_owned(),
            running: exit.is_none(),
            exit_code: exit.as_ref().and_then(|exit| exit.code),
            signal: exit.and_then(|exit| exit.signal),
        }
    }))
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
    fn a_session_of_a_channel_is_missing_without_an_error_for_any_owner() {
        let terminals = Terminals::default();
        assert!(owned_session(&terminals, "channel-1", "main").unwrap().is_none());
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
