use crate::updates::UpdateInfo;
use localcodepilot_core::projects::Project;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env, fs, io,
    path::{Path, PathBuf},
};

const APPLICATION_DIRECTORY: &str = "LocalCodePilot";
const SCAN_ROOTS_FILE: &str = "scan-roots.txt";
const PROJECT_CACHE_FILE: &str = "project-cache.json";
const WORKSPACE_STATE_FILE: &str = "workspace-state.json";
const WORKSPACE_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkspaceState {
    pub schema_version: u32,
    pub last_page: String,
    pub focused_project: Option<PathBuf>,
    pub favorite_projects: Vec<PathBuf>,
    pub hidden_recent_projects: Vec<PathBuf>,
    pub projects: Vec<ProjectHistory>,
    pub preparations: Vec<PreparationHistory>,
    pub last_update_check: u64,
    pub update_remind_after: u64,
    pub cached_update: Option<UpdateInfo>,
    /// Compose volumes the user confirmed as belonging to an environment that has no
    /// containers, for example after `compose down`.
    pub compose_data_confirmations: Vec<ComposeDataConfirmation>,
    /// Last state filter on Projects (`running`, `problem`, `stopped`; empty = all).
    pub project_filter: String,
    /// `list` or empty for cards.
    pub project_view: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ComposeDataConfirmation {
    /// `docker` or `podman`.
    pub engine: String,
    pub directory: PathBuf,
    pub volumes: Vec<String>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            last_page: "overview".into(),
            focused_project: None,
            favorite_projects: Vec::new(),
            hidden_recent_projects: Vec::new(),
            projects: Vec::new(),
            preparations: Vec::new(),
            last_update_check: 0,
            update_remind_after: 0,
            cached_update: None,
            compose_data_confirmations: Vec::new(),
            project_filter: String::new(),
            project_view: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProjectHistory {
    pub path: PathBuf,
    pub last_activity: u64,
    pub services: Vec<ServiceHistory>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServiceHistory {
    pub id: String,
    pub name: String,
    pub command: String,
    pub status: String,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub updated_at: u64,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PreparationHistory {
    pub project_path: PathBuf,
    pub status: String,
    pub updated_at: u64,
    pub steps: Vec<String>,
    pub logs: Vec<String>,
}

impl WorkspaceState {
    pub fn touch_project(&mut self, path: &Path, timestamp: u64) {
        if let Some(project) = self
            .projects
            .iter_mut()
            .find(|project| project.path == path)
        {
            project.last_activity = project.last_activity.max(timestamp);
        } else {
            self.projects.push(ProjectHistory {
                path: path.to_path_buf(),
                last_activity: timestamp,
                ..ProjectHistory::default()
            });
        }
    }

    pub fn project(&self, path: &Path) -> Option<&ProjectHistory> {
        self.projects.iter().find(|project| project.path == path)
    }

    pub fn service(&self, id: &str) -> Option<&ServiceHistory> {
        self.projects
            .iter()
            .flat_map(|project| &project.services)
            .find(|service| service.id == id)
    }

    pub fn service_mut(&mut self, id: &str) -> Option<&mut ServiceHistory> {
        self.projects
            .iter_mut()
            .flat_map(|project| &mut project.services)
            .find(|service| service.id == id)
    }

    pub fn record_service(&mut self, project_path: &Path, service: ServiceHistory) {
        let project = if let Some(project) = self
            .projects
            .iter_mut()
            .find(|project| project.path == project_path)
        {
            project
        } else {
            self.projects.push(ProjectHistory {
                path: project_path.to_path_buf(),
                ..ProjectHistory::default()
            });
            self.projects.last_mut().expect("project was just inserted")
        };
        project.last_activity = project.last_activity.max(service.updated_at);
        if let Some(existing) = project
            .services
            .iter_mut()
            .find(|existing| existing.id == service.id)
        {
            *existing = service;
        } else {
            project.services.push(service);
        }
    }

    pub fn record_preparation(&mut self, mut preparation: PreparationHistory) {
        const MAX_HISTORY: usize = 40;
        const MAX_LOG_LINES: usize = 500;
        if preparation.logs.len() > MAX_LOG_LINES {
            preparation.logs = preparation
                .logs
                .split_off(preparation.logs.len() - MAX_LOG_LINES);
        }
        if let Some(existing) = self.preparations.iter_mut().find(|entry| {
            entry.project_path == preparation.project_path && entry.status == "em andamento"
        }) {
            *existing = preparation;
        } else {
            self.preparations.push(preparation);
        }
        self.preparations
            .sort_by_key(|entry| std::cmp::Reverse(entry.updated_at));
        self.preparations.truncate(MAX_HISTORY);
    }
}

pub fn load_scan_roots() -> Option<Vec<PathBuf>> {
    let contents = fs::read_to_string(config_file_path()?).ok()?;
    Some(parse_existing_roots(&contents))
}

pub fn save_scan_roots(roots: &[PathBuf]) -> io::Result<()> {
    let path = config_file_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "diretório de configuração do usuário não encontrado",
        )
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serialize_roots(roots))
}

pub fn load_project_cache() -> Vec<Project> {
    project_cache_file_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str::<Vec<Project>>(&contents).ok())
        .map(|projects| {
            projects
                .into_iter()
                .filter(|project| project.path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_project_cache(projects: &[Project]) -> io::Result<()> {
    let path = project_cache_file_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "diretório de configurações do usuário não encontrado",
        )
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string(projects).map_err(io::Error::other)?;
    fs::write(path, contents)
}

pub fn load_workspace_state() -> WorkspaceState {
    let mut state = workspace_state_file_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str::<WorkspaceState>(&contents).ok())
        .unwrap_or_default();
    state.schema_version = WORKSPACE_SCHEMA_VERSION;
    state
}

pub fn save_workspace_state(state: &WorkspaceState) -> io::Result<()> {
    let path = workspace_state_file_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "diretório de configuração do usuário não encontrado",
        )
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    fs::write(path, contents)
}

fn parse_existing_roots(contents: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .filter_map(|path| path.canonicalize().ok())
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn serialize_roots(roots: &[PathBuf]) -> String {
    roots
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n")
}

fn config_file_path() -> Option<PathBuf> {
    config_directory().map(|path| path.join(SCAN_ROOTS_FILE))
}

fn workspace_state_file_path() -> Option<PathBuf> {
    config_directory().map(|path| path.join(WORKSPACE_STATE_FILE))
}

fn project_cache_file_path() -> Option<PathBuf> {
    config_directory().map(|path| path.join(PROJECT_CACHE_FILE))
}

fn config_directory() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        return env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join(APPLICATION_DIRECTORY));
    }
    if cfg!(target_os = "macos") {
        return home_directory().map(|path| {
            path.join("Library")
                .join("Application Support")
                .join(APPLICATION_DIRECTORY)
        });
    }
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home_directory().map(|path| path.join(".config")))
        .map(|path| path.join("localcodepilot"))
}

