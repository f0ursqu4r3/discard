//! The memory prompt split into a fixed part and a volatile part for native provider resume.

use discard_core::{
    agent::AgentConfig, capability::ApprovalPolicy, delegation::DelegationServices,
    engine::{MemoryOptions, MemoryService, PreparedMemory}, memory::{MemoryStore, Scope},
    memory_items::{ItemKind, MutationOrigin},
    model::{AgentId, Channel, ChannelId, ProjectId, WorkspaceMode}, store::{Store, WorktreeRecord},
    workspace::GitWorkspace, worktree_access::WorktreeAccess, write_requests::WriteRequests,
};
use discard_mcp::MemoryRuntime;
use std::{path::Path, sync::{Arc, Mutex}};

fn git(path: &Path, args: &[&str]) {
    let output = discard_core::process::spawn_std(std::process::Command::new("git").args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"]).args(args).current_dir(path).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

struct Fx {
    runtime: Arc<MemoryRuntime>,
    memory: Arc<MemoryStore>,
    main: AgentId,
    project: ProjectId,
    general: ChannelId,
    main_checkout: ChannelId,
    unbound: ChannelId,
    store: Arc<Mutex<Store>>,
    access: Arc<WorktreeAccess>,
    repo: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn agent_config(name: &str, delegation: bool) -> AgentConfig {
    AgentConfig { workspace_write: true, sandbox_autorun: false, native_resume: false, delegation, reasoning_effort: None, name: name.into(), avatar: None, provider: "fake".into(), model: Some("test".into()), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }
}

async fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo"); std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]); git(&repo, &["commit", "--allow-empty", "-m", "initial"]);
    git(&repo, &["branch", "spare"]);
    let store = Store::open_in_memory().unwrap();
    let main = store.create_agent(&agent_config("Main", true)).unwrap();
    let project = store.import_project_folder(&repo).unwrap();
    store.set_project_main_agent(&project.id, &main).unwrap();
    let general = store.ensure_project_general_channel(&project.id, &main).unwrap();
    let on_main = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "main".into(), agent_id: main.clone(), workspace_mode: WorkspaceMode::Worktree };
    store.create_named_repository_channel(&on_main, "", Some("main"), Some(&WorktreeRecord { channel_id: on_main.id.clone(), path: repo.to_string_lossy().into(), repo: repo.to_string_lossy().into(), branch: "main".into(), base_commit: String::new(), owned: false })).unwrap();
    let unbound = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "spare".into(), agent_id: main.clone(), workspace_mode: WorkspaceMode::Worktree };
    store.create_named_repository_channel(&unbound, "", Some("spare"), None).unwrap();
    let store = Arc::new(Mutex::new(store));
    let access = Arc::new(WorktreeAccess::new(store.clone()));
    let workspace = Arc::new(GitWorkspace::new(&dir.path().join("worktrees"), store.clone()));
    let memory = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    memory.create(&Scope::Global, "core/user.md", "Kyle").unwrap();
    memory.create_item(&Scope::Global, ItemKind::Instruction, "Answer briefly.", None, MutationOrigin::User).unwrap();
    memory.create_item(&Scope::Project(project.id.clone()), ItemKind::Preference, "Prefer tabs.", None, MutationOrigin::User).unwrap();
    memory.create_item(&Scope::Project(project.id.clone()), ItemKind::Fact, "Cargo builds the workspace.", None, MutationOrigin::User).unwrap();
    memory.create_item(&Scope::Global, ItemKind::Fact, "Bananas are yellow.", None, MutationOrigin::User).unwrap();
    let services = DelegationServices { store: store.clone(), requests: Arc::default(), delegator: Arc::default() };
    let runtime = MemoryRuntime::start_with_delegation(memory.clone(), &dir.path().join("runtime"), None, Some(access.clone()), Some((Arc::new(WriteRequests::default()), workspace)), Some(services)).await.unwrap();
    Fx { runtime, memory, main, project: project.id, general: general.id, main_checkout: on_main.id, unbound: unbound.id, store, access, repo, _dir: dir }
}

impl Fx {
    fn turn(&self, channel: &ChannelId, query: &str, requests: bool, delegation: bool) -> PreparedMemory {
        let prepared = self.runtime.prepare_response(&self.main, Some(&self.project), channel, Some("Be Scout."), false, MemoryOptions { query, workspace: None, main_checkout_requests: requests, delegation }).unwrap();
        self.runtime.finish(&prepared.token);
        prepared
    }
}

