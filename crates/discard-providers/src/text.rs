//! Process lifetime and diagnostics for isolated, one-shot text requests.

use discard_core::{provider::ProviderError, redact::redact};
use nix::{sys::signal::{killpg, Signal}, unistd::Pid};
use std::{collections::VecDeque, path::Path, process::Stdio, sync::{Arc, Mutex}, time::Duration};
use tokio::{io::{AsyncBufReadExt, BufReader}, process::{Child, Command}, task::JoinHandle};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) fn protocol(message: &str) -> ProviderError { ProviderError::Protocol(message.into()) }
pub(crate) fn timed_out() -> ProviderError { protocol("The model did not answer in time.") }

pub(crate) struct TextProcess {
    pub child: Child,
    pgid: Option<i32>,
    tail: Arc<Mutex<VecDeque<String>>>,
    stderr_task: JoinHandle<()>,
}

impl TextProcess {
    pub fn spawn(mut command: Command, cwd: &Path) -> Result<Self, ProviderError> {
        let mut child = discard_core::process::spawn_tokio(command.current_dir(cwd)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .kill_on_drop(true).process_group(0)).map_err(|e| ProviderError::Spawn(e.to_string()))?;
        let pgid = child.id().and_then(|id| i32::try_from(id).ok());
        let stderr = child.stderr.take().expect("piped stderr");
        let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
        let reader_tail = tail.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut tail = reader_tail.lock().unwrap_or_else(|e| e.into_inner());
                if tail.len() == 20 { tail.pop_front(); }
                // Keep diagnostics in memory only. Prompts and results must not be logged.
                tail.push_back(redact(&line));
            }
        });
        Ok(Self { child, pgid, tail, stderr_task })
    }

    fn kill_group(&mut self) {
        // Retain the process-group ID even after wait() reaps the group leader.
        // Clear it after cleanup so a later drop cannot signal a reused group ID.
        if let Some(pgid) = self.pgid.take() { let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL); }
    }

    pub async fn finish<T>(&mut self, result: Result<T, ProviderError>) -> Result<T, ProviderError> {
        self.kill_group();
        let _ = self.child.wait().await;
        let _ = tokio::time::timeout(Duration::from_millis(100), &mut self.stderr_task).await;
        self.stderr_task.abort();
        result.map_err(|error| match error {
            ProviderError::Protocol(message) if message != "The model did not answer in time." => {
                let tail = self.tail.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect::<Vec<_>>().join("\n");
                protocol(&if tail.is_empty() { message } else { format!("{message}\n{tail}") })
            }
            other => other,
        })
    }
}

impl Drop for TextProcess {
    fn drop(&mut self) {
        self.kill_group();
        self.stderr_task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    async fn wait_for_exit(pid: i32) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while nix::sys::signal::kill(Pid::from_raw(pid), None).is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("process group must exit");
    }

    #[tokio::test]
    async fn completion_kills_descendants_after_the_leader_has_already_exited() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 300 </dev/null >/dev/null 2>/dev/null & echo $!"]);
        let mut process = TextProcess::spawn(command, dir.path()).unwrap();
        let mut output = String::new();
        process.child.stdout.take().unwrap().read_to_string(&mut output).await.unwrap();
        let pid = output.trim().parse().unwrap();
        process.child.wait().await.unwrap();
        assert!(process.child.id().is_none());
        process.finish(Ok(())).await.unwrap();
        assert!(process.pgid.is_none(), "finished guards must not signal a potentially reused process-group ID on drop");
        wait_for_exit(pid).await;
    }

    #[tokio::test]
    async fn timeout_kills_the_process_group_and_preserves_the_timeout_message() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "echo diagnostic >&2; sleep 300 & echo $!; wait"]);
        let mut process = TextProcess::spawn(command, dir.path()).unwrap();
        let mut lines = BufReader::new(process.child.stdout.take().unwrap()).lines();
        let pid = lines.next_line().await.unwrap().unwrap().parse().unwrap();
        let result = tokio::time::timeout(Duration::from_millis(20), process.child.wait()).await;
        assert!(result.is_err());
        let error = process.finish::<()>(Err(timed_out())).await.unwrap_err();
        assert!(matches!(error, ProviderError::Protocol(message) if message == "The model did not answer in time."));
        wait_for_exit(pid).await;
    }
}
