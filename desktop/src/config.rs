use crate::{secrets::mask_lines, storage, updates::UpdateInfo};
use localcodepilot_core::{custom_commands::CustomCommand, projects::Project};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env, io,
    path::{Path, PathBuf},
};

const APPLICATION_DIRECTORY: &str = "LocalCodePilot";
const SCAN_ROOTS_FILE: &str = "scan-roots.txt";
const PROJECT_CACHE_FILE: &str = "project-cache.json";
const WORKSPACE_STATE_FILE: &str = "workspace-state.json";
const WORKSPACE_SCHEMA_VERSION: u32 = 3;
/// Saved logs older than this are removed when the app opens.
pub const LOG_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;

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
    /// The user collapsed the sidebar to icons. The sidebar starts expanded.
    pub sidebar_collapsed: bool,
    /// Start commands the person defined for projects without a detected one.
    pub custom_commands: Vec<CustomCommand>,
    /// Identities (SHA-256 of folder, program, arguments and resolved script) of the
    /// project commands the person confirmed before their first run. Older histories have
    /// none, so every project asks once after the update.
    pub confirmed_commands: Vec<String>,
    /// The last time the configuration files could not be read, until the person clicks
    /// "Entendi" in Configurações (T021).
    pub config_recovery: Option<ConfigRecovery>,
}

/// A recovery of the configuration files, kept so the notice can be read again after it is
/// closed. Holds paths and the parser's cause, never the content of the files.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConfigRecovery {
    /// Every unreadable file had a valid copy, which was restored.
    pub restored: bool,
    pub at: u64,
    pub causes: Vec<String>,
    pub preserved: Vec<PathBuf>,
    pub restored_from: Vec<PathBuf>,
}

impl ConfigRecovery {
    pub fn from_recoveries(recoveries: &[storage::Recovery], at: u64) -> Option<Self> {
        if recoveries.is_empty() {
            return None;
        }
        Some(Self {
            restored: recoveries
                .iter()
                .all(|recovery| recovery.restored_from.is_some()),
            at,
            causes: recoveries
                .iter()
                .map(|recovery| {
                    let name = recovery
                        .file
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    format!("{name}: {}", recovery.cause)
                })
                .collect(),
            preserved: recoveries
                .iter()
                .flat_map(|recovery| recovery.preserved.iter().cloned())
                .collect(),
            restored_from: recoveries
                .iter()
                .filter_map(|recovery| recovery.restored_from.clone())
                .collect(),
        })
    }
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
            sidebar_collapsed: false,
            custom_commands: Vec::new(),
            confirmed_commands: Vec::new(),
            config_recovery: None,
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

