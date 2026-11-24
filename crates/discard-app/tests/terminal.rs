//! A real shell on a real pseudo terminal, without the Tauri window.
#![cfg(unix)]

use discard_app::desktop::terminal::{
    MAX_TABS, TAB_LIMIT, Terminals, login_spec,
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
    let (session, number) = terminals.create("channel-1", "project-1", shell_spec(dir.path().to_path_buf())).unwrap();
    assert_eq!(number, 1);
    let heard = Arc::new(Mutex::new(Heard::default()));
    session.attach("main", sink(&heard));

    session.write(b"echo hello\n".to_vec()).unwrap();
    wait_for("the echo output", || contains(&heard.lock().unwrap().output, "hello"));

    // The registry returns the same session for its ID.
    assert_eq!(terminals.get(session.id()).unwrap().id(), session.id());

    // A listener that attaches later gets the earlier output as scrollback, then the new output.
    let late = Arc::new(Mutex::new(Heard::default()));
    let attached = session.attach("main", sink(&late));
    assert!(contains(&attached.scrollback, "hello"));
    session.write(b"echo again\n".to_vec()).unwrap();
    wait_for("the later output", || contains(&late.lock().unwrap().output, "again"));
    assert!(!contains(&late.lock().unwrap().output, "hello"), "scrollback must not repeat on the stream");

    terminals.close_channel("channel-1", None);
    assert!(terminals.get(session.id()).is_none());
}

#[test]
fn the_shell_starts_in_the_given_folder() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (session, _) = terminals.create("channel-1", "project-1", shell_spec(dir.path().to_path_buf())).unwrap();
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
    let (session, _) = terminals.create("channel-1", "project-1", shell_spec(dir.path().to_path_buf())).unwrap();
    let heard = Arc::new(Mutex::new(Heard::default()));
    session.attach("main", sink(&heard));
    session.write(b"echo bye; exit 3\n".to_vec()).unwrap();
    wait_for("the exit notice", || heard.lock().unwrap().exit.is_some());
    let heard = heard.lock().unwrap();
    assert_eq!(heard.exit.as_ref().unwrap().code, Some(3));
    assert!(contains(&heard.output, "bye"));
    drop(heard);
    // The exited session keeps its output until it closes.
    assert!(terminals.get(session.id()).is_some());
    assert!(contains(&session.attach("main", sink(&Arc::new(Mutex::new(Heard::default())))).scrollback, "bye"));
    assert_eq!(session.exit().unwrap().code, Some(3));
    assert!(session.write(b"x".to_vec()).is_err());
    terminals.close_channel("channel-1", None);
}

#[test]
fn closing_a_session_kills_its_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (session, _) = terminals.create("channel-1", "project-1", shell_spec(dir.path().to_path_buf())).unwrap();
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
    let (in_tree, _) = terminals.create("in-tree", "p1", shell_spec(tree.clone())).unwrap();
    let (elsewhere, _) = terminals.create("elsewhere", "p1", shell_spec(other.clone())).unwrap();
    let (other_project, _) = terminals.create("other-project", "p2", shell_spec(other.clone())).unwrap();

    terminals.close_inside(&tree, None);
    assert!(terminals.get(in_tree.id()).is_none());
    assert!(terminals.get(elsewhere.id()).is_some());

    terminals.close_project("p1", None);
    assert!(terminals.get(elsewhere.id()).is_none());
    assert!(terminals.get(other_project.id()).is_some());

    // A session whose folder disappears is closed by the sweep.
    std::fs::remove_dir(&other).unwrap();
    assert_eq!(terminals.close_missing(), 1);
    assert!(terminals.get(other_project.id()).is_none());
}

#[test]
fn a_missing_shell_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = shell_spec(dir.path().to_path_buf());
    spec.shell = "/nope/not-a-shell".into();
    let error = Terminals::default().create("channel-1", "project-1", spec).err().expect("spawn must fail");
    assert!(error.contains("/nope/not-a-shell"), "{error}");
}

