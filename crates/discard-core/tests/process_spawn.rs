//! Concurrent process setup must not keep another child's output pipe open.
#![cfg(target_os = "macos")]

use std::{
    io::Read,
    process::{Command, Stdio},
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

#[test]
fn concurrent_spawns_do_not_hold_other_children_output_open() {
    let barrier = Arc::new(Barrier::new(17));
    let mut long_workers = Vec::new();
    for _ in 0..4 {
        let barrier = barrier.clone();
        long_workers.push(thread::spawn(move || {
            barrier.wait();
            let mut children = Vec::new();
            for _ in 0..48 {
                children.push(
                    discard_core::process::spawn_std(
                        Command::new("/bin/sleep")
                            .arg("5")
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped()),
                    )
                    .unwrap(),
                );
                thread::sleep(Duration::from_millis(25));
            }
            for mut child in children {
                child.wait().unwrap();
            }
        }));
    }
    let mut short_workers = Vec::new();
    for _ in 0..12 {
        let barrier = barrier.clone();
        short_workers.push(thread::spawn(move || {
            barrier.wait();
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut timings = Vec::new();
            while Instant::now() < deadline {
                let start = Instant::now();
                let mut child = discard_core::process::spawn_std(
                    Command::new("/usr/bin/true")
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null()),
                )
                .unwrap();
                assert!(child.wait().unwrap().success());
                let exited = start.elapsed();
                let mut bytes = Vec::new();
                child
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                timings.push((exited, start.elapsed()));
            }
            timings
        }));
    }
    let start = Instant::now();
    barrier.wait();
    let timings: Vec<_> = short_workers
        .into_iter()
        .flat_map(|worker| worker.join().unwrap())
        .collect();
    for worker in long_workers {
        worker.join().unwrap();
    }
    let failures: Vec<_> = timings
        .iter()
        .filter(|(_, elapsed)| *elapsed >= Duration::from_secs(2))
        .collect();
    eprintln!(
        "short checks: {}; delayed EOF checks: {}; wall time: {:?}; failed (exit, EOF): {:?}",
        timings.len(),
        failures.len(),
        start.elapsed(),
        failures
    );
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "stress test exceeded its runtime bound"
    );
    assert!(
        failures.is_empty(),
        "short children exited but their output pipes stayed open: {failures:?}"
    );
}
