//! A real shell on a real pseudo terminal, without the Tauri window.
#![cfg(unix)]

use discard_app::desktop::terminal::{
    Terminals, login_spec,
    session::{Event, Exit, Sink},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Heard {
    output: Vec<u8>,
    exit: Option<Exit>,
}

fn sink(heard: &Arc<Mutex<Heard>>) -> Sink {
    let heard = heard.clone();
    Box::new(move |event| {
        let mut heard = heard.lock().unwrap();
        match event {
            Event::Output(bytes) => heard.output.extend(bytes),
            Event::Exit(exit) => heard.exit = Some(exit),
            Event::Moved | Event::Closed(_) => {}
        }
        true
    })
}

fn wait_for(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn shell_spec(cwd: PathBuf) -> discard_app::desktop::terminal::session::SpawnSpec {
    // `-i` only: a login shell would read the profile of the machine that runs the test.
    let mut spec = login_spec("/bin/sh".into(), cwd, 80, 24);
    spec.args = vec!["-i".into()];
    spec
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    String::from_utf8_lossy(haystack).contains(needle)
}

#[test]
fn a_shell_runs_a_command_and_its_output_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (session, created) = terminals.open("channel-1", "project-1", shell_spec(dir.path().to_path_buf())).unwrap();
    assert!(created);
    let heard = Arc::new(Mutex::new(Heard::default()));
    session.attach("main", sink(&heard));

    session.write(b"echo hello\n".to_vec()).unwrap();
    wait_for("the echo output", || contains(&heard.lock().unwrap().output, "hello"));

    // A second open of the same channel returns the same session.
    let (again, created) = terminals.open("channel-1", "project-1", shell_spec(dir.path().to_path_buf())).unwrap();
    assert!(!created);
    assert_eq!(again.id(), session.id());

    // A listener that attaches later gets the earlier output as scrollback, then the new output.
    let late = Arc::new(Mutex::new(Heard::default()));
    let attached = session.attach("main", sink(&late));
    assert!(contains(&attached.scrollback, "hello"));
    session.write(b"echo again\n".to_vec()).unwrap();
    wait_for("the later output", || contains(&late.lock().unwrap().output, "again"));
    assert!(!contains(&late.lock().unwrap().output, "hello"), "scrollback must not repeat on the stream");

    terminals.close_channel("channel-1", None);
    assert!(terminals.get("channel-1").is_none());
}

#[test]
fn the_shell_starts_in_the_given_folder() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (session, _) = terminals.open("channel-1", "project-1", shell_spec(dir.path().to_path_buf())).unwrap();
    let heard = Arc::new(Mutex::new(Heard::default()));
    session.attach("main", sink(&heard));
    session.write(b"pwd -P\n".to_vec()).unwrap();
    let expected = dir.path().canonicalize().unwrap().to_string_lossy().into_owned();
    wait_for("the working folder", || contains(&heard.lock().unwrap().output, &expected));
    terminals.close_all();
}

#[test]
fn the_exit_code_arrives_after_the_output_and_the_session_stays() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (session, _) = terminals.open("channel-1", "project-1", shell_spec(dir.path().to_path_buf())).unwrap();
    let heard = Arc::new(Mutex::new(Heard::default()));
    session.attach("main", sink(&heard));
    session.write(b"echo bye; exit 3\n".to_vec()).unwrap();
    wait_for("the exit notice", || heard.lock().unwrap().exit.is_some());
    let heard = heard.lock().unwrap();
    assert_eq!(heard.exit.as_ref().unwrap().code, Some(3));
    assert!(contains(&heard.output, "bye"));
    drop(heard);
    // The exited session keeps its output until it closes.
    assert!(terminals.get("channel-1").is_some());
    assert!(contains(&session.attach("main", sink(&Arc::new(Mutex::new(Heard::default())))).scrollback, "bye"));
    assert_eq!(session.exit().unwrap().code, Some(3));
    assert!(session.write(b"x".to_vec()).is_err());
    terminals.close_channel("channel-1", None);
}

#[test]
fn closing_a_session_kills_its_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (session, _) = terminals.open("channel-1", "project-1", shell_spec(dir.path().to_path_buf())).unwrap();
    let heard = Arc::new(Mutex::new(Heard::default()));
    session.attach("main", sink(&heard));
    // A background child that would outlive a shell that only the shell pid receives a signal.
    session.write(b"sleep 60 & echo child=$!\n".to_vec()).unwrap();
    wait_for("the child pid", || String::from_utf8_lossy(&heard.lock().unwrap().output).contains("child="));
    let text = String::from_utf8_lossy(&heard.lock().unwrap().output).into_owned();
    let pid: i32 = text.split("child=").last().unwrap().chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap();
    let alive = |pid: i32| nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok();
    assert!(alive(pid));

    terminals.close_channel("channel-1", None);
    wait_for("the shell to end", || session.exit().is_some());
    wait_for("the background child to end", || !alive(pid));
}

#[test]
fn closing_by_folder_and_by_project_selects_the_right_sessions() {
    let root = tempfile::tempdir().unwrap();
    let tree = root.path().join("tree");
    let other = root.path().join("other");
    std::fs::create_dir(&tree).unwrap();
    std::fs::create_dir(&other).unwrap();
    let terminals = Terminals::default();
    terminals.open("in-tree", "p1", shell_spec(tree.clone())).unwrap();
    terminals.open("elsewhere", "p1", shell_spec(other.clone())).unwrap();
    terminals.open("other-project", "p2", shell_spec(other.clone())).unwrap();

    terminals.close_inside(&tree, None);
    assert!(terminals.get("in-tree").is_none());
    assert!(terminals.get("elsewhere").is_some());

    terminals.close_project("p1", None);
    assert!(terminals.get("elsewhere").is_none());
    assert!(terminals.get("other-project").is_some());

    // A session whose folder disappears is closed by the sweep.
    std::fs::remove_dir(&other).unwrap();
    assert_eq!(terminals.close_missing(), 1);
    assert!(terminals.get("other-project").is_none());
}

#[test]
fn a_missing_shell_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = shell_spec(dir.path().to_path_buf());
    spec.shell = "/nope/not-a-shell".into();
    let error = Terminals::default().open("channel-1", "project-1", spec).err().expect("spawn must fail");
    assert!(error.contains("/nope/not-a-shell"), "{error}");
}
