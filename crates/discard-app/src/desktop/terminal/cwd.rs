//! Pick the working folder of a channel terminal.
use discard_core::{model::ChannelId, store::Store};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

/// The terminal needs a project channel. A direct message has none.
pub const NO_PROJECT: &str = "Direct messages have no terminal. Open a project channel to use the terminal.";

/// The first folder that exists: the bound worktree, then the repository main checkout, then the project folder.
/// The repository folder is read only when the worktree is missing, because finding it can scan the project.
pub fn choose(
    worktree: Option<PathBuf>,
    repository: impl FnOnce() -> Option<PathBuf>,
    project: PathBuf,
    is_dir: impl Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    if let Some(path) = worktree.filter(|path| is_dir(path)) {
        return Ok(path);
    }
    if let Some(path) = repository().filter(|path| is_dir(path)) {
        return Ok(path);
    }
    if is_dir(&project) {
        return Ok(project);
    }
    Err(format!("The project folder {} does not exist.", project.display()))
}

/// The folder and the project ID of a channel. Errors for a direct message.
/// The store lock is held only while the saved rows are read.
pub fn for_channel(store: &Mutex<Store>, channel: &ChannelId) -> Result<(PathBuf, String), String> {
    let (project, record, repository_id) = {
        let store = store.lock().unwrap_or_else(|error| error.into_inner());
        let loaded = store.load_channel(channel).map_err(|error| error.to_string())?;
        let project_id = loaded.project_id.ok_or(NO_PROJECT)?;
        let project = store
            .list_projects()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|project| project.id == project_id)
            .ok_or("This project is no longer available.")?;
        let record = store.worktree_for(channel).map_err(|error| error.to_string())?;
        let repository_id = store
            .project_channel_metadata(channel)
            .map_err(|error| error.to_string())?
            .map(|metadata| metadata.repository)
            .unwrap_or_default();
        (project, record, repository_id)
    };
    let project_path = PathBuf::from(&project.path);
    let recorded_repository = record
        .as_ref()
        .map(|record| PathBuf::from(&record.repo))
        .filter(|path| !path.as_os_str().is_empty());
    let cwd = choose(
        record.map(|record| PathBuf::from(record.path)),
        || {
            recorded_repository.or_else(|| {
                discard_core::worktree::discover_repositories(&project_path)
                    .ok()?
                    .into_iter()
                    .find(|repository| repository.id == repository_id)
                    .map(|repository| repository.path)
            })
        },
        project_path.clone(),
        |path| path.is_dir(),
    )?;
    Ok((cwd, project.id.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::{
        agent::AgentConfig,
        capability::ApprovalPolicy,
        model::{AgentId, Channel, Project, ProjectId, WorkspaceMode},
        store::WorktreeRecord,
    };

    fn exists(paths: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |path| paths.iter().any(|candidate| Path::new(candidate) == path)
    }

    #[test]
    fn a_bound_worktree_wins_when_it_exists() {
        let cwd = choose(Some("/w".into()), || panic!("not read"), "/p".into(), exists(&["/w", "/r", "/p"]));
        assert_eq!(cwd.unwrap(), Path::new("/w"));
    }

    #[test]
    fn a_missing_worktree_falls_back_to_the_repository() {
        let cwd = choose(Some("/w".into()), || Some("/r".into()), "/p".into(), exists(&["/r", "/p"]));
        assert_eq!(cwd.unwrap(), Path::new("/r"));
    }

    #[test]
    fn no_worktree_uses_the_repository_then_the_project() {
        assert_eq!(choose(None, || Some("/r".into()), "/p".into(), exists(&["/r", "/p"])).unwrap(), Path::new("/r"));
        assert_eq!(choose(None, || Some("/r".into()), "/p".into(), exists(&["/p"])).unwrap(), Path::new("/p"));
        assert_eq!(choose(None, || None, "/p".into(), exists(&["/p"])).unwrap(), Path::new("/p"));
    }

    #[test]
    fn a_missing_project_folder_is_an_error() {
        let error = choose(None, || None, "/p".into(), exists(&[])).unwrap_err();
        assert!(error.contains("/p"), "{error}");
    }

    fn agent(store: &Store) -> AgentId {
        store
            .create_agent(&AgentConfig {
                workspace_write: false,
                sandbox_autorun: false,
                delegation: false, native_resume: false,
                reasoning_effort: None,
                name: "Agent".into(),
                provider: "codex".into(),
                model: Some("model".into()),
                avatar: None,
                system_prompt: String::new(),
                policy: ApprovalPolicy::Ask,
                plugin_write: false,
            })
            .unwrap()
    }

    fn channel(id: &str, project: Option<&str>, agent_id: &AgentId) -> Channel {
        Channel {
            id: ChannelId(id.into()),
            project_id: project.map(|project| ProjectId(project.into())),
            name: id.into(),
            agent_id: agent_id.clone(),
            workspace_mode: WorkspaceMode::MainCheckout,
        }
    }

    #[test]
    fn a_channel_without_a_project_has_no_terminal() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        {
            let store = store.lock().unwrap();
            let agent = agent(&store);
            store.insert_channel(&channel("dm", None, &agent)).unwrap();
        }
        assert_eq!(for_channel(&store, &ChannelId("dm".into())).unwrap_err(), NO_PROJECT);
    }

    #[test]
    fn a_project_channel_uses_its_worktree_then_the_project_folder() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("project");
        let worktree = root.path().join("worktree");
        std::fs::create_dir(&project_dir).unwrap();
        std::fs::create_dir(&worktree).unwrap();
        let store = Mutex::new(Store::open_in_memory().unwrap());
        {
            let store = store.lock().unwrap();
            store.insert_project(&Project { id: ProjectId("p1".into()), name: "p".into(), path: project_dir.to_string_lossy().into() }).unwrap();
            let agent = agent(&store);
            store.insert_channel(&channel("c1", Some("p1"), &agent)).unwrap();
            store.insert_channel(&channel("c2", Some("p1"), &agent)).unwrap();
            store
                .save_worktree(&WorktreeRecord {
                    channel_id: ChannelId("c2".into()),
                    path: worktree.to_string_lossy().into(),
                    branch: "b".into(),
                    base_commit: String::new(),
                    repo: String::new(),
                    owned: true,
                })
                .unwrap();
        }
        let (cwd, project) = for_channel(&store, &ChannelId("c1".into())).unwrap();
        assert_eq!((cwd.as_path(), project.as_str()), (project_dir.as_path(), "p1"));
        assert_eq!(for_channel(&store, &ChannelId("c2".into())).unwrap().0, worktree);
        std::fs::remove_dir(&worktree).unwrap();
        assert_eq!(for_channel(&store, &ChannelId("c2".into())).unwrap().0, project_dir);
    }
}
