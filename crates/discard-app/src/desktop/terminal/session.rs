//! One shell on a pseudo terminal. This file has no Tauri types, so tests can run it alone.
//! It never logs what the shell reads or writes.
use super::ring::{Ring, SCROLLBACK_BYTES};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, mpsc},
    thread,
    time::{Duration, Instant},
};

/// How long the exit notice waits for the reader to drain the last output.
const DRAIN_WAIT: Duration = Duration::from_millis(300);

/// What a session tells its listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Raw output bytes. A chunk can end inside a UTF-8 sequence.
    Output(Vec<u8>),
    /// The shell ended. The session keeps its output.
    Exit(Exit),
    /// The app closed the session. The text says why.
    Closed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exit {
    pub code: Option<i32>,
    pub signal: Option<String>,
}

/// Receives events in order. Returns `false` when nobody listens any more.
pub type Sink = Box<dyn Fn(Event) -> bool + Send + Sync>;

pub struct SpawnSpec {
    pub shell: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
}

/// What a new listener needs to catch up.
pub struct Attached {
    pub scrollback: Vec<u8>,
    pub exit: Option<Exit>,
}

struct State {
    ring: Ring,
    sink: Option<Sink>,
    reader_done: bool,
    child_gone: bool,
    exit: Option<Exit>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl Shared {
    /// Deliver `event` to the listener. A dead listener goes away.
    fn emit(state: &mut State, event: Event) {
        if let Some(sink) = &state.sink
            && !sink(event)
        {
            state.sink = None;
        }
    }
}

pub struct Session {
    id: String,
    cwd: PathBuf,
    shared: Arc<Shared>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    input: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 }
}