fn home_directory() -> Option<PathBuf> {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn parses_existing_roots_and_ignores_missing_or_duplicate_paths() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("localcodepilot-config-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let contents = format!(
            "{}\n{}\n{}",
            root.display(),
            root.display(),
            root.join("missing").display()
        );

        let parsed = parse_existing_roots(&contents);

        assert_eq!(parsed, vec![root.canonicalize().unwrap()]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn serializes_one_root_per_line() {
        let roots = [PathBuf::from("first"), PathBuf::from("second")];
        assert_eq!(serialize_roots(&roots), "first\nsecond");
    }

    #[test]
    fn serializes_and_restores_workspace_history() {
        let mut state = WorkspaceState {
            last_page: "processes".into(),
            focused_project: Some(PathBuf::from("project")),
            favorite_projects: vec![PathBuf::from("project")],
            hidden_recent_projects: vec![PathBuf::from("fixture")],
            ..WorkspaceState::default()
        };
        state.touch_project(Path::new("opened"), 41);
        state.record_service(
            Path::new("project"),
            ServiceHistory {
                id: "project::npm run dev".into(),
                name: "Frontend".into(),
                command: "npm run dev".into(),
                status: "running".into(),
                port: Some(5173),
                url: Some("http://localhost:5173".into()),
                updated_at: 42,
                logs: vec!["ready".into()],
            },
        );

        let json = serde_json::to_string(&state).unwrap();
        let restored: WorkspaceState = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, state);
        assert_eq!(
            restored.project(Path::new("opened")).unwrap().last_activity,
            41
        );
        assert!(
            restored
                .hidden_recent_projects
                .contains(&PathBuf::from("fixture"))
        );
        assert_eq!(
            restored.service("project::npm run dev").unwrap().port,
            Some(5173)
        );
    }

    #[test]
    fn restores_workspace_state_without_recent_visibility_field() {
        let state: WorkspaceState = serde_json::from_str(r#"{"schema_version":3}"#).unwrap();
        assert!(state.hidden_recent_projects.is_empty());
    }
}