    pub fn record_service(&mut self, project_path: &Path, mut service: ServiceHistory) {
        service.logs = mask_lines(&service.logs);
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
        preparation.logs = mask_lines(&preparation.logs);
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

    /// Every saved log line, of services and preparations.
    fn saved_logs_mut(&mut self) -> impl Iterator<Item = (u64, &mut Vec<String>)> {
        let services = self
            .projects
            .iter_mut()
            .flat_map(|project| &mut project.services)
            .map(|service| (service.updated_at, &mut service.logs));
        let preparations = self
            .preparations
            .iter_mut()
            .map(|preparation| (preparation.updated_at, &mut preparation.logs));
        services.chain(preparations)
    }

    /// Empties the logs saved more than [`LOG_RETENTION_SECONDS`] before `now`.
    /// Names, commands, states and steps stay. Returns whether anything changed.
    pub fn expire_old_logs(&mut self, now: u64) -> bool {
        let mut changed = false;
        for (updated_at, logs) in self.saved_logs_mut() {
            if now.saturating_sub(updated_at) > LOG_RETENTION_SECONDS && !logs.is_empty() {
                logs.clear();
                changed = true;
            }
        }
        changed
    }

    /// Masks secrets in logs saved by versions that did not mask them.
    pub fn mask_saved_logs(&mut self) -> bool {
        let mut changed = false;
        for (_, logs) in self.saved_logs_mut() {
            let masked = mask_lines(logs);
            if masked != *logs {
                *logs = masked;
                changed = true;
            }
        }
        changed
    }

    /// Removes every saved log, of services and preparations.
    pub fn clear_saved_logs(&mut self) {
        for (_, logs) in self.saved_logs_mut() {
            logs.clear();
        }
    }
}

/// Reads the scan roots. `None` without a recovery is a first run; `None` with one means the
/// file could not be read and was preserved, so the default folders are used.
pub fn load_scan_roots() -> (Option<Vec<PathBuf>>, Option<storage::Recovery>) {
    let Some(path) = config_file_path() else {
        return (None, None);
    };
    load_scan_roots_from(&path)
}

fn load_scan_roots_from(path: &Path) -> (Option<Vec<PathBuf>>, Option<storage::Recovery>) {
    storage::read(path, |bytes| {
        std::str::from_utf8(bytes)
            .map(parse_existing_roots)
            .map_err(|_| "o arquivo não é um texto UTF-8 válido".to_owned())
    })
    .split()
}

pub fn save_scan_roots(roots: &[PathBuf]) -> io::Result<()> {
    let path = config_file_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "diretório de configuração do usuário não encontrado",
        )
    })?;
    storage::write(&path, serialize_roots(roots).as_bytes(), |bytes| {
        std::str::from_utf8(bytes).is_ok()
    })
}

/// The cache is rebuilt by the project scan, so an unreadable one is only preserved, without
/// a notice.
pub fn load_project_cache() -> Vec<Project> {
    project_cache_file_path()
        .and_then(|path| storage::read(&path, parse_project_cache).split().0)
        .map(|projects| {
            projects
                .into_iter()
                .filter(|project| project.path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_project_cache(bytes: &[u8]) -> Result<Vec<Project>, String> {
    serde_json::from_slice(bytes).map_err(|error| json_error(&error))
}

pub fn save_project_cache(projects: &[Project]) -> io::Result<()> {
    let path = project_cache_file_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "diretório de configurações do usuário não encontrado",
        )
    })?;
    let contents = serde_json::to_string(projects).map_err(io::Error::other)?;
    storage::write(&path, contents.as_bytes(), |bytes| {
        parse_project_cache(bytes).is_ok()
    })
}

pub fn load_workspace_state() -> (WorkspaceState, Option<storage::Recovery>) {
    match workspace_state_file_path() {
        Some(path) => load_workspace_state_from(&path),
        None => (WorkspaceState::default(), None),
    }
}

fn load_workspace_state_from(path: &Path) -> (WorkspaceState, Option<storage::Recovery>) {
    let (state, recovery) = storage::read(path, parse_workspace_state).split();
    let state = state.unwrap_or_default();
    if state.schema_version > WORKSPACE_SCHEMA_VERSION {
        // Without a checked copy, the file of the newer version is never written over.
        storage::keep_copy_or_protect(path, &format!(".versao-{}", state.schema_version));
    }
    (migrate_workspace_state(state), recovery)
}

fn parse_workspace_state(bytes: &[u8]) -> Result<WorkspaceState, String> {
    serde_json::from_slice(bytes).map_err(|error| json_error(&error))
}

/// Brings a state saved by any earlier version to the current schema. A state saved by a
/// newer version keeps its known fields; the whole file was copied to `<file>.versao-<n>`.
/// Versions 1 to 3 only added fields, which `#[serde(default)]` fills; a future change of
/// meaning gets its own step here.
fn migrate_workspace_state(mut state: WorkspaceState) -> WorkspaceState {
    state.schema_version = WORKSPACE_SCHEMA_VERSION;
    state
}

/// The parser error without the content: serde messages may quote values from the file.
fn json_error(error: &serde_json::Error) -> String {
    let kind = match error.classify() {
        serde_json::error::Category::Io => "erro de leitura",
        serde_json::error::Category::Syntax => "JSON inválido",
        serde_json::error::Category::Data => "formato inesperado",
        serde_json::error::Category::Eof => "arquivo incompleto",
    };
    format!("{kind} (linha {}, coluna {})", error.line(), error.column())
}

pub fn save_workspace_state(state: &WorkspaceState) -> io::Result<()> {
    let path = workspace_state_file_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "diretório de configuração do usuário não encontrado",
        )
    })?;
    save_workspace_state_to(&path, state)
}