/// Replace each UUID with `<idN>`, numbered by first appearance, so random ids do not change the text.
fn normalize(text: &str) -> String {
    let (bytes, mut out, mut seen, mut i) = (text.as_bytes(), String::new(), Vec::<&str>::new(), 0);
    let shape = [8, 4, 4, 4, 12];
    while i < bytes.len() {
        let mut j = i;
        let mut ok = true;
        for (n, len) in shape.iter().enumerate() {
            if n > 0 { if bytes.get(j) == Some(&b'-') { j += 1 } else { ok = false; break } }
            if bytes.get(j..j + len).is_some_and(|c| c.iter().all(u8::is_ascii_hexdigit)) { j += len } else { ok = false; break }
        }
        if ok {
            let id = &text[i..j];
            let n = seen.iter().position(|s| *s == id).unwrap_or_else(|| { seen.push(id); seen.len() - 1 });
            out.push_str(&format!("<id{n}>"));
            i = j;
        } else {
            let c = text[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

fn golden(name: &str, actual: &str) {
    let actual = normalize(actual);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    if std::env::var("GOLDEN_WRITE").is_ok() { std::fs::write(&path, &actual).unwrap(); }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), actual, "{name}");
}

#[tokio::test]
async fn system_prompt_is_unchanged_by_the_split() {
    let f = fx().await;
    golden("general_delegation.txt", &f.turn(&f.general, "how does cargo work", false, true).system_prompt);
    golden("main_checkout_requests.txt", &f.turn(&f.main_checkout, "how does cargo work", true, false).system_prompt);
    golden("main_checkout_denied.txt", &f.turn(&f.main_checkout, "nothing", false, false).system_prompt);
    golden("unbound.txt", &f.turn(&f.unbound, "bananas", false, false).system_prompt);
}

fn ids(parts: &discard_core::engine::ResumableParts) -> Vec<&str> { parts.live_ids.iter().map(String::as_str).collect() }

#[tokio::test]
async fn parts_separate_fixed_and_volatile_lines() {
    let f = fx().await;
    let on_main = f.turn(&f.main_checkout, "q", true, false).parts.unwrap();
    for needle in ["Be Scout.", "Answer briefly.", "Prefer tabs.", "Kyle", "Worktree access:", "Project access:"] {
        assert!(on_main.fixed_prompt.contains(needle), "fixed lacks {needle}");
        assert!(!on_main.session_state.contains(needle), "volatile has {needle}");
    }
    for needle in ["checked out in the main checkout", "Primary / Moderator"] {
        assert!(on_main.session_state.contains(needle), "volatile lacks {needle}");
        assert!(!on_main.fixed_prompt.contains(needle), "fixed has {needle}");
    }
    let general = f.turn(&f.general, "q", false, true).parts.unwrap();
    assert!(general.fixed_prompt.contains("Delegation:") && !general.session_state.contains("Delegation:"));
    let unbound = f.turn(&f.unbound, "q", false, false).parts.unwrap();
    assert!(unbound.session_state.contains("This branch channel has no worktree") && !unbound.fixed_prompt.contains("no worktree"));
    // No volatile line: the state is empty.
    let plain = f.runtime.prepare_summary(&f.main, Some(&f.project), &f.general).unwrap();
    f.runtime.finish(&plain.token);
    assert_eq!(plain.parts.unwrap().session_state, "");
}

#[tokio::test]
async fn approval_outcomes_are_volatile() {
    let f = fx().await;
    let linked = f._dir.path().join("linked");
    git(&f.repo, &["worktree", "add", "-q", "-b", "feat", linked.to_str().unwrap()]);
    let channel = Channel { id: ChannelId::new(), project_id: Some(f.project.clone()), name: "feat".into(), agent_id: f.main.clone(), workspace_mode: WorkspaceMode::Worktree };
    f.store.lock().unwrap().create_named_repository_channel(&channel, "", Some("feat"), Some(&WorktreeRecord { channel_id: channel.id.clone(), path: linked.to_string_lossy().into(), repo: f.repo.to_string_lossy().into(), branch: "feat".into(), base_commit: String::new(), owned: false })).unwrap();
    let request = f.access.request_removal(&channel.id, &f.main).unwrap();
    f.access.decide(&channel.id, &request.id, false).unwrap();
    let parts = f.turn(&channel.id, "q", false, false).parts.unwrap();
    assert!(parts.session_state.contains("Recent worktree approval outcomes (data):"), "{}", parts.session_state);
    assert!(!parts.fixed_prompt.contains("Recent worktree approval outcomes"));
}

#[tokio::test]
async fn capture_pause_is_not_in_the_fixed_prompt() {
    let f = fx().await;
    let before = f.turn(&f.general, "q", false, false);
    assert!(!before.parts.as_ref().unwrap().capture_paused);
    let bananas = f.memory.list_items(&[Scope::Global]).unwrap().into_iter().find(|i| i.content.contains("Bananas")).unwrap();
    f.memory.forget_item_in(&bananas.id, bananas.version, Some(&f.general)).unwrap();
    let after = f.turn(&f.general, "q", false, false);
    let parts = after.parts.unwrap();
    assert!(parts.capture_paused);
    assert!(after.system_prompt.contains("Automatic capture is paused"));
    assert!(!parts.fixed_prompt.contains("Automatic capture is paused") && !parts.session_state.contains("Automatic capture is paused"));
    // Pausing leaves the fixed prompt unchanged.
    assert_eq!(parts.fixed_prompt, before.parts.unwrap().fixed_prompt);
}

#[tokio::test]
async fn selected_excludes_instruction_and_preference_items() {
    let f = fx().await;
    let parts = f.turn(&f.general, "cargo", false, false).parts.unwrap();
    assert_eq!(parts.selected.len(), 1, "{:?}", parts.selected);
    let item = &parts.selected[0];
    assert_eq!((item.text.as_str(), item.kind.as_str(), item.scope.as_str(), item.version), ("Cargo builds the workspace.", "fact", &*format!("project:{}", f.project.0), 1));
    assert!(!item.id.is_empty());
    // Every live item, mandatory ones included.
    assert_eq!(parts.live_ids.len(), 4, "{:?}", ids(&parts));
    assert!(parts.live_ids.contains(&item.id));
    // With no query match, nothing is selected, but live ids stay.
    let none = f.turn(&f.general, "zzzz", false, false).parts.unwrap();
    assert!(none.selected.is_empty() && none.live_ids.len() == 4);
}

#[tokio::test]
async fn live_ids_omit_deleted_items() {
    let f = fx().await;
    let bananas = f.memory.list_items(&[Scope::Global]).unwrap().into_iter().find(|i| i.content.contains("Bananas")).unwrap();
    f.memory.forget_item(&bananas.id, bananas.version).unwrap();
    let parts = f.turn(&f.general, "bananas", false, false).parts.unwrap();
    assert!(!parts.live_ids.contains(&bananas.id));
    assert!(parts.selected.is_empty());
    assert_eq!(parts.live_ids.len(), 3);
}

#[tokio::test]
async fn changing_an_instruction_changes_core_hash() {
    let f = fx().await;
    let first = f.turn(&f.general, "q", false, false).parts.unwrap();
    assert_eq!(first.core_hash, f.turn(&f.general, "q", false, false).parts.unwrap().core_hash);
    assert_eq!(first.core_hash.len(), 64);
    // A non-mandatory fact does not change it.
    f.memory.create_item(&Scope::Global, ItemKind::Fact, "Water is wet.", None, MutationOrigin::User).unwrap();
    assert_eq!(first.core_hash, f.turn(&f.general, "q", false, false).parts.unwrap().core_hash);
    let instruction = f.memory.list_items(&[Scope::Global]).unwrap().into_iter().find(|i| i.kind == ItemKind::Instruction).unwrap();
    f.memory.update_item(&instruction.id, instruction.version, "Answer at length.", MutationOrigin::User).unwrap();
    let changed = f.turn(&f.general, "q", false, false).parts.unwrap();
    assert_ne!(first.core_hash, changed.core_hash);
    assert_ne!(first.fixed_prompt, changed.fixed_prompt);
}

#[tokio::test]
async fn resume_dir_is_stable_per_channel_and_agent() {
    use std::os::unix::fs::PermissionsExt;
    let f = fx().await;
    let a = f.runtime.resume_dir(&f.general, &f.main).unwrap();
    assert_eq!(a, f.runtime.resume_dir(&f.general, &f.main).unwrap());
    assert_ne!(a, f.runtime.resume_dir(&f.unbound, &f.main).unwrap());
    assert_ne!(a, f.runtime.resume_dir(&f.general, &AgentId("other".into())).unwrap());
    assert!(a.is_dir());
    assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(a, f._dir.path().join("sessions").join(&f.general.0).join(&f.main.0));
    for bad in ["", ".", "..", "../x", "a/b", "a\\b"] {
        assert!(f.runtime.resume_dir(&ChannelId(bad.into()), &f.main).is_none(), "{bad:?}");
        assert!(f.runtime.resume_dir(&f.general, &AgentId(bad.into())).is_none(), "{bad:?}");
    }
}
