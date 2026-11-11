//! Delegation in the app: the `Delegator` that starts sub-agent threads for the `delegate` tool, and the registry of
//! the tasks that run.
//!
//! A `delegate` call arrives on a worker thread of the MCP server. It creates one thread for each task, starts the
//! thread's engine as a parked thread (the thread is never opened or followed), sends the brief, and blocks until
//! every task has a final state. Updates of the thread engines reach the call through `SubAgents::observe`, which
//! `AppCore::apply_runtime` calls for every update. The call holds no lock while it waits.
use super::channels::Channels;
use super::core::SubAgentNote;
use discard_core::delegation::{DelegateResult, DelegateTask, Delegator, MAX_TASKS, SUB_AGENT_TIMEOUT};
use discard_core::model::{AgentId, ChannelId, MessageAuthor, ThreadState};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Mutex, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How a task ended, as the registry reports it to the waiting call.
#[derive(Debug)]
pub struct Ended {
    pub index: usize,
    pub thread: String,
    pub state: ThreadState,
    /// The sub-agent's final message. Empty when it saved none.
    pub text: String,
    /// The `Update::Error` and `Update::SetupNeeded` texts of the run.
    pub detail: String,
}

struct Task {
    parent: String,
    call: u64,
    index: usize,
    tx: Sender<Ended>,
    detail: Vec<String>,
    /// A stop was requested. A stop that arrives before the brief started the turn is sent again after the brief.
    cancel_requested: bool,
}

/// One `delegate` call that runs: the tasks that did not start yet still hold a place under the limit.
struct CallState {
    parent: String,
    unstarted: usize,
}

#[derive(Default)]
struct Inner {
    /// The sub-agent tasks that run now, by thread channel ID.
    tasks: HashMap<String, Task>,
    calls: HashMap<u64, CallState>,
    /// Parent channels whose running turn was stopped. A task that starts later in the same turn is stopped at once.
    /// The mark ends with the turn.
    stopped: HashSet<String>,
}

/// The sub-agent tasks that run now, the calls that start them, and the stops of parent turns.
#[derive(Default)]
pub struct SubAgents {
    inner: Mutex<Inner>,
    calls: AtomicU64,
}

impl SubAgents {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> { self.inner.lock().unwrap_or_else(|e| e.into_inner()) }

    /// Start a call of `count` tasks for `parent`, or say how many sub-agents of the parent run now when `limit`
    /// would be exceeded. The check and the reservation are one step, so parallel calls cannot pass the limit together.
    pub fn begin_call(&self, parent: &str, count: usize, limit: usize) -> Result<u64, usize> {
        let mut inner = self.inner();
        let running = inner.tasks.values().filter(|task| task.parent == parent).count()
            + inner.calls.values().filter(|call| call.parent == parent).map(|call| call.unstarted).sum::<usize>();
        if running + count > limit { return Err(running); }
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        inner.calls.insert(call, CallState { parent: parent.to_owned(), unstarted: count });
        Ok(call)
    }

    /// End a call. Its tasks that did not start give their places back.
    pub fn end_call(&self, call: u64) { self.inner().calls.remove(&call); }

    /// A task of the call will not start: it was cancelled or could not be created.
    pub fn skip(&self, call: u64) {
        if let Some(state) = self.inner().calls.get_mut(&call) { state.unstarted = state.unstarted.saturating_sub(1); }
    }

    /// Track a task before its engine starts, so no update is missed. A task of a parent whose turn was stopped
    /// starts with its stop already requested.
    pub fn register(&self, thread: &str, parent: &str, call: u64, index: usize, tx: Sender<Ended>) {
        let mut inner = self.inner();
        if let Some(state) = inner.calls.get_mut(&call) { state.unstarted = state.unstarted.saturating_sub(1); }
        let cancel_requested = inner.stopped.contains(parent);
        inner.tasks.insert(thread.to_owned(), Task { parent: parent.to_owned(), call, index, tx, detail: Vec::new(), cancel_requested });
    }

    /// Stop tracking a task. Returns whether it was tracked.
    pub fn forget(&self, thread: &str) -> bool { self.inner().tasks.remove(thread).is_some() }

    pub fn is_running(&self, thread: &str) -> bool { self.inner().tasks.contains_key(thread) }

    /// The running tasks of a parent channel, in no particular order.
    pub fn threads_of(&self, parent: &str) -> Vec<String> {
        self.inner().tasks.iter().filter(|(_, task)| task.parent == parent).map(|(thread, _)| thread.clone()).collect()
    }