fn save_workspace_state_to(path: &Path, state: &WorkspaceState) -> io::Result<()> {
    let contents = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    storage::write(path, contents.as_bytes(), |bytes| {
        parse_workspace_state(bytes).is_ok()
    })
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

pub(crate) fn config_directory() -> Option<PathBuf> {
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
    use std::{fs, time::SystemTime};

    const DAY: u64 = 24 * 60 * 60;

    fn service(id: &str, updated_at: u64, logs: &[&str]) -> ServiceHistory {
        ServiceHistory {
            id: id.into(),
            name: "dev".into(),
            command: "npm run dev".into(),
            status: "parado".into(),
            updated_at,
            logs: logs.iter().map(|line| line.to_string()).collect(),
            ..ServiceHistory::default()
        }
    }

    fn preparation(updated_at: u64, logs: &[&str]) -> PreparationHistory {
        PreparationHistory {
            project_path: PathBuf::from("exemplo"),
            status: "concluído".into(),
            updated_at,
            steps: vec!["npm install".into()],
            logs: logs.iter().map(|line| line.to_string()).collect(),
        }
    }

    #[test]
    fn logs_older_than_thirty_days_expire_and_metadata_stays() {
        let now = 100 * DAY;
        let mut state = WorkspaceState::default();
        let project = Path::new("exemplo");
        state.record_service(project, service("velho", now - 31 * DAY, &["linha antiga"]));
        state.record_service(project, service("novo", now - 29 * DAY, &["linha recente"]));
        state.record_preparation(preparation(now - 31 * DAY, &["instalação antiga"]));
        state.record_preparation(preparation(now - 29 * DAY, &["instalação recente"]));

        assert!(state.expire_old_logs(now));
        let old = state.service("velho").unwrap();
        assert!(old.logs.is_empty());
        assert_eq!(old.command, "npm run dev");
        assert_eq!(state.service("novo").unwrap().logs, ["linha recente"]);
        let logs: Vec<_> = state
            .preparations
            .iter()
            .map(|entry| entry.logs.clone())
            .collect();
        assert!(logs.contains(&Vec::new()));
        assert!(logs.contains(&vec!["instalação recente".to_string()]));
        assert!(
            state
                .preparations
                .iter()
                .all(|entry| entry.steps == ["npm install"])
        );
        assert!(!state.expire_old_logs(now), "nothing left to expire");
    }

    #[test]
    fn clearing_the_history_removes_every_saved_log() {
        let mut state = WorkspaceState::default();
        state.record_service(Path::new("exemplo"), service("a", 1, &["x"]));
        state.record_preparation(preparation(1, &["y"]));
        state.clear_saved_logs();
        assert!(state.service("a").unwrap().logs.is_empty());
        assert!(state.preparations[0].logs.is_empty());
        assert_eq!(state.preparations[0].steps, ["npm install"]);
    }

    #[test]
    fn logs_saved_by_older_versions_are_masked_when_loaded() {
        let mut state = WorkspaceState::default();
        state.projects.push(ProjectHistory {
            path: PathBuf::from("exemplo"),
            services: vec![service("a", 1, &["API_TOKEN=valor-ficticio"])],
            ..ProjectHistory::default()
        });
        assert!(state.mask_saved_logs());
        assert_eq!(state.service("a").unwrap().logs, ["API_TOKEN=••••"]);
        assert!(!state.mask_saved_logs());
    }

    /// R6: after a start and stop of a project that prints secrets, the
    /// saved file holds none of them.
    #[test]
    fn saved_workspace_never_contains_secret_values_from_logs() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("localcodepilot-t011-{nonce}"));
        let path = root.join(WORKSPACE_STATE_FILE);
        let printed = [
            "Iniciando com DB_PASSWORD=senha-ficticia-1",
            "Authorization: Bearer token-ficticio-2",
            "conectado a postgres://app:senha-ficticia-3@localhost/app",
            r#"{"apiKey": "chave-ficticia-4"}"#,
        ];
        let mut state = WorkspaceState::default();
        let project = Path::new("exemplo");
        // Iniciar: the service is recorded while running...
        let mut running = service("exemplo:dev", 10, &printed);
        running.status = "rodando".into();
        state.record_service(project, running);
        save_workspace_state_to(&path, &state).unwrap();
        // ...and Parar records it again with the same output.
        state.record_service(project, service("exemplo:dev", 20, &printed));
        state.record_preparation(preparation(20, &printed));
        save_workspace_state_to(&path, &state).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        for secret in [
            "senha-ficticia-1",
            "token-ficticio-2",
            "senha-ficticia-3",
            "chave-ficticia-4",
        ] {
            assert!(!saved.contains(secret), "{secret} was saved");
        }
        assert!(saved.contains("DB_PASSWORD=••••"));
        let (loaded, _) = load_workspace_state_from(&path);
        assert_eq!(
            loaded.service("exemplo:dev").unwrap().logs.len(),
            printed.len()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_older_history_confirms_no_command() {
        // A state saved before LCP-SEC-001 knows the command line of a service, not the
        // identity the person confirmed: the first start after the update still asks.
        let old = r#"{
            "schema_version": 3,
            "projects": [{"path": "C:/projetos/loja", "services": [
                {"id": "loja-dev", "name": "dev", "command": "npm run dev", "status": "Rodando"}
            ]}]
        }"#;
        let state: WorkspaceState = serde_json::from_str(old).unwrap();
        assert_eq!(state.projects[0].services[0].command, "npm run dev");
        assert!(state.confirmed_commands.is_empty());
    }

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

    #[test]
    fn the_sidebar_starts_expanded_and_keeps_the_user_choice() {
        let saved_before_the_option: WorkspaceState =
            serde_json::from_str(r#"{"schema_version":3}"#).unwrap();
        assert!(!saved_before_the_option.sidebar_collapsed);
        assert!(!WorkspaceState::default().sidebar_collapsed);

        let collapsed = WorkspaceState {
            sidebar_collapsed: true,
            ..WorkspaceState::default()
        };
        let restored: WorkspaceState =
            serde_json::from_str(&serde_json::to_string(&collapsed).unwrap()).unwrap();
        assert!(restored.sidebar_collapsed);
    }

    /// A folder of its own under the system temp, removed when the test ends.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!("localcodepilot-t021-{name}-{nonce}"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const FIXTURES: [(&str, &str); 3] = [
        (
            "alpha8",
            include_str!("../../tests/fixtures/config/alpha8/workspace-state.json"),
        ),
        (
            "alpha9",
            include_str!("../../tests/fixtures/config/alpha9/workspace-state.json"),
        ),
        (
            "alpha10",
            include_str!("../../tests/fixtures/config/alpha10/workspace-state.json"),
        ),
    ];

    /// Whether `loaded` has every value of `saved`. Fields added later may appear as well.
    fn keeps(loaded: &serde_json::Value, saved: &serde_json::Value) -> bool {
        match (loaded, saved) {
            (serde_json::Value::Object(loaded), serde_json::Value::Object(saved)) => saved
                .iter()
                .all(|(key, value)| loaded.get(key).is_some_and(|found| keeps(found, value))),
            (serde_json::Value::Array(loaded), serde_json::Value::Array(saved)) => {
                loaded.len() == saved.len()
                    && loaded
                        .iter()
                        .zip(saved)
                        .all(|(found, value)| keeps(found, value))
            }
            _ => loaded == saved,
        }
    }

    #[test]
    fn states_saved_by_alpha_8_to_10_load_and_save_without_loss() {
        for (version, contents) in FIXTURES {
            let folder = TempFolder::new(version);
            let path = folder.0.join(WORKSPACE_STATE_FILE);
            fs::write(&path, contents).unwrap();
            let (first, recovery) = load_workspace_state_from(&path);
            assert!(recovery.is_none(), "{version}");

            // Every field the version saved is read back with the same value.
            let saved: serde_json::Value = serde_json::from_str(contents).unwrap();
            let loaded = serde_json::to_value(&first).unwrap();
            for (field, value) in saved.as_object().unwrap() {
                assert!(
                    keeps(&loaded[field], value),
                    "{version}: {field}: {} != {value}",
                    loaded[field]
                );
            }

            // The second opening, after a save, reads the same state.
            save_workspace_state_to(&path, &first).unwrap();
            let (second, recovery) = load_workspace_state_from(&path);
            assert!(recovery.is_none(), "{version}");
            assert_eq!(second, first, "{version}");
        }
    }

    #[test]
    fn a_state_from_a_newer_version_is_copied_before_anything_is_written() {
        let folder = TempFolder::new("futuro");
        let path = folder.0.join(WORKSPACE_STATE_FILE);
        let newer = r#"{"schema_version": 9, "favorite_projects": ["a"], "campo_novo": [1, 2]}"#;
        fs::write(&path, newer).unwrap();
        let (state, recovery) = load_workspace_state_from(&path);
        assert!(recovery.is_none());
        assert_eq!(state.favorite_projects, [PathBuf::from("a")]);
        assert_eq!(state.schema_version, WORKSPACE_SCHEMA_VERSION);
        save_workspace_state_to(&path, &state).unwrap();
        let copy = folder.0.join(format!("{WORKSPACE_STATE_FILE}.versao-9"));
        assert_eq!(fs::read_to_string(copy).unwrap(), newer);
    }

    #[test]
    fn an_unreadable_state_is_restored_from_the_last_copy_without_quoting_it() {
        let folder = TempFolder::new("ilegivel");
        let path = folder.0.join(WORKSPACE_STATE_FILE);
        let mut state = WorkspaceState {
            favorite_projects: vec![PathBuf::from("primeiro")],
            ..WorkspaceState::default()
        };
        save_workspace_state_to(&path, &state).unwrap();
        state.favorite_projects.push(PathBuf::from("segundo"));
        save_workspace_state_to(&path, &state).unwrap();
        // A value of the wrong type: serde would quote it in its message.
        fs::write(&path, r#"{"favorite_projects": "senha-secreta"}"#).unwrap();

        let (restored, recovery) = load_workspace_state_from(&path);
        let recovery = recovery.expect("the file could not be read");
        assert_eq!(restored.favorite_projects, [PathBuf::from("primeiro")]);
        assert!(
            !recovery.cause.contains("senha-secreta"),
            "{}",
            recovery.cause
        );
        let notice = ConfigRecovery::from_recoveries(&[recovery], 1).unwrap();
        assert!(notice.restored);
        assert!(!format!("{notice:?}").contains("senha-secreta"));
    }

    #[test]
    fn unreadable_scan_roots_are_preserved_and_reported() {
        let folder = TempFolder::new("pastas");
        let path = folder.0.join(SCAN_ROOTS_FILE);
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let (roots, recovery) = load_scan_roots_from(&path);
        assert!(roots.is_none());
        let recovery = recovery.expect("the file could not be read");
        assert!(recovery.restored_from.is_none());
        assert_eq!(
            fs::read(&recovery.preserved[0]).unwrap(),
            [0xff, 0xfe, 0x00]
        );
        let notice = ConfigRecovery::from_recoveries(&[recovery], 1).unwrap();
        assert!(!notice.restored);
    }
}