impl Session {
    /// Start the shell. The pty and the child come up under the process-wide spawn lock.
    pub fn spawn(spec: SpawnSpec, sink: Option<Sink>) -> Result<Arc<Session>, String> {
        let mut command = CommandBuilder::new(&spec.shell);
        command.args(&spec.args);
        command.cwd(&spec.cwd);
        for key in &spec.env_remove {
            command.env_remove(key);
        }
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        let started = discard_core::process::with_spawn_lock(|| -> Result<_, String> {
            let pair = native_pty_system().openpty(size(spec.cols, spec.rows)).map_err(|error| error.to_string())?;
            let child = pair.slave.spawn_command(command).map_err(|error| error.to_string())?;
            drop(pair.slave);
            Ok((pair.master, child))
        });
        let (master, mut child) = started.map_err(|error| format!("Could not start the shell {}: {error}", spec.shell.display()))?;
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = master.try_clone_reader().map_err(|error| error.to_string())?;
        let mut writer = master.take_writer().map_err(|error| error.to_string())?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State { ring: Ring::new(SCROLLBACK_BYTES), sink, reader_done: false, child_gone: false, exit: None }),
            changed: Condvar::new(),
        });

        let reading = shared.clone();
        thread::Builder::new().name("terminal-read".into()).spawn(move || {
            let mut buffer = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        let mut state = lock(&reading.state);
                        state.ring.push(&buffer[..count]);
                        Shared::emit(&mut state, Event::Output(buffer[..count].to_vec()));
                    }
                }
            }
            lock(&reading.state).reader_done = true;
            reading.changed.notify_all();
        }).map_err(|error| error.to_string())?;

        let waiting = shared.clone();
        thread::Builder::new().name("terminal-wait".into()).spawn(move || {
            let status = child.wait();
            let exit = match status {
                Ok(status) => Exit { code: i32::try_from(status.exit_code()).ok(), signal: status.signal().map(str::to_owned) },
                Err(_) => Exit { code: None, signal: None },
            };
            let mut state = lock(&waiting.state);
            state.child_gone = true;
            waiting.changed.notify_all();
            // Let the reader pass on the last output before the exit notice.
            let (mut state, _) = waiting.changed.wait_timeout_while(state, DRAIN_WAIT, |state| !state.reader_done).unwrap_or_else(|error| error.into_inner());
            state.exit = Some(exit.clone());
            Shared::emit(&mut state, Event::Exit(exit));
        }).map_err(|error| error.to_string())?;

        let (input, queue) = mpsc::channel::<Vec<u8>>();
        thread::Builder::new().name("terminal-write".into()).spawn(move || {
            for chunk in queue {
                if writer.write_all(&chunk).and_then(|()| writer.flush()).is_err() {
                    break;
                }
            }
        }).map_err(|error| error.to_string())?;

        Ok(Arc::new(Session {
            id: uuid::Uuid::new_v4().to_string(),
            cwd: spec.cwd,
            shared,
            master: Mutex::new(master),
            input: Mutex::new(Some(input)),
            killer: Mutex::new(killer),
            pid,
        }))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    /// Make `sink` the listener and return what it missed. The output lock makes the cut exact:
    /// bytes before it are in the scrollback, bytes after it go to `sink`.
    pub fn attach(&self, sink: Sink) -> Attached {
        let mut state = lock(&self.shared.state);
        state.sink = Some(sink);
        Attached { scrollback: state.ring.snapshot(), exit: state.exit.clone() }
    }

    /// Stop sending events. The session keeps its output.
    pub fn detach(&self) {
        lock(&self.shared.state).sink = None;
    }

    pub fn exit(&self) -> Option<Exit> {
        lock(&self.shared.state).exit.clone()
    }

    /// Queue input for the shell. Order is the order of calls.
    pub fn write(&self, data: Vec<u8>) -> Result<(), String> {
        if self.exit().is_some() {
            return Err("The shell has ended. Restart the terminal.".into());
        }
        match lock(&self.input).as_ref() {
            Some(input) => input.send(data).map_err(|_| "The terminal input is closed.".to_string()),
            None => Err("The terminal input is closed.".into()),
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        if self.exit().is_some() {
            return Ok(());
        }
        lock(&self.master).resize(size(cols, rows)).map_err(|error| error.to_string())
    }

    /// Tell the listener why the session ends, then stop sending events.
    pub fn announce_close(&self, reason: &str) {
        let mut state = lock(&self.shared.state);
        Shared::emit(&mut state, Event::Closed(reason.to_owned()));
        state.sink = None;
    }

    /// End the shell and its process group. Sends hang-up first, then kill after `grace`.
    /// This blocks for up to `grace`. Callers off the UI path run it on a thread.
    pub fn terminate(&self, grace: Duration) {
        lock(&self.shared.state).sink = None;
        *lock(&self.input) = None;
        if lock(&self.shared.state).child_gone {
            return;
        }
        self.signal_group(false);
        let deadline = Instant::now() + grace;
        let mut state = lock(&self.shared.state);
        while !state.child_gone {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
            state = self.shared.changed.wait_timeout(state, left).unwrap_or_else(|error| error.into_inner()).0;
        }
        let gone = state.child_gone;
        drop(state);
        if !gone {
            self.signal_group(true);
        }
    }

    /// Signal the process group of the shell. Only call this while the shell runs, so a reused ID cannot be hit.
    fn signal_group(&self, kill: bool) {
        #[cfg(unix)]
        if let Some(pid) = self.pid.and_then(|pid| i32::try_from(pid).ok()) {
            use nix::{sys::signal::{Signal, killpg}, unistd::Pid};
            // The child is a session leader, so its process group ID is its process ID.
            let signal = if kill { Signal::SIGKILL } else { Signal::SIGHUP };
            let _ = killpg(Pid::from_raw(pid), signal);
            // An interactive shell puts each background job in its own process group. Some shells, such as
            // dash, do not pass the hang-up on, so signal the rest of the session too.
            #[cfg(target_os = "linux")]
            for member in session_members(pid) {
                let _ = nix::sys::signal::kill(Pid::from_raw(member), signal);
            }
            return;
        }
        let _ = kill;
        let _ = lock(&self.killer).kill();
    }
}

/// The session ID in the text of `/proc/<pid>/stat`. The command name can hold spaces and brackets,
/// so the fields are counted from the last `)`.
#[cfg(any(target_os = "linux", test))]
fn stat_session(stat: &str) -> Option<i32> {
    stat.rsplit_once(')')?.1.split_whitespace().nth(3)?.parse().ok()
}

/// The processes in the session that `leader` leads.
#[cfg(target_os = "linux")]
fn session_members(leader: i32) -> Vec<i32> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|stat| stat_session(&stat)) == Some(leader))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_id_is_read_after_the_command_name() {
        assert_eq!(stat_session("4242 (a) b) c) S 1 4242 77 34816 4242 4194304"), Some(77));
        assert_eq!(stat_session("no fields"), None);
    }

    #[test]
    fn size_never_has_a_zero_side() {
        let value = size(0, 0);
        assert_eq!((value.cols, value.rows), (1, 1));
    }
}