    /// Whether a stop was requested for the task. A stop that comes before the brief started the turn is lost, so
    /// the call asks again after it sent the brief.
    pub fn cancel_requested(&self, thread: &str) -> bool { self.inner().tasks.get(thread).is_some_and(|task| task.cancel_requested) }

    /// Whether the running turn of the parent was stopped.
    pub fn parent_stopped(&self, parent: &str) -> bool { self.inner().stopped.contains(parent) }

    /// Mark the tasks of a parent as stopped and return their threads, for the caller to send the stop. With
    /// `turn_running`, the stop is also recorded for the parent, so tasks that start later in the same turn stop too.
    pub fn request_cancel_parent(&self, parent: &str, turn_running: bool) -> Vec<String> {
        if turn_running { self.inner().stopped.insert(parent.to_owned()); }
        self.request_cancel(|task| task.parent == parent)
    }

    /// The turn of a parent ended: the recorded stop has no meaning for the next turn.
    pub fn parent_turn_ended(&self, parent: &str) { self.inner().stopped.remove(parent); }

    /// Mark the tasks of one call as stopped and return their threads.
    pub fn request_cancel_call(&self, call: u64) -> Vec<String> { self.request_cancel(|task| task.call == call) }

    fn request_cancel(&self, matches: impl Fn(&Task) -> bool) -> Vec<String> {
        let mut inner = self.inner();
        inner.tasks.iter_mut().filter(|(_, task)| matches(task)).map(|(thread, task)| { task.cancel_requested = true; thread.clone() }).collect()
    }

    /// Read an update of `channel`. A task keeps the error texts, and ends with `Update::SubAgentFinished`.
    pub(crate) fn observe(&self, channel: &str, note: &SubAgentNote) {
        match note {
            SubAgentNote::Detail(text) | SubAgentNote::SetupNeeded(text) => {
                if let Some(task) = self.inner().tasks.get_mut(channel) { task.detail.push(text.clone()); }
            }
            SubAgentNote::Finished { state, text } => { self.end(channel, *state, text); }
            _ => {}
        }
    }

    /// End a task with a final state. Returns whether it was running.
    pub fn end(&self, thread: &str, state: ThreadState, text: &str) -> bool {
        let Some(task) = self.inner().tasks.remove(thread) else { return false };
        // The call may have stopped waiting; the send is then of no interest.
        let _ = task.tx.send(Ended { index: task.index, thread: thread.to_owned(), state, text: text.to_owned(), detail: task.detail.join(" ") });
        true
    }
}

/// The time limits of a run. Tests make them short.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// A task is stopped after this long, and its task returns "timed out".
    pub timeout: Duration,
    /// After the stop, how long the call waits for the engine to confirm before it gives up on the task.
    pub grace: Duration,
}
impl Default for Limits {
    fn default() -> Self { Self { timeout: SUB_AGENT_TIMEOUT, grace: Duration::from_secs(30) } }
}

/// Starts the sub-agent threads of `delegate` calls.
pub struct AppDelegator {
    // Channels owns the launcher, which owns the MCP runtime, which owns the slot that holds this delegator.
    channels: Weak<Channels>,
    limits: Limits,
}

impl AppDelegator {
    pub fn new(channels: Weak<Channels>) -> Self { Self { channels, limits: Limits::default() } }
    pub fn with_limits(mut self, limits: Limits) -> Self { self.limits = limits; self }
}

/// Stops the tasks of a call that is still running when the call ends early or unwinds.
struct CallGuard<'a> { core: &'a super::core::AppCore, call: u64 }
impl Drop for CallGuard<'_> {
    fn drop(&mut self) {
        self.core.cancel_sub_agent_call(self.call);
        self.core.sub_agents().end_call(self.call);
    }
}

/// The result of a task that did not start because the parent turn was stopped or ended. It has no thread.
fn not_started(name: &str) -> DelegateResult {
    DelegateResult { thread_channel_id: ChannelId(String::new()), name: name.to_owned(), state: ThreadState::Cancelled, text: "The turn that asked for it was stopped before this sub-agent started.".into() }
}

fn result_of(ended: Ended, name: &str) -> DelegateResult {
    let text = if ended.state == ThreadState::Done { ended.text } else { ended.detail };
    DelegateResult { thread_channel_id: ChannelId(ended.thread), name: name.to_owned(), state: ended.state, text }
}