fn numbers(terminals: &Terminals, channel: &str) -> Vec<u32> {
    terminals.tabs(channel).iter().map(|(number, _)| *number).collect()
}

#[test]
fn a_channel_runs_several_sessions_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (first, one) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    let (second, two) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    assert_eq!((one, two), (1, 2));
    let ids: Vec<String> = terminals.tabs("c").iter().map(|(_, session)| session.id().to_owned()).collect();
    assert_eq!(ids, [first.id(), second.id()]);
    assert_eq!(terminals.channel_of(second.id()), Some(("c".to_owned(), "p".to_owned())));
    assert_eq!(terminals.channel_of("nope"), None);
    terminals.close_all();
}

#[test]
fn the_ninth_session_of_a_channel_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    for _ in 0..MAX_TABS {
        terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    }
    let error = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).err().expect("ninth must fail");
    assert_eq!(error, TAB_LIMIT);
    assert!(terminals.create("other", "p", shell_spec(dir.path().to_path_buf())).is_ok());
    terminals.close_all();
}

#[test]
fn closing_a_channel_ends_all_its_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (a, _) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    let (b, _) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    let (keep, _) = terminals.create("d", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    terminals.close_channel("c", None);
    assert!(terminals.tabs("c").is_empty());
    assert!(terminals.get(a.id()).is_none() && terminals.get(b.id()).is_none());
    wait_for("both shells to end", || a.exit().is_some() && b.exit().is_some());
    assert!(terminals.get(keep.id()).is_some());
    terminals.close_all();
}

#[test]
fn a_new_tab_after_a_close_takes_the_highest_number_plus_one() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let sessions: Vec<_> = (0..3).map(|_| terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap().0).collect();
    terminals.close_session(sessions[2].id(), None);
    let (_, number) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    assert_eq!(number, 3);
    terminals.close_session(sessions[0].id(), None);
    let (_, number) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    assert_eq!(number, 4);
    assert_eq!(numbers(&terminals, "c"), [2, 3, 4]);
    terminals.close_all();
}

#[test]
fn restart_keeps_position_and_number() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (first, _) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    let (second, _) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    let (third, _) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    let (fresh, number) = terminals.restart(second.id(), shell_spec(dir.path().to_path_buf())).unwrap();
    assert_eq!(number, 2);
    assert_ne!(fresh.id(), second.id());
    assert!(terminals.get(second.id()).is_none());
    let ids: Vec<String> = terminals.tabs("c").iter().map(|(_, session)| session.id().to_owned()).collect();
    assert_eq!(ids, [first.id(), fresh.id(), third.id()]);
    assert_eq!(numbers(&terminals, "c"), [1, 2, 3]);
    assert_eq!(terminals.channel_of(fresh.id()), Some(("c".to_owned(), "p".to_owned())));
    wait_for("the old shell to end", || second.exit().is_some());
    terminals.close_all();
}

#[test]
fn a_failed_restart_keeps_the_old_tab() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (old, _) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    let mut spec = shell_spec(dir.path().to_path_buf());
    spec.shell = "/nope/not-a-shell".into();
    assert!(terminals.restart(old.id(), spec).is_err());
    assert_eq!(terminals.get(old.id()).unwrap().id(), old.id());
    assert_eq!(numbers(&terminals, "c"), [1]);
    assert!(old.exit().is_none());
    terminals.close_all();
}

#[test]
fn kill_keeps_the_tab_and_ends_the_shell() {
    let dir = tempfile::tempdir().unwrap();
    let terminals = Terminals::default();
    let (session, _) = terminals.create("c", "p", shell_spec(dir.path().to_path_buf())).unwrap();
    terminals.kill(session.id(), "Stopped.");
    wait_for("the shell to end", || session.exit().is_some());
    assert!(terminals.get(session.id()).is_some());
    assert_eq!(numbers(&terminals, "c"), [1]);
    terminals.close_all();
}