impl Delegator for AppDelegator {
    fn delegate(&self, parent: &ChannelId, calling_agent: &AgentId, tasks: Vec<DelegateTask>) -> Result<Vec<DelegateResult>, String> {
        let channels = self.channels.upgrade().ok_or("Discard is closing.")?;
        let core = channels.core.clone();
        // Sub-agents belong to a turn of the parent. A call outside a running turn starts nothing.
        if !core.channel_running(&parent.0) { return Err("This channel has no running turn, so no sub-agent started. Call delegate only while you answer a request.".into()); }
        let author = {
            let store = channels.store.lock().unwrap_or_else(|e| e.into_inner());
            let config = store.load_agent(calling_agent).map_err(|e| e.to_string())?;
            MessageAuthor { agent_id: calling_agent.clone(), name: config.name, provider: config.provider, avatar: config.avatar }
        };
        let call = core.sub_agents().begin_call(&parent.0, tasks.len(), MAX_TASKS).map_err(|running| format!(
            "A parent runs at most {MAX_TASKS} sub-agents at one time. {running} already run for this channel, and this call asks for {}. Wait for them to finish, or give fewer tasks. No sub-agent started.", tasks.len()))?;
        let _guard = CallGuard { core: &core, call };
        let (tx, rx) = mpsc::channel::<Ended>();
        let mut results: Vec<Option<DelegateResult>> = tasks.iter().map(|_| None).collect();
        // Thread ID to task index, for the tasks that have not ended.
        let mut waiting: HashMap<String, usize> = HashMap::new();

        for (index, task) in tasks.iter().enumerate() {
            // A stop of the parent turn, or the end of the turn, cancels the tasks that did not start. They get no thread.
            if core.sub_agents().parent_stopped(&parent.0) || !core.channel_running(&parent.0) {
                core.sub_agents().skip(call);
                results[index] = Some(not_started(&task.name));
                continue;
            }
            let created = channels.store.lock().unwrap_or_else(|e| e.into_inner()).create_sub_agent_thread(parent, &task.name, &task.agent);
            let thread = match created {
                Ok(thread) => thread,
                // No thread exists, so the result has no thread ID.
                Err(error) => { core.sub_agents().skip(call); results[index] = Some(DelegateResult { thread_channel_id: ChannelId(String::new()), name: task.name.clone(), state: ThreadState::Failed, text: error.to_string() }); continue; }
            };
            let id = thread.id.0.clone();
            core.sub_agents().register(&id, &parent.0, call, index, tx.clone());
            match channels.start_sub_agent_thread(&thread, task.brief.clone(), author.clone()) {
                Ok(()) => {
                    waiting.insert(id.clone(), index);
                    // A stop that came before the brief started the turn found nothing to stop.
                    if core.sub_agents().cancel_requested(&id) { core.cancel_thread(&id); }
                }
                Err(error) => {
                    core.sub_agents().forget(&id);
                    channels.fail_sub_agent_thread(&thread.id, &error);
                    results[index] = Some(DelegateResult { thread_channel_id: thread.id.clone(), name: task.name.clone(), state: ThreadState::Failed, text: error });
                }
            }
        }
        drop(tx);

        let mut deadline = Instant::now() + self.limits.timeout;
        let mut stopped = false;
        while !waiting.is_empty() {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(ended) => {
                    waiting.remove(&ended.thread);
                    let index = ended.index;
                    results[index] = Some(result_of(ended, &tasks[index].name));
                }
                Err(RecvTimeoutError::Timeout) if !stopped => {
                    // The first final state wins: set "timed out" before the stop makes the engine say "cancelled".
                    stopped = true;
                    for thread in waiting.keys() {
                        let _ = channels.store.lock().unwrap_or_else(|e| e.into_inner()).finish_sub_agent_thread(&ChannelId(thread.clone()), ThreadState::TimedOut);
                        core.cancel_thread(thread);
                    }
                    deadline = Instant::now() + self.limits.grace;
                }
                Err(error) => {
                    // The engines did not confirm the stop in time, or the channel closed: end what is left.
                    let (state, text) = match error {
                        RecvTimeoutError::Timeout => (ThreadState::TimedOut, "The sub-agent did not stop in time."),
                        RecvTimeoutError::Disconnected => (ThreadState::Failed, "The sub-agent session ended before it finished."),
                    };
                    for (thread, index) in waiting.drain() {
                        core.sub_agents().forget(&thread);
                        results[index] = Some(DelegateResult { thread_channel_id: ChannelId(thread), name: tasks[index].name.clone(), state, text: text.into() });
                    }
                }
            }
        }
        Ok(results.into_iter().map(|result| result.expect("every task has a result once none is waiting")).collect())
    }
}
