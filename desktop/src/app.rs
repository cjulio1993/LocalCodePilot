#[path = "build_steps.rs"]
mod build_steps;
#[path = "command_trust.rs"]
mod command_trust;
#[path = "command_ui.rs"]
mod command_ui;
#[path = "components.rs"]
mod components;
#[path = "container_ui.rs"]
mod container_ui;
#[path = "database_ui.rs"]
mod database_ui;
#[path = "databases_tab_ui.rs"]
mod databases_tab_ui;
use database_ui::{DatabaseStatus, managed_info};
#[path = "environment_ui.rs"]
mod environment_ui;
#[path = "help_ui.rs"]
mod help_ui;
#[path = "language_ui.rs"]
mod language_ui;
#[path = "migration_ui.rs"]
mod migration_ui;
#[path = "notice.rs"]
mod notice;
#[path = "process_state.rs"]
mod process_state;
#[path = "project_ui.rs"]
mod project_ui;
use process_state::{
    HttpHealthCheckResult, HttpHealthStatus, ManagedProcesses, PortConflict,
    PortTerminationRequest, ProcessFailure, RunningProcess,
};
#[path = "recovery_ui.rs"]
mod recovery_ui;
#[path = "whats_new_ui.rs"]
mod whats_new_ui;
use semver::Version;

use crate::{
    config, theme, update_progress_ui,
    updates::session::{ActiveRun, UpdateConfirmation, UpdateSession},
    updates::{self, UpdateFailure, UpdateInfo},
};
use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, RichText, Stroke};
use egui_phosphor::regular::{
    ARROW_SQUARE_OUT, BOOK_OPEN, CARET_DOUBLE_LEFT, CARET_DOUBLE_RIGHT, CIRCLE, CODE_SIMPLE,
    FOLDER_OPEN, GEAR, PLAY, PLUS, STAR, TERMINAL,
};
use localcodepilot_core::{
    custom_commands::{self, CustomCommand},
    discovery::DiscoveryService,
    environments::{
        InstallationPlan, InstallationStep, InstallationStepKind, RequirementStatus,
        RuntimeRequirement,
    },
    paths::{display_path, distinguishing_folders, plain_path},
    ports::{LocalServerUrl, Port, extract_local_urls, port_in_use, strip_terminal_sequences},
    processes::{ProcessState, ProjectProcess},
    projects::Project,
    runtimes::RuntimeKind,
    technologies::TechnologyKind,
};
use localcodepilot_platform::{
    filesystem::{FilesystemProjectSource, legacy_installation_root, without_legacy_root},
    installations::{
        ProgramInventory, WindowsWingetProvider, configure_background_command, find_executable,
        inspect_program, managed_bin_directory, refreshed_environment_path,
    },
    ports::{
        PortAvailability, PortOwner, answers_http, inspect_tcp_port, listener_process_ids,
        listening_ports, process_descends_from, process_name, process_started_at,
        terminate_process_tree,
    },
};
use localcodepilot_runtime::database_connection::{
    ConnectionOutcome, ConnectionTest, read_credentials, test_connection,
};
use localcodepilot_runtime::databases::{
    DatabaseEngine, DatabaseRequirement, detect_database_requirements, prepare_sqlite_database,
    probe_local_tcp, probe_sqlite_database,
};

/// The login test of a project's database runs during the project check: short enough not
/// to hold the check, since a closed port is answered at once and only a server that
/// answers is logged into.
const DATABASE_LOGIN_TIMEOUT: Duration = Duration::from_secs(5);
/// Below this window width, in points (so zoom counts), the sidebar shows icons only.
const SIDEBAR_FOLD_WIDTH: f32 = 900.0;
use localcodepilot_runtime::{
    ManifestRuntimeDetector, detect_clojure_java_requirement, detect_clojure_requirement,
    detect_dart_requirement, detect_go_requirement, detect_haskell_requirement,
    detect_java_requirement, detect_node_requirement, detect_php_extensions,
    detect_php_requirement, detect_processes, detect_python_requirement, detect_zig_requirement,
    node_package_manager_spec, node_version_matches, runtime_version_matches,
};
use sha2::{Digest, Sha384};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::{OsStr, OsString},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant, SystemTime},
};

const NOTIFICATION_DURATION: Duration = Duration::from_millis(4_200);
const NOTIFICATION_FADE: Duration = Duration::from_millis(1_000);
const UPDATE_RESULT_NOTIFICATION_DURATION: Duration = Duration::from_secs(12);
const NOTIFICATION_MAX_WIDTH: f32 = 420.0;
const PROCESS_START_TIMEOUT: Duration = Duration::from_secs(30);
const PROCESS_START_CHECK_INTERVAL: Duration = Duration::from_millis(500);
const PROCESS_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const PROCESS_PORT_LOSS_GRACE: Duration = Duration::from_secs(6);
/// How often a running process without a known port is asked which ports it listens on.
const LISTENER_PROBE_INTERVAL: Duration = Duration::from_secs(3);
const HTTP_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(10);
const HTTP_HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(3);
const INSTALLATION_STEP_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Projects,
    /// Ambiente: what this computer has for the projects (T037.05, T038.03).
    Environment,
    Help,
    Settings,
    /// The page of one project (T038.01): `open_project` says which.
    Project,
}

impl Page {
    fn key(self) -> &'static str {
        match self {
            Self::Projects => "projects",
            Self::Environment => "environment",
            Self::Help => "help",
            Self::Settings => "settings",
            // The project may be gone on the next start: Projetos opens instead.
            Self::Project => "projects",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "projects" => Self::Projects,
            "environment" => Self::Environment,
            "help" => Self::Help,
            "settings" => Self::Settings,
            // Projetos is the start page (T038.02). The old pages "overview", "processes",
            // "containers" and "databases" (T038.04) and unknown keys open it.
            _ => Self::Projects,
        }
    }
}

/// A folder of installed programs that an older version saved with the scanned folders,
/// removed when the app opened (#98). Shown on Projects until the person closes the notice.
struct ScanRootMigration {
    removed: PathBuf,
    /// Whether the cleaned list was saved; if not, the removal happens again next time.
    saved: bool,
}

/// Removes the folder of installed programs an older version saved by mistake (#98).
fn migrate_scan_roots(
    saved: Option<Vec<PathBuf>>,
    default_roots: &[PathBuf],
) -> (Option<Vec<PathBuf>>, Option<ScanRootMigration>) {
    let Some(roots) = saved else {
        return (None, None);
    };
    let Some((remaining, removed)) =
        legacy_installation_root().and_then(|legacy| without_legacy_root(&roots, &legacy))
    else {
        return (Some(roots), None);
    };
    // Nothing of the person's own is left: start again from the common folders.
    let remaining = if remaining.is_empty() {
        default_roots.to_vec()
    } else {
        remaining
    };
    let saved = config::save_scan_roots(&remaining).is_ok();
    (Some(remaining), Some(ScanRootMigration { removed, saved }))
}

struct Notification {
    message: String,
    created_at: Instant,
    duration: Duration,
    /// While visible, other notifications do not replace it.
    priority: bool,
}

impl Notification {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            created_at: Instant::now(),
            duration: NOTIFICATION_DURATION,
            priority: false,
        }
    }

    /// The outcome of an update, shown when the app opens. It stays longer and
    /// is not replaced by the "projects updated" notice that follows the scan.
    fn update_result(message: impl Into<String>) -> Self {
        Self {
            duration: UPDATE_RESULT_NOTIFICATION_DURATION,
            priority: true,
            ..Self::new(message)
        }
    }

    fn blocks_replacement(&self) -> bool {
        self.priority && self.created_at.elapsed() < self.duration
    }
}

enum ProcessAction {
    /// "Detalhes" of a process row on the project page (T037.02).
    ToggleDetails(String),
    Start(String),
    Stop(String),
    Restart(String),
    ClearLogs(String),
    ViewLogs(String),
    OpenUrl(String),
    ResolvePortConflict(String),
}

#[derive(Debug, Clone, Default)]
struct ProjectEnvironment {
    command_count: usize,
    missing_programs: Vec<String>,
    /// The project compiles C/C++ (CMake, Meson, Visual Studio, `make run`) and this machine
    /// has no compiler (T033.03).
    missing_compiler: bool,
    missing_dependencies: Vec<String>,
    install_steps: Vec<DependencyInstall>,
    runtime_requirements: Vec<RuntimeRequirement>,
    missing_php_extensions: Vec<String>,
    laravel_constraint: Option<String>,
    databases: Vec<DatabaseStatus>,
    /// Manifests that exist but cannot be read, such as an invalid `package.json`.
    manifest_errors: Vec<String>,
    /// Several plausible entries (two `.pl` files, two Cabal executables): the card asks for
    /// the command and says why, and "Detalhes técnicos" lists them (T028.01).
    ambiguous_entry: Option<String>,
    /// Each command the app found: (name, effective command, working folder), for
    /// "Detalhes técnicos".
    detected_commands: Vec<(String, String, String)>,
}

#[derive(Debug, Clone, Default)]
struct RuntimeInventory {
    node: Option<ProgramInventory>,
    python: Option<ProgramInventory>,
    php: Option<ProgramInventory>,
    java: Option<ProgramInventory>,
    go: Option<ProgramInventory>,
    dart: Option<ProgramInventory>,
    babashka: Option<ProgramInventory>,
    stack: Option<ProgramInventory>,
    zig: Option<ProgramInventory>,
    php_extensions: HashSet<String>,
}

#[derive(Debug, Clone)]
struct DependencyInstall {
    label: String,
    program: String,
    args: Vec<String>,
    working_directory: PathBuf,
}

enum DependencyInstallEvent {
    Log(String),
    Finished(Result<InstallationOutcome, String>),
}

struct InstallationOutcome {
    environment_path: Option<OsString>,
}

#[derive(Debug, Clone)]
enum DependencyInstallStatus {
    Running,
    Cancelling,
    Succeeded,
    Failed { error: String, suggestion: String },
}

struct DependencyInstallSession {
    receiver: Option<Receiver<DependencyInstallEvent>>,
    logs: VecDeque<String>,
    status: DependencyInstallStatus,
    plan: InstallationPlan,
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaravelMigrationPhase {
    Confirm,
    Unavailable,
    Previewing,
    Ready,
    Applying,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
struct LaravelMigrationFiles {
    composer_json: String,
    composer_lock: Option<Vec<u8>>,
}

enum LaravelMigrationEvent {
    Log(String),
    PreviewFinished(Result<LaravelMigrationFiles, String>),
    ApplyFinished(Result<PathBuf, String>),
}

struct LaravelMigration {
    project_path: PathBuf,
    project_name: String,
    current_constraint: String,
    target_constraint: String,
    php_version: String,
    phase: LaravelMigrationPhase,
    receiver: Option<Receiver<LaravelMigrationEvent>>,
    logs: VecDeque<String>,
    preview: Option<LaravelMigrationFiles>,
    backup_path: Option<PathBuf>,
    error: Option<String>,
}

impl DependencyInstallSession {
    fn is_running(&self) -> bool {
        matches!(
            self.status,
            DependencyInstallStatus::Running | DependencyInstallStatus::Cancelling
        )
    }
}

impl ProjectEnvironment {
    fn is_ready(&self) -> bool {
        self.command_count > 0
            && !self.missing_compiler
            && self.missing_programs.is_empty()
            && self.missing_dependencies.is_empty()
            && self.runtime_requirements.iter().all(|requirement| {
                matches!(requirement.status, RequirementStatus::Available { .. })
            })
            && self.missing_php_extensions.is_empty()
    }

    fn problem_details(&self) -> Option<String> {
        if let Some(requirement) = self.runtime_requirements.iter().find(|requirement| {
            matches!(requirement.status, RequirementStatus::Incompatible { .. })
        }) && let RequirementStatus::Incompatible {
            installed_version,
            required_version,
            ..
        } = &requirement.status
        {
            Some(format!(
                "{} {installed_version} não atende ao projeto ({required_version})",
                requirement.runtime
            ))
        } else if self.missing_compiler {
            Some(missing_compiler_sentence())
        } else if let Some(requirement) = self
            .runtime_requirements
            .iter()
            .find(|requirement| matches!(requirement.status, RequirementStatus::Missing))
        {
            // "latest" and "stable" are what the app recommends, not a version to show.
            Some(match requirement.version.value.as_str() {
                "latest" | "stable" => format!("Instale {}", requirement.runtime),
                version => format!("Instale {} {version}", requirement.runtime),
            })
        } else if !self.missing_programs.is_empty() {
            Some(format!("Instale: {}", self.missing_programs.join(", ")))
        } else if !self.missing_php_extensions.is_empty() {
            Some(format!(
                "Extensões PHP ausentes: {}",
                self.missing_php_extensions.join(", ")
            ))
        } else if !self.missing_dependencies.is_empty() {
            Some(format!(
                "Instale as dependências de: {}",
                self.missing_dependencies.join(", ")
            ))
        } else if let Some(rejection) = self.databases.iter().find_map(DatabaseStatus::rejection) {
            Some(rejection)
        } else if let Some(reason) = self
            .databases
            .iter()
            .find_map(DatabaseStatus::not_preparable_here)
        {
            Some(reason)
        } else if self.databases.iter().any(DatabaseStatus::not_running) {
            Some("O banco de dados deste projeto não está rodando.".into())
        } else if self.databases.iter().any(|database| {
            database.requirement.engine == DatabaseEngine::Sqlite
                && database.available == Some(false)
        }) {
            Some("Arquivo SQLite será preparado ao iniciar o ambiente".into())
        } else {
            None
        }
    }

    fn can_install_dependencies(&self) -> bool {
        self.missing_programs.is_empty()
            && !self.install_steps.is_empty()
            && self.runtime_requirements.iter().all(|requirement| {
                matches!(requirement.status, RequirementStatus::Available { .. })
            })
    }

    fn can_prepare_environment(&self) -> bool {
        if cfg!(target_os = "windows") {
            self.missing_compiler || self.needs_environment_preparation()
        } else {
            // macOS: only the Apple command line tools, through the Apple installer (T033.03).
            cfg!(target_os = "macos")
                && self.missing_compiler
                && !self.needs_environment_preparation()
        }
    }

    fn needs_environment_preparation(&self) -> bool {
        self.runtime_requirements.iter().any(|requirement| {
            matches!(
                requirement.status,
                RequirementStatus::Missing | RequirementStatus::Incompatible { .. }
            )
        }) || self.missing_programs.iter().any(|program| {
            matches!(program.as_str(), "composer" | "pnpm" | "yarn" | "bun")
                || is_build_tool(program)
        }) || !self.missing_php_extensions.is_empty()
    }

    fn legacy_laravel_constraint(&self) -> Option<&str> {
        self.laravel_constraint
            .as_deref()
            .filter(|constraint| laravel_major(constraint).is_some_and(|major| major <= 11))
    }
}

enum ProjectCardAction {
    ViewDetails(PathBuf),
    ViewContainers(PathBuf),
    StopContainers(PathBuf),
    Open(PathBuf),
    OpenUrl(String),
    Start(PathBuf),
    Stop(PathBuf),
    PrepareEnvironment(PathBuf),
    InstallDependencies(PathBuf),
    ViewDependencyLogs(PathBuf),
    /// The output of the last run of the project's migrations (T027).
    ViewMigrationLogs(PathBuf),
    DefineCommand(PathBuf),
    EditCommand(PathBuf, String),
    RemoveCommand(PathBuf, String),
    PrepareLaravelMigration(PathBuf),
    ToggleFavorite(PathBuf),
    ToggleRecentVisibility(PathBuf),
    /// "Exportar diagnóstico", in "Detalhes técnicos" (T034).
    ExportDiagnostic(PathBuf),
    /// "Iniciar" of a Compose project, with the environment's checks (T038.02).
    StartContainers(PathBuf),
    /// The project's name, row or "Detalhes": opens its page (T038.01).
    OpenProject(PathBuf),
    /// A guided fix of the project's database, confirmed first.
    DatabaseFix(PathBuf, database_ui::DatabaseFix),
    /// "Parar o banco de dados", in "Detalhes técnicos".
    StopDatabase(PathBuf),
    /// "Testar conexão" of a remote database, in "Detalhes técnicos".
    TestRemoteDatabase(PathBuf),
}

enum DiscoveryEvent {
    Projects(Vec<Project>),
    Processes(Vec<ProjectProcess>),
    Environments(HashMap<PathBuf, ProjectEnvironment>),
    Failed(String),
}

pub struct LocalCodePilot {
    page: Page,
    /// Process rows of the project page with their details open (T037.02).
    expanded_processes: HashSet<String>,
    /// The project whose page is open (T038.01).
    open_project: Option<PathBuf>,
    /// The window is too narrow for the full sidebar (zoom included): it shows icons only.
    sidebar_narrow: bool,
    /// "Parar" of the databases of Ambiente, waiting for the confirmation (T037.06).
    database_stop_request: Option<databases_tab_ui::DatabaseStopRequest>,
    projects: Vec<Project>,
    /// The managed processes and their ports (T025).
    managed: ManagedProcesses,
    /// Pending migrations of Laravel projects, checked and run on "Iniciar" (T027).
    migrations: migration_ui::MigrationState,
    migration_targets: HashMap<PathBuf, migration_ui::MigrationTarget>,
    /// Lets background work ask for a new frame.
    repaint: egui::Context,
    project_environments: HashMap<PathBuf, ProjectEnvironment>,
    focused_project: Option<PathBuf>,
    dependency_installs: HashMap<PathBuf, DependencyInstallSession>,
    dependency_terminal: Option<PathBuf>,
    environment_plan_request: Option<(PathBuf, Receiver<Result<InstallationPlan, String>>)>,
    environment_plan_confirmation: Option<InstallationPlan>,
    /// Project commands waiting for their first-run confirmation (LCP-SEC-001).
    command_confirmation: Option<command_trust::CommandConfirmation>,
    environment_path: Option<OsString>,
    laravel_migration: Option<LaravelMigration>,
    logo_texture: egui::TextureHandle,
    username: String,
    user_initials: String,
    search: String,
    notification: Option<Notification>,
    scan_roots: Vec<PathBuf>,
    scan_root_migration: Option<ScanRootMigration>,
    discovery: Option<Receiver<DiscoveryEvent>>,
    workspace_state: config::WorkspaceState,
    workspace_dirty: bool,
    /// The configuration recovery notice is shown at the top until closed (T021).
    config_recovery_notice: bool,
    /// The last search for projects failed, and why (T036).
    discovery_error: Option<String>,
    /// "LocalCodePilot atualizado para {versão}", while open (T035).
    whats_new: Option<whats_new_ui::WhatsNew>,
    /// "Ver novidades", focused again on the frame after the modal closes.
    whats_new_focus_return: Option<egui::Id>,
    last_workspace_save: Instant,
    workspace_save_error_reported: bool,
    /// The update: check, offer, download, confirmation and restart (T023).
    update_session: UpdateSession,
    container_state: container_ui::ContainerState,
    project_ui: project_ui::ProjectUiState,
    /// Preparing and starting the databases of projects (T019.04).
    database: database_ui::DatabaseState,
    help: help_ui::HelpState,
    languages: language_ui::LanguageState,
    command_draft: Option<command_ui::CommandDraft>,
    /// "Limpar histórico de logs?" is open.
    confirm_clear_history: bool,
}

impl LocalCodePilot {
    pub fn new(cc: &eframe::CreationContext<'_>, startup_update: updates::StartupUpdate) -> Self {
        theme::configure(&cc.egui_ctx);
        theme::install_fonts(&cc.egui_ctx);
        let default_source = FilesystemProjectSource::common_locations();
        let (loaded_roots, roots_recovery) = config::load_scan_roots();
        let (saved_roots, scan_root_migration) =
            migrate_scan_roots(loaded_roots, default_source.roots());
        let configured_roots = saved_roots.unwrap_or_else(|| default_source.roots().to_vec());
        let source = FilesystemProjectSource::new(configured_roots);
        let scan_roots = source.roots().to_vec();
        let cached_projects = config::load_project_cache();
        let environment_path = refreshed_environment_path().ok();
        // A clean install has no saved state yet: the news of an update never show there.
        let state_existed = config::workspace_state_exists();
        let (mut workspace_state, state_recovery) = config::load_workspace_state();
        let app_version = Version::parse(whats_new_ui::app_version())
            .expect("the package version follows semantic versioning");
        let show_news = whats_new_ui::should_show(
            &app_version,
            workspace_state.last_seen_version.as_deref(),
            state_existed,
            startup_update.updated_by_app,
        );
        // Without the news, this version counts as seen; with them, closing records it.
        let mut news_seen_changed = false;
        if !show_news {
            let seen = whats_new_ui::seen_after(
                &app_version,
                workspace_state.last_seen_version.as_deref(),
            );
            news_seen_changed = workspace_state.last_seen_version.as_deref() != Some(&seen);
            workspace_state.last_seen_version = Some(seen);
        }
        let recoveries = state_recovery
            .into_iter()
            .chain(roots_recovery)
            .collect::<Vec<_>>();
        let config_recovery =
            config::ConfigRecovery::from_recoveries(&recoveries, unix_timestamp());
        let config_recovery_notice = config_recovery.is_some();
        if config_recovery.is_some() {
            workspace_state.config_recovery = config_recovery;
        }
        // Older versions saved logs unmasked and without a time limit.
        let saved_logs_changed =
            workspace_state.mask_saved_logs() | workspace_state.expire_old_logs(unix_timestamp());
        let receiver = spawn_discovery(
            source,
            environment_path.clone(),
            workspace_state.custom_commands.clone(),
            cc.egui_ctx.clone(),
        );
        let logo_texture = load_logo_texture(&cc.egui_ctx);
        let username = current_username();
        let user_initials = initials_from_username(&username);
        let recovered_preparation = workspace_state
            .preparations
            .iter_mut()
            .filter(|preparation| preparation.status == "em andamento")
            .map(|preparation| {
                preparation.status = "interrompido".into();
            })
            .count()
            > 0;
        let page = Page::from_key(&workspace_state.last_page);
        let focused_project = workspace_state.focused_project.clone();
        let now = unix_timestamp();
        // When the installer did not finish, the notice with "Tentar de novo" comes back
        // even if a reminder was postponed: the person had just asked to update.
        let installer_failed = startup_update.failure.is_some();
        let available_update = workspace_state
            .cached_update
            .clone()
            .filter(updates::cached_update_is_newer)
            .filter(|_| installer_failed || workspace_state.update_remind_after <= now);
        // After an update by the app, the news modal says it ("LocalCodePilot atualizado
        // para {versão}"): the short notice would repeat it.
        let updated_notice_replaced = show_news && startup_update.updated_by_app;
        let startup_notice = startup_update
            .notice
            .filter(|_| !updated_notice_replaced)
            .or_else(|| {
                startup_update
                    .failure
                    .as_ref()
                    .map(|failure| format!("{}. {}", failure.title(), failure.message()))
            });
        let update_check = (now.saturating_sub(workspace_state.last_update_check)
            >= updates::CHECK_INTERVAL_SECONDS)
            .then(|| updates::spawn_update_check(cc.egui_ctx.clone()));
        Self {
            page,
            projects: cached_projects,
            expanded_processes: HashSet::new(),
            open_project: None,
            sidebar_narrow: false,
            database_stop_request: None,
            managed: ManagedProcesses::default(),
            migrations: migration_ui::MigrationState::default(),
            migration_targets: HashMap::new(),
            repaint: cc.egui_ctx.clone(),
            project_environments: HashMap::new(),
            focused_project,
            dependency_installs: HashMap::new(),
            dependency_terminal: None,
            environment_plan_request: None,
            environment_plan_confirmation: None,
            command_confirmation: None,
            environment_path,
            laravel_migration: None,
            logo_texture,
            username,
            user_initials,
            search: String::new(),
            notification: Some(match startup_notice {
                Some(update_result) => Notification::update_result(update_result),
                None => Notification::new("Procurando projetos na máquina..."),
            }),
            scan_roots,
            scan_root_migration,
            discovery: Some(receiver),
            workspace_state,
            workspace_dirty: recovered_preparation
                || saved_logs_changed
                || config_recovery_notice
                || news_seen_changed,
            config_recovery_notice,
            discovery_error: None,
            whats_new: show_news.then(|| whats_new_ui::WhatsNew::current(None)),
            whats_new_focus_return: None,
            last_workspace_save: Instant::now(),
            workspace_save_error_reported: false,
            update_session: UpdateSession::new(
                update_check,
                available_update,
                startup_update.failure,
            ),
            container_state: container_ui::ContainerState::default(),
            project_ui: project_ui::ProjectUiState::default(),
            database: database_ui::DatabaseState::default(),
            help: help_ui::HelpState::default(),
            languages: language_ui::LanguageState::default(),
            command_draft: None,
            confirm_clear_history: false,
        }
    }

    fn notify(&mut self, message: impl Into<String>) {
        if self
            .notification
            .as_ref()
            .is_some_and(Notification::blocks_replacement)
        {
            return;
        }
        self.notification = Some(Notification::new(message));
    }

    /// A notice that must be read: it stays longer and the routine ones that follow (such as
    /// "projects updated" after the scan at start) do not replace it.
    fn notify_important(&mut self, message: impl Into<String>) {
        self.notification = Some(Notification::update_result(message));
    }

    fn is_favorite(&self, path: &Path) -> bool {
        self.workspace_state
            .favorite_projects
            .iter()
            .any(|favorite| favorite == path)
    }

    fn toggle_favorite(&mut self, path: &Path) {
        if let Some(index) = self
            .workspace_state
            .favorite_projects
            .iter()
            .position(|favorite| favorite == path)
        {
            self.workspace_state.favorite_projects.remove(index);
            self.notify("Projeto removido dos favoritos");
        } else {
            self.workspace_state
                .favorite_projects
                .push(path.to_path_buf());
            self.notify("Projeto adicionado aos favoritos");
        }
        self.workspace_dirty = true;
    }

    fn touch_project(&mut self, path: &Path) {
        self.workspace_state.touch_project(path, unix_timestamp());
        self.workspace_dirty = true;
    }

    fn toggle_recent_visibility(&mut self, path: &Path) {
        let hidden = &mut self.workspace_state.hidden_recent_projects;
        if let Some(index) = hidden.iter().position(|entry| entry == path) {
            hidden.remove(index);
            self.notify("Projeto visível nos recentes");
        } else {
            hidden.push(path.to_path_buf());
            self.notify("Projeto ocultado dos recentes");
        }
        self.workspace_dirty = true;
    }

    fn record_process_history(&mut self, id: &str) {
        let Some(process) = self
            .managed
            .processes
            .iter()
            .find(|process| process.id == id)
            .cloned()
        else {
            return;
        };
        let running = self.managed.running.get(id);
        let logs = running
            .map(|running| {
                let skip = running.logs.len().saturating_sub(200);
                running.logs.iter().skip(skip).cloned().collect()
            })
            .unwrap_or_else(|| {
                self.workspace_state
                    .service(id)
                    .map(|service| service.logs.clone())
                    .unwrap_or_default()
            });
        let existing = self.workspace_state.service(id).cloned();
        let port = self
            .managed
            .assigned_ports
            .get(id)
            .copied()
            .map(Port::value)
            .or_else(|| existing.as_ref().and_then(|service| service.port))
            .or_else(|| process.expected_port.map(Port::value));
        let url = running
            .and_then(|running| {
                port.and_then(|port| {
                    running
                        .urls
                        .iter()
                        .rev()
                        .find(|url| url.port().value() == port)
                })
                .or_else(|| running.urls.last())
            })
            .map(|url| url.address().to_owned())
            .or_else(|| existing.and_then(|service| service.url));
        let command = process.command_line();
        let status = process_state_key(process.state).to_owned();
        let project_path = process.project_path.clone();
        self.workspace_state.record_service(
            &project_path,
            config::ServiceHistory {
                id: process.id,
                name: process.name,
                command,
                status,
                port,
                url,
                updated_at: unix_timestamp(),
                logs,
            },
        );
        self.workspace_dirty = true;
    }

    fn persist_workspace_state(&mut self, force: bool) {
        let page = self.page.key().to_owned();
        if self.workspace_state.last_page != page {
            self.workspace_state.last_page = page;
            self.workspace_dirty = true;
        }
        if self.workspace_state.focused_project != self.focused_project {
            self.workspace_state.focused_project = self.focused_project.clone();
            self.workspace_dirty = true;
        }
        if !self.workspace_dirty
            || (!force && self.last_workspace_save.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        match config::save_workspace_state(&self.workspace_state) {
            Ok(()) => {
                self.workspace_dirty = false;
                self.workspace_save_error_reported = false;
                self.last_workspace_save = Instant::now();
            }
            Err(error) if !self.workspace_save_error_reported => {
                self.workspace_save_error_reported = true;
                self.notify(format!("Não foi possível salvar o contexto: {error}"));
            }
            Err(_) => {}
        }
    }

    fn poll_update_check(&mut self) {
        let now = unix_timestamp();
        if let Some(outcome) = self
            .update_session
            .poll_check(now, self.workspace_state.update_remind_after)
        {
            self.workspace_state.last_update_check = now;
            self.workspace_state.cached_update = outcome.cached_update;
            self.workspace_dirty = true;
        }
    }

    /// "Atualizar agora": with projects running, ask first (CA-5); otherwise download.
    fn request_update_install(&mut self, update: &UpdateInfo, ctx: egui::Context) {
        let active = self.managed.active_runs();
        self.update_session.request_install(update, &active, ctx);
    }

    /// A new check of the projects is starting: no earlier login result may still read as
    /// "Banco de dados conectado" while it runs.
    fn forget_database_tests(&mut self) {
        for environment in self.project_environments.values_mut() {
            for database in &mut environment.databases {
                database.connection = None;
            }
        }
    }

    fn launch_update(&mut self, prepared: &updates::PreparedUpdate, ctx: &egui::Context) {
        self.persist_workspace_state(true);
        match updates::launch_prepared_update(prepared) {
            Ok(()) => self
                .update_session
                .launched(updates::test_close_delay(), ctx),
            Err(error) => self.update_session.launch_failed(error),
        }
    }

    fn poll_update_install(&mut self, ctx: &egui::Context) {
        let active = self.managed.active_runs();
        if let Some(prepared) = self.update_session.poll_install(&active, ctx) {
            self.launch_update(&prepared, ctx);
        }
    }

    fn show_update_banner(&mut self, ui: &mut egui::Ui) {
        let Some(update) = self.update_session.available.clone() else {
            return;
        };
        let automatic_update = updates::automatic_update_available(&update);
        let refused = self.update_session.refused_for_safety();
        let installing = self.update_session.in_progress();
        let progress = self.update_session.progress();
        let mut remind_later = false;
        let mut start_install = false;
        Frame::new()
            .fill(theme::PRIMARY.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0, theme::PRIMARY.gamma_multiply(0.65)))
            .corner_radius(10)
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Nova versão do LocalCodePilot disponível")
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.label(
                            RichText::new(update_versions_line(&update.version))
                                .color(theme::MUTED)
                                .size(10.0),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(updates::summary_line(&update)).size(11.0),
                            )
                            .wrap(),
                        );
                        if !automatic_update {
                            ui.label(
                                RichText::new(updates::manual_download_note(&update))
                                    .color(theme::MUTED)
                                    .size(10.0),
                            );
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(!installing, egui::Button::new("Lembrar depois"))
                            .clicked()
                        {
                            remind_later = true;
                        }
                        if automatic_update {
                            // Depois de uma recusa por segurança, "Atualizar agora" seria só
                            // tentar de novo o mesmo arquivo: a ação passa a ser a página oficial.
                            if !refused
                                && ui
                                    .add_enabled(
                                        !installing,
                                        egui::Button::new("Atualizar agora")
                                            .fill(theme::PRIMARY.gamma_multiply(0.72)),
                                    )
                                    .clicked()
                            {
                                start_install = true;
                            }
                        } else if ui
                            .add(
                                egui::Button::new("Baixar atualização")
                                    .fill(theme::PRIMARY.gamma_multiply(0.72)),
                            )
                            .clicked()
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(
                                updates::manual_download_url(&update),
                            ));
                        }
                        if ui.link("Ver novidades").clicked() {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(&update.release_url));
                        }
                    });
                });
                if installing {
                    ui.add_space(8.0);
                    update_progress_ui::show(ui, &progress);
                }
                if let Some(error) = &self.update_session.install_error {
                    ui.add_space(8.0);
                    if update_failure_panel(ui, error, Some(&update.release_url), "banner") {
                        start_install = true;
                    }
                }
            });
        ui.add_space(14.0);
        if start_install {
            self.request_update_install(&update, ui.ctx().clone());
        }
        if remind_later {
            self.workspace_state.update_remind_after =
                unix_timestamp().saturating_add(updates::CHECK_INTERVAL_SECONDS);
            self.update_session.available = None;
            self.workspace_dirty = true;
            self.notify("O lembrete de atualização voltará amanhã");
        }
    }

    fn show_notification(&mut self, root_ui: &mut egui::Ui) {
        let Some(notification) = &self.notification else {
            return;
        };
        let elapsed = notification.created_at.elapsed();
        if elapsed >= notification.duration {
            self.notification = None;
            return;
        }

        let fade_start = notification.duration.saturating_sub(NOTIFICATION_FADE);
        let opacity = if elapsed < fade_start {
            1.0
        } else {
            let fade_elapsed = elapsed - fade_start;
            1.0 - fade_elapsed.as_secs_f32() / NOTIFICATION_FADE.as_secs_f32()
        };
        let message = notification.message.clone();
        let root_ui_width = root_ui.ctx().content_rect().width();
        root_ui
            .ctx()
            .request_repaint_after(Duration::from_millis(50));

        egui::Area::new(egui::Id::unique("notification"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-24.0, -24.0])
            .show(root_ui, |ui| {
                Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(
                        26,
                        29,
                        38,
                        (220.0 * opacity) as u8,
                    ))
                    .stroke(Stroke::new(1.0, theme::BORDER.gamma_multiply(opacity)))
                    .corner_radius(10)
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        // Long messages (an update result, for example) wrap inside the
                        // window instead of growing past its left edge.
                        let width = NOTIFICATION_MAX_WIDTH
                            .min(root_ui_width - 48.0 - 28.0)
                            .max(160.0);
                        ui.set_max_width(width);
                        ui.horizontal_top(|ui| {
                            ui.label(
                                RichText::new(CIRCLE)
                                    .color(theme::SUCCESS.gamma_multiply(opacity))
                                    .size(8.0),
                            );
                            let label = ui.add(
                                egui::Label::new(
                                    RichText::new(message)
                                        .color(theme::TEXT.gamma_multiply(opacity))
                                        .size(11.0),
                                )
                                .wrap(),
                            );
                            // Screen readers announce each new notification (T027 CA-11).
                            notice::mark_live(ui, label.id);
                        });
                    });
            });
    }

    fn sidebar(&mut self, root_ui: &mut egui::Ui) {
        self.sidebar_narrow = root_ui.available_width() < SIDEBAR_FOLD_WIDTH;
        let collapsed = self.workspace_state.sidebar_collapsed || self.sidebar_narrow;
        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(if collapsed { 68.0 } else { 248.0 })
            .frame(
                Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(Margin::same(14))
                    .stroke(Stroke::new(1.0_f32, theme::BORDER)),
            )
            .show(root_ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if !collapsed {
                        ui.add(
                            egui::Image::new(&self.logo_texture)
                                .fit_to_exact_size(egui::vec2(36.0, 36.0))
                                .alt_text("Logo do LocalCodePilot"),
                        );
                        ui.label(RichText::new("LocalCodePilot").strong().size(14.0));
                    }
                    // In a narrow window the sidebar stays folded: no toggle to offer.
                    let narrow = self.sidebar_narrow;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if narrow {
                            return;
                        }
                        let (icon, name) = if collapsed {
                            (CARET_DOUBLE_RIGHT, "Expandir menu")
                        } else {
                            (CARET_DOUBLE_LEFT, "Recolher menu")
                        };
                        let toggle = ui
                            .add(
                                egui::Button::new(
                                    RichText::new(icon).color(theme::MUTED).size(16.0),
                                )
                                .frame(false)
                                .min_size(egui::vec2(36.0, 36.0)),
                            )
                            .on_hover_text(name);
                        toggle.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, name)
                        });
                        if toggle.clicked() {
                            self.workspace_state.sidebar_collapsed = !collapsed;
                            self.workspace_dirty = true;
                        }
                    });
                });
                ui.add_space(18.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        nav_group(ui, collapsed, "Trabalho");
                        self.nav_button(
                            ui,
                            Page::Projects,
                            FOLDER_OPEN,
                            "Projetos",
                            Some(self.projects.len()),
                        );
                        self.nav_button(
                            ui,
                            Page::Environment,
                            egui_phosphor::regular::DESKTOP,
                            "Ambiente",
                            None,
                        );
                        ui.add_space(14.0);
                        nav_group(ui, collapsed, "Aplicativo");
                        self.nav_button(ui, Page::Help, BOOK_OPEN, "Ajuda", None);
                        self.nav_button(ui, Page::Settings, GEAR, "Configurações", None);
                    });
            });
    }

    /// One page in the sidebar: a real button, so it takes keyboard focus, activates with
    /// Enter and tells assistive technologies its name and whether it is the current page.
    fn nav_button(
        &mut self,
        ui: &mut egui::Ui,
        page: Page,
        icon: &str,
        label: &str,
        projects: Option<usize>,
    ) {
        let collapsed = self.workspace_state.sidebar_collapsed || self.sidebar_narrow;
        // A project's page belongs to Projetos.
        let selected = if page == Page::Projects {
            matches!(self.page, Page::Projects | Page::Project)
        } else {
            self.page == page
        };
        let color = if selected {
            Color32::WHITE
        } else {
            theme::MUTED
        };
        let name = nav_name(label, projects);
        let size = egui::vec2(ui.available_width(), 38.0);
        let mut button = if collapsed {
            egui::Button::selectable(selected, RichText::new(icon).color(color).size(18.0))
        } else {
            egui::Button::selectable(
                selected,
                RichText::new(format!("{icon}   {label}"))
                    .color(color)
                    .size(13.0),
            )
            .right_text(
                RichText::new(projects.map(|count| count.to_string()).unwrap_or_default())
                    .color(theme::MUTED)
                    .size(11.0),
            )
        }
        .min_size(size)
        .corner_radius(8);
        if selected {
            button = button.fill(theme::PRIMARY.gamma_multiply(0.13));
        }
        let mut response = ui.add(button);
        // The visible text starts with an icon glyph; give assistive technologies plain words.
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name));
        if collapsed {
            if let Some(count) = projects {
                ui.painter().text(
                    response.rect.right_top() + egui::vec2(-4.0, 3.0),
                    egui::Align2::RIGHT_TOP,
                    count.to_string(),
                    egui::FontId::proportional(10.0),
                    theme::MUTED,
                );
            }
            response = response.on_hover_text(&name);
        }
        if response.clicked() {
            self.page = page;
        }
    }

    fn topbar(&mut self, root_ui: &mut egui::Ui) {
        egui::Panel::top("topbar")
            .exact_size(52.0)
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(Margin::symmetric(26, 9))
                    .stroke(Stroke::new(1.0_f32, theme::BORDER)),
            )
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.page_title()).size(13.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        Frame::new()
                            .fill(Color32::from_rgb(38, 53, 82))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(56, 81, 123)))
                            .corner_radius(9)
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.label(RichText::new(&self.user_initials).strong().size(10.0));
                            })
                            .response
                            .on_hover_text(&self.username);
                    });
                });
            });
    }

    fn page_title(&self) -> &'static str {
        match self.page {
            Page::Projects | Page::Project => "Projetos",
            Page::Environment => "Ambiente",
            Page::Help => "Ajuda",
            Page::Settings => "Configurações",
        }
    }

    fn poll_discovery(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.discovery else {
            return;
        };
        match receiver.try_recv() {
            Ok(DiscoveryEvent::Projects(projects)) => {
                self.discovery_error = None;
                self.notify(format!(
                    "{} projeto(s) encontrado(s) · atualizando diagnósticos...",
                    projects.len()
                ));
                self.projects = projects;
                let _ = config::save_project_cache(&self.projects);
                self.refresh_containers(ctx.clone());
            }
            Ok(DiscoveryEvent::Processes(processes)) => {
                self.managed.processes = processes;
                self.managed.assigned_ports.clear();
                self.managed.port_conflicts.clear();
                self.managed.failures.clear();
            }
            Ok(DiscoveryEvent::Environments(environments)) => {
                self.project_environments = environments;
                self.discovery = None;
                self.notify("Projetos e ambientes atualizados");
            }
            Ok(DiscoveryEvent::Failed(error)) => {
                self.notify(format!("Não foi possível concluir a varredura: {error}"));
                // The Overview says it too, instead of "Nenhum projeto encontrado" (T036).
                self.discovery_error = Some(error);
                self.discovery = None;
            }
            Err(TryRecvError::Disconnected) => {
                self.notify("A varredura foi interrompida");
                self.discovery = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn refresh_projects(&mut self, ctx: &egui::Context) {
        if self.managed.has_active_processes() {
            self.notify("Pare os processos ativos antes de atualizar os projetos");
            return;
        }
        if self.discovery.is_some() {
            return;
        }
        self.notify("Atualizando a lista de projetos...");
        self.forget_database_tests();
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            self.environment_path.clone(),
            self.workspace_state.custom_commands.clone(),
            ctx.clone(),
        ));
    }

    fn add_scan_root(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.managed.has_active_processes() {
            self.notify("Pare os processos ativos antes de adicionar uma pasta");
            return;
        }
        let path = path.canonicalize().unwrap_or(path);
        if self.scan_roots.iter().any(|root| root == &path) {
            self.notify("Essa pasta já faz parte da varredura");
            return;
        }
        self.scan_roots.push(path);
        self.notify(match config::save_scan_roots(&self.scan_roots) {
            Ok(()) => "Pasta adicionada. Procurando projetos...".into(),
            Err(error) => format!("Pasta adicionada, mas não foi possível salvar: {error}"),
        });
        self.forget_database_tests();
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            self.environment_path.clone(),
            self.workspace_state.custom_commands.clone(),
            ctx.clone(),
        ));
    }

    fn remove_scan_root(&mut self, ctx: &egui::Context, path: &Path) {
        if self.managed.has_active_processes() {
            self.notify("Pare os processos ativos antes de remover uma pasta");
            return;
        }
        self.scan_roots.retain(|root| root != path);
        self.notify(match config::save_scan_roots(&self.scan_roots) {
            Ok(()) => "Pasta removida. Atualizando os projetos...".into(),
            Err(error) => format!("Pasta removida, mas não foi possível salvar: {error}"),
        });
        self.forget_database_tests();
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            self.environment_path.clone(),
            self.workspace_state.custom_commands.clone(),
            ctx.clone(),
        ));
    }

    fn reset_scan_roots(&mut self, ctx: &egui::Context) {
        if self.managed.has_active_processes() {
            self.notify("Pare os processos ativos antes de restaurar as pastas");
            return;
        }
        self.scan_roots = FilesystemProjectSource::common_locations().roots().to_vec();
        self.notify(match config::save_scan_roots(&self.scan_roots) {
            Ok(()) => "Pastas padrão restauradas. Atualizando os projetos...".into(),
            Err(error) => format!("Pastas restauradas, mas não foi possível salvar: {error}"),
        });
        self.forget_database_tests();
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            self.environment_path.clone(),
            self.workspace_state.custom_commands.clone(),
            ctx.clone(),
        ));
    }

    fn projects_page(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let has_active_processes = self.managed.has_active_processes();
        let scanning = self.discovery.is_some();
        let can_change_roots = !scanning && !has_active_processes;
        let mut add_folder = false;
        let mut refresh = false;
        let mut toggle_roots = false;
        let mut summary = |ui: &mut egui::Ui| {
            ui.label(RichText::new("Projetos").strong().size(22.0));
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("{} ·", project_count_text(self.projects.len())))
                    .color(theme::MUTED)
                    .size(13.0),
            );
            if ui.link(scan_roots_text(self.scan_roots.len())).clicked() {
                toggle_roots = true;
            }
            if scanning {
                ui.spinner();
                ui.label(RichText::new("Procurando...").color(theme::MUTED));
            }
        };
        let add_label = format!("{PLUS}  Adicionar pasta");
        let mut buttons = |ui: &mut egui::Ui| {
            add_folder = announced(
                ui.add_enabled(
                    can_change_roots,
                    egui::Button::new(&add_label).fill(theme::PRIMARY_FILL),
                ),
                egui::Role::Button,
                &add_label,
            )
            .clicked();
            refresh = ui
                .add_enabled(can_change_roots, egui::Button::new("Atualizar"))
                .clicked();
        };
        // One line when it fits; in a small window the buttons move to their own line. The
        // buttons are added left to right in both cases, so Tab follows the reading order.
        if ui.available_width() >= 640.0 {
            ui.horizontal(|ui| {
                summary(ui);
                let width = button_width(ui, &add_label)
                    + button_width(ui, "Atualizar")
                    + ui.spacing().item_spacing.x;
                ui.add_space((ui.available_width() - width).max(0.0));
                buttons(ui);
            });
        } else {
            ui.horizontal_wrapped(|ui| summary(ui));
            ui.horizontal(|ui| buttons(ui));
        }
        // The protection stays; the sentence names who holds it and leads there (CA-9).
        if has_active_processes {
            let running = self.running_projects();
            let names = running
                .iter()
                .map(|(_, name)| name.clone())
                .collect::<Vec<_>>();
            ui.label(
                RichText::new(folder_lock_sentence(&names))
                    .color(theme::WARNING)
                    .size(12.0),
            );
            let mut open = None;
            let mut open_running = false;
            ui.horizontal_wrapped(|ui| {
                for (path, name) in running.iter().take(3) {
                    let label = format!("Ver {name}");
                    if announced(ui.link(&label), egui::Role::Link, &label).clicked() {
                        open = Some(path.clone());
                    }
                }
                if running.len() > 3
                    && announced(
                        ui.link("Ver os projetos rodando"),
                        egui::Role::Link,
                        "Ver os projetos rodando",
                    )
                    .clicked()
                {
                    open_running = true;
                }
            });
            if let Some(path) = open {
                self.open_project_page(ctx, path);
            }
            if open_running {
                self.workspace_state.project_filter = "running".to_owned();
                self.workspace_dirty = true;
            }
        }
        if let Some(migration) = &self.scan_root_migration {
            ui.add_space(6.0);
            let removed = format!(
                "Pasta removida da busca: {}",
                display_path(&migration.removed)
            );
            let saved = migration.saved;
            let mut details = |ui: &mut egui::Ui| {
                notice::detail_line(ui, &removed);
                if !saved {
                    notice::detail_line(
                        ui,
                        "Não foi possível salvar a alteração. O LocalCodePilot tentará novamente na próxima abertura.",
                    );
                }
            };
            let response = notice::Notice::new(
                egui::Id::unique("scan-root-migration"),
                "Removemos uma pasta de programas da busca de projetos.",
            )
            .action("Ver pastas examinadas", true)
            .dismiss("Fechar")
            .show_response(
                ui,
                notice::NoticeParts {
                    resolve: None,
                    details: Some(&mut details),
                },
            );
            if response.action {
                self.project_ui.show_roots = true;
            }
            if response.dismiss {
                self.scan_root_migration = None;
            }
        }
        if toggle_roots {
            self.project_ui.show_roots = !self.project_ui.show_roots;
        }
        let mut root_to_remove = None;
        if self.project_ui.show_roots {
            ui.add_space(6.0);
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(8)
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("O LocalCodePilot procura projetos nestas pastas:")
                            .size(13.0),
                    );
                    ui.add_space(4.0);
                    for root in &self.scan_roots {
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add_enabled(
                                        can_change_roots,
                                        egui::Button::new("Remover").small(),
                                    )
                                    .clicked()
                                {
                                    root_to_remove = Some(root.clone());
                                }
                                // A long path is cut to the space left, never widening the page.
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(display_path(root))
                                                .color(theme::MUTED)
                                                .monospace()
                                                .size(12.0),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(display_path(root));
                                });
                            });
                        });
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if announced(
                            ui.add_enabled(can_change_roots, egui::Button::new(&add_label)),
                            egui::Role::Button,
                            &add_label,
                        )
                        .clicked()
                        {
                            add_folder = true;
                        }
                        if ui
                            .add_enabled(
                                can_change_roots,
                                egui::Button::new("Restaurar pastas padrão"),
                            )
                            .clicked()
                        {
                            self.reset_scan_roots(ctx);
                        }
                    });
                });
        }
        if let Some(root) = root_to_remove {
            self.remove_scan_root(ctx, &root);
        }
        if refresh {
            self.refresh_projects(ctx);
        }
        if add_folder
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Escolha uma pasta com projetos")
                .pick_folder()
        {
            self.add_scan_root(ctx, path);
        }
        ui.add_space(10.0);
        self.project_catalog(ui);
    }

    fn find_available_port_after(&self, preferred: Port) -> Option<Port> {
        (1..=20).find_map(|offset| {
            let value = preferred.value().checked_add(offset)?;
            let candidate = Port::new(value)?;
            self.managed
                .port_is_available(candidate)
                .then_some(candidate)
        })
    }

    /// The project commands among `ids` not yet confirmed with their current identity.
    /// Commands the person wrote are theirs and never ask.
    fn unconfirmed_commands(&self, ids: &[String]) -> Vec<command_trust::PendingCommand> {
        ids.iter()
            .filter_map(|id| {
                self.managed
                    .processes
                    .iter()
                    .find(|process| process.id == *id)
            })
            .filter(|process| !custom_commands::is_custom(process))
            .map(command_trust::pending_command)
            .filter(|command| {
                !self
                    .workspace_state
                    .confirmed_commands
                    .contains(&command.identity)
            })
            .collect()
    }

    /// Opens the confirmation for `commands`; true when the start has to wait for it.
    fn ask_before_running(
        &mut self,
        project_path: &Path,
        commands: Vec<command_trust::PendingCommand>,
        then: command_trust::AfterConfirm,
    ) -> bool {
        if commands.is_empty() {
            return false;
        }
        let project_name = self
            .projects
            .iter()
            .find(|project| project.path == project_path)
            .map_or_else(
                || display_path(project_path),
                |project| project.name.clone(),
            );
        self.command_confirmation = Some(command_trust::CommandConfirmation {
            project_name,
            project_path: project_path.to_path_buf(),
            git_remote: command_trust::git_remote(project_path),
            commands,
            then,
            details_open: false,
            focused: false,
            origin: self.repaint.memory(|memory| memory.focused()),
        });
        true
    }

    fn show_command_confirmation(&mut self, root_ui: &mut egui::Ui) {
        let Some(mut confirmation) = self.command_confirmation.take() else {
            return;
        };
        match command_trust::show(root_ui, &mut confirmation) {
            None => self.command_confirmation = Some(confirmation),
            // Nothing runs and nothing is recorded.
            Some(command_trust::Decision::Cancel) => {}
            Some(command_trust::Decision::Confirm) => {
                for command in &confirmation.commands {
                    if !self
                        .workspace_state
                        .confirmed_commands
                        .contains(&command.identity)
                    {
                        self.workspace_state
                            .confirmed_commands
                            .push(command.identity.clone());
                    }
                }
                self.workspace_dirty = true;
                match confirmation.then {
                    command_trust::AfterConfirm::StartProject(path) => self.start_project(&path),
                    command_trust::AfterConfirm::StartProcess(id) => self.start_process(&id),
                }
            }
        }
    }

    fn start_process(&mut self, id: &str) {
        let Some(index) = self
            .managed
            .processes
            .iter()
            .position(|process| process.id == id)
        else {
            self.notify("O comando selecionado não está mais disponível");
            return;
        };
        if self.discovery.is_some() {
            self.notify("Aguarde a varredura terminar antes de iniciar um processo");
            return;
        }
        if self
            .managed
            .running
            .get(id)
            .is_some_and(|running| !running.finished)
        {
            self.notify(format!(
                "{} já está em execução",
                self.managed.processes[index].name
            ));
            return;
        }
        let project_path = self.managed.processes[index].project_path.clone();
        let pending = self.unconfirmed_commands(&[id.to_owned()]);
        if self.ask_before_running(
            &project_path,
            pending,
            command_trust::AfterConfirm::StartProcess(id.to_owned()),
        ) {
            return;
        }
        let process = self.managed.processes[index].clone();
        if !process.working_directory.is_dir() {
            self.managed.processes[index].state = ProcessState::Failed;
            self.managed.processes[index].process_id = None;
            self.managed.processes[index].exit_code = None;
            self.managed.failures.insert(
                id.to_owned(),
                ProcessFailure::FolderMissing(process.working_directory.clone()),
            );
            self.notify(format!(
                "Não foi possível iniciar {}: a pasta '{}' não existe mais",
                process.name,
                display_path(&process.working_directory)
            ));
            return;
        }
        if !process.prepare.is_empty() && !self.managed.built.remove(id) {
            self.managed.build_logs.remove(id);
            let repaint = self.repaint.clone();
            self.start_build(index, &repaint);
            return;
        }
        if find_executable(&process.program, self.environment_path.as_deref()).is_none() {
            self.managed.processes[index].state = ProcessState::Failed;
            self.managed.processes[index].process_id = None;
            self.managed.processes[index].exit_code = None;
            self.managed.failures.insert(
                id.to_owned(),
                ProcessFailure::ProgramMissing(process.program.clone()),
            );
            self.notify(format!(
                "Não foi possível iniciar {}: o programa '{}' não foi encontrado no PATH",
                process.name, process.program
            ));
            return;
        }
        let mut assigned_port = None;
        let mut alternative_port = None;
        if let Some(port) = process.expected_port {
            if self.managed.port_is_available(port) {
                assigned_port = Some(port);
                self.managed.port_conflicts.remove(id);
            } else if process.port_override.is_some()
                && let Some(available_port) = self.find_available_port_after(port)
            {
                assigned_port = Some(available_port);
                alternative_port = Some(available_port);
                self.managed.port_conflicts.remove(id);
            } else {
                let owner = match inspect_tcp_port(port) {
                    PortAvailability::Occupied { owner } => owner,
                    PortAvailability::Available => None,
                };
                self.managed.failures.insert(
                    id.to_owned(),
                    ProcessFailure::PortTaken {
                        port,
                        owner: owner.clone(),
                    },
                );
                let conflict = PortConflict { port, owner };
                self.managed.processes[index].state = ProcessState::Failed;
                self.managed.processes[index].process_id = None;
                self.managed.processes[index].exit_code = None;
                self.notify(port_conflict_message(&process.name, &conflict));
                self.managed.port_conflicts.insert(id.to_owned(), conflict);
                self.record_process_history(id);
                return;
            }
        }
        self.managed.processes[index].state = ProcessState::Starting;
        self.managed.processes[index].exit_code = None;
        self.managed.failures.remove(id);
        if let Some(port) = assigned_port {
            self.managed.assigned_ports.insert(id.to_owned(), port);
        }
        let mut command =
            process_command_on_port(&process, alternative_port, self.environment_path.as_deref());
        command
            .current_dir(plain_path(&process.working_directory))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        match command.spawn() {
            Ok(mut child) => {
                self.managed.port_conflicts.remove(id);
                let process_id = child.id();
                let (sender, output) = mpsc::channel();
                if let Some(stdout) = child.stdout.take() {
                    spawn_output_reader(stdout, sender.clone());
                }
                if let Some(stderr) = child.stderr.take() {
                    spawn_output_reader(stderr, sender);
                }
                let mut logs = VecDeque::new();
                if let (Some(preferred), Some(selected)) = (process.expected_port, alternative_port)
                {
                    logs.push_back(format!(
                        "[LocalCodePilot] Porta {} ocupada. O serviço será iniciado na porta {}.",
                        preferred.value(),
                        selected.value()
                    ));
                }
                let now = Instant::now();
                self.managed.running.insert(
                    id.to_owned(),
                    RunningProcess {
                        child,
                        output,
                        logs,
                        urls: Vec::new(),
                        finished: false,
                        started_at: now,
                        last_health_check: now,
                        port_unavailable_since: None,
                        verified_listener: None,
                        foreign_listener: None,
                        http_health: None,
                        http_health_receiver: None,
                        last_http_health_check: None,
                        listener_probe: None,
                        last_listener_probe: None,
                    },
                );
                self.managed.processes[index].state = if assigned_port.is_some() {
                    ProcessState::Starting
                } else {
                    ProcessState::Running
                };
                self.managed.processes[index].process_id = Some(process_id);
                self.managed.processes[index].exit_code = None;
                if let (Some(preferred), Some(selected)) = (process.expected_port, alternative_port)
                {
                    self.notify(format!(
                        "{} iniciando na porta {} porque a porta {} está ocupada",
                        process.name,
                        selected.value(),
                        preferred.value()
                    ));
                } else if let Some(port) = assigned_port {
                    self.notify(format!(
                        "{} iniciando na porta {} (PID {process_id})",
                        process.name,
                        port.value()
                    ));
                } else {
                    self.notify(format!("{} iniciado (PID {process_id})", process.name));
                }
                self.record_process_history(id);
            }
            Err(error) => {
                self.managed.assigned_ports.remove(id);
                self.managed.processes[index].state = ProcessState::Failed;
                self.managed.processes[index].process_id = None;
                self.managed.processes[index].exit_code = None;
                self.managed.failures.insert(
                    id.to_owned(),
                    ProcessFailure::SpawnFailed(error.to_string()),
                );
                self.notify(format!(
                    "Não foi possível iniciar {}: {error}",
                    process.name
                ));
                self.record_process_history(id);
            }
        }
    }

    fn stop_process(&mut self, id: &str) {
        if self.stop_build(id) {
            self.notify("Compilação interrompida");
            return;
        }
        let Some(running) = self.managed.running.get_mut(id) else {
            return;
        };
        if running.finished {
            return;
        }
        terminate_process(&mut running.child);
        running.finished = true;
        self.managed.assigned_ports.remove(id);
        self.managed.failures.remove(id);
        if let Some(process) = self
            .managed
            .processes
            .iter_mut()
            .find(|process| process.id == id)
        {
            process.state = ProcessState::Stopped;
            process.process_id = None;
            process.exit_code = None;
            let process_name = process.name.clone();
            self.notify(format!("{process_name} interrompido"));
        }
        self.record_process_history(id);
    }

    fn start_project(&mut self, project_path: &Path) {
        if self.projects.iter().any(|project| {
            project.path == project_path
                && project.technologies.contains(&TechnologyKind::WordPress)
                && localcodepilot_runtime::containers::wordpress_uses_compose(
                    project_path,
                    project_path,
                )
        }) {
            self.show_project_page(project_path.to_path_buf());
            self.notify(
                "Este projeto usa o banco definido no Compose. Inicie os serviços em O que roda, na página do projeto.",
            );
            return;
        }
        let environment = self
            .project_environments
            .get(project_path)
            .cloned()
            .or_else(|| {
                self.projects
                    .iter()
                    .find(|project| project.path == project_path)
                    .map(|project| {
                        inspect_project_environment(
                            project,
                            &self.managed.processes,
                            self.environment_path.as_deref(),
                        )
                    })
            })
            .unwrap_or_default();
        if !environment.is_ready() {
            self.notify(
                environment
                    .problem_details()
                    .unwrap_or_else(|| "Nenhum serviço executável foi detectado".into()),
            );
            return;
        }
        // Every new command of the project in one confirmation, before anything happens
        // (not even the SQLite file is created).
        let pending = self.unconfirmed_commands(&project_process_ids(
            &self.managed.processes,
            project_path,
            false,
        ));
        if self.ask_before_running(
            project_path,
            pending,
            command_trust::AfterConfirm::StartProject(project_path.to_path_buf()),
        ) {
            return;
        }
        // A local database the project needs must accept the project's login before any of
        // its processes start (CA-5); on Windows the app prepares or starts its own server.
        if self.database.jobs.contains_key(project_path) {
            return;
        }
        match self.project_database_gate(project_path, &environment) {
            database_ui::DatabaseGate::Ready => {
                self.database.failures.remove(project_path);
                // A port change confirmed by the login: it stays, and Detalhes says so.
                if let Some(change) = self.database.port_changes.remove(project_path) {
                    self.database
                        .actions
                        .entry(project_path.to_path_buf())
                        .or_default()
                        .push(format!(
                            "porta alterada de {} para {} em {}; cópia em {}",
                            change.from,
                            change.to,
                            display_path(&change.env),
                            display_path(&change.backup)
                        ));
                }
            }
            database_ui::DatabaseGate::Waiting => return,
            database_ui::DatabaseGate::Blocked(failure) => {
                self.record_database_failure(project_path, failure);
                return;
            }
        }
        for database in detect_database_requirements(project_path) {
            if database.engine == DatabaseEngine::Sqlite {
                if let Err(error) = prepare_sqlite_database(&database, project_path) {
                    self.notify(format!("Não foi possível preparar SQLite: {error}"));
                    return;
                }
                if let Some(environment) = self.project_environments.get_mut(project_path) {
                    for existing in &mut environment.databases {
                        if existing.requirement.source == database.source {
                            existing.available = Some(true);
                        }
                    }
                }
            }
        }
        // Pending migrations of a Laravel project run, after a confirmation, before its
        // processes start (T027).
        if let migration_ui::MigrationGate::Waiting = self.migration_gate(project_path) {
            return;
        }
        let process_ids = project_process_ids(&self.managed.processes, project_path, false);
        let pending_processes: Vec<_> = process_ids
            .iter()
            .filter_map(|id| {
                self.managed
                    .processes
                    .iter()
                    .find(|process| process.id == *id)
            })
            .cloned()
            .collect();
        let mut first_conflict = None;
        for process in &pending_processes {
            let Some(port) = process.expected_port else {
                continue;
            };
            if !self.managed.port_is_available(port) && process.port_override.is_none() {
                let owner = match inspect_tcp_port(port) {
                    PortAvailability::Occupied { owner } => owner,
                    PortAvailability::Available => None,
                };
                self.managed.failures.insert(
                    process.id.clone(),
                    ProcessFailure::PortTaken {
                        port,
                        owner: owner.clone(),
                    },
                );
                let conflict = PortConflict { port, owner };
                if let Some(process) = self
                    .managed
                    .processes
                    .iter_mut()
                    .find(|candidate| candidate.id == process.id)
                {
                    process.state = ProcessState::Failed;
                    process.process_id = None;
                    process.exit_code = None;
                }
                self.managed
                    .port_conflicts
                    .insert(process.id.clone(), conflict.clone());
                first_conflict.get_or_insert_with(|| (process.name.clone(), conflict));
            } else {
                self.managed.port_conflicts.remove(&process.id);
            }
        }
        if let Some((process_name, conflict)) = first_conflict {
            self.show_project_page(project_path.to_path_buf());
            self.notify(format!(
                "O ambiente não foi iniciado. {}",
                port_conflict_message(&process_name, &conflict)
            ));
            return;
        }

        self.focused_project = Some(project_path.to_path_buf());
        for process_id in process_ids {
            self.start_process(&process_id);
        }
    }

    fn stop_project(&mut self, project_path: &Path) {
        let process_ids = project_process_ids(&self.managed.processes, project_path, true);
        for process_id in process_ids {
            self.stop_process(&process_id);
        }
    }

    fn install_dependencies(&mut self, project_path: &Path, repaint: egui::Context) {
        self.request_environment_plan(project_path, repaint);
    }

    fn request_environment_plan(&mut self, project_path: &Path, repaint: egui::Context) {
        if self
            .dependency_installs
            .get(project_path)
            .is_some_and(DependencyInstallSession::is_running)
        {
            self.dependency_terminal = Some(project_path.to_path_buf());
            return;
        }
        if self.environment_plan_request.is_some() {
            self.notify("Aguarde a preparação do plano atual");
            return;
        }
        if let Some((active_path, _)) = self
            .dependency_installs
            .iter()
            .find(|(_, session)| session.is_running())
        {
            let active_path = active_path.clone();
            self.dependency_terminal = Some(active_path);
            self.notify("Já existe uma preparação em andamento nesta máquina");
            return;
        }
        let Some(environment) = self.project_environments.get(project_path) else {
            self.notify("O diagnóstico desse projeto não está disponível");
            return;
        };
        if !environment.can_install_dependencies() && !environment.can_prepare_environment() {
            self.notify(
                environment
                    .problem_details()
                    .unwrap_or_else(|| "Não há etapas de preparação disponíveis".into()),
            );
            return;
        }
        let project_path = project_path.to_path_buf();
        let plan_project_path = project_path.clone();
        let environment = environment.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = resolve_environment_plan(&plan_project_path, &environment);
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        self.environment_plan_request = Some((project_path, receiver));
        self.notify("Consultando versões e preparando o plano...");
    }

    fn poll_environment_plan(&mut self) {
        let Some((_project_path, receiver)) = self.environment_plan_request.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("A preparação do plano foi interrompida".into()),
        };
        self.environment_plan_request = None;
        match result {
            Ok(plan) => self.environment_plan_confirmation = Some(plan),
            Err(error) => self.notify(error),
        }
    }

    fn start_environment_plan(&mut self, plan: InstallationPlan, repaint: egui::Context) {
        let project_path = plan.project_path.clone();
        let verb = PlanVerb::of(&plan);
        let cancel = Arc::new(AtomicBool::new(false));
        let runner_cancel = cancel.clone();
        let runner_plan = plan.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = run_installation_plan(&runner_plan, &sender, &runner_cancel);
            let _ = sender.send(DependencyInstallEvent::Finished(result));
            repaint.request_repaint();
        });
        self.dependency_installs.insert(
            project_path.clone(),
            DependencyInstallSession {
                receiver: Some(receiver),
                logs: VecDeque::new(),
                status: DependencyInstallStatus::Running,
                plan,
                cancel,
            },
        );
        self.record_environment_preparation(&project_path, "em andamento");
        self.dependency_terminal = Some(project_path);
        self.notify(verb.started());
    }

    fn poll_dependency_installs(&mut self, ctx: &egui::Context) {
        let mut completed = Vec::new();
        for (project_path, session) in &mut self.dependency_installs {
            if session.is_running() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            let mut result = None;
            while let Some(receiver) = session.receiver.as_ref() {
                match receiver.try_recv() {
                    Ok(DependencyInstallEvent::Log(line)) => {
                        session.logs.push_back(line);
                        if session.logs.len() > 1_000 {
                            session.logs.pop_front();
                        }
                    }
                    Ok(DependencyInstallEvent::Finished(finished)) => {
                        result = Some(finished);
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        result = Some(Err("A instalação foi interrompida".into()));
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
            if let Some(result) = result {
                session.receiver = None;
                session.status = match &result {
                    Ok(_) => DependencyInstallStatus::Succeeded,
                    Err(error) => {
                        let failure = format!("[erro] {error}");
                        if session.logs.back() != Some(&failure) {
                            session.logs.push_back(failure);
                        }
                        let logs = session.logs.iter().cloned().collect::<Vec<_>>().join("\n");
                        DependencyInstallStatus::Failed {
                            error: error.clone(),
                            suggestion: dependency_error_suggestion(&logs, error),
                        }
                    }
                };
                completed.push((
                    project_path.clone(),
                    result,
                    PlanVerb::of(&session.plan),
                    prepares_compiler(&session.plan),
                ));
            }
        }

        for (project_path, result, verb, compiler) in completed {
            self.record_environment_preparation(
                &project_path,
                if result.is_ok() {
                    "concluído"
                } else {
                    "falhou"
                },
            );
            match result {
                Ok(outcome) => {
                    if let Some(path) = outcome.environment_path {
                        self.environment_path = Some(path);
                    }
                    if let Some(project) = self
                        .projects
                        .iter()
                        .find(|project| project.path == project_path)
                    {
                        let refreshed_processes =
                            project_processes(project, &self.workspace_state.custom_commands);
                        self.managed
                            .processes
                            .retain(|process| process.project_path != project_path);
                        self.managed.processes.extend(refreshed_processes);
                        let environment = inspect_project_environment(
                            project,
                            &self.managed.processes,
                            self.environment_path.as_deref(),
                        );
                        let ready = environment.is_ready();
                        self.project_environments
                            .insert(project_path.clone(), environment);
                        self.notify(verb.finished(ready));
                    }
                }
                Err(error) if error == PLAN_CANCELLED => {
                    self.dependency_terminal = Some(project_path);
                    self.notify(verb.cancelled_while_running());
                }
                Err(error) => {
                    self.dependency_terminal = Some(project_path);
                    if compiler {
                        self.notify(COMPILER_FAILED);
                    } else {
                        self.notify(verb.failed(&error));
                    }
                }
            }
        }
    }

    fn record_environment_preparation(&mut self, project_path: &Path, status: &str) {
        let Some(session) = self.dependency_installs.get(project_path) else {
            return;
        };
        self.workspace_state
            .record_preparation(config::PreparationHistory {
                project_path: project_path.to_path_buf(),
                status: status.into(),
                updated_at: unix_timestamp(),
                steps: session
                    .plan
                    .steps
                    .iter()
                    .map(installation_step_command)
                    .collect(),
                logs: session.logs.iter().cloned().collect(),
            });
        self.workspace_dirty = true;
    }

    fn prepare_laravel_migration(&mut self, project_path: &Path) {
        let project_name = self
            .projects
            .iter()
            .find(|project| project.path == project_path)
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "Projeto Laravel".into());
        match create_laravel_migration(project_path, project_name) {
            Ok(migration) => self.laravel_migration = Some(migration),
            Err(error) => self.notify(error),
        }
    }

    fn start_laravel_migration_preview(&mut self, repaint: egui::Context) {
        let Some(migration) = self.laravel_migration.as_mut() else {
            return;
        };
        let project_path = migration.project_path.clone();
        let target_constraint = migration.target_constraint.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = preview_laravel_migration(&project_path, &target_constraint, &sender);
            let _ = sender.send(LaravelMigrationEvent::PreviewFinished(result));
            repaint.request_repaint();
        });
        migration.phase = LaravelMigrationPhase::Previewing;
        migration.receiver = Some(receiver);
        migration.logs.clear();
        migration.preview = None;
        migration.error = None;
    }

    fn apply_laravel_migration(&mut self, repaint: egui::Context) {
        let Some(migration) = self.laravel_migration.as_mut() else {
            return;
        };
        let Some(preview) = migration.preview.clone() else {
            migration.error = Some("A simulação precisa ser concluída antes da aplicação".into());
            migration.phase = LaravelMigrationPhase::Failed;
            return;
        };
        let project_path = migration.project_path.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = apply_laravel_migration_files(&project_path, &preview, &sender);
            let _ = sender.send(LaravelMigrationEvent::ApplyFinished(result));
            repaint.request_repaint();
        });
        migration.phase = LaravelMigrationPhase::Applying;
        migration.receiver = Some(receiver);
        migration.error = None;
    }

    fn poll_laravel_migration(&mut self, ctx: &egui::Context) {
        let Some(migration) = self.laravel_migration.as_mut() else {
            return;
        };
        if matches!(
            migration.phase,
            LaravelMigrationPhase::Previewing | LaravelMigrationPhase::Applying
        ) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let mut finished = false;
        let mut applied_project = None;
        while let Some(receiver) = migration.receiver.as_ref() {
            match receiver.try_recv() {
                Ok(LaravelMigrationEvent::Log(line)) => {
                    migration.logs.push_back(line);
                    if migration.logs.len() > 1_000 {
                        migration.logs.pop_front();
                    }
                }
                Ok(LaravelMigrationEvent::PreviewFinished(result)) => {
                    finished = true;
                    match result {
                        Ok(preview) => {
                            migration.preview = Some(preview);
                            migration.phase = LaravelMigrationPhase::Ready;
                        }
                        Err(error) => {
                            migration.error = Some(error);
                            migration.phase = LaravelMigrationPhase::Failed;
                        }
                    }
                    break;
                }
                Ok(LaravelMigrationEvent::ApplyFinished(result)) => {
                    finished = true;
                    match result {
                        Ok(backup_path) => {
                            migration.backup_path = Some(backup_path);
                            migration.phase = LaravelMigrationPhase::Completed;
                            applied_project = Some(migration.project_path.clone());
                        }
                        Err(error) => {
                            migration.error = Some(error);
                            migration.phase = LaravelMigrationPhase::Failed;
                        }
                    }
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    migration.error = Some("A operação de migração foi interrompida".into());
                    migration.phase = LaravelMigrationPhase::Failed;
                    finished = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if finished {
            migration.receiver = None;
        }
        if let Some(project_path) = applied_project {
            self.dependency_installs.remove(&project_path);
            if let Some(project) = self
                .projects
                .iter()
                .find(|project| project.path == project_path)
            {
                self.project_environments.insert(
                    project_path,
                    inspect_project_environment(
                        project,
                        &self.managed.processes,
                        self.environment_path.as_deref(),
                    ),
                );
            }
            self.notify("Migração aplicada. Revise e teste o projeto.");
        }
    }

    fn restart_process(&mut self, id: &str) {
        self.stop_process(id);
        self.start_process(id);
    }

    fn request_port_owner_termination(&mut self, id: &str) {
        let Some(conflict) = self.managed.port_conflicts.get(id).cloned() else {
            self.notify("A porta já está livre. Tente iniciar o serviço novamente.");
            return;
        };
        if conflict.owner.is_none() {
            self.notify(
                "A porta está ocupada, mas o Windows não informou qual processo a utiliza.",
            );
            return;
        }
        self.managed.port_termination = Some(PortTerminationRequest {
            process_id: id.to_owned(),
            conflict,
        });
    }

    fn terminate_port_owner_and_retry(&mut self, request: &PortTerminationRequest) {
        let Some(owner) = &request.conflict.owner else {
            return;
        };
        match inspect_tcp_port(request.conflict.port) {
            PortAvailability::Available => {
                self.managed.port_conflicts.remove(&request.process_id);
                self.notify("A porta já foi liberada. Tentando iniciar o serviço...");
                self.start_process(&request.process_id);
            }
            PortAvailability::Occupied {
                owner: Some(current_owner),
            } if current_owner.process_id != owner.process_id => {
                self.managed.failures.insert(
                    request.process_id.clone(),
                    ProcessFailure::PortTaken {
                        port: request.conflict.port,
                        owner: Some(current_owner.clone()),
                    },
                );
                let conflict = PortConflict {
                    port: request.conflict.port,
                    owner: Some(current_owner),
                };
                self.managed
                    .port_conflicts
                    .insert(request.process_id.clone(), conflict.clone());
                self.notify(port_conflict_message("O serviço", &conflict));
            }
            PortAvailability::Occupied { .. } => match terminate_process_tree(owner.process_id) {
                Ok(()) => {
                    self.managed.port_conflicts.remove(&request.process_id);
                    self.notify(format!(
                        "PID {} encerrado. Tentando iniciar o serviço...",
                        owner.process_id
                    ));
                    self.start_process(&request.process_id);
                }
                Err(error) => self.notify(error),
            },
        }
    }

    fn show_port_termination_confirmation(&mut self, root_ui: &mut egui::Ui) {
        let Some(request) = self.managed.port_termination.clone() else {
            return;
        };
        let Some(owner) = request.conflict.owner.as_ref() else {
            self.managed.port_termination = None;
            return;
        };
        let process_name = owner
            .process_name
            .as_deref()
            .unwrap_or("processo desconhecido");
        let mut open = true;
        let mut cancel = false;
        let mut confirm = false;

        egui::Window::new("Liberar porta ocupada")
            .id(egui::Id::unique("port-termination-confirmation"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(root_ui, |ui| {
                ui.label(
                    RichText::new(format!(
                        "A porta {} está sendo usada por {process_name} (PID {}).",
                        request.conflict.port.value(),
                        owner.process_id
                    ))
                    .strong(),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Encerrar esse processo pode interromper outro projeto ou aplicativo. Depois disso, o LocalCodePilot tentará iniciar o serviço novamente.",
                    )
                    .color(theme::MUTED),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                    let can_terminate = owner.process_id > 4
                        && owner.process_id != std::process::id();
                    if ui
                        .add_enabled(
                            can_terminate,
                            egui::Button::new("Encerrar e tentar novamente")
                                .fill(Color32::from_rgb(235, 87, 87).gamma_multiply(0.22)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });

        if confirm {
            self.managed.port_termination = None;
            self.terminate_port_owner_and_retry(&request);
        } else if cancel || !open {
            self.managed.port_termination = None;
        }
    }

    /// CA-5: updating closes the app and ends the processes it manages. Containers started
    /// by Compose run detached and keep running, so the text speaks only of processes.
    fn show_update_confirmation(&mut self, root_ui: &mut egui::Ui) {
        if self.update_session.confirmation.is_none() {
            return;
        }
        let runs: Vec<(String, &'static str)> = self
            .managed
            .processes
            .iter()
            .filter(|process| {
                matches!(
                    process.state,
                    ProcessState::Starting | ProcessState::Running
                )
            })
            .map(|process| {
                let label = if process.name.is_empty() || process.name == process.project_name {
                    process.project_name.clone()
                } else {
                    format!("{} · {}", process.project_name, process.name)
                };
                let state = if process.state == ProcessState::Starting {
                    "Iniciando"
                } else {
                    "Rodando"
                };
                (label, state)
            })
            .collect();
        let focus = std::mem::take(&mut self.update_session.confirmation_focus);
        let mut later = false;
        let mut confirm = false;
        egui::Window::new("update-confirmation")
            .id(egui::Id::unique("update-confirmation"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(480.0)
            .show(root_ui, |ui| {
                ui.label(RichText::new("Há projetos em execução.").strong().size(15.0));
                ui.add_space(6.0);
                ui.label(
                    "Para instalar a atualização, o LocalCodePilot precisará fechar e encerrar os processos que gerencia. Salve seu trabalho antes de continuar.",
                );
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(140.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (label, state) in &runs {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(label).strong().size(12.0));
                                ui.label(RichText::new(*state).color(theme::MUTED).size(11.0));
                            });
                        }
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let not_now = ui.button("Agora não");
                    if focus {
                        not_now.request_focus();
                    }
                    if not_now.clicked() {
                        later = true;
                    }
                    if ui
                        .add(
                            egui::Button::new("Atualizar e encerrar processos")
                                .fill(Color32::from_rgb(235, 87, 87).gamma_multiply(0.22)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        let escape = root_ui.input(|input| input.key_pressed(egui::Key::Escape));
        if confirm {
            let active = self.managed.active_runs();
            match self.update_session.confirmation.take() {
                Some(UpdateConfirmation::BeforeDownload(update)) => {
                    self.update_session.confirmed_runs = active;
                    self.update_session
                        .start_install(&update, root_ui.ctx().clone());
                }
                Some(UpdateConfirmation::BeforeRestart(prepared)) => {
                    self.update_session.confirmed_runs = active;
                    self.launch_update(&prepared, &root_ui.ctx().clone());
                }
                None => {}
            }
        } else if later || escape {
            // "Agora não": nothing is downloaded or installed; the app and its processes
            // keep running. A file already prepared is discarded.
            self.update_session.decline();
        }
    }

    fn show_clear_history_confirmation(&mut self, root_ui: &mut egui::Ui) {
        if !self.confirm_clear_history {
            return;
        }
        let choice = components::Confirmation {
            id: "clear-history",
            title: "Limpar histórico de logs?",
            consequence: "Os logs salvos de serviços e de preparação serão removidos deste computador. Esta ação não pode ser desfeita.",
            confirm: "Limpar histórico",
            destructive: true,
        }
        .show(root_ui.ctx(), |_| {});
        match choice {
            Some(components::Choice::Confirm) => {
                self.confirm_clear_history = false;
                self.workspace_state.clear_saved_logs();
                self.workspace_dirty = true;
                self.persist_workspace_state(true);
                self.notify("Histórico de logs removido.");
            }
            Some(components::Choice::Cancel) => self.confirm_clear_history = false,
            None => {}
        }
    }

    fn clear_process_logs(&mut self, id: &str) {
        if let Some(running) = self.managed.running.get_mut(id) {
            running.logs.clear();
            self.record_process_history(id);
            self.notify("Saída do processo limpa");
        } else if let Some(history) = self.workspace_state.service_mut(id) {
            history.logs.clear();
            self.workspace_dirty = true;
            self.notify("Histórico de logs removido");
        } else {
            self.notify("Esse processo ainda não possui saída para limpar");
        }
    }

    fn show_terminal(&mut self, root_ui: &mut egui::Ui) {
        let Some(process_id) = self.managed.terminal.clone() else {
            return;
        };
        let Some(process) = self
            .managed
            .processes
            .iter()
            .find(|process| process.id == process_id)
            .cloned()
        else {
            self.managed.terminal = None;
            return;
        };
        let process_history = self.workspace_state.service(&process_id).cloned();
        let (logs, finished) = self
            .managed
            .running
            .get(&process_id)
            .map(|running| {
                (
                    running.logs.iter().cloned().collect::<Vec<_>>(),
                    running.finished,
                )
            })
            .or_else(|| {
                process_history
                    .as_ref()
                    .map(|history| (history.logs.clone(), true))
            })
            .unwrap_or_default();
        let assigned_port = self
            .managed
            .assigned_ports
            .get(&process_id)
            .copied()
            .or_else(|| {
                process_history
                    .as_ref()
                    .and_then(|history| history.port)
                    .and_then(Port::new)
            });
        let failure_suggestion = (process.state == ProcessState::Failed
            || process_history
                .as_ref()
                .is_some_and(|history| history.status == "failed"))
        .then(|| process_error_suggestion(&logs))
        .flatten();
        let mut open = true;
        let mut clear_logs = false;
        let mut stop_process = false;
        let mut restart_process = false;

        egui::Window::new(format!("{TERMINAL}  Terminal · {}", process.name))
            .id(egui::Id::unique("process-terminal"))
            .open(&mut open)
            .default_size([820.0, 520.0])
            .min_size([480.0, 300.0])
            .resizable(true)
            .collapsible(false)
            .show(root_ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (status, color) = process_state_display(process.state);
                    runtime_badge(ui, status, color);
                    if process.state == ProcessState::Stopped
                        && let Some(history) = &process_history
                    {
                        ui.label(
                            RichText::new(format!(
                                "Última execução: {} · {}",
                                relative_time(history.updated_at),
                                service_history_status(&history.status)
                            ))
                            .color(theme::MUTED),
                        );
                    }
                    if let Some(pid) = process.process_id {
                        ui.label(RichText::new(format!("PID {pid}")).color(theme::MUTED));
                    }
                    if let Some(port) = assigned_port.or(process.expected_port) {
                        let adjusted = process
                            .expected_port
                            .is_some_and(|expected| expected != port);
                        ui.label(
                            RichText::new(if adjusted {
                                format!("Porta {} · ajustada automaticamente", port.value())
                            } else {
                                format!("Porta {}", port.value())
                            })
                            .color(if adjusted {
                                theme::SUCCESS
                            } else {
                                theme::MUTED
                            }),
                        );
                    }
                    ui.label(
                        RichText::new(process.command_line())
                            .color(theme::MUTED)
                            .monospace(),
                    );
                });
                ui.label(
                    RichText::new(display_path(&process.working_directory))
                        .color(Color32::from_rgb(112, 124, 145))
                        .monospace()
                        .size(10.0),
                );
                if let Some(suggestion) = &failure_suggestion {
                    ui.add_space(8.0);
                    Frame::new()
                        .fill(Color32::from_rgb(255, 205, 75).gamma_multiply(0.08))
                        .stroke(Stroke::new(
                            1.0,
                            Color32::from_rgb(255, 205, 75).gamma_multiply(0.35),
                        ))
                        .corner_radius(6)
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("Sugestão de correção: {suggestion}"))
                                    .color(Color32::from_rgb(255, 215, 112)),
                            );
                        });
                }
                ui.add_space(8.0);
                Frame::new()
                    .fill(Color32::from_rgb(7, 10, 15))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(39, 48, 64)))
                    .corner_radius(6)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_min_height(350.0);
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                if logs.is_empty() {
                                    ui.label(
                                        RichText::new(if finished {
                                            "O processo terminou sem gerar saída."
                                        } else {
                                            "Aguardando saída do processo..."
                                        })
                                        .color(Color32::from_rgb(126, 138, 157))
                                        .monospace(),
                                    );
                                } else {
                                    for line in &logs {
                                        ui.label(
                                            RichText::new(line)
                                                .color(Color32::from_rgb(205, 214, 226))
                                                .monospace()
                                                .size(11.0),
                                        );
                                    }
                                }
                            });
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Limpar").clicked() {
                        clear_logs = true;
                    }
                    if process.state == ProcessState::Running {
                        if ui.button("Reiniciar").clicked() {
                            restart_process = true;
                        }
                        if ui
                            .add(
                                egui::Button::new("Parar")
                                    .fill(Color32::from_rgb(235, 87, 87).gamma_multiply(0.18)),
                            )
                            .clicked()
                        {
                            stop_process = true;
                        }
                    }
                });
            });

        if clear_logs {
            self.clear_process_logs(&process_id);
        }
        if restart_process {
            self.restart_process(&process_id);
        } else if stop_process {
            self.stop_process(&process_id);
        }
        if !open {
            self.managed.terminal = None;
        }
    }

    fn show_environment_plan_confirmation(&mut self, root_ui: &mut egui::Ui) {
        let Some(plan) = self.environment_plan_confirmation.clone() else {
            return;
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        let window_height = (root_ui.max_rect().height() - 64.0).max(120.0);
        egui::Window::new(plan_window_title(&plan))
            .id(egui::Id::unique(("environment-plan", &plan.project_path)))
            .open(&mut open)
            .default_width(620.0)
            .default_height(window_height.min(640.0))
            .max_height(window_height)
            .resizable(true)
            .collapsible(false)
            .show(root_ui, |ui| {
                let details_height = (ui.available_height() - 48.0).max(32.0);
                egui::ScrollArea::vertical()
                    .id_salt(("environment-plan-details", &plan.project_path))
                    .max_height(details_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(RichText::new(&plan.title).strong().size(16.0));
                        if let Some(notice) = compiler_notice(&plan) {
                            ui.add(egui::Label::new(RichText::new(notice).strong()).wrap());
                        }
                        ui.label(
                            RichText::new(display_path(&plan.project_path))
                                .color(theme::MUTED)
                                .monospace()
                                .size(10.0),
                        );
                        // This window is the confirmation of these commands (LCP-SEC-001);
                        // it does not vouch for what the project's files do later.
                        ui.add(
                            egui::Label::new(
                                "Os arquivos do projeto podem mudar depois desta confirmação.",
                            )
                            .wrap(),
                        );
                        ui.add_space(10.0);
                        if !plan.requirements.is_empty() {
                            ui.label(RichText::new("Requisitos").strong().size(12.0));
                            for requirement in &plan.requirements {
                                let status = match &requirement.status {
                                    RequirementStatus::Available { version, .. } => {
                                        format!("instalado: {version}")
                                    }
                                    RequirementStatus::Missing => "não encontrado".into(),
                                    RequirementStatus::Incompatible {
                                        installed_version, ..
                                    } => format!("incompatível: {installed_version}"),
                                    RequirementStatus::Pending => "verificando".into(),
                                };
                                ui.label(format!(
                                    "{} {} · {status}",
                                    requirement.runtime, requirement.version.value
                                ));
                            }
                            ui.add_space(10.0);
                        }
                        for (index, step) in plan.steps.iter().enumerate() {
                            Frame::new()
                                .fill(theme::SURFACE)
                                .corner_radius(6)
                                .inner_margin(Margin::same(10))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!("{}. {}", index + 1, step.label))
                                            .strong(),
                                    );
                                    ui.label(RichText::new(&step.detail).color(theme::MUTED).size(10.0));
                                    if let Some(source) = &step.source {
                                        ui.label(
                                            RichText::new(format!("Origem: {source}"))
                                                .color(theme::MUTED)
                                                .size(9.0),
                                        );
                                    }
                                    ui.label(
                                        RichText::new(installation_step_command(step))
                                            .color(Color32::from_rgb(150, 164, 185))
                                            .monospace()
                                            .size(9.0),
                                    );
                                });
                            ui.add_space(6.0);
                        }
                        if plan.requires_elevation() {
                            ui.label(
                                RichText::new(
                                    "O Windows solicitará autorização de administrador durante a instalação.",
                                )
                                .color(Color32::from_rgb(255, 205, 75))
                                .size(10.0),
                            );
                        }
                    });
                ui.separator();
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let cancel_button = ui.button("Cancelar");
                    // The keyboard focus moves into the window when it opens, on the safe
                    // choice, as in the command confirmation (T033.03, CA-9).
                    let focus_key = egui::Id::unique(("environment-plan-focus", &plan.project_path));
                    if !ui.data(|data| data.get_temp::<bool>(focus_key).unwrap_or(false)) {
                        cancel_button.request_focus();
                        ui.data_mut(|data| data.insert_temp(focus_key, true));
                    }
                    if cancel_button.clicked() {
                        cancel = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(plan_confirm_label(&plan))
                                .fill(theme::PRIMARY.gamma_multiply(0.72)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if confirm || cancel || !open || root_ui.input(|input| input.key_pressed(egui::Key::Escape))
        {
            root_ui.data_mut(|data| {
                data.remove::<bool>(egui::Id::unique((
                    "environment-plan-focus",
                    &plan.project_path,
                )));
            });
        }

        let escape = root_ui.input(|input| input.key_pressed(egui::Key::Escape));
        if confirm {
            self.environment_plan_confirmation = None;
            self.start_environment_plan(plan, root_ui.ctx().clone());
        } else if cancel || !open || escape {
            self.environment_plan_confirmation = None;
            self.notify(PlanVerb::of(&plan).cancelled_before_running());
        }
    }

    fn show_dependency_terminal(&mut self, root_ui: &mut egui::Ui) {
        let Some(project_path) = self.dependency_terminal.clone() else {
            return;
        };
        let Some(session) = self.dependency_installs.get(&project_path) else {
            self.dependency_terminal = None;
            return;
        };
        let project_name = self
            .projects
            .iter()
            .find(|project| project.path == project_path)
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "Projeto".into());
        let logs = session.logs.iter().cloned().collect::<Vec<_>>();
        let status = session.status.clone();
        let retry_plan = session.plan.clone();
        let verb = PlanVerb::of(&session.plan);
        let offers_laravel_migration = matches!(&status, DependencyInstallStatus::Failed { .. })
            && composer_security_advisory(&logs.join("\n"))
            && project_path.join("composer.json").is_file();
        let mut open = true;
        let mut clear_logs = false;
        let mut retry = false;
        let mut cancel_installation = false;
        let mut prepare_migration = false;

        egui::Window::new(format!("{TERMINAL}  {} · {project_name}", verb.action()))
            .id(egui::Id::unique(("dependency-terminal", &project_path)))
            .open(&mut open)
            .default_size([860.0, 560.0])
            .min_size([500.0, 340.0])
            .resizable(true)
            .collapsible(false)
            .show(root_ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (label, color) = match &status {
                        DependencyInstallStatus::Running => ("Instalando", theme::PRIMARY),
                        DependencyInstallStatus::Cancelling => {
                            ("Cancelando", Color32::from_rgb(255, 205, 75))
                        }
                        DependencyInstallStatus::Succeeded => ("Concluído", theme::SUCCESS),
                        DependencyInstallStatus::Failed { .. } => {
                            ("Falhou", Color32::from_rgb(235, 87, 87))
                        }
                    };
                    runtime_badge(ui, label, color);
                    if matches!(
                        &status,
                        DependencyInstallStatus::Running | DependencyInstallStatus::Cancelling
                    ) {
                        ui.spinner();
                    }
                });
                ui.label(
                    RichText::new(display_path(&project_path))
                        .color(Color32::from_rgb(112, 124, 145))
                        .monospace()
                        .size(10.0),
                );
                ui.add_space(8.0);
                Frame::new()
                    .fill(Color32::from_rgb(7, 10, 15))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(39, 48, 64)))
                    .corner_radius(6)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_min_height(330.0);
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                if logs.is_empty() {
                                    ui.label(
                                        RichText::new("Preparando a instalação...")
                                            .color(Color32::from_rgb(126, 138, 157))
                                            .monospace(),
                                    );
                                } else {
                                    for line in &logs {
                                        ui.label(
                                            RichText::new(line)
                                                .color(Color32::from_rgb(205, 214, 226))
                                                .monospace()
                                                .size(11.0),
                                        );
                                    }
                                }
                            });
                    });

                if let DependencyInstallStatus::Failed { error, suggestion } = &status {
                    ui.add_space(10.0);
                    Frame::new()
                        .fill(Color32::from_rgb(255, 205, 75).gamma_multiply(0.08))
                        .stroke(Stroke::new(
                            1.0,
                            Color32::from_rgb(255, 205, 75).gamma_multiply(0.55),
                        ))
                        .corner_radius(7)
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("Possíveis soluções")
                                    .strong()
                                    .color(Color32::from_rgb(255, 205, 75)),
                            );
                            ui.label(suggestion);
                            ui.label(
                                RichText::new(error)
                                    .color(theme::MUTED)
                                    .monospace()
                                    .size(9.0),
                            );
                        });
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Limpar logs").clicked() {
                        clear_logs = true;
                    }
                    if matches!(&status, DependencyInstallStatus::Running)
                        && ui.button(verb.cancel_button()).clicked()
                    {
                        cancel_installation = true;
                    }
                    if matches!(&status, DependencyInstallStatus::Failed { .. })
                        && ui
                            .add(
                                egui::Button::new("Tentar de novo")
                                    .fill(theme::PRIMARY.gamma_multiply(0.72)),
                            )
                            .clicked()
                    {
                        retry = true;
                    }
                    if offers_laravel_migration
                        && ui
                            .add(
                                egui::Button::new("Preparar migração segura")
                                    .fill(Color32::from_rgb(255, 205, 75).gamma_multiply(0.22)),
                            )
                            .clicked()
                    {
                        prepare_migration = true;
                    }
                });
            });

        if clear_logs && let Some(session) = self.dependency_installs.get_mut(&project_path) {
            session.logs.clear();
        }
        if cancel_installation
            && let Some(session) = self.dependency_installs.get_mut(&project_path)
        {
            session.cancel.store(true, Ordering::Relaxed);
            session.status = DependencyInstallStatus::Cancelling;
        }
        if retry {
            self.start_environment_plan(retry_plan, root_ui.ctx().clone());
        }
        if prepare_migration {
            self.prepare_laravel_migration(&project_path);
        }
        if !open {
            self.dependency_terminal = None;
        }
    }

    fn show_laravel_migration(&mut self, root_ui: &mut egui::Ui) {
        let Some(migration) = self.laravel_migration.as_ref() else {
            return;
        };
        let phase = migration.phase;
        let project_name = migration.project_name.clone();
        let current_constraint = migration.current_constraint.clone();
        let target_constraint = migration.target_constraint.clone();
        let php_version = migration.php_version.clone();
        let project_path = migration.project_path.clone();
        let logs = migration.logs.iter().cloned().collect::<Vec<_>>();
        let error = migration.error.clone();
        let backup_path = migration.backup_path.clone();
        let preview_was_ready = migration.preview.is_some();
        let mut open = true;
        let mut start_preview = false;
        let mut apply = false;
        let mut close = false;

        egui::Window::new("Migração segura do Laravel")
            .id(egui::Id::unique("laravel-migration"))
            .open(&mut open)
            .default_size([720.0, 520.0])
            .min_size([500.0, 360.0])
            .resizable(true)
            .collapsible(false)
            .show(root_ui, |ui| {
                ui.heading(&project_name);
                ui.label(
                    RichText::new(display_path(&project_path))
                        .color(theme::MUTED)
                        .monospace()
                        .size(10.0),
                );
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    runtime_badge(ui, &format!("PHP {php_version}"), runtime_color(RuntimeKind::Php));
                    runtime_badge(
                        ui,
                        &format!("Laravel {current_constraint} → {target_constraint}"),
                        technology_color(TechnologyKind::Laravel),
                    );
                });
                ui.add_space(10.0);

                match phase {
                    LaravelMigrationPhase::Confirm => {
                        ui.label("O primeiro passo executa somente uma simulação em uma pasta temporária.");
                        ui.label(
                            RichText::new(
                                "O projeto original, vendor e seus arquivos de código não serão alterados.",
                            )
                            .strong()
                            .color(theme::SUCCESS),
                        );
                        ui.add_space(8.0);
                        ui.label("A simulação irá:");
                        ui.label("• copiar composer.json e composer.lock, quando existir;");
                        ui.label(format!(
                            "• testar laravel/framework {target_constraint} sem instalar pacotes, plugins ou scripts;"
                        ));
                        ui.label("• impedir a aplicação se as dependências forem incompatíveis.");
                    }
                    LaravelMigrationPhase::Unavailable => {
                        ui.label(
                            RichText::new(format!(
                                "O PHP {php_version} instalado não permite testar {target_constraint}."
                            ))
                            .strong()
                            .color(Color32::from_rgb(255, 205, 75)),
                        );
                        ui.label(
                            "O projeto continuará na versão atual. Quando o suporte a Docker estiver disponível, a simulação poderá usar PHP 8.2 ou superior dentro do container sem exigir a atualização do PHP da máquina.",
                        );
                        ui.label(
                            "O container resolve a compatibilidade do runtime; a atualização do Laravel continuará opcional e sujeita à confirmação.",
                        );
                    }
                    LaravelMigrationPhase::Ready => {
                        ui.label(
                            RichText::new("A simulação foi concluída sem alterar o projeto.")
                                .strong()
                                .color(theme::SUCCESS),
                        );
                        ui.label(
                            "Aplicar substituirá composer.json e composer.lock e executará composer install. Um backup será criado antes da alteração.",
                        );
                    }
                    LaravelMigrationPhase::Completed => {
                        ui.label(
                            RichText::new("Migração das dependências concluída.")
                                .strong()
                                .color(theme::SUCCESS),
                        );
                        if let Some(backup_path) = &backup_path {
                            ui.label(format!("Backup: {}", backup_path.display()));
                        }
                        ui.label("Revise e teste a aplicação, pois versões principais podem exigir ajustes no código.");
                    }
                    LaravelMigrationPhase::Failed => {
                        ui.label(
                            RichText::new("A migração não pôde ser concluída.")
                                .strong()
                                .color(Color32::from_rgb(235, 87, 87)),
                        );
                        if let Some(error) = &error {
                            ui.label(error);
                        }
                        ui.label(if preview_was_ready {
                            "composer.json e composer.lock foram restaurados. Confira os logs antes de tentar novamente."
                        } else {
                            "A falha ocorreu na cópia isolada; o projeto original não foi alterado."
                        });
                    }
                    LaravelMigrationPhase::Previewing => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Testando a resolução das dependências em uma cópia isolada...");
                        });
                    }
                    LaravelMigrationPhase::Applying => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Aplicando os manifests e instalando as dependências...");
                        });
                    }
                }

                if !logs.is_empty() {
                    ui.add_space(10.0);
                    Frame::new()
                        .fill(Color32::from_rgb(7, 10, 15))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(39, 48, 64)))
                        .corner_radius(6)
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            egui::ScrollArea::both()
                                .max_height(240.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    for line in &logs {
                                        ui.label(
                                            RichText::new(line)
                                                .color(Color32::from_rgb(205, 214, 226))
                                                .monospace()
                                                .size(10.0),
                                        );
                                    }
                                });
                        });
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| match phase {
                    LaravelMigrationPhase::Confirm | LaravelMigrationPhase::Failed => {
                        if ui
                            .add(
                                egui::Button::new("Testar em cópia isolada")
                                    .fill(theme::PRIMARY.gamma_multiply(0.72)),
                            )
                            .clicked()
                        {
                            start_preview = true;
                        }
                        if ui.button("Manter versão atual").clicked() {
                            close = true;
                        }
                    }
                    LaravelMigrationPhase::Ready => {
                        if ui
                            .add(
                                egui::Button::new("Confirmar e aplicar ao projeto")
                                    .fill(Color32::from_rgb(255, 205, 75).gamma_multiply(0.22)),
                            )
                            .clicked()
                        {
                            apply = true;
                        }
                        if ui.button("Cancelar").clicked() {
                            close = true;
                        }
                    }
                    LaravelMigrationPhase::Completed => {
                        if ui.button("Fechar").clicked() {
                            close = true;
                        }
                    }
                    LaravelMigrationPhase::Unavailable => {
                        if ui.button("Manter versão atual").clicked() {
                            close = true;
                        }
                    }
                    LaravelMigrationPhase::Previewing | LaravelMigrationPhase::Applying => {}
                });
            });

        if start_preview {
            self.start_laravel_migration_preview(root_ui.ctx().clone());
        }
        if apply {
            self.apply_laravel_migration(root_ui.ctx().clone());
        }
        if close || !open {
            self.laravel_migration = None;
        }
    }

    fn poll_processes(&mut self, ctx: &egui::Context) {
        let process_states: HashMap<_, _> = self
            .managed
            .processes
            .iter()
            .map(|process| (process.id.clone(), process.state))
            .collect();
        let http_process_ids: HashSet<_> = self
            .managed
            .processes
            .iter()
            .filter(|process| {
                self.projects
                    .iter()
                    .find(|project| project.path == process.project_path)
                    .is_some_and(|project| process_exposes_application_url(project, process))
            })
            .map(|process| process.id.clone())
            .collect();
        let assigned_ports = self.managed.assigned_ports.clone();
        let mut state_updates = Vec::new();
        let mut ready_updates = Vec::new();
        let mut history_updates = Vec::new();
        let mut discovered_ports = Vec::new();
        let mut has_running_process = false;
        let now = Instant::now();
        for (id, running) in &mut self.managed.running {
            if let Some(receiver) = &running.http_health_receiver {
                match receiver.try_recv() {
                    Ok(result) => {
                        running.http_health = Some(result);
                        running.http_health_receiver = None;
                    }
                    Err(TryRecvError::Disconnected) => {
                        running.http_health_receiver = None;
                    }
                    Err(TryRecvError::Empty) => {}
                }
            }
            while let Ok(line) = running.output.try_recv() {
                if !history_updates.contains(id) {
                    history_updates.push(id.clone());
                }
                // DevTools (Flutter, Dart, Node inspectors) is a debugger, never the site.
                for url in extract_local_urls(&line)
                    .into_iter()
                    .filter(|url| !url.address().contains("/devtools"))
                {
                    if !running.urls.contains(&url) {
                        running.urls.push(url);
                    }
                }
                running.logs.push_back(line);
                if running.logs.len() > 500 {
                    running.logs.pop_front();
                }
            }
            if running.finished {
                continue;
            }
            let state = process_states
                .get(id)
                .copied()
                .unwrap_or(ProcessState::Stopped);
            match running.child.try_wait() {
                Ok(Some(status)) => {
                    running.finished = true;
                    let exit_state =
                        ProcessState::after_exit(status.code(), assigned_ports.contains_key(id));
                    if exit_state == ProcessState::Completed {
                        running.logs.push_back(
                            "[LocalCodePilot] Aplicação concluída com sucesso (código 0).".into(),
                        );
                        state_updates.push((id.clone(), exit_state, status.code(), None));
                        continue;
                    }
                    // A server that closed because its port was taken says so, with the
                    // program holding the port, instead of "closed unexpectedly".
                    let recent = running
                        .logs
                        .iter()
                        .rev()
                        .take(40)
                        .map(String::as_str)
                        .collect::<Vec<_>>();
                    let taken = port_in_use(&recent).and_then(|named| {
                        named
                            .and_then(Port::new)
                            .or_else(|| assigned_ports.get(id).copied())
                    });
                    if let Some(port) = taken {
                        let owner = match inspect_tcp_port(port) {
                            PortAvailability::Occupied { owner } => owner,
                            PortAvailability::Available => None,
                        };
                        if let Some(owner) = &owner {
                            running.logs.push_back(format!(
                                "[LocalCodePilot] {}",
                                port_owner_detail(port, owner)
                            ));
                        }
                        running.logs.push_back(format!(
                            "[LocalCodePilot] Falha: outro programa está usando a porta {}.",
                            port.value()
                        ));
                        state_updates.push((
                            id.clone(),
                            ProcessState::Failed,
                            status.code(),
                            Some(ProcessFailure::PortTaken { port, owner }),
                        ));
                        continue;
                    }
                    let stopped_during_startup = state == ProcessState::Starting;
                    let failure = if stopped_during_startup {
                        ProcessFailure::ClosedBeforeReady
                    } else {
                        ProcessFailure::Exited
                    };
                    let reason = if stopped_during_startup {
                        "o processo encerrou antes de disponibilizar o serviço".to_owned()
                    } else if let Some(code) = status.code() {
                        format!("o processo encerrou inesperadamente com código {code}")
                    } else {
                        "o processo encerrou inesperadamente".to_owned()
                    };
                    running
                        .logs
                        .push_back(format!("[LocalCodePilot] Falha: {reason}."));
                    state_updates.push((
                        id.clone(),
                        ProcessState::Failed,
                        status.code(),
                        Some(failure),
                    ));
                    continue;
                }
                Ok(None) => has_running_process = true,
                Err(error) => {
                    running.finished = true;
                    state_updates.push((
                        id.clone(),
                        ProcessState::Failed,
                        None,
                        Some(ProcessFailure::WaitFailed(error.to_string())),
                    ));
                    continue;
                }
            }

            let Some(port) = assigned_ports.get(id).copied() else {
                // No known port: once the server listens, its port becomes the one the app
                // checks, opens and watches, as if it had been declared.
                if state == ProcessState::Running
                    && http_process_ids.contains(id)
                    && let Some(port) = probe_listening_port(running, now)
                {
                    discovered_ports.push((id.clone(), port));
                }
                continue;
            };
            let check_interval = if state == ProcessState::Starting {
                PROCESS_START_CHECK_INTERVAL
            } else {
                PROCESS_HEALTH_CHECK_INTERVAL
            };
            if now.duration_since(running.last_health_check) < check_interval {
                continue;
            }
            running.last_health_check = now;
            // The port only counts when this process (or a child) listens on it; otherwise the
            // app would mark the project as running and check another program's site.
            let listeners = listener_process_ids(port);
            let belongs = |process_id: &u32| {
                still_verified(running.verified_listener, *process_id, process_started_at)
                    || process_descends_from(*process_id, running.child.id())
            };
            let own_listener = listeners
                .iter()
                .copied()
                .find(|process_id| belongs(process_id));
            // Windows lets a server on 0.0.0.0 share the port with another program on
            // 127.0.0.1, and the browser and the check then reach that other program: a port
            // conflict, not the project's site. Listeners of the project itself on IPv4 and
            // IPv6 are not a conflict.
            let foreign_listener = listeners
                .iter()
                .copied()
                .find(|process_id| !belongs(process_id));
            let availability = match (own_listener, foreign_listener) {
                (Some(_), Some(process_id)) => PortAvailability::Occupied {
                    owner: Some(PortOwner {
                        process_id,
                        process_name: process_name(process_id),
                    }),
                },
                (Some(process_id), None) => PortAvailability::Occupied {
                    owner: Some(PortOwner {
                        process_id,
                        process_name: None,
                    }),
                },
                (None, _) => inspect_tcp_port(port),
            };
            let port_is_listening = match port_listener(
                availability,
                running.verified_listener,
                running.child.id(),
                process_descends_from,
                process_started_at,
            ) {
                PortListener::Project(process_id) => {
                    running.verified_listener = process_id.map(|process_id| VerifiedListener {
                        process_id,
                        started_at: process_started_at(process_id),
                    });
                    running.foreign_listener = None;
                    true
                }
                PortListener::Foreign(owner) => {
                    running.foreign_listener = Some(owner);
                    false
                }
                PortListener::None => false,
            };
            if state == ProcessState::Starting {
                if port_is_listening {
                    running.port_unavailable_since = None;
                    running.logs.push_back(format!(
                        "[LocalCodePilot] Serviço disponível na porta {}.",
                        port.value()
                    ));
                    ready_updates.push((id.clone(), port));
                } else if now.duration_since(running.started_at) >= PROCESS_START_TIMEOUT {
                    let failure = match &running.foreign_listener {
                        Some(owner) => ProcessFailure::PortTaken {
                            port,
                            owner: Some(owner.clone()),
                        },
                        None => ProcessFailure::NoResponse(port),
                    };
                    let reason = match &running.foreign_listener {
                        Some(owner) => {
                            // Technical details go to the log; the reason stays plain.
                            running.logs.push_back(format!(
                                "[LocalCodePilot] {}",
                                port_owner_detail(port, owner)
                            ));
                            format!(
                                "outro programa está usando a porta {}; este projeto não iniciou nessa porta",
                                port.value()
                            )
                        }
                        None => format!(
                            "a porta {} não respondeu dentro do tempo esperado",
                            port.value()
                        ),
                    };
                    running
                        .logs
                        .push_back(format!("[LocalCodePilot] Falha: {reason}."));
                    terminate_process(&mut running.child);
                    running.finished = true;
                    state_updates.push((id.clone(), ProcessState::Failed, None, Some(failure)));
                }
                continue;
            }

            if state == ProcessState::Running {
                if port_is_listening {
                    running.port_unavailable_since = None;
                    if http_process_ids.contains(id) {
                        let target = running
                            .urls
                            .iter()
                            .rev()
                            .find(|url| url.port() == port)
                            .or_else(|| running.urls.last())
                            .map(|url| url.address().to_owned())
                            .unwrap_or_else(|| format!("http://localhost:{}", port.value()));
                        schedule_http_health_check(running, target, now);
                    }
                } else {
                    let unavailable_since = running.port_unavailable_since.get_or_insert(now);
                    if now.duration_since(*unavailable_since) >= PROCESS_PORT_LOSS_GRACE {
                        // Another program holding the port is a different problem from a server
                        // that stopped answering, and the card says which one it was.
                        let failure = match &running.foreign_listener {
                            Some(owner) => {
                                running.logs.push_back(format!(
                                    "[LocalCodePilot] {}",
                                    port_owner_detail(port, owner)
                                ));
                                ProcessFailure::PortTaken {
                                    port,
                                    owner: Some(owner.clone()),
                                }
                            }
                            None => {
                                running.logs.push_back(format!(
                                    "[LocalCodePilot] Falha: a porta {} deixou de responder por {} segundos.",
                                    port.value(),
                                    PROCESS_PORT_LOSS_GRACE.as_secs()
                                ));
                                ProcessFailure::PortLost(port)
                            }
                        };
                        terminate_process(&mut running.child);
                        running.finished = true;
                        state_updates.push((id.clone(), ProcessState::Failed, None, Some(failure)));
                    }
                }
            }
        }
        for (id, port) in discovered_ports {
            if let Some(running) = self.managed.running.get_mut(&id) {
                running.logs.push_back(format!(
                    "[LocalCodePilot] O processo está ouvindo na porta {}.",
                    port.value()
                ));
                if !running.urls.iter().any(|url| url.port() == port) {
                    running.urls.extend(extract_local_urls(&format!(
                        "http://localhost:{}",
                        port.value()
                    )));
                }
            }
            self.managed.assigned_ports.insert(id.clone(), port);
            history_updates.push(id);
        }
        for (id, port) in ready_updates {
            let notification = self
                .managed
                .processes
                .iter_mut()
                .find(|process| process.id == id)
                .filter(|process| process.state == ProcessState::Starting)
                .map(|process| {
                    process.state = ProcessState::Running;
                    format!("{} disponível na porta {}", process.name, port.value())
                });
            if let Some(message) = notification {
                self.notify(message);
            }
            self.record_process_history(&id);
        }
        for (id, state, exit_code, failure) in state_updates {
            self.managed.assigned_ports.remove(&id);
            match &failure {
                Some(failure) => {
                    self.managed.failures.insert(id.clone(), failure.clone());
                }
                None => {
                    self.managed.failures.remove(&id);
                }
            }
            if let Some(process) = self
                .managed
                .processes
                .iter_mut()
                .find(|process| process.id == id)
            {
                process.state = state;
                process.process_id = None;
                process.exit_code = exit_code;
                let message = match (failure, exit_code) {
                    (Some(failure), _) => {
                        format!("{}: {}", process.project_name, failure.sentence())
                    }
                    (None, Some(code)) => {
                        format!("{} finalizado com código {code}", process.name)
                    }
                    (None, None) => format!("{} foi finalizado", process.name),
                };
                self.notify(message);
            }
            self.record_process_history(&id);
        }
        for id in history_updates {
            self.record_process_history(&id);
        }
        if has_running_process {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// The processes of the project whose page is open (T038.01, T038.04): one row per
    /// service with its state, port and actions. The page shows the project's header.
    fn processes_page(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let query = String::new();
        if !self.managed.processes.iter().any(|process| {
            self.focused_project
                .as_ref()
                .is_some_and(|path| process.project_path == *path)
        }) {
            return;
        }

        let mut projects = self.projects.clone();
        projects.sort_by_key(|project| {
            if self.focused_project.as_ref() == Some(&project.path) {
                0
            } else if !project_process_ids(&self.managed.processes, &project.path, true).is_empty()
            {
                1
            } else {
                2
            }
        });

        for project in &projects {
            if self
                .focused_project
                .as_ref()
                .is_some_and(|path| project.path != *path)
            {
                continue;
            }
            let project_matches = project.name.to_lowercase().contains(&query)
                || project
                    .path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&query)
                || project.display_stack().to_lowercase().contains(&query);
            let mut commands: Vec<_> = self
                .managed
                .processes
                .iter()
                .filter(|process| process.project_path == project.path)
                .filter(|process| {
                    project_matches || process_matches_search(process, Some(project), &query)
                })
                .cloned()
                .collect();
            commands.sort_by_key(|command| {
                if matches!(
                    command.state,
                    ProcessState::Running | ProcessState::Starting
                ) {
                    0
                } else {
                    1
                }
            });
            if commands.is_empty() {
                continue;
            }
            let environment = self
                .project_environments
                .get(&project.path)
                .cloned()
                .unwrap_or_default();
            let (fill, stroke, margin) = (Color32::TRANSPARENT, Stroke::NONE, 0);
            Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(12)
                .inner_margin(Margin::same(margin))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for command in commands {
                        let port_conflict = self.managed.port_conflicts.get(&command.id).cloned();
                        let assigned_port = self.managed.assigned_ports.get(&command.id).copied();
                        let process_history = self.workspace_state.service(&command.id).cloned();
                        let failure_suggestion = (command.state == ProcessState::Failed
                            && port_conflict.is_none())
                        .then(|| {
                            self.managed.running
                                .get(&command.id)
                                .map(|running| running.logs.iter().cloned().collect::<Vec<_>>())
                                .or_else(|| {
                                    process_history
                                        .as_ref()
                                        .map(|history| history.logs.clone())
                                })
                                .and_then(|logs| process_error_suggestion(&logs))
                        })
                        .flatten();
                        let server_url = if command.state == ProcessState::Running
                            && process_exposes_application_url(project, &command)
                        {
                            self.managed.running
                                .get(&command.id)
                                .and_then(|running| {
                                    assigned_port
                                        .and_then(|port| {
                                            running
                                                .urls
                                                .iter()
                                                .rev()
                                                .find(|url| url.port() == port)
                                        })
                                        .or_else(|| running.urls.last())
                                })
                                .cloned()
                                .or_else(|| {
                                    assigned_port.and_then(|port| {
                                        extract_local_urls(&format!(
                                            "http://localhost:{}",
                                            port.value()
                                        ))
                                        .pop()
                                    })
                                })
                        } else {
                            None
                        };
                        let (http_health, http_health_checking) = self
                            .managed
                            .running
                            .get(&command.id)
                            .map(|running| {
                                let health = server_url.as_ref().and_then(|server_url| {
                                    running
                                        .http_health
                                        .as_ref()
                                        .filter(|health| health.url == server_url.address())
                                        .cloned()
                                });
                                (health, running.http_health_receiver.is_some())
                            })
                            .unwrap_or((None, false));
                        let (state_label, state_color) = if port_conflict.is_some() {
                            ("Porta ocupada", Color32::from_rgb(235, 87, 87))
                        } else if matches!(
                            http_health.as_ref().map(|health| &health.status),
                            Some(HttpHealthStatus::Unreachable { .. })
                        ) {
                            ("HTTP sem resposta", Color32::from_rgb(235, 87, 87))
                        } else if matches!(
                            http_health.as_ref().map(|health| &health.status),
                            Some(HttpHealthStatus::Attention { .. })
                        ) {
                            ("HTTP com atenção", Color32::from_rgb(255, 205, 75))
                        } else if matches!(
                            http_health.as_ref().map(|health| &health.status),
                            Some(HttpHealthStatus::Healthy { .. })
                        ) {
                            ("Saudável", theme::SUCCESS)
                        } else if command.state == ProcessState::Running && server_url.is_some() {
                            ("HTTP não verificado", theme::PRIMARY)
                        } else {
                            process_state_display(command.state)
                        };
                        // One row per service (T037.02): Serviço · Estado · Porta, then the
                        // actions, always in the same place. A failure says why and what to do
                        // under the row; the rest is one click away in Detalhes.
                        let details_open = self.expanded_processes.contains(&command.id);
                        let failure_reason = (command.state == ProcessState::Failed)
                            .then(|| {
                                self.managed
                                    .failures
                                    .get(&command.id)
                                    .map(ProcessFailure::sentence)
                            })
                            .flatten();
                        let build_logs = if self.managed.running.contains_key(&command.id) {
                            None
                        } else {
                            self.managed
                                .build_logs
                                .get(&command.id)
                                .or_else(|| {
                                    self.managed.builds.get(&command.id).map(|job| &job.logs)
                                })
                                .filter(|logs| !logs.is_empty())
                        };
                        let has_logs = self.managed.running.contains_key(&command.id)
                            || process_history
                                .as_ref()
                                .is_some_and(|history| !history.logs.is_empty());
                        let port_text = assigned_port
                            .or(command.expected_port)
                            .map_or_else(|| "—".to_owned(), |port| port.value().to_string());
                        Frame::new()
                            .fill(theme::BACKGROUND)
                            .stroke(Stroke::new(1.0, theme::BORDER.gamma_multiply(0.6)))
                            .corner_radius(8)
                            .inner_margin(Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                // The actions stay on the row when they fit; in a narrow window
                                // they go to a second line, right-aligned, and the row keeps
                                // Serviço, Estado and Porta readable.
                                let narrow = ui.available_width() < 760.0;
                                let mut actions = |ui: &mut egui::Ui| {
                                        let details = if details_open {
                                            "Ocultar detalhes"
                                        } else {
                                            "Detalhes"
                                        };
                                        if ui.button(details).clicked() {
                                            action = Some(ProcessAction::ToggleDetails(
                                                command.id.clone(),
                                            ));
                                        }
                                        if has_logs
                                            && ui
                                                .button(RichText::new(format!(
                                                    "{TERMINAL}  Ver logs"
                                                )))
                                                .clicked()
                                        {
                                            action =
                                                Some(ProcessAction::ViewLogs(command.id.clone()));
                                        }
                                        match command.state {
                                            ProcessState::Running => {
                                                if ui.button("Reiniciar").clicked() {
                                                    action = Some(ProcessAction::Restart(
                                                        command.id.clone(),
                                                    ));
                                                }
                                                if ui
                                                    .add(
                                                        egui::Button::new("Parar")
                                                            .fill(theme::DANGER.gamma_multiply(0.18))
                                                            .stroke(Stroke::new(
                                                                1.0,
                                                                theme::DANGER.gamma_multiply(0.65),
                                                            )),
                                                    )
                                                    .clicked()
                                                {
                                                    action = Some(ProcessAction::Stop(
                                                        command.id.clone(),
                                                    ));
                                                }
                                            }
                                            ProcessState::Starting => {
                                                // The compilation can be interrupted (T028.02).
                                                if self.managed.builds.contains_key(&command.id)
                                                    && ui.button("Parar").clicked()
                                                {
                                                    action = Some(ProcessAction::Stop(
                                                        command.id.clone(),
                                                    ));
                                                }
                                                ui.add_enabled(
                                                    false,
                                                    egui::Button::new("Iniciando..."),
                                                );
                                            }
                                            ProcessState::Stopped
                                            | ProcessState::Completed
                                            | ProcessState::Failed => {
                                                let start = ui.add_enabled(
                                                    environment.is_ready(),
                                                    egui::Button::new(RichText::new(format!(
                                                        "{PLAY}  {}",
                                                        if port_conflict.is_some()
                                                            || command.state == ProcessState::Failed
                                                        {
                                                            "Tentar de novo"
                                                        } else {
                                                            "Iniciar"
                                                        }
                                                    )))
                                                    .fill(theme::PRIMARY.gamma_multiply(0.72)),
                                                );
                                                if start.clicked() {
                                                    action = Some(ProcessAction::Start(
                                                        command.id.clone(),
                                                    ));
                                                }
                                            }
                                        }
                                        if let Some(server_url) = &server_url
                                            && ui
                                                .button(RichText::new(format!(
                                                    "{ARROW_SQUARE_OUT}  Abrir site"
                                                )))
                                                .on_hover_text(server_url.address())
                                                .clicked()
                                        {
                                            action = Some(ProcessAction::OpenUrl(
                                                server_url.address().to_owned(),
                                            ));
                                        }
                                };
                                ui.horizontal(|ui| {
                                    let column = |width: f32| {
                                        (egui::vec2(width, 24.0), Layout::left_to_right(Align::Center))
                                    };
                                    let (size, layout) = column(160.0);
                                    ui.allocate_ui_with_layout(size, layout, |ui| {
                                        ui.set_min_width(size.x);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&command.name)
                                                    .strong()
                                                    .size(theme::TEXT_BODY),
                                            )
                                            .truncate(),
                                        );
                                    });
                                    let (size, layout) = column(130.0);
                                    ui.allocate_ui_with_layout(size, layout, |ui| {
                                        ui.set_min_width(size.x);
                                        runtime_badge(ui, state_label, state_color);
                                    });
                                    let (size, layout) = column(56.0);
                                    ui.allocate_ui_with_layout(size, layout, |ui| {
                                        ui.set_min_width(size.x);
                                        ui.label(
                                            RichText::new(&port_text)
                                                .monospace()
                                                .color(theme::MUTED)
                                                .size(theme::TEXT_DETAIL),
                                        )
                                        .on_hover_text("Porta");
                                    });
                                    if !narrow {
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            actions(ui);
                                        });
                                    }
                                });
                                if narrow {
                                    // A line one button tall: the layout would take all the height left.
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(ui.available_width(), 32.0),
                                        Layout::right_to_left(Align::Center),
                                        |ui| actions(ui),
                                    );
                                }
                                if let Some(conflict) = &port_conflict {
                                    ui.add_space(4.0);
                                    Frame::new()
                                        .fill(theme::DANGER.gamma_multiply(0.08))
                                        .stroke(Stroke::new(1.0, theme::DANGER.gamma_multiply(0.4)))
                                        .corner_radius(6)
                                        .inner_margin(Margin::same(8))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label(
                                                    RichText::new(port_conflict_details(conflict))
                                                        .color(Color32::from_rgb(245, 144, 144))
                                                        .size(theme::TEXT_DETAIL),
                                                );
                                                if conflict.owner.is_some()
                                                    && ui
                                                        .add(
                                                            egui::Button::new("Encerrar processo")
                                                                .small(),
                                                        )
                                                        .clicked()
                                                {
                                                    action =
                                                        Some(ProcessAction::ResolvePortConflict(
                                                            command.id.clone(),
                                                        ));
                                                }
                                            });
                                        });
                                } else if failure_reason.is_some() || failure_suggestion.is_some()
                                {
                                    ui.add_space(4.0);
                                    Frame::new()
                                        .fill(theme::WARNING.gamma_multiply(0.07))
                                        .stroke(Stroke::new(1.0, theme::WARNING.gamma_multiply(0.3)))
                                        .corner_radius(6)
                                        .inner_margin(Margin::same(8))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            if let Some(reason) = &failure_reason {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(reason)
                                                            .color(Color32::from_rgb(255, 215, 112))
                                                            .size(theme::TEXT_DETAIL),
                                                    )
                                                    .wrap(),
                                                );
                                            }
                                            if let Some(suggestion) = &failure_suggestion {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(format!(
                                                            "Sugestão: {suggestion}"
                                                        ))
                                                        .color(Color32::from_rgb(255, 215, 112))
                                                        .size(theme::TEXT_DETAIL),
                                                    )
                                                    .wrap(),
                                                );
                                            }
                                        });
                                }
                                // A failed compilation shows its output without a click: it is
                                // the reason the program did not start.
                                let show_build = build_logs.is_some()
                                    && (details_open || command.state == ProcessState::Failed);
                                if !details_open && !show_build {
                                    return;
                                }
                                ui.add_space(4.0);
                                if details_open {
                                    if let Some(server_url) = &server_url {
                                        let (message, color) = match &http_health {
                                            Some(health) => http_health_display(&health.status),
                                            None if http_health_checking => (
                                                "Verificando resposta HTTP...".to_owned(),
                                                theme::MUTED,
                                            ),
                                            None => (
                                                "Aguardando verificação HTTP...".to_owned(),
                                                theme::MUTED,
                                            ),
                                        };
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {message}",
                                                server_url.address()
                                            ))
                                            .color(color)
                                            .size(theme::TEXT_DETAIL),
                                        );
                                    }
                                    if let (Some(preferred), Some(selected)) =
                                        (command.expected_port, assigned_port)
                                        && preferred != selected
                                    {
                                        ui.label(
                                            RichText::new(format!(
                                                "Porta {} · ajustada automaticamente (preferida: {})",
                                                selected.value(),
                                                preferred.value()
                                            ))
                                            .color(theme::SUCCESS)
                                            .size(theme::TEXT_DETAIL),
                                        );
                                    } else if server_url.is_none()
                                        && port_conflict.is_none()
                                        && let Some(port) = command.expected_port
                                    {
                                        ui.label(
                                            RichText::new(format!(
                                                "Porta prevista: {} · verificada antes de iniciar",
                                                port.value()
                                            ))
                                            .color(theme::MUTED)
                                            .size(theme::TEXT_DETAIL),
                                        );
                                    }
                                    if !matches!(
                                        command.state,
                                        ProcessState::Running | ProcessState::Starting
                                    ) && let Some(history) = &process_history
                                        && history.updated_at > 0
                                    {
                                        ui.label(
                                            RichText::new(format!(
                                                "Última execução: {} · {}{}",
                                                relative_time(history.updated_at),
                                                service_history_status(&history.status),
                                                history
                                                    .port
                                                    .map(|port| format!(" · porta {port}"))
                                                    .unwrap_or_default()
                                            ))
                                            .color(theme::MUTED)
                                            .size(theme::TEXT_DETAIL),
                                        );
                                    }
                                    ui.label(
                                        RichText::new(command.command_line())
                                            .color(theme::MUTED)
                                            .monospace()
                                            .size(theme::TEXT_DETAIL),
                                    );
                                    ui.horizontal_wrapped(|ui| {
                                        if command.working_directory != command.project_path
                                            && let Ok(relative) = command
                                                .working_directory
                                                .strip_prefix(&command.project_path)
                                        {
                                            ui.label(
                                                RichText::new(format!(
                                                    "Pasta: {}",
                                                    relative.display()
                                                ))
                                                .color(theme::MUTED)
                                                .size(theme::TEXT_DETAIL),
                                            );
                                        }
                                        if let Some(process_id) = command.process_id {
                                            ui.label(
                                                RichText::new(format!("PID {process_id}"))
                                                    .color(theme::MUTED)
                                                    .monospace()
                                                    .size(theme::TEXT_DETAIL),
                                            );
                                        }
                                        if let Some(exit_code) = command.exit_code {
                                            ui.label(
                                                RichText::new(format!(
                                                    "Código de saída: {exit_code}"
                                                ))
                                                .color(theme::MUTED)
                                                .monospace()
                                                .size(theme::TEXT_DETAIL),
                                            );
                                        }
                                    });
                                }
                                if show_build && let Some(logs) = build_logs {
                                    egui::CollapsingHeader::new(format!("Saída ({})", logs.len()))
                                        .id_salt(format!("build-logs-{}", command.id))
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            egui::ScrollArea::vertical()
                                                .max_height(160.0)
                                                .stick_to_bottom(true)
                                                .show(ui, |ui| {
                                                    for line in logs {
                                                        ui.label(
                                                            RichText::new(line)
                                                                .monospace()
                                                                .size(theme::TEXT_DETAIL),
                                                        );
                                                    }
                                                });
                                        });
                                }
                                if details_open
                                    && let Some(running) = self.managed.running.get(&command.id)
                                    && !running.logs.is_empty()
                                {
                                    egui::CollapsingHeader::new(format!(
                                        "Saída ({})",
                                        running.logs.len()
                                    ))
                                    .id_salt(format!("logs-{}", command.id))
                                    .show(ui, |ui| {
                                        if ui
                                            .add(egui::Button::new("Limpar saída").small())
                                            .clicked()
                                        {
                                            action =
                                                Some(ProcessAction::ClearLogs(command.id.clone()));
                                        }
                                        egui::ScrollArea::vertical()
                                            .max_height(160.0)
                                            .stick_to_bottom(true)
                                            .show(ui, |ui| {
                                                for line in &running.logs {
                                                    ui.label(
                                                        RichText::new(line)
                                                            .color(theme::MUTED)
                                                            .monospace()
                                                            .size(theme::TEXT_DETAIL),
                                                    );
                                                }
                                            });
                                    });
                                }
                            });
                        ui.add_space(4.0);
                    }
                });
            ui.add_space(12.0);
        }
        match action {
            Some(ProcessAction::Start(id)) => self.start_process(&id),
            Some(ProcessAction::Stop(id)) => self.stop_process(&id),
            Some(ProcessAction::Restart(id)) => self.restart_process(&id),
            Some(ProcessAction::ClearLogs(id)) => self.clear_process_logs(&id),
            Some(ProcessAction::ToggleDetails(id)) => {
                if !self.expanded_processes.remove(&id) {
                    self.expanded_processes.insert(id);
                }
            }
            Some(ProcessAction::ViewLogs(id)) => self.managed.terminal = Some(id),
            Some(ProcessAction::OpenUrl(url)) => {
                ui.ctx().open_url(egui::OpenUrl::new_tab(&url));
                self.notify(format!("Abrindo {url}"));
            }
            Some(ProcessAction::ResolvePortConflict(id)) => {
                self.request_port_owner_termination(&id);
            }
            None => {}
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        self.config_recovery_section(ui);
        let available_update = self.update_session.available.clone();
        let automatic_update = available_update
            .as_ref()
            .is_some_and(updates::automatic_update_available);
        let mut start_install = false;
        let mut retry_check = false;
        let mut open_news = None;
        let refused = self.update_session.refused_for_safety();
        let updating = self.update_session.in_progress();
        let progress = self.update_session.progress();
        ui.heading("Atualizações");
        ui.add_space(12.0);
        Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // The installed version and what it does on the left, the actions on the
                // right (T037.05).
                let explanation = "A consulta automática acontece uma vez por dia ao abrir o app. Você pode verificar agora a qualquer momento.";
                let version = format!("LocalCodePilot {}", updates::current_version());
                components::settings_row(ui, &version, explanation, |ui| {
                    if ui.add_enabled(self.update_session.check.is_none() && !updating, egui::Button::new("Verificar se há atualizações")).clicked() {
                        retry_check = true;
                    }
                    // The news of this version, again (T035).
                    let news = ui.button("Ver novidades");
                    if news.clicked() {
                        open_news = Some(news.id);
                    }
                    if automatic_update
                        && !refused
                        && ui
                            .add_enabled(
                                !updating,
                                egui::Button::new("Atualizar agora")
                                    .fill(theme::PRIMARY.gamma_multiply(0.72)),
                            )
                            .clicked()
                    {
                        start_install = true;
                    }
                    if let Some(update) = &available_update
                        && !automatic_update
                        && ui
                            .add(
                                egui::Button::new("Baixar atualização")
                                    .fill(theme::PRIMARY.gamma_multiply(0.72)),
                            )
                            .clicked()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(
                            updates::manual_download_url(update),
                        ));
                    }
                });
                if let Some(update) = &available_update {
                    if !automatic_update {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(updates::manual_download_note(update))
                                .color(theme::MUTED)
                                .size(11.0),
                        );
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(update_versions_line(&update.version))
                            .color(theme::MUTED)
                            .size(11.0),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(updates::summary_line(update)).size(11.0))
                            .wrap(),
                    );
                }
                ui.add_space(6.0);
                if self.update_session.check.is_some() {
                    ui.horizontal(|ui| { ui.spinner(); ui.label("Verificando atualizações..."); });
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                } else if let Some(result) = &self.update_session.check_result {
                    match result {
                        Ok(message) => {
                            ui.label(RichText::new(message).color(theme::SUCCESS).size(11.0));
                        }
                        Err(failure) => {
                            if update_failure_panel(ui, failure, None, "check") {
                                retry_check = true;
                            }
                        }
                    }
                }
                if updating {
                    update_progress_ui::show(ui, &progress);
                } else if let Some(error) = &self.update_session.install_error {
                    let release_url = available_update.as_ref().map(|update| update.release_url.as_str());
                    if update_failure_panel(ui, error, release_url, "settings") {
                        start_install = true;
                    }
                }
                if self.workspace_state.last_update_check > 0 {
                    ui.label(RichText::new(format!("Última consulta concluída: {}", relative_time(self.workspace_state.last_update_check))).color(theme::MUTED).size(10.0));
                }
            });
        if let Some(origin) = open_news {
            self.open_whats_new(Some(origin));
        }
        if retry_check && self.update_session.check.is_none() && !updating {
            self.update_session.manual_check = true;
            self.workspace_state.update_remind_after = 0;
            self.workspace_dirty = true;
            self.update_session.check_result = None;
            self.update_session.check = Some(updates::spawn_update_check(ui.ctx().clone()));
        }
        if start_install && let Some(update) = available_update {
            self.request_update_install(&update, ui.ctx().clone());
        }
        // The languages moved to Ambiente (T037.05).
        ui.add_space(24.0);
        ui.heading("Logs e histórico");
        ui.add_space(8.0);
        let clear = components::settings_row(
            ui,
            "Logs salvos",
            "Remove os logs de serviços e de preparação salvos neste computador. Não interrompe projetos em execução.",
            |ui| ui.button("Limpar histórico").clicked(),
        );
        if clear {
            self.confirm_clear_history = true;
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new("Histórico de preparação")
                .strong()
                .size(theme::TEXT_BODY),
        );
        ui.add_space(4.0);
        if self.workspace_state.preparations.is_empty() {
            ui.label(
                RichText::new("Nenhuma preparação foi executada nesta máquina.")
                    .color(theme::MUTED),
            );
        } else {
            for preparation in self.workspace_state.preparations.iter().take(10) {
                let project_name = preparation
                    .project_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Projeto");
                let status_color = match preparation.status.as_str() {
                    "concluído" => theme::SUCCESS,
                    "falhou" => Color32::from_rgb(255, 112, 112),
                    _ => theme::PRIMARY,
                };
                Frame::new()
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(project_name).strong());
                            ui.label(
                                RichText::new(&preparation.status)
                                    .color(status_color)
                                    .size(10.0),
                            );
                        });
                        ui.label(
                            RichText::new(format!(
                                "{} etapa(s) · {} linha(s) de log",
                                preparation.steps.len(),
                                preparation.logs.len()
                            ))
                            .color(theme::MUTED)
                            .size(10.0),
                        );
                        egui::CollapsingHeader::new("Etapas e logs")
                            .id_salt((
                                &preparation.project_path,
                                preparation.updated_at,
                                &preparation.status,
                            ))
                            .show(ui, |ui| {
                                for (index, step) in preparation.steps.iter().enumerate() {
                                    ui.label(
                                        RichText::new(format!("{}. {step}", index + 1))
                                            .monospace()
                                            .size(9.0),
                                    );
                                }
                                if !preparation.logs.is_empty() {
                                    ui.separator();
                                    for line in &preparation.logs {
                                        ui.label(
                                            RichText::new(line)
                                                .color(theme::MUTED)
                                                .monospace()
                                                .size(9.0),
                                        );
                                    }
                                }
                            });
                    });
                ui.add_space(6.0);
            }
        }
    }

    fn content(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        let margin = if root_ui.available_width() < 720.0 {
            16
        } else {
            36
        };
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(Margin::same(margin)),
            )
            .show(root_ui, |ui| {
                self.show_update_banner(ui);
                self.config_recovery_banner(ui);
                egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                    Page::Projects => self.projects_page(&ctx, ui),
                    Page::Project => self.project_page(ui),
                    Page::Settings => self.settings_page(ui),
                    Page::Environment => self.environment_page(ui),
                    Page::Help => self.help_page(ui),
                });
            });
        // Cards and Containers both ask to confirm container actions.
        self.container_confirmation(&ctx);
        self.command_dialog(&ctx);
        self.migration_log_window(&ctx);
    }
}

impl eframe::App for LocalCodePilot {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_discovery(ctx);
        self.poll_environment_plan();
        self.poll_dependency_installs(ctx);
        self.poll_laravel_migration(ctx);
        self.poll_processes(ctx);
        self.poll_builds(ctx);
        self.poll_update_check();
        self.poll_update_install(ctx);
        self.poll_migrations();
        self.poll_containers(ctx);
        self.poll_engine_start(ctx);
        self.poll_languages(ctx);
        self.poll_database_jobs(ctx);
        self.persist_workspace_state(false);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.sidebar(ui);
        self.topbar(ui);
        self.content(ui);
        self.show_terminal(ui);
        self.show_environment_plan_confirmation(ui);
        self.show_whats_new(ui);
        self.show_command_confirmation(ui);
        self.show_dependency_terminal(ui);
        self.show_laravel_migration(ui);
        self.show_port_termination_confirmation(ui);
        self.show_clear_history_confirmation(ui);
        self.show_update_confirmation(ui);
        self.engine_start_confirmation(ui);
        self.migration_confirmation(ui);
        self.show_database_confirmation(ui);
        self.show_notification(ui);
    }
}

impl Drop for LocalCodePilot {
    fn drop(&mut self) {
        let active_ids: Vec<_> = self
            .managed
            .processes
            .iter()
            .filter(|process| {
                matches!(
                    process.state,
                    ProcessState::Starting | ProcessState::Running
                )
            })
            .map(|process| process.id.clone())
            .collect();
        for id in &active_ids {
            if let Some(process) = self
                .managed
                .processes
                .iter_mut()
                .find(|process| process.id == *id)
            {
                process.state = ProcessState::Stopped;
            }
            self.record_process_history(id);
        }
        self.persist_workspace_state(true);
        for running in self.managed.running.values_mut() {
            if !running.finished {
                terminate_process(&mut running.child);
            }
        }
        // After the projects: their database servers, with the servers' own shutdown.
        self.stop_session_databases();
    }
}

fn spawn_discovery(
    source: FilesystemProjectSource,
    environment_path: Option<OsString>,
    custom_commands: Vec<CustomCommand>,
    repaint: egui::Context,
) -> Receiver<DiscoveryEvent> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        // Debug builds only: lets the isolated test instance show a failed search (T036).
        if cfg!(debug_assertions) && std::env::var_os("LCP_TESTE_FALHA_VARREDURA").is_some() {
            let _ = sender.send(DiscoveryEvent::Failed(
                "a pasta de teste não pôde ser lida".into(),
            ));
            repaint.request_repaint();
            return;
        }
        let service = DiscoveryService::new(source, ManifestRuntimeDetector);
        let catalog = match service.discover() {
            Ok(catalog) => catalog,
            Err(error) => {
                let _ = sender.send(DiscoveryEvent::Failed(error.to_string()));
                repaint.request_repaint();
                return;
            }
        };
        let projects = catalog.into_projects();
        if sender
            .send(DiscoveryEvent::Projects(projects.clone()))
            .is_err()
        {
            return;
        }
        repaint.request_repaint();
        let processes = projects
            .iter()
            .flat_map(|project| project_processes(project, &custom_commands))
            .collect::<Vec<_>>();
        if sender
            .send(DiscoveryEvent::Processes(processes.clone()))
            .is_err()
        {
            return;
        }
        repaint.request_repaint();
        let runtime_inventory = inspect_runtime_inventory(environment_path.as_deref());
        let environments = projects
            .iter()
            .map(|project| {
                (
                    project.path.clone(),
                    inspect_project_environment_with_inventory(
                        project,
                        &processes,
                        environment_path.as_deref(),
                        &runtime_inventory,
                    ),
                )
            })
            .collect();
        let _ = sender.send(DiscoveryEvent::Environments(environments));
        repaint.request_repaint();
    });
    receiver
}

/// Detected commands of a project, then those the person defined for it.
fn project_processes(project: &Project, custom_commands: &[CustomCommand]) -> Vec<ProjectProcess> {
    let mut processes = detect_processes(project);
    processes.extend(custom_commands::processes_for(project, custom_commands));
    processes
}

fn load_logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../../assets/branding/icon-512.png"))
            .expect("the embedded LocalCodePilot logo must be a valid PNG");
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    );
    ctx.load_texture("localcodepilot-logo", image, egui::TextureOptions::LINEAR)
}

fn composer_security_advisory(details: &str) -> bool {
    let details = details.to_ascii_lowercase();
    details.contains("affected by security advisories")
        || details.contains("block-insecure")
        || (details.contains("security advisories") && details.contains("pksa-"))
}

fn create_laravel_migration(
    project_path: &Path,
    project_name: String,
) -> Result<LaravelMigration, String> {
    let composer_path = project_path.join("composer.json");
    let contents = fs::read_to_string(&composer_path)
        .map_err(|error| format!("Não foi possível ler {}: {error}", composer_path.display()))?;
    let document: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|error| format!("O composer.json é inválido: {error}"))?;
    let current_constraint = document
        .get("require")
        .and_then(|require| require.get("laravel/framework"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "laravel/framework não foi encontrado no composer.json".to_owned())?
        .to_owned();
    let (php_version, php_major, php_minor) = installed_php_version()?;
    let (target_major, phase) = if php_major > 8 || (php_major == 8 && php_minor >= 3) {
        (13, LaravelMigrationPhase::Confirm)
    } else if php_major == 8 && php_minor >= 2 {
        (12, LaravelMigrationPhase::Confirm)
    } else {
        (12, LaravelMigrationPhase::Unavailable)
    };

    Ok(LaravelMigration {
        project_path: project_path.to_path_buf(),
        project_name,
        current_constraint,
        target_constraint: format!("^{target_major}.0"),
        php_version,
        phase,
        receiver: None,
        logs: VecDeque::new(),
        preview: None,
        backup_path: None,
        error: None,
    })
}

fn installed_php_version() -> Result<(String, u64, u64), String> {
    let mut command = Command::new("php");
    configure_background_command(&mut command);
    let output = command
        .args([
            "-r",
            "echo PHP_MAJOR_VERSION.'.'.PHP_MINOR_VERSION.'.'.PHP_RELEASE_VERSION;",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("Não foi possível consultar a versão do PHP: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Não foi possível consultar a versão do PHP: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let mut parts = version.split('.');
    let major = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| format!("Versão do PHP não reconhecida: {version}"))?;
    let minor = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| format!("Versão do PHP não reconhecida: {version}"))?;
    Ok((version, major, minor))
}

fn prepare_laravel_migration_files(
    composer_json: &str,
    composer_lock: Option<Vec<u8>>,
    target_constraint: &str,
) -> Result<LaravelMigrationFiles, String> {
    let mut document: serde_json::Value = serde_json::from_str(composer_json)
        .map_err(|error| format!("O composer.json é inválido: {error}"))?;
    let require = document
        .get_mut("require")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "A seção require não foi encontrada no composer.json".to_owned())?;
    let framework = require
        .get_mut("laravel/framework")
        .ok_or_else(|| "laravel/framework não foi encontrado no composer.json".to_owned())?;
    *framework = serde_json::Value::String(target_constraint.to_owned());
    let mut composer_json = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("Não foi possível preparar o composer.json: {error}"))?;
    composer_json.push('\n');
    Ok(LaravelMigrationFiles {
        composer_json,
        composer_lock,
    })
}

fn preview_laravel_migration(
    project_path: &Path,
    target_constraint: &str,
    sender: &mpsc::Sender<LaravelMigrationEvent>,
) -> Result<LaravelMigrationFiles, String> {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let preview_directory = std::env::temp_dir()
        .join("LocalCodePilot")
        .join(format!("laravel-preview-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&preview_directory)
        .map_err(|error| format!("Não foi possível criar a cópia temporária: {error}"))?;

    let result = (|| {
        let composer_json = fs::read_to_string(project_path.join("composer.json"))
            .map_err(|error| format!("Não foi possível ler composer.json: {error}"))?;
        let composer_lock = fs::read(project_path.join("composer.lock")).ok();
        let files =
            prepare_laravel_migration_files(&composer_json, composer_lock, target_constraint)?;
        fs::write(
            preview_directory.join("composer.json"),
            &files.composer_json,
        )
        .map_err(|error| format!("Não foi possível preparar composer.json: {error}"))?;
        if let Some(lock) = &files.composer_lock {
            fs::write(preview_directory.join("composer.lock"), lock)
                .map_err(|error| format!("Não foi possível copiar composer.lock: {error}"))?;
        }
        let _ = sender.send(LaravelMigrationEvent::Log(format!(
            "Simulando laravel/framework {target_constraint} em {}",
            preview_directory.display()
        )));
        run_laravel_migration_command(
            &preview_directory,
            &[
                "update",
                "laravel/framework",
                "--with-all-dependencies",
                "--no-install",
                "--no-plugins",
                "--no-scripts",
                "--no-interaction",
            ],
            sender,
        )?;
        let composer_json = fs::read_to_string(preview_directory.join("composer.json"))
            .map_err(|error| format!("Não foi possível ler o resultado da simulação: {error}"))?;
        let composer_lock = fs::read(preview_directory.join("composer.lock"))
            .map_err(|error| format!("O Composer não gerou composer.lock: {error}"))?;
        Ok(LaravelMigrationFiles {
            composer_json,
            composer_lock: Some(composer_lock),
        })
    })();

    if preview_directory.starts_with(std::env::temp_dir()) {
        let _ = fs::remove_dir_all(&preview_directory);
    }
    result
}

fn apply_laravel_migration_files(
    project_path: &Path,
    files: &LaravelMigrationFiles,
    sender: &mpsc::Sender<LaravelMigrationEvent>,
) -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let backup_path = std::env::temp_dir()
        .join("LocalCodePilot")
        .join("backups")
        .join(format!("{}-{nonce}", std::process::id()));
    fs::create_dir_all(&backup_path)
        .map_err(|error| format!("Não foi possível criar o backup: {error}"))?;
    let composer_json_path = project_path.join("composer.json");
    let composer_lock_path = project_path.join("composer.lock");
    fs::copy(&composer_json_path, backup_path.join("composer.json"))
        .map_err(|error| format!("Não foi possível salvar o backup de composer.json: {error}"))?;
    let had_lock = composer_lock_path.is_file();
    if had_lock {
        fs::copy(&composer_lock_path, backup_path.join("composer.lock")).map_err(|error| {
            format!("Não foi possível salvar o backup de composer.lock: {error}")
        })?;
    }

    let apply_result = (|| {
        fs::write(&composer_json_path, &files.composer_json)
            .map_err(|error| format!("Não foi possível atualizar composer.json: {error}"))?;
        if let Some(lock) = &files.composer_lock {
            fs::write(&composer_lock_path, lock)
                .map_err(|error| format!("Não foi possível atualizar composer.lock: {error}"))?;
        }
        let _ = sender.send(LaravelMigrationEvent::Log(
            "Manifests aplicados. Instalando dependências sem executar scripts...".into(),
        ));
        run_laravel_migration_command(
            project_path,
            &["install", "--no-scripts", "--no-interaction"],
            sender,
        )
    })();

    if let Err(error) = apply_result {
        let _ = fs::copy(backup_path.join("composer.json"), &composer_json_path);
        if had_lock {
            let _ = fs::copy(backup_path.join("composer.lock"), &composer_lock_path);
        } else if composer_lock_path.is_file() {
            let _ = fs::remove_file(&composer_lock_path);
        }
        return Err(format!(
            "{error}. composer.json e composer.lock foram restaurados. Backup: {}",
            backup_path.display()
        ));
    }

    Ok(backup_path)
}

fn run_laravel_migration_command(
    working_directory: &Path,
    args: &[&str],
    sender: &mpsc::Sender<LaravelMigrationEvent>,
) -> Result<(), String> {
    let _ = sender.send(LaravelMigrationEvent::Log(format!(
        "$ composer {}",
        args.join(" ")
    )));
    let process = ProjectProcess {
        id: "laravel-migration".into(),
        project_name: String::new(),
        project_path: working_directory.to_path_buf(),
        working_directory: working_directory.to_path_buf(),
        name: "Migração do Laravel".into(),
        program: "composer".into(),
        args: args.iter().map(|argument| (*argument).to_owned()).collect(),
        state: ProcessState::Starting,
        process_id: None,
        exit_code: None,
        expected_port: None,
        port_override: None,
        prepare: Vec::new(),
    };
    let mut child = process_command(&process)
        .current_dir(plain_path(working_directory))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Não foi possível executar o Composer: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .map(|stdout| spawn_laravel_migration_reader(stdout, sender.clone()));
    let stderr = child
        .stderr
        .take()
        .map(|stderr| spawn_laravel_migration_reader(stderr, sender.clone()));
    let status = child
        .wait()
        .map_err(|error| format!("Não foi possível aguardar o Composer: {error}"))?;
    if let Some(reader) = stdout {
        let _ = reader.join();
    }
    if let Some(reader) = stderr {
        let _ = reader.join();
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Composer encerrou com código {}",
            status
                .code()
                .map_or_else(|| "desconhecido".into(), |code| code.to_string())
        ))
    }
}

fn spawn_laravel_migration_reader(
    reader: impl Read + Send + 'static,
    sender: mpsc::Sender<LaravelMigrationEvent>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            let line = strip_terminal_sequences(&line);
            if sender.send(LaravelMigrationEvent::Log(line)).is_err() {
                break;
            }
        }
    })
}

fn resolve_environment_plan(
    project_path: &Path,
    environment: &ProjectEnvironment,
) -> Result<InstallationPlan, String> {
    resolve_environment_plan_with_tools_directory(
        project_path,
        environment,
        managed_bin_directory(),
    )
}

fn resolve_environment_plan_with_tools_directory(
    project_path: &Path,
    environment: &ProjectEnvironment,
    tools_directory: Option<PathBuf>,
) -> Result<InstallationPlan, String> {
    let mut steps = Vec::new();
    if environment.missing_compiler {
        steps.extend(compiler_steps(project_path));
    }
    let node_constraints = environment
        .runtime_requirements
        .iter()
        .filter(|requirement| requirement.runtime == RuntimeKind::Node)
        .map(|requirement| requirement.version.value.clone())
        .collect::<Vec<_>>();
    let needs_node = environment.runtime_requirements.iter().any(|requirement| {
        requirement.runtime == RuntimeKind::Node
            && matches!(
                requirement.status,
                RequirementStatus::Missing | RequirementStatus::Incompatible { .. }
            )
    });

    if needs_node {
        let constraint_label = node_constraints.join(" e ");
        let provider = WindowsWingetProvider;
        let install_step =
            provider.resolve_node_installation(
                &constraint_label,
                project_path,
                |_, candidate| {
                    Some(node_constraints.iter().all(|constraint| {
                        node_version_matches(constraint, candidate) != Some(false)
                    }))
                },
            )?;
        let selected_version = command_argument_after(&install_step, "--version")
            .ok_or_else(|| "O plano do Node.js não informou a versão selecionada".to_owned())?;
        steps.push(install_step);
        steps.push(InstallationStep {
            label: "Atualizar o ambiente do LocalCodePilot".into(),
            detail: "Recarregar o PATH de usuário e de sistema".into(),
            source: Some("Windows".into()),
            requires_elevation: false,
            kind: InstallationStepKind::RefreshEnvironment,
        });
        steps.push(InstallationStep {
            label: "Validar Node.js".into(),
            detail: format!("Confirmar a versão instalada ({selected_version})"),
            source: None,
            requires_elevation: false,
            kind: InstallationStepKind::VerifyCommand {
                program: "node".into(),
                args: vec!["--version".into()],
                version_constraint: Some(selected_version),
            },
        });
        steps.push(InstallationStep {
            label: "Validar npm".into(),
            detail: "Confirmar o gerenciador de pacotes incluído no Node.js".into(),
            source: None,
            requires_elevation: false,
            kind: InstallationStepKind::VerifyCommand {
                program: "npm".into(),
                args: vec!["--version".into()],
                version_constraint: None,
            },
        });
    }

    for runtime in [
        RuntimeKind::Python,
        RuntimeKind::Java,
        RuntimeKind::Php,
        RuntimeKind::Go,
        RuntimeKind::Dart,
        RuntimeKind::Clojure,
        RuntimeKind::Haskell,
        RuntimeKind::Zig,
    ] {
        let requirements = environment
            .runtime_requirements
            .iter()
            .filter(|requirement| requirement.runtime == runtime)
            .collect::<Vec<_>>();
        if !requirements.iter().any(|requirement| {
            matches!(
                requirement.status,
                RequirementStatus::Missing | RequirementStatus::Incompatible { .. }
            )
        }) {
            continue;
        }
        let constraints = requirements
            .iter()
            .map(|requirement| requirement.version.value.clone())
            .collect::<Vec<_>>();
        let constraint_label = constraints.join(" e ");
        let provider = WindowsWingetProvider;
        let matches_all =
            |_: &str, candidate: &str| {
                Some(constraints.iter().all(|constraint| {
                    runtime_version_matches(constraint, candidate) != Some(false)
                }))
            };
        let install_step = match runtime {
            RuntimeKind::Python => provider.resolve_python_installation(
                &constraint_label,
                project_path,
                matches_all,
            )?,
            RuntimeKind::Java => {
                provider.resolve_java_installation(&constraint_label, project_path, matches_all)?
            }
            RuntimeKind::Php => {
                provider.resolve_php_installation(&constraint_label, project_path, matches_all)?
            }
            RuntimeKind::Go => {
                provider.resolve_go_installation(&constraint_label, project_path, matches_all)?
            }
            RuntimeKind::Dart => {
                provider.resolve_dart_installation(&constraint_label, project_path, matches_all)?
            }
            RuntimeKind::Clojure => provider.resolve_babashka_installation(
                &constraint_label,
                project_path,
                matches_all,
            )?,
            RuntimeKind::Haskell => {
                provider.resolve_stack_installation(&constraint_label, project_path, matches_all)?
            }
            RuntimeKind::Zig => {
                provider.resolve_zig_installation(&constraint_label, project_path, matches_all)?
            }
            // Not installed by the app: Rust, Node and Flutter have their own installers, and
            // the other languages are only recognized for now.
            _ => continue,
        };
        let selected_version = command_argument_after(&install_step, "--version")
            .ok_or_else(|| format!("O plano de {runtime} não informou a versão selecionada"))?;
        steps.push(install_step);
        steps.push(InstallationStep {
            label: "Atualizar o ambiente do LocalCodePilot".into(),
            detail: format!("Recarregar o PATH depois da instalação de {runtime}"),
            source: Some("Windows".into()),
            requires_elevation: false,
            kind: InstallationStepKind::RefreshEnvironment,
        });
        let (program, args) = match runtime {
            RuntimeKind::Python => ("python", vec!["--version".into()]),
            RuntimeKind::Java => ("java", vec!["-version".into()]),
            RuntimeKind::Php => (
                "php",
                vec!["-r".into(), "echo PHP_VERSION, PHP_EOL;".into()],
            ),
            RuntimeKind::Go => ("go", vec!["version".into()]),
            RuntimeKind::Dart => ("dart", vec!["--version".into()]),
            RuntimeKind::Clojure => ("bb", vec!["--version".into()]),
            RuntimeKind::Haskell => ("stack", vec!["--version".into()]),
            RuntimeKind::Zig => ("zig", vec!["version".into()]),
            _ => continue,
        };
        steps.push(InstallationStep {
            label: format!("Validar {runtime}"),
            detail: format!("Confirmar a versão instalada ({selected_version})"),
            source: None,
            requires_elevation: false,
            kind: InstallationStepKind::VerifyCommand {
                program: program.into(),
                args,
                version_constraint: Some(constraint_label.clone()),
            },
        });
        if runtime == RuntimeKind::Java {
            steps.push(InstallationStep {
                label: "Validar compilador Java".into(),
                detail: "Confirmar o javac fornecido pelo JDK".into(),
                source: None,
                requires_elevation: false,
                kind: InstallationStepKind::VerifyCommand {
                    program: "javac".into(),
                    args: vec!["-version".into()],
                    version_constraint: Some(constraint_label),
                },
            });
        }
    }

    // CMake or Meson for a C/C++ project that compiles before it runs (T028.02).
    for program in environment
        .missing_programs
        .iter()
        .filter(|program| is_build_tool(program))
    {
        steps.push(WindowsWingetProvider.resolve_build_tool_installation(program, project_path)?);
        steps.push(InstallationStep {
            label: "Atualizar o ambiente do LocalCodePilot".into(),
            detail: format!("Recarregar o PATH depois da instalação de {program}"),
            source: Some("Windows".into()),
            requires_elevation: false,
            kind: InstallationStepKind::RefreshEnvironment,
        });
        steps.push(InstallationStep {
            label: format!("Validar {program}"),
            detail: "Confirmar que o programa responde".into(),
            source: None,
            requires_elevation: false,
            kind: InstallationStepKind::VerifyCommand {
                program: program.clone(),
                args: vec!["--version".into()],
                version_constraint: None,
            },
        });
    }

    if environment
        .missing_programs
        .iter()
        .any(|program| program == "composer")
    {
        let install_directory = tools_directory.clone().ok_or_else(|| {
            "Não foi possível localizar a pasta local de ferramentas do usuário".to_owned()
        })?;
        steps.push(InstallationStep {
            label: "Instalar Composer".into(),
            detail: "Validar o SHA-384 do instalador oficial antes de executá-lo".into(),
            source: Some("getcomposer.org".into()),
            requires_elevation: false,
            kind: InstallationStepKind::InstallComposer { install_directory },
        });
        steps.push(InstallationStep {
            label: "Atualizar o ambiente do LocalCodePilot".into(),
            detail: "Adicionar a pasta gerenciada de ferramentas ao ambiente dos processos".into(),
            source: Some("LocalCodePilot".into()),
            requires_elevation: false,
            kind: InstallationStepKind::RefreshEnvironment,
        });
        steps.push(InstallationStep {
            label: "Validar Composer".into(),
            detail: "Confirmar a instalação do gerenciador de dependências PHP".into(),
            source: None,
            requires_elevation: false,
            kind: InstallationStepKind::VerifyCommand {
                program: "composer".into(),
                args: vec!["--version".into()],
                version_constraint: None,
            },
        });
    }

    for extension in &environment.missing_php_extensions {
        if !supported_php_extension(extension) {
            return Err(format!(
                "A extensão PHP ext-{extension} requer instalação manual porque ainda não está na lista controlada do LocalCodePilot"
            ));
        }
        steps.push(InstallationStep {
            label: format!("Habilitar extensão PHP {extension}"),
            detail: "Criar backup do php.ini e habilitar somente a extensão declarada".into(),
            source: Some("composer.json".into()),
            requires_elevation: false,
            kind: InstallationStepKind::ConfigurePhpExtension {
                extension: extension.clone(),
            },
        });
        steps.push(InstallationStep {
            label: format!("Validar extensão PHP {extension}"),
            detail: "Confirmar que a extensão foi carregada pelo PHP".into(),
            source: None,
            requires_elevation: false,
            kind: InstallationStepKind::VerifyCommand {
                program: "php".into(),
                args: vec![
                    "-r".into(),
                    format!(
                        "if (!extension_loaded('{extension}')) {{ exit(1); }} echo PHP_VERSION, PHP_EOL;"
                    ),
                ],
                version_constraint: None,
            },
        });
    }

    let mut package_managers = environment
        .install_steps
        .iter()
        .filter(|step| matches!(step.program.as_str(), "pnpm" | "yarn" | "bun"))
        .map(|step| {
            (
                step.program.clone(),
                node_package_manager_spec(&step.working_directory),
                step.working_directory.clone(),
            )
        })
        .collect::<Vec<_>>();
    package_managers.sort();
    package_managers.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1);

    for (manager, spec, working_directory) in package_managers {
        if !environment
            .missing_programs
            .iter()
            .any(|program| program == &manager)
        {
            continue;
        }
        if manager == "bun" {
            let constraint = spec.split_once('@').map_or("latest", |(_, value)| value);
            let provider = WindowsWingetProvider;
            steps.push(provider.resolve_bun_installation(constraint, &working_directory)?);
            steps.push(InstallationStep {
                label: "Atualizar o ambiente do LocalCodePilot".into(),
                detail: "Recarregar o PATH depois da instalação do Bun".into(),
                source: Some("Windows".into()),
                requires_elevation: false,
                kind: InstallationStepKind::RefreshEnvironment,
            });
        } else {
            steps.push(InstallationStep {
                label: "Validar Corepack".into(),
                detail: "Confirmar o gerenciador de ferramentas incluído no Node.js LTS".into(),
                source: None,
                requires_elevation: false,
                kind: InstallationStepKind::VerifyCommand {
                    program: "corepack".into(),
                    args: vec!["--version".into()],
                    version_constraint: None,
                },
            });
            steps.push(InstallationStep {
                label: format!("Preparar {spec}"),
                detail: "Baixar e ativar a versão declarada pelo projeto".into(),
                source: Some("Node.js Corepack".into()),
                requires_elevation: false,
                kind: InstallationStepKind::Command {
                    program: "corepack".into(),
                    args: vec!["install".into(), "--global".into(), spec],
                    working_directory: working_directory.clone(),
                },
            });
            let install_directory = tools_directory.clone().ok_or_else(|| {
                "Não foi possível localizar a pasta local de ferramentas do usuário".to_owned()
            })?;
            steps.push(InstallationStep {
                label: format!("Criar comando {manager}"),
                detail:
                    "Disponibilizar um atalho gerenciado sem alterar a instalação global do Node.js"
                        .into(),
                source: Some("LocalCodePilot".into()),
                requires_elevation: false,
                kind: InstallationStepKind::CreateCorepackShim {
                    manager: manager.clone(),
                    install_directory,
                },
            });
            steps.push(InstallationStep {
                label: "Atualizar o ambiente do LocalCodePilot".into(),
                detail: "Adicionar a pasta gerenciada ao ambiente dos processos".into(),
                source: Some("LocalCodePilot".into()),
                requires_elevation: false,
                kind: InstallationStepKind::RefreshEnvironment,
            });
        }
        steps.push(InstallationStep {
            label: format!("Validar {manager}"),
            detail: "Confirmar o gerenciador usado pelo projeto".into(),
            source: None,
            requires_elevation: false,
            kind: InstallationStepKind::VerifyCommand {
                program: manager,
                args: vec!["--version".into()],
                version_constraint: None,
            },
        });
    }

    steps.extend(
        environment
            .install_steps
            .iter()
            .map(|step| InstallationStep {
                // Labels that already start with a verb ("Criar", "Gerar") are kept.
                label: if step.label.starts_with(char::is_uppercase) {
                    step.label.clone()
                } else {
                    format!("Instalar {}", step.label)
                },
                detail: format!(
                    "Executar no projeto: {} {}",
                    step.program,
                    step.args.join(" ")
                ),
                source: Some(PROJECT_MANIFEST_SOURCE.into()),
                requires_elevation: false,
                kind: InstallationStepKind::Command {
                    program: step.program.clone(),
                    args: step.args.clone(),
                    working_directory: step.working_directory.clone(),
                },
            }),
    );

    if steps.is_empty() {
        return Err("Não há etapas de preparação para este projeto".into());
    }
    Ok(InstallationPlan {
        project_path: project_path.to_path_buf(),
        title: "Revise as etapas antes de continuar".into(),
        requirements: environment.runtime_requirements.clone(),
        steps,
    })
}

/// Source of the plan steps that install the project's own dependencies.
const PROJECT_MANIFEST_SOURCE: &str = "Manifesto do projeto";

/// Error returned when the person cancels a running plan.
const PLAN_CANCELLED: &str = "A preparação foi cancelada";

/// The verb of a preparation flow (T001): the card's action ("Instalar dependências" or
/// "Preparar ambiente") names the confirmation, the progress window, the cancel button and
/// every notice, from the first click to the result. It comes from the plan itself: only
/// the project's manifest installs dependencies; anything that changes the machine prepares
/// the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanVerb {
    InstallDependencies,
    PrepareEnvironment,
}

impl PlanVerb {
    fn of(plan: &InstallationPlan) -> Self {
        if installs_only_dependencies(plan) {
            Self::InstallDependencies
        } else {
            Self::PrepareEnvironment
        }
    }

    /// Title of the confirmation (#101, LCP-SEC-001).
    fn confirmation_title(self) -> &'static str {
        match self {
            Self::InstallDependencies => "Instalar dependências deste projeto?",
            Self::PrepareEnvironment => "Preparar este projeto?",
        }
    }

    /// The confirm button and the progress window use the card's action.
    fn action(self) -> &'static str {
        match self {
            Self::InstallDependencies => "Instalar dependências",
            Self::PrepareEnvironment => "Preparar ambiente",
        }
    }

    fn started(self) -> &'static str {
        match self {
            Self::InstallDependencies => "Instalando dependências em segundo plano...",
            Self::PrepareEnvironment => "Preparando o ambiente em segundo plano...",
        }
    }

    fn cancel_button(self) -> &'static str {
        match self {
            Self::InstallDependencies => "Cancelar instalação de dependências",
            Self::PrepareEnvironment => "Cancelar preparação do ambiente",
        }
    }

    fn cancelled_before_running(self) -> &'static str {
        match self {
            Self::InstallDependencies => "Instalação de dependências cancelada antes da execução.",
            Self::PrepareEnvironment => "Preparação do ambiente cancelada antes da execução.",
        }
    }

    fn cancelled_while_running(self) -> &'static str {
        match self {
            Self::InstallDependencies => "Instalação de dependências cancelada.",
            Self::PrepareEnvironment => "Preparação do ambiente cancelada.",
        }
    }

    fn finished(self, ready: bool) -> &'static str {
        match (self, ready) {
            (Self::InstallDependencies, true) => "Dependências instaladas. O projeto está pronto.",
            (Self::PrepareEnvironment, true) => "Ambiente preparado. O projeto está pronto.",
            (Self::InstallDependencies, false) => {
                "As dependências foram instaladas, mas o projeto ainda precisa de atenção."
            }
            (Self::PrepareEnvironment, false) => {
                "O ambiente foi preparado, mas o projeto ainda precisa de atenção."
            }
        }
    }

    fn failed(self, error: &str) -> String {
        match self {
            Self::InstallDependencies => {
                format!("Não foi possível instalar as dependências: {error}")
            }
            Self::PrepareEnvironment => format!("Não foi possível preparar o ambiente: {error}"),
        }
    }
}

fn plan_window_title(plan: &InstallationPlan) -> &'static str {
    PlanVerb::of(plan).confirmation_title()
}

fn plan_confirm_label(plan: &InstallationPlan) -> &'static str {
    PlanVerb::of(plan).action()
}

fn installs_only_dependencies(plan: &InstallationPlan) -> bool {
    plan.steps
        .iter()
        .all(|step| step.source.as_deref() == Some(PROJECT_MANIFEST_SOURCE))
}

fn command_argument_after(step: &InstallationStep, argument: &str) -> Option<String> {
    let InstallationStepKind::Command { args, .. } = &step.kind else {
        return None;
    };
    args.windows(2)
        .find(|values| values[0] == argument)
        .map(|values| values[1].clone())
}

fn installation_step_command(step: &InstallationStep) -> String {
    match &step.kind {
        // An argument with spaces is quoted, as typed in a terminal (`--override "--wait …"`).
        InstallationStepKind::Command { program, args, .. } => std::iter::once(program.clone())
            .chain(args.iter().map(|arg| {
                if arg.contains(' ') {
                    format!("\"{arg}\"")
                } else {
                    arg.clone()
                }
            }))
            .collect::<Vec<_>>()
            .join(" "),
        InstallationStepKind::RefreshEnvironment => "Atualizar PATH".into(),
        InstallationStepKind::VerifyCommand { program, args, .. } => {
            std::iter::once(program.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" ")
        }
        InstallationStepKind::InstallComposer { install_directory } => format!(
            "baixar e validar https://getcomposer.org/installer em {}",
            install_directory.display()
        ),
        InstallationStepKind::ConfigurePhpExtension { extension } => {
            format!("habilitar ext-{extension} no php.ini")
        }
        InstallationStepKind::VerifyCppCompiler { .. } => {
            if cfg!(windows) {
                format!(
                    "vswhere -latest -products * -requires {}",
                    localcodepilot_runtime::build_languages::MSVC_COMPONENT
                )
            } else if cfg!(target_os = "macos") {
                "xcode-select -p".into()
            } else {
                "cc --version".into()
            }
        }
        InstallationStepKind::CreateCorepackShim {
            manager,
            install_directory,
        } => format!(
            "criar {} em {}",
            if cfg!(windows) {
                format!("{manager}.cmd")
            } else {
                manager.clone()
            },
            install_directory.display()
        ),
    }
}

fn run_installation_plan(
    plan: &InstallationPlan,
    sender: &mpsc::Sender<DependencyInstallEvent>,
    cancel: &AtomicBool,
) -> Result<InstallationOutcome, String> {
    // The current PATH plus the language install folders, like the processes the app starts.
    let mut environment_path = refreshed_environment_path().ok();
    for (index, step) in plan.steps.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(PLAN_CANCELLED.into());
        }
        let _ = sender.send(DependencyInstallEvent::Log(format!(
            "[etapa {}/{}] {}",
            index + 1,
            plan.steps.len(),
            step.label
        )));
        let _ = sender.send(DependencyInstallEvent::Log(format!(
            "$ {}",
            installation_step_command(step)
        )));

        match &step.kind {
            InstallationStepKind::RefreshEnvironment => {
                environment_path = Some(refreshed_environment_path()?);
            }
            InstallationStepKind::VerifyCommand {
                program,
                args,
                version_constraint,
            } => {
                let args = args.iter().map(String::as_str).collect::<Vec<_>>();
                let inventory = inspect_program(program, &args, environment_path.as_deref())
                    .ok_or_else(|| format!("{program} não foi encontrado depois da instalação"))?;
                if let Some(constraint) = version_constraint
                    && runtime_version_matches(constraint, &inventory.version) == Some(false)
                {
                    return Err(format!(
                        "{} foi encontrado, mas a versão esperada é {constraint}",
                        inventory.version,
                    ));
                }
                let _ = sender.send(DependencyInstallEvent::Log(format!(
                    "[verificado] {} em {}",
                    inventory.version,
                    inventory.path.display()
                )));
            }
            InstallationStepKind::InstallComposer { install_directory } => {
                install_composer(
                    install_directory,
                    environment_path.as_deref(),
                    sender,
                    cancel,
                )?;
            }
            InstallationStepKind::ConfigurePhpExtension { extension } => {
                configure_php_extension(extension, environment_path.as_deref(), sender)?;
            }
            InstallationStepKind::CreateCorepackShim {
                manager,
                install_directory,
            } => create_corepack_shim(manager, install_directory)?,
            InstallationStepKind::VerifyCppCompiler { wait } => {
                verify_cpp_compiler(*wait, environment_path.as_deref(), sender, cancel)?;
            }
            InstallationStepKind::Command {
                program,
                args,
                working_directory,
            } => {
                let command_line = installation_step_command(step);
                let mut command = installation_command(program, args, environment_path.as_deref())?;
                let mut child = command
                    .current_dir(plain_path(working_directory))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|error| {
                        format!("Não foi possível executar {command_line}: {error}")
                    })?;
                let stdout_reader = child
                    .stdout
                    .take()
                    .map(|stdout| spawn_dependency_output_reader(stdout, sender.clone()));
                let stderr_reader = child
                    .stderr
                    .take()
                    .map(|stderr| spawn_dependency_output_reader(stderr, sender.clone()));
                let step_started = Instant::now();
                let step_timeout = if args.iter().any(|arg| {
                    arg == localcodepilot_runtime::build_languages::BUILD_TOOLS_PACKAGE
                        || arg == localcodepilot_runtime::build_languages::BUILD_TOOLS_WORKLOAD
                }) {
                    COMPILER_STEP_TIMEOUT
                } else {
                    INSTALLATION_STEP_TIMEOUT
                };
                let status = loop {
                    if cancel.load(Ordering::Relaxed) {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(PLAN_CANCELLED.into());
                    }
                    if step_started.elapsed() >= step_timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!(
                            "{} excedeu o limite de {} minutos",
                            step.label,
                            step_timeout.as_secs() / 60
                        ));
                    }
                    match child.try_wait() {
                        Ok(Some(status)) => break status,
                        Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                        Err(error) => {
                            return Err(format!(
                                "Não foi possível aguardar {command_line}: {error}"
                            ));
                        }
                    }
                };
                if let Some(reader) = stdout_reader {
                    let _ = reader.join();
                }
                if let Some(reader) = stderr_reader {
                    let _ = reader.join();
                }
                // WinGet reports an installed package with no applicable update as nonzero.
                // Continue only to refresh the environment and validate the actual runtime.
                if program == "winget"
                    && args.first().is_some_and(|arg| arg == "install")
                    && matches!(status.code(), Some(-1978335189 | -1978335135))
                {
                    environment_path = Some(refreshed_environment_path()?);
                    let _ = sender.send(DependencyInstallEvent::Log(
                        "[informação] Pacote já instalado; a versão será validada nas próximas etapas".into(),
                    ));
                } else if program == "xcode-select" {
                    // It only opens the Apple installer; whether the tools arrive is what the
                    // next step checks (it also answers nonzero when they are being installed).
                    let _ = sender.send(DependencyInstallEvent::Log(
                        "[informação] Conclua a instalação na janela da Apple".into(),
                    ));
                } else if !status.success() {
                    let error = format!(
                        "{} falhou com código de saída {}",
                        step.label,
                        status
                            .code()
                            .map_or_else(|| "desconhecido".into(), |code| code.to_string())
                    );
                    let _ = sender.send(DependencyInstallEvent::Log(format!("[erro] {error}")));
                    return Err(error);
                }
            }
        }
        let _ = sender.send(DependencyInstallEvent::Log(format!(
            "[concluído] {}",
            step.label
        )));
    }
    Ok(InstallationOutcome { environment_path })
}

/// Confirms the C/C++ compiler; with `wait`, checks again every few seconds until the Apple
/// installer finishes, the person cancels, or the step's time runs out.
fn verify_cpp_compiler(
    wait: bool,
    environment_path: Option<&OsStr>,
    sender: &mpsc::Sender<DependencyInstallEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let started = Instant::now();
    let mut announced = false;
    loop {
        if localcodepilot_runtime::build_languages::cpp_compiler_available(environment_path) {
            let _ = sender.send(DependencyInstallEvent::Log(
                "[verificado] compilador C/C++ encontrado".into(),
            ));
            return Ok(());
        }
        if !wait {
            return Err("o compilador C/C++ não foi encontrado depois da instalação".into());
        }
        if !announced {
            announced = true;
            let _ = sender.send(DependencyInstallEvent::Log(
                "[aguardando] instalador da Apple; o LocalCodePilot confere a cada poucos segundos"
                    .into(),
            ));
        }
        if started.elapsed() >= INSTALLATION_STEP_TIMEOUT {
            return Err(format!(
                "o compilador não apareceu em {} minutos; conclua o instalador da Apple e use Preparar ambiente de novo",
                INSTALLATION_STEP_TIMEOUT.as_secs() / 60
            ));
        }
        for _ in 0..30 {
            if cancel.load(Ordering::Relaxed) {
                return Err(PLAN_CANCELLED.into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn install_composer(
    install_directory: &Path,
    environment_path: Option<&OsStr>,
    sender: &mpsc::Sender<DependencyInstallEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    const SIGNATURE_URL: &str = "https://composer.github.io/installer.sig";
    const INSTALLER_URL: &str = "https://getcomposer.org/installer";
    if cancel.load(Ordering::Relaxed) {
        return Err(PLAN_CANCELLED.into());
    }
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build();
    let agent: ureq::Agent = config.into();
    let expected = agent
        .get(SIGNATURE_URL)
        .header("User-Agent", "LocalCodePilot-Environment-Preparer")
        .call()
        .map_err(|error| format!("Não foi possível obter a assinatura do Composer: {error}"))?
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("A assinatura do Composer é inválida: {error}"))?;
    let installer = agent
        .get(INSTALLER_URL)
        .header("User-Agent", "LocalCodePilot-Environment-Preparer")
        .call()
        .map_err(|error| format!("Não foi possível baixar o Composer: {error}"))?
        .body_mut()
        .read_to_vec()
        .map_err(|error| format!("Não foi possível ler o instalador do Composer: {error}"))?;
    let actual = lowercase_hex(&Sha384::digest(&installer));
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err("A assinatura SHA-384 do instalador do Composer não confere".into());
    }
    let _ = sender.send(DependencyInstallEvent::Log(
        "[verificado] assinatura SHA-384 do instalador do Composer".into(),
    ));
    fs::create_dir_all(install_directory).map_err(|error| {
        format!(
            "Não foi possível criar {}: {error}",
            install_directory.display()
        )
    })?;
    let setup_path = install_directory.join("composer-setup.php");
    fs::write(&setup_path, installer)
        .map_err(|error| format!("Não foi possível salvar o instalador: {error}"))?;
    let args = vec![
        setup_path.display().to_string(),
        "--install-dir".into(),
        install_directory.display().to_string(),
        "--filename".into(),
        "composer.phar".into(),
    ];
    let output = installation_command("php", &args, environment_path)?
        .current_dir(plain_path(install_directory))
        .output()
        .map_err(|error| format!("Não foi possível executar o instalador do Composer: {error}"));
    let _ = fs::remove_file(&setup_path);
    let output = output?;
    for line in String::from_utf8_lossy(&output.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&output.stderr).lines())
    {
        let _ = sender.send(DependencyInstallEvent::Log(strip_terminal_sequences(line)));
    }
    if !output.status.success() {
        return Err(format!(
            "O instalador oficial do Composer encerrou com código {}",
            output
                .status
                .code()
                .map_or_else(|| "desconhecido".into(), |code| code.to_string())
        ));
    }
    #[cfg(target_os = "windows")]
    fs::write(
        install_directory.join("composer.cmd"),
        "@echo off\r\nphp \"%~dp0composer.phar\" %*\r\n",
    )
    .map_err(|error| format!("Não foi possível criar o comando Composer: {error}"))?;
    Ok(())
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn supported_php_extension(extension: &str) -> bool {
    matches!(
        extension,
        "curl"
            | "exif"
            | "fileinfo"
            | "gd"
            | "intl"
            | "mbstring"
            | "mysqli"
            | "openssl"
            | "pdo_mysql"
            | "sodium"
            | "zip"
    )
}

fn create_corepack_shim(manager: &str, install_directory: &Path) -> Result<(), String> {
    if !matches!(manager, "pnpm" | "yarn") {
        return Err(format!("O gerenciador {manager} não é permitido"));
    }
    fs::create_dir_all(install_directory).map_err(|error| {
        format!(
            "Não foi possível criar {}: {error}",
            install_directory.display()
        )
    })?;
    #[cfg(target_os = "windows")]
    {
        let path = install_directory.join(format!("{manager}.cmd"));
        fs::write(&path, format!("@echo off\r\ncorepack {manager} %*\r\n"))
            .map_err(|error| format!("Não foi possível criar {}: {error}", path.display()))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = install_directory.join(manager);
        fs::write(
            &path,
            format!("#!/bin/sh\nexec corepack {manager} \"$@\"\n"),
        )
        .map_err(|error| format!("Não foi possível criar {}: {error}", path.display()))?;
        let mut permissions = fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn configure_php_extension(
    extension: &str,
    environment_path: Option<&OsStr>,
    sender: &mpsc::Sender<DependencyInstallEvent>,
) -> Result<(), String> {
    if !supported_php_extension(extension) {
        return Err(format!("A extensão PHP ext-{extension} não é permitida"));
    }
    let check_args = vec![
        "-r".into(),
        format!("exit(extension_loaded('{extension}') ? 0 : 1);"),
    ];
    if installation_command("php", &check_args, environment_path)
        .and_then(|mut command| command.status().map_err(|error| error.to_string()))
        .is_ok_and(|status| status.success())
    {
        let _ = sender.send(DependencyInstallEvent::Log(format!(
            "[verificado] ext-{extension} já está ativa"
        )));
        return Ok(());
    }

    let ini_args = vec!["-r".into(), "echo php_ini_loaded_file(), PHP_EOL;".into()];
    let output = installation_command("php", &ini_args, environment_path)?
        .output()
        .map_err(|error| format!("Não foi possível localizar o php.ini: {error}"))?;
    let loaded_path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let php_executable = find_executable("php", environment_path)
        .ok_or_else(|| "O PHP não foi encontrado para configurar extensões".to_owned())?;
    let php_directory = php_executable
        .parent()
        .ok_or_else(|| "A pasta da instalação do PHP não foi encontrada".to_owned())?;
    let ini_path = if loaded_path.is_empty() {
        let target = php_directory.join("php.ini");
        let template = ["php.ini-development", "php.ini-production"]
            .iter()
            .map(|name| php_directory.join(name))
            .find(|path| path.is_file())
            .ok_or_else(|| {
                "O PHP não possui php.ini nem um modelo de configuração utilizável".to_owned()
            })?;
        fs::copy(&template, &target)
            .map_err(|error| format!("Não foi possível criar {}: {error}", target.display()))?;
        target
    } else {
        PathBuf::from(loaded_path)
    };
    let dll_name = format!("php_{extension}.dll");
    if cfg!(windows) && !php_directory.join("ext").join(&dll_name).is_file() {
        return Err(format!(
            "A instalação atual do PHP não fornece ext/{dll_name}"
        ));
    }
    let contents = fs::read_to_string(&ini_path)
        .map_err(|error| format!("Não foi possível ler {}: {error}", ini_path.display()))?;
    let backup_path = ini_path.with_extension("ini.localcodepilot.bak");
    if !backup_path.is_file() {
        fs::copy(&ini_path, &backup_path).map_err(|error| {
            format!(
                "Não foi possível criar o backup {}: {error}",
                backup_path.display()
            )
        })?;
    }
    let mut found = false;
    let mut updated = Vec::new();
    for line in contents.lines() {
        let normalized = line.trim().trim_start_matches(';').trim();
        let enables_extension = normalized.eq_ignore_ascii_case(&format!("extension={extension}"))
            || normalized.eq_ignore_ascii_case(&format!("extension={dll_name}"));
        if enables_extension {
            updated.push(format!("extension={extension}"));
            found = true;
        } else {
            updated.push(line.to_owned());
        }
    }
    if !found {
        updated.push(format!("extension={extension}"));
    }
    let mut updated = updated.join("\n");
    updated.push('\n');
    fs::write(&ini_path, updated)
        .map_err(|error| format!("Não foi possível atualizar {}: {error}", ini_path.display()))?;
    let _ = sender.send(DependencyInstallEvent::Log(format!(
        "[configurado] ext-{extension} em {}; backup em {}",
        ini_path.display(),
        backup_path.display()
    )));
    Ok(())
}

fn installation_command(
    program: &str,
    args: &[String],
    environment_path: Option<&OsStr>,
) -> Result<Command, String> {
    let executable = find_executable(program, environment_path)
        .ok_or_else(|| format!("O comando {program} não foi encontrado"))?;
    // Batch files included: the standard library runs them through cmd.exe and quotes them.
    let mut command = Command::new(plain_path(&executable));
    command.args(args);
    if let Some(path) = environment_path {
        command.env("PATH", path);
    }
    localcodepilot_platform::toolchains::apply(&mut command, program, environment_path);
    configure_background_command(&mut command);
    Ok(command)
}

#[cfg(test)]
fn run_dependency_installs(
    steps: &[DependencyInstall],
    sender: &mpsc::Sender<DependencyInstallEvent>,
) -> Result<(), String> {
    let plan = InstallationPlan {
        project_path: PathBuf::new(),
        title: "Instalação de dependências".into(),
        requirements: Vec::new(),
        steps: steps
            .iter()
            .map(|step| InstallationStep {
                label: step.label.clone(),
                detail: String::new(),
                source: None,
                requires_elevation: false,
                kind: InstallationStepKind::Command {
                    program: step.program.clone(),
                    args: step.args.clone(),
                    working_directory: step.working_directory.clone(),
                },
            })
            .collect(),
    };
    run_installation_plan(&plan, sender, &AtomicBool::new(false)).map(|_| ())
}

fn spawn_dependency_output_reader(
    reader: impl Read + Send + 'static,
    sender: mpsc::Sender<DependencyInstallEvent>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            let line = strip_terminal_sequences(&line);
            if sender.send(DependencyInstallEvent::Log(line)).is_err() {
                break;
            }
        }
    })
}

fn dependency_error_suggestion(logs: &str, error: &str) -> String {
    let details = format!("{logs}\n{error}").to_ascii_lowercase();
    if composer_security_advisory(&details) {
        "O Composer bloqueou versões vulneráveis. 1. Recomendado: atualize laravel/framework para uma versão ainda suportada e compatível com seu PHP — Laravel 12 requer PHP 8.2 ou superior; Laravel 13 requer PHP 8.3 ou superior. 2. Revise também as outras dependências no composer.json antes de tentar novamente. 3. Se um advisory não afetar este projeto, ignore somente o código PKSA específico e documente o motivo. Evite desativar globalmente o bloqueio de segurança. Como não existe composer.lock, gere e versione o arquivo após uma instalação segura."
            .into()
    } else if [
        "enotfound",
        "eai_again",
        "timed out",
        "timeout",
        "could not resolve",
        "failed to download",
        "network error",
    ]
    .iter()
    .any(|pattern| details.contains(pattern))
    {
        "Verifique a conexão, o proxy e o endereço do registro de pacotes. Depois tente novamente."
            .into()
    } else if [
        "eacces",
        "eperm",
        "permission denied",
        "access is denied",
        "acesso negado",
    ]
    .iter()
    .any(|pattern| details.contains(pattern))
    {
        "Feche programas que estejam usando essa pasta e confira se sua conta possui permissão de escrita no projeto."
            .into()
    } else if ["enospc", "no space left", "espaço insuficiente"]
        .iter()
        .any(|pattern| details.contains(pattern))
    {
        "Libere espaço no disco usado pelo projeto e execute a instalação novamente.".into()
    } else if ["eresolve", "peer dependency", "conflicting peer"]
        .iter()
        .any(|pattern| details.contains(pattern))
    {
        "Há versões de pacotes incompatíveis. Revise as dependências indicadas no primeiro erro e alinhe suas versões no arquivo do projeto."
            .into()
    } else if [
        "unsupported engine",
        "ebadengine",
        "requires node",
        "your php version",
        "does not satisfy that requirement",
    ]
    .iter()
    .any(|pattern| details.contains(pattern))
    {
        "A versão do runtime não atende ao projeto. Instale ou selecione a versão de Node.js ou PHP indicada nos logs."
            .into()
    } else if ["unauthorized", "authentication", "401", "403"]
        .iter()
        .any(|pattern| details.contains(pattern))
    {
        "O registro recusou o acesso. Autentique sua conta ou confira o token configurado para dependências privadas."
            .into()
    } else if ["integrity", "checksum", "lockfile", "frozen lockfile"]
        .iter()
        .any(|pattern| details.contains(pattern))
    {
        "O cache ou o lockfile pode estar inconsistente. Limpe o cache do gerenciador e regenere o lockfile somente após revisar as alterações."
            .into()
    } else if ["404", "not found", "could not find package"]
        .iter()
        .any(|pattern| details.contains(pattern))
    {
        "Confira o nome e a versão do pacote, além do registro configurado no projeto.".into()
    } else if details.contains("package.json")
        && ["parse", "json", "unexpected token"]
            .iter()
            .any(|pattern| details.contains(pattern))
    {
        "O package.json parece inválido. Corrija o JSON indicado nos logs e tente novamente.".into()
    } else if details.contains("ext-")
        && ["missing", "requires", "enable"]
            .iter()
            .any(|pattern| details.contains(pattern))
    {
        "Uma extensão do PHP está ausente. Habilite a extensão indicada nos logs na instalação do PHP."
            .into()
    } else {
        "Leia a primeira mensagem de erro nos logs, corrija o arquivo ou requisito indicado e tente novamente."
            .into()
    }
}

fn process_error_suggestion(logs: &[String]) -> Option<String> {
    if logs.is_empty() {
        return None;
    }
    let details = logs.join("\n").to_ascii_lowercase();
    let suggestion = if details.contains("não ficou disponível")
        || details.contains("did not become available")
    {
        "Confira nos logs se o servidor aceitou a porta informada e se terminou de carregar. Depois, tente iniciar novamente."
    } else if details.contains("deixou de responder") {
        "O processo continuou aberto, mas parou de escutar a porta. Verifique o erro imediatamente anterior nos logs e reinicie o serviço."
    } else if details.contains("eaddrinuse")
        || details.contains("address already in use")
        || details.contains("endereço já está em uso")
    {
        "Outra aplicação ocupou a porta durante a inicialização. Tente novamente para o LocalCodePilot selecionar uma nova porta."
    } else if details.contains("cannot find module")
        || details.contains("module not found")
        || details.contains("failed to resolve import")
        || details.contains("could not resolve")
    {
        "Há uma dependência ou importação ausente. Instale as dependências do projeto e confira o nome do módulo indicado nos logs."
    } else if details.contains("command not found")
        || details.contains("não é reconhecido como um comando")
        || details.contains("is not recognized as an internal or external command")
    {
        "Um programa usado pelo script não foi encontrado. Instale-o ou adicione-o ao PATH e tente novamente."
    } else if details.contains("permission denied") || details.contains("eacces") {
        "O sistema negou acesso a um arquivo ou porta. Confira o caminho indicado e as permissões da pasta do projeto."
    } else if details.contains("syntaxerror") || details.contains("syntax error") {
        "Existe um erro de sintaxe. Abra o primeiro arquivo e linha indicados nos logs, corrija-os e tente novamente."
    } else {
        "Use Ver logs para encontrar a primeira mensagem de erro antes do encerramento. Depois de corrigir a causa, use Tentar de novo."
    };
    Some(suggestion.into())
}

fn process_command(process: &ProjectProcess) -> Command {
    process_command_on_port(process, None, None)
}

fn process_command_on_port(
    process: &ProjectProcess,
    port: Option<Port>,
    environment_path: Option<&OsStr>,
) -> Command {
    let args = port
        .and_then(|port| process.args_with_port(port))
        .unwrap_or_else(|| process.args.clone());
    #[cfg(target_os = "windows")]
    {
        let executable = find_executable(&process.program, environment_path)
            .unwrap_or_else(|| PathBuf::from(&process.program));
        // Batch files (npm.cmd, mvnw.cmd, gradlew.bat) included: the standard library runs
        // them through cmd.exe with correct quoting, which a hand-made `cmd /C` did not do
        // for paths with spaces.
        let mut command = Command::new(plain_path(&executable));
        command.args(&args);
        configure_background_command(&mut command);
        if let Some(path) = environment_path {
            command.env("PATH", path);
        }
        // Swift needs the Visual Studio developer environment, and Stack a folder without
        // spaces: set up by the app.
        localcodepilot_platform::toolchains::apply(
            &mut command,
            &process.program,
            environment_path,
        );
        command
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut command = Command::new(&process.program);
        command.args(&args);
        if let Some(path) = environment_path {
            command.env("PATH", path);
        }
        command
    }
}

/// A line of a program's output. Most tools write UTF-8; Windows tools without a console of
/// their own (MSBuild, cl) write in the OEM code page, so "versão" arrived as "vers�o".
fn decode_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    #[cfg(windows)]
    if let Some(text) = decode_oem(bytes) {
        return text;
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(windows)]
fn decode_oem(bytes: &[u8]) -> Option<String> {
    use windows_sys::Win32::Globalization::{CP_OEMCP, MultiByteToWideChar};
    let length = i32::try_from(bytes.len()).ok()?;
    // SAFETY: the input pointer and length describe `bytes`; the first call writes nothing
    // and returns the size, the second writes at most `wide.len()` units.
    let size = unsafe {
        MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), length, std::ptr::null_mut(), 0)
    };
    if size <= 0 {
        return None;
    }
    let mut wide = vec![0u16; usize::try_from(size).ok()?];
    let written = unsafe {
        MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), length, wide.as_mut_ptr(), size)
    };
    if written <= 0 {
        return None;
    }
    Some(String::from_utf16_lossy(
        &wide[..usize::try_from(written).ok()?],
    ))
}

/// The program's name without folder or extension, lowercased. Both separators count, so a
/// Windows path saved in the workspace reads the same on any system.
fn program_stem(program: &str) -> String {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_lowercase()
}

fn is_msbuild(program: &str) -> bool {
    program_stem(program) == "msbuild"
}

/// The process compiles C/C++ before it runs: CMake, Meson or MSBuild steps, or `make run`.
fn compiles_cpp(process: &ProjectProcess) -> bool {
    process.prepare.iter().any(|step| {
        matches!(
            program_stem(&step.program).as_str(),
            "cmake" | "meson" | "msbuild"
        )
    }) || (matches!(program_stem(&process.program).as_str(), "make" | "gmake")
        && process.args == ["run"])
}

/// What the card says when the compiler is missing, for this system (T033.03).
fn missing_compiler_sentence() -> String {
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        return "Falta um compilador C/C++ para este projeto. Use Preparar ambiente.".into();
    }
    let package = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| localcodepilot_runtime::build_languages::linux_compiler_package(&text));
    linux_compiler_sentence(package)
}

fn linux_compiler_sentence(package: Option<&str>) -> String {
    match package {
        Some(package) => format!(
            "Falta um compilador C/C++. Instale o pacote {package} da sua distribuição e use Atualizar."
        ),
        None => "Falta um compilador C/C++. Instale o pacote de desenvolvimento da sua distribuição (por exemplo, build-essential ou gcc) e use Atualizar.".into(),
    }
}

/// The steps that install the C/C++ compiler (T033.03): the Microsoft Build Tools with the
/// C++ workload on Windows; on macOS, the Apple installer, then a wait for it to finish.
fn compiler_steps(project_path: &Path) -> Vec<InstallationStep> {
    use localcodepilot_runtime::build_languages::{BUILD_TOOLS_PACKAGE, BUILD_TOOLS_WORKLOAD};
    if cfg!(target_os = "macos") {
        return vec![
            InstallationStep {
                label: "Abrir o instalador da Apple".into(),
                detail: "As ferramentas de linha de comando da Apple são instaladas pela janela do macOS".into(),
                source: Some("Apple · xcode-select".into()),
                requires_elevation: false,
                kind: InstallationStepKind::Command {
                    program: "xcode-select".into(),
                    args: vec!["--install".into()],
                    working_directory: project_path.to_path_buf(),
                },
            },
            InstallationStep {
                label: "Verificar compilador".into(),
                detail: "Aguardar a conclusão no instalador da Apple e confirmar o compilador".into(),
                source: None,
                requires_elevation: false,
                kind: InstallationStepKind::VerifyCppCompiler { wait: true },
            },
        ];
    }
    if !cfg!(target_os = "windows") {
        // Linux: the card explains the package; the app does not install with sudo.
        return Vec::new();
    }
    let verify = InstallationStep {
        label: "Verificar compilador".into(),
        detail: "Confirmar o compilador C/C++ instalado".into(),
        source: None,
        requires_elevation: false,
        kind: InstallationStepKind::VerifyCppCompiler { wait: false },
    };
    // A Build Tools 2022 without C++: its own installer adds the workload.
    if let (Some(installation), Some(installer)) = (
        localcodepilot_runtime::build_languages::build_tools_without_cpp(),
        localcodepilot_runtime::build_languages::visual_studio_installer(),
    ) {
        return vec![
            InstallationStep {
                label: "Instalar ferramentas C/C++".into(),
                detail: "Adicionar o compilador C/C++ da Microsoft (MSVC) e o SDK do Windows ao Visual Studio Build Tools 2022 já instalado".into(),
                source: Some("Microsoft · Visual Studio Installer".into()),
                requires_elevation: true,
                // The Visual Studio Installer does not ask for elevation itself (it answers
                // 5007): Windows asks for it through Start-Process -Verb RunAs, and the exit
                // code of the installer is the step's.
                kind: InstallationStepKind::Command {
                    program: "powershell".into(),
                    args: vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        elevated_installer_command(&installer, &installation),
                    ],
                    working_directory: project_path.to_path_buf(),
                },
            },
            verify,
        ];
    }
    vec![
        InstallationStep {
            label: "Instalar ferramentas C/C++".into(),
            detail: "Baixar e instalar o Visual Studio Build Tools 2022 com o compilador C/C++ da Microsoft (MSVC) e o SDK do Windows".into(),
            source: Some(format!("Microsoft · winget · {BUILD_TOOLS_PACKAGE}")),
            requires_elevation: true,
            kind: InstallationStepKind::Command {
                program: "winget".into(),
                args: vec![
                    "install".into(),
                    "--id".into(),
                    BUILD_TOOLS_PACKAGE.into(),
                    "--exact".into(),
                    "--source".into(),
                    "winget".into(),
                    "--accept-package-agreements".into(),
                    "--accept-source-agreements".into(),
                    "--disable-interactivity".into(),
                    "--override".into(),
                    format!(
                        "--wait --passive --norestart --add {BUILD_TOOLS_WORKLOAD} --includeRecommended"
                    ),
                ],
                working_directory: project_path.to_path_buf(),
            },
        },
        verify,
    ]
}

/// PowerShell that runs the Visual Studio Installer as administrator to add the C++ workload
/// to an installation, and exits with the installer's code.
fn elevated_installer_command(installer: &Path, installation: &Path) -> String {
    use localcodepilot_runtime::build_languages::BUILD_TOOLS_WORKLOAD;
    // Single-quoted PowerShell strings: a quote inside a path is doubled.
    let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
    format!(
        "$p = Start-Process -Verb RunAs -Wait -PassThru -FilePath '{}' -ArgumentList 'modify --installPath \"{}\" --add {BUILD_TOOLS_WORKLOAD} --includeRecommended --passive --norestart'; exit $p.ExitCode",
        quote(installer),
        quote(installation)
    )
}

/// The plan installs the C/C++ compiler.
fn prepares_compiler(plan: &InstallationPlan) -> bool {
    plan.steps
        .iter()
        .any(|step| matches!(step.kind, InstallationStepKind::VerifyCppCompiler { .. }))
}

/// The sentence of the confirmation of a plan that installs the compiler.
fn compiler_notice(plan: &InstallationPlan) -> Option<&'static str> {
    plan.steps.iter().find_map(|step| match step.kind {
        InstallationStepKind::VerifyCppCompiler { wait: true } => Some(
            "Instalar as ferramentas de linha de comando da Apple? O instalador do macOS será aberto. Siga as etapas nele e volte ao LocalCodePilot para verificar o compilador.",
        ),
        InstallationStepKind::VerifyCppCompiler { wait: false } => Some(COMPILER_NOTICE_WINDOWS),
        _ => None,
    })
}

const COMPILER_NOTICE_WINDOWS: &str = "Instalar as ferramentas C/C++ da Microsoft? A instalação ocupa cerca de 5 GB (medido numa instalação de teste; o tamanho do download depende do que já está instalado), e o instalador da Microsoft mostra o andamento. O Windows pode pedir permissão de administrador. Depois, o LocalCodePilot verificará o compilador.";

/// A plan that installs the compiler failed or was not concluded.
pub(super) const COMPILER_FAILED: &str =
    "Não foi possível preparar o compilador C/C++. Confira o motivo em Ver logs e tente novamente.";

/// The Build Tools download and install can take longer than other packages.
const COMPILER_STEP_TIMEOUT: Duration = Duration::from_secs(90 * 60);

fn is_build_tool(program: &str) -> bool {
    localcodepilot_platform::installations::BUILD_TOOL_PACKAGES
        .iter()
        .any(|(name, ..)| *name == program)
}

fn spawn_output_reader(reader: impl Read + Send + 'static, sender: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        // Not `lines()`: it stops at the first line that is not UTF-8, and tools such as
        // MSBuild write in the console code page ("Gerando Código..."), so the rest of the
        // output was lost.
        let mut reader = BufReader::new(reader);
        let mut bytes = Vec::new();
        while matches!(reader.read_until(b'\n', &mut bytes), Ok(read) if read > 0) {
            let text = decode_output(&bytes);
            let line = strip_terminal_sequences(text.trim_end_matches(['\r', '\n']));
            bytes.clear();
            if sender.send(line).is_err() {
                break;
            }
        }
    });
}

fn terminate_process(child: &mut Child) {
    // Keep the descendants before terminating their parent. Some launchers
    // exit first, and taskkill can be unavailable in restricted environments.
    // Without the process table only the Child itself is terminated.
    let mut system = sysinfo::System::new().ok();
    let root = sysinfo::Pid::from_u32(child.id());
    let mut tree = vec![root];
    if let Some(system) = system.as_mut() {
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, false);
        let mut index = 0;
        while index < tree.len() {
            let parent = tree[index];
            for (pid, process) in system.processes() {
                if process.parent() == Some(parent) && !tree.contains(pid) {
                    tree.push(*pid);
                }
            }
            index += 1;
        }
    }
    #[cfg(target_os = "windows")]
    {
        let process_id = child.id().to_string();
        let mut command = Command::new("taskkill");
        configure_background_command(&mut command);
        if command
            .args(["/PID", &process_id, "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            let _ = child.wait();
        }
    }
    let _ = child.kill();
    for pid in tree.into_iter().rev().filter(|pid| *pid != root) {
        if let Some(process) = system.as_ref().and_then(|system| system.process(pid)) {
            #[cfg(not(windows))]
            let _ = process.kill();
            #[cfg(windows)]
            {
                use windows_sys::Win32::{
                    Foundation::CloseHandle,
                    System::Threading::{
                        OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
                        WaitForSingleObject,
                    },
                };
                // Only PIDs captured as descendants of our own Child are used.
                // Native termination also works when taskkill's enumeration is denied.
                unsafe {
                    let handle = OpenProcess(
                        PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                        0,
                        process.pid().as_u32(),
                    );
                    if !handle.is_null() {
                        if TerminateProcess(handle, 1) != 0 {
                            WaitForSingleObject(handle, 5_000);
                        }
                        CloseHandle(handle);
                    }
                }
            }
        }
    }
    let _ = child.wait();
}

fn schedule_http_health_check(running: &mut RunningProcess, url: String, now: Instant) {
    if running.http_health_receiver.is_some() {
        return;
    }
    let target_changed = running
        .http_health
        .as_ref()
        .is_none_or(|health| health.url != url);
    let interval_elapsed = running
        .last_http_health_check
        .is_none_or(|last_check| now.duration_since(last_check) >= HTTP_HEALTH_CHECK_INTERVAL);
    if !target_changed && !interval_elapsed {
        return;
    }
    if target_changed {
        running.http_health = None;
    }
    running.last_http_health_check = Some(now);
    let (sender, receiver) = mpsc::channel();
    running.http_health_receiver = Some(receiver);
    std::thread::spawn(move || {
        let _ = sender.send(probe_http_health(url));
    });
}

fn probe_http_health(url: String) -> HttpHealthCheckResult {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_HEALTH_CHECK_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    let started_at = Instant::now();
    let status = match agent
        .get(&url)
        .header("User-Agent", "LocalCodePilot-Health-Check")
        .call()
    {
        Ok(response) => {
            classify_http_health(response.status().as_u16(), started_at.elapsed().as_millis())
        }
        Err(error) => HttpHealthStatus::Unreachable {
            error: error.to_string(),
        },
    };
    HttpHealthCheckResult { url, status }
}

fn classify_http_health(status_code: u16, response_time_ms: u128) -> HttpHealthStatus {
    if (200..400).contains(&status_code) {
        HttpHealthStatus::Healthy {
            status_code,
            response_time_ms,
        }
    } else {
        HttpHealthStatus::Attention {
            status_code,
            response_time_ms,
        }
    }
}

fn http_health_display(status: &HttpHealthStatus) -> (String, Color32) {
    match status {
        HttpHealthStatus::Healthy {
            status_code,
            response_time_ms,
        } => (
            format!("HTTP {status_code} · {response_time_ms} ms"),
            theme::SUCCESS,
        ),
        HttpHealthStatus::Attention {
            status_code,
            response_time_ms,
        } => (
            format!("HTTP {status_code} · {response_time_ms} ms · requer atenção"),
            Color32::from_rgb(255, 205, 75),
        ),
        HttpHealthStatus::Unreachable { error } => (
            format!("HTTP sem resposta · {error}"),
            Color32::from_rgb(235, 87, 87),
        ),
    }
}

fn project_process_ids(
    processes: &[ProjectProcess],
    project_path: &Path,
    active_only: bool,
) -> Vec<String> {
    processes
        .iter()
        .filter(|process| process.project_path == project_path)
        .filter(|process| {
            !active_only
                || matches!(
                    process.state,
                    ProcessState::Running | ProcessState::Starting
                )
        })
        .filter(|process| {
            active_only
                || !matches!(
                    process.state,
                    ProcessState::Running | ProcessState::Starting
                )
        })
        .map(|process| process.id.clone())
        .collect()
}

fn process_matches_search(
    process: &ProjectProcess,
    project: Option<&Project>,
    query: &str,
) -> bool {
    query.is_empty()
        || process.project_name.to_lowercase().contains(query)
        || process.name.to_lowercase().contains(query)
        || process.command_line().to_lowercase().contains(query)
        || process
            .expected_port
            .is_some_and(|port| port.value().to_string().contains(query))
        || process
            .project_path
            .to_string_lossy()
            .to_lowercase()
            .contains(query)
        || process
            .working_directory
            .to_string_lossy()
            .to_lowercase()
            .contains(query)
        || project.is_some_and(|project| project.display_stack().to_lowercase().contains(query))
}

/// Collects the answer of a listener probe, or starts one every few seconds. Prefers the port
/// of an address the process printed, and skips debugger ports that are not the site.
fn probe_listening_port(running: &mut RunningProcess, now: Instant) -> Option<Port> {
    if let Some(receiver) = &running.listener_probe {
        match receiver.try_recv() {
            Ok(ports) => {
                running.listener_probe = None;
                return choose_listening_port(&ports, &running.urls);
            }
            Err(TryRecvError::Disconnected) => running.listener_probe = None,
            Err(TryRecvError::Empty) => return None,
        }
    }
    if running
        .last_listener_probe
        .is_some_and(|last| now.duration_since(last) < LISTENER_PROBE_INTERVAL)
    {
        return None;
    }
    running.last_listener_probe = Some(now);
    let root = running.child.id();
    let (sender, receiver) = mpsc::channel();
    running.listener_probe = Some(receiver);
    std::thread::spawn(move || {
        // Only ports that answer like a web server: tools such as the Gradle daemon listen
        // on ports of their own before the site is up.
        let ports = listening_ports(root)
            .into_iter()
            .filter(|port| answers_http(*port))
            .collect::<Vec<_>>();
        let _ = sender.send(ports);
    });
    None
}

/// Node (9229), Java (5005) and live-reload (35729) debug ports are not the site.
fn choose_listening_port(ports: &[Port], printed: &[LocalServerUrl]) -> Option<Port> {
    const DEBUG_PORTS: [u16; 4] = [5005, 9229, 9230, 35729];
    printed
        .iter()
        .rev()
        .map(LocalServerUrl::port)
        .find(|port| ports.contains(port))
        .or_else(|| {
            ports
                .iter()
                .copied()
                .find(|port| !DEBUG_PORTS.contains(&port.value()))
        })
}

fn process_exposes_application_url(project: &Project, process: &ProjectProcess) -> bool {
    // A Flutter app for the desktop opens its own window; the ports it listens on belong to
    // the debugger, not to a site.
    if process.program.eq_ignore_ascii_case("flutter")
        && process.args.windows(2).any(|pair| {
            pair[0] == "-d" && matches!(pair[1].as_str(), "windows" | "macos" | "linux")
        })
    {
        return false;
    }
    if project.technologies.contains(&TechnologyKind::Laravel) {
        return process.program.eq_ignore_ascii_case("php")
            && process
                .args
                .first()
                .is_some_and(|argument| argument == "artisan")
            && process
                .args
                .get(1)
                .is_some_and(|argument| argument == "serve");
    }
    if project.technologies.contains(&TechnologyKind::Lumen) {
        return process.program.eq_ignore_ascii_case("php")
            && process
                .args
                .first()
                .is_some_and(|argument| argument == "-S");
    }
    true
}

fn port_conflict_details(conflict: &PortConflict) -> String {
    match &conflict.owner {
        Some(owner) => format!(
            "Porta {} ocupada por {} · PID {}",
            conflict.port.value(),
            owner
                .process_name
                .as_deref()
                .unwrap_or("processo desconhecido"),
            owner.process_id
        ),
        None => format!("Porta {} ocupada por outro processo", conflict.port.value()),
    }
}

fn port_conflict_message(process_name: &str, conflict: &PortConflict) -> String {
    format!(
        "Não foi possível iniciar {process_name}: {}.",
        port_conflict_details(conflict).to_lowercase()
    )
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn relative_time(timestamp: u64) -> String {
    let elapsed = unix_timestamp().saturating_sub(timestamp);
    match elapsed {
        0..=59 => "agora".into(),
        60..=3_599 => format!("há {} min", elapsed / 60),
        3_600..=86_399 => format!("há {} h", elapsed / 3_600),
        _ => format!("há {} dia(s)", elapsed / 86_400),
    }
}

fn update_versions_line(new_version: &str) -> String {
    format!(
        "Versão atual: {} · Nova versão: {new_version}",
        updates::current_version()
    )
}

/// Erro de atualização em camadas: título e mensagem simples, a ação segura e, sob "Detalhes
/// técnicos", o erro original com um botão de copiar. Retorna `true` quando o usuário pede "Tentar
/// de novo". Na falha de verificação não há tentar de novo: a página oficial vira a ação principal.
fn update_failure_panel(
    ui: &mut egui::Ui,
    failure: &UpdateFailure,
    release_url: Option<&str>,
    id: &str,
) -> bool {
    let mut retry = false;
    ui.label(
        RichText::new(failure.title())
            .strong()
            .color(theme::DANGER)
            .size(12.0),
    );
    ui.label(
        RichText::new(failure.message())
            .color(theme::MUTED)
            .size(11.0),
    );
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let release = release_url.zip(failure.release_page_label());
        if failure.can_retry() {
            if ui.button("Tentar de novo").clicked() {
                retry = true;
            }
            if let Some((url, label)) = release
                && ui.link(label).clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        } else if let Some((url, label)) = release
            && ui
                .add(egui::Button::new(label).fill(theme::PRIMARY.gamma_multiply(0.72)))
                .clicked()
        {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
    });
    let details_key = egui::Id::unique(("update-failure-details", id));
    if notice::disclosure(ui, details_key, "Detalhes técnicos", false) {
        ui.label(
            RichText::new(&failure.technical)
                .monospace()
                .color(theme::MUTED)
                .size(10.0),
        );
        if ui.button("Copiar erro técnico").clicked() {
            ui.ctx().copy_text(failure.technical.clone());
        }
    }
    retry
}

fn process_state_key(state: ProcessState) -> &'static str {
    match state {
        ProcessState::Stopped => "stopped",
        ProcessState::Completed => "completed",
        ProcessState::Starting => "starting",
        ProcessState::Running => "running",
        ProcessState::Failed => "failed",
    }
}

fn service_history_status(status: &str) -> &'static str {
    match status {
        "starting" => "inicialização interrompida",
        "running" => "executado com sucesso",
        "failed" => "falhou",
        "completed" => "concluído com sucesso",
        _ => "finalizado",
    }
}

fn process_state_display(state: ProcessState) -> (&'static str, Color32) {
    match state {
        ProcessState::Stopped => ("Parado", theme::MUTED),
        ProcessState::Completed => ("Concluído", theme::SUCCESS),
        ProcessState::Starting => ("Iniciando", Color32::from_rgb(255, 205, 75)),
        ProcessState::Running => ("Executando", theme::SUCCESS),
        ProcessState::Failed => ("Falhou", Color32::from_rgb(235, 87, 87)),
    }
}

fn inspect_project_environment(
    project: &Project,
    processes: &[ProjectProcess],
    environment_path: Option<&OsStr>,
) -> ProjectEnvironment {
    let runtime_inventory = inspect_runtime_inventory(environment_path);
    inspect_project_environment_with_inventory(
        project,
        processes,
        environment_path,
        &runtime_inventory,
    )
}

fn inspect_runtime_inventory(environment_path: Option<&OsStr>) -> RuntimeInventory {
    RuntimeInventory {
        node: inspect_program("node", &["--version"], environment_path),
        python: inspect_program("python", &["--version"], environment_path),
        php: inspect_program(
            "php",
            &["-r", "echo PHP_VERSION, PHP_EOL;"],
            environment_path,
        ),
        java: inspect_program("java", &["-version"], environment_path),
        go: inspect_program("go", &["version"], environment_path),
        dart: inspect_program("dart", &["--version"], environment_path),
        babashka: inspect_program("bb", &["--version"], environment_path),
        stack: inspect_program("stack", &["--version"], environment_path),
        zig: inspect_program("zig", &["version"], environment_path),
        php_extensions: inspect_php_extensions(environment_path),
    }
}

fn inspect_project_environment_with_inventory(
    project: &Project,
    processes: &[ProjectProcess],
    environment_path: Option<&OsStr>,
    runtime_inventory: &RuntimeInventory,
) -> ProjectEnvironment {
    let project_processes: Vec<_> = processes
        .iter()
        .filter(|process| process.project_path == project.path)
        .collect();
    let mut missing_programs = Vec::new();
    let mut missing_dependencies = Vec::new();
    let mut install_steps = Vec::new();
    let mut required_php_extensions = project_processes
        .iter()
        .map(|process| process.working_directory.as_path())
        .chain(std::iter::once(project.path.as_path()))
        .flat_map(|directory| detect_php_extensions(directory, &project.path))
        .collect::<Vec<_>>();
    required_php_extensions.sort();
    required_php_extensions.dedup();
    let missing_php_extensions = required_php_extensions
        .into_iter()
        .filter(|extension| !runtime_inventory.php_extensions.contains(extension))
        .collect::<Vec<_>>();
    let mut runtime_requirements = Vec::new();
    for directory in project_processes
        .iter()
        .map(|process| process.working_directory.as_path())
        .chain(std::iter::once(project.path.as_path()))
    {
        let Some(mut requirement) = detect_node_requirement(directory, &project.path) else {
            continue;
        };
        if runtime_requirements
            .iter()
            .any(|existing: &RuntimeRequirement| {
                existing.version == requirement.version && existing.source == requirement.source
            })
        {
            continue;
        }
        requirement.status =
            runtime_requirement_status(&requirement, runtime_inventory.node.as_ref());
        runtime_requirements.push(requirement);
    }
    for directory in project_processes
        .iter()
        .map(|process| process.working_directory.as_path())
        .chain(std::iter::once(project.path.as_path()))
    {
        for (detected, inventory) in [
            (
                detect_python_requirement(directory, &project.path),
                runtime_inventory.python.as_ref(),
            ),
            (
                detect_php_requirement(directory, &project.path),
                runtime_inventory.php.as_ref(),
            ),
            (
                detect_java_requirement(directory, &project.path),
                runtime_inventory.java.as_ref(),
            ),
            (
                detect_go_requirement(directory, &project.path),
                runtime_inventory.go.as_ref(),
            ),
            (
                detect_dart_requirement(directory, &project.path),
                runtime_inventory.dart.as_ref(),
            ),
            (
                detect_clojure_requirement(directory, &project.path),
                runtime_inventory.babashka.as_ref(),
            ),
            (
                detect_clojure_java_requirement(directory, &project.path),
                runtime_inventory.java.as_ref(),
            ),
            (
                detect_haskell_requirement(directory, &project.path),
                runtime_inventory.stack.as_ref(),
            ),
            (
                detect_zig_requirement(directory, &project.path),
                runtime_inventory.zig.as_ref(),
            ),
        ] {
            let Some(mut requirement) = detected else {
                continue;
            };
            if runtime_requirements
                .iter()
                .any(|existing: &RuntimeRequirement| {
                    existing.runtime == requirement.runtime
                        && existing.version == requirement.version
                        && existing.source == requirement.source
                })
            {
                continue;
            }
            requirement.status = runtime_requirement_status(&requirement, inventory);
            runtime_requirements.push(requirement);
        }
    }
    let laravel_constraint = project_processes
        .iter()
        .map(|process| process.working_directory.as_path())
        .chain(std::iter::once(project.path.as_path()))
        .find_map(read_laravel_constraint);

    for process in &project_processes {
        let programs = if process.prepare.is_empty() {
            vec![process.program.clone()]
        } else {
            process
                .prepare
                .iter()
                .map(|step| step.program.clone())
                .collect()
        };
        for program in programs {
            // MSBuild comes with the C/C++ Build Tools: a missing one is a missing compiler.
            if is_msbuild(&program) {
                continue;
            }
            if find_executable(&program, environment_path).is_none()
                && !missing_programs.contains(&program)
            {
                missing_programs.push(program);
            }
        }

        let uses_node = matches!(process.program.as_str(), "npm" | "pnpm" | "yarn" | "bun");
        let missing_node_dependencies = uses_node
            && node_project_needs_dependencies(&process.working_directory)
            && !has_node_dependencies(&process.working_directory, &project.path);
        if missing_node_dependencies {
            if !missing_dependencies.iter().any(|item| item == "Node.js") {
                missing_dependencies.push("Node.js".to_owned());
            }
            if !install_steps.iter().any(|step: &DependencyInstall| {
                step.program == process.program
                    && step.working_directory == process.working_directory
            }) {
                install_steps.push(DependencyInstall {
                    label: format!("dependências Node.js com {}", process.program),
                    program: process.program.clone(),
                    args: vec!["install".into()],
                    working_directory: process.working_directory.clone(),
                });
            }
        }

        let uses_python = process.program == "python"
            || process.program.ends_with("python.exe")
            || process.program.ends_with("/python");
        let requirements_file = process.working_directory.join("requirements.txt");
        let pyproject_file = process.working_directory.join("pyproject.toml");
        let virtual_python = if cfg!(windows) {
            process
                .working_directory
                .join(".venv")
                .join("Scripts")
                .join("python.exe")
        } else {
            process
                .working_directory
                .join(".venv")
                .join("bin")
                .join("python")
        };
        if uses_python
            && (requirements_file.is_file() || pyproject_file.is_file())
            && !virtual_python.is_file()
        {
            if !missing_dependencies.iter().any(|item| item == "Python") {
                missing_dependencies.push("Python".to_owned());
            }
            if !install_steps.iter().any(|step: &DependencyInstall| {
                step.label == "ambiente virtual Python"
                    && step.working_directory == process.working_directory
            }) {
                install_steps.push(DependencyInstall {
                    label: "ambiente virtual Python".into(),
                    program: "python".into(),
                    args: vec!["-m".into(), "venv".into(), ".venv".into()],
                    working_directory: process.working_directory.clone(),
                });
                let args = if requirements_file.is_file() {
                    vec![
                        "-m".into(),
                        "pip".into(),
                        "install".into(),
                        "-r".into(),
                        "requirements.txt".into(),
                    ]
                } else {
                    vec![
                        "-m".into(),
                        "pip".into(),
                        "install".into(),
                        "-e".into(),
                        ".".into(),
                    ]
                };
                install_steps.push(DependencyInstall {
                    label: "dependências Python na .venv".into(),
                    program: virtual_python.to_string_lossy().into_owned(),
                    args,
                    working_directory: process.working_directory.clone(),
                });
            }
        }

        // Stack keeps its build in `.stack-work`; before the first build it also downloads the
        // GHC compiler, which takes a while, so it happens here and not on "Iniciar".
        if process.program == "stack" && !process.working_directory.join(".stack-work").is_dir() {
            if !missing_dependencies
                .iter()
                .any(|item| item == "Haskell (Stack)")
            {
                missing_dependencies.push("Haskell (Stack)".to_owned());
            }
            if !install_steps.iter().any(|step: &DependencyInstall| {
                step.program == "stack" && step.working_directory == process.working_directory
            }) {
                for (label, args) in [
                    (
                        "compilador GHC do projeto (stack setup)",
                        vec!["setup".into()],
                    ),
                    ("dependências Haskell (stack build)", vec!["build".into()]),
                ] {
                    install_steps.push(DependencyInstall {
                        label: label.into(),
                        program: "stack".into(),
                        args,
                        working_directory: process.working_directory.clone(),
                    });
                }
            }
        }

        // Mix keeps the project's dependencies in `deps`.
        if process.program == "mix"
            && process.working_directory.join("mix.exs").is_file()
            && !process.working_directory.join("deps").is_dir()
        {
            if !missing_dependencies
                .iter()
                .any(|item| item == "Elixir (Mix)")
            {
                missing_dependencies.push("Elixir (Mix)".to_owned());
            }
            if !install_steps.iter().any(|step: &DependencyInstall| {
                step.program == "mix" && step.working_directory == process.working_directory
            }) {
                // A fresh Elixir has no Hex or Rebar, and `deps.get` would stop to ask for
                // them; each step does nothing when they are already there.
                for (label, tool) in [
                    ("gerenciador de pacotes Hex", "local.hex"),
                    ("ferramenta de build Rebar", "local.rebar"),
                ] {
                    install_steps.push(DependencyInstall {
                        label: label.into(),
                        program: "mix".into(),
                        args: vec![tool.into(), "--force".into(), "--if-missing".into()],
                        working_directory: process.working_directory.clone(),
                    });
                }
                install_steps.push(DependencyInstall {
                    label: "dependências Elixir (mix deps.get)".into(),
                    program: "mix".into(),
                    args: vec!["deps.get".into()],
                    working_directory: process.working_directory.clone(),
                });
            }
        }

        // Ruby gems go to the project's vendor/bundle, so a missing folder means they are not
        // installed yet, like Composer's vendor.
        let uses_bundler = matches!(process.program.as_str(), "ruby" | "bundle" | "rackup")
            && process.working_directory.join("Gemfile").is_file();
        if uses_bundler
            && !process
                .working_directory
                .join("vendor")
                .join("bundle")
                .is_dir()
        {
            if !missing_dependencies
                .iter()
                .any(|item| item == "Ruby (Bundler)")
            {
                missing_dependencies.push("Ruby (Bundler)".to_owned());
            }
            if !install_steps.iter().any(|step: &DependencyInstall| {
                step.program == "bundle" && step.working_directory == process.working_directory
            }) {
                install_steps.push(DependencyInstall {
                    label: "Bundler na pasta do projeto (gems em vendor/bundle)".into(),
                    program: "bundle".into(),
                    args: ["config", "set", "--local", "path", "vendor/bundle"]
                        .map(str::to_owned)
                        .to_vec(),
                    working_directory: process.working_directory.clone(),
                });
                install_steps.push(DependencyInstall {
                    label: "dependências Ruby (bundle install)".into(),
                    program: "bundle".into(),
                    args: vec!["install".into()],
                    working_directory: process.working_directory.clone(),
                });
            }
        }

        let uses_composer = process.program == "composer"
            || (process.program == "php"
                && process.working_directory.join("composer.json").is_file());
        let missing_composer_dependencies = uses_composer
            && !has_dependency_directory(&process.working_directory, &project.path, "vendor");
        if missing_composer_dependencies {
            if !missing_dependencies.iter().any(|item| item == "Composer") {
                missing_dependencies.push("Composer".to_owned());
            }
            if find_executable("composer", environment_path).is_none()
                && !missing_programs.iter().any(|item| item == "composer")
            {
                missing_programs.push("composer".to_owned());
            }
            if !install_steps.iter().any(|step: &DependencyInstall| {
                step.program == "composer" && step.working_directory == process.working_directory
            }) {
                install_steps.push(DependencyInstall {
                    label: "dependências Composer".into(),
                    program: "composer".into(),
                    args: vec!["install".into()],
                    working_directory: process.working_directory.clone(),
                });
            }
        }

        // A cloned Laravel project has `.env.example` but no `.env`, and answers every page with
        // an error until it has one with an application key. The key needs `vendor`, so these
        // steps come after Composer.
        let serves_laravel = process.program == "php"
            && process
                .args
                .first()
                .is_some_and(|argument| argument == "artisan");
        let directory = &process.working_directory;
        // Lumen serves with `php -S` from its root and has no `key:generate`: only the copy.
        let serves_lumen = process.program == "php"
            && process
                .args
                .first()
                .is_some_and(|argument| argument == "-S")
            && directory.join("artisan").is_file();
        if (serves_laravel || serves_lumen)
            && directory.join(".env.example").is_file()
            && !directory.join(".env").is_file()
            && !install_steps.iter().any(|step: &DependencyInstall| {
                step.args.first().is_some_and(|argument| argument == "-r")
                    && step.working_directory == *directory
            })
        {
            let framework = if serves_laravel { "Laravel" } else { "Lumen" };
            let configuration = format!("configuração do {framework}");
            if !missing_dependencies.contains(&configuration) {
                missing_dependencies.push(configuration);
            }
            install_steps.push(DependencyInstall {
                label: "Criar o arquivo .env a partir de .env.example".into(),
                program: "php".into(),
                args: vec!["-r".into(), "copy('.env.example', '.env');".into()],
                working_directory: directory.clone(),
            });
        }
        if serves_laravel
            && directory.join(".env.example").is_file()
            && !directory.join(".env").is_file()
            && !install_steps.iter().any(|step: &DependencyInstall| {
                step.args
                    .get(1)
                    .is_some_and(|argument| argument == "key:generate")
                    && step.working_directory == *directory
            })
        {
            install_steps.push(DependencyInstall {
                label: "Gerar a chave da aplicação Laravel (APP_KEY)".into(),
                program: "php".into(),
                args: vec!["artisan".into(), "key:generate".into(), "--ansi".into()],
                working_directory: directory.clone(),
            });
            // New projects keep sessions, cache and jobs in SQLite: without tables every page
            // fails with "no such table". The database is new here, so nothing is lost.
            if fs::read_to_string(directory.join(".env.example")).is_ok_and(|example| {
                example
                    .lines()
                    .any(|line| line.trim() == "DB_CONNECTION=sqlite")
            }) {
                install_steps.push(DependencyInstall {
                    label: "Criar as tabelas do banco SQLite (php artisan migrate)".into(),
                    program: "php".into(),
                    args: vec![
                        "artisan".into(),
                        "migrate".into(),
                        "--force".into(),
                        "--no-interaction".into(),
                    ],
                    working_directory: directory.clone(),
                });
            }
        }

        let uses_pub = matches!(process.program.as_str(), "dart" | "flutter");
        let missing_pub_dependencies = uses_pub
            && process.working_directory.join("pubspec.yaml").is_file()
            && !process
                .working_directory
                .join(".dart_tool")
                .join("package_config.json")
                .is_file();
        if missing_pub_dependencies {
            let dependency_name = if process.program == "flutter" {
                "Flutter"
            } else {
                "Dart"
            };
            if !missing_dependencies
                .iter()
                .any(|item| item == dependency_name)
            {
                missing_dependencies.push(dependency_name.to_owned());
            }
            if !install_steps.iter().any(|step: &DependencyInstall| {
                step.program == process.program
                    && step.working_directory == process.working_directory
            }) {
                install_steps.push(DependencyInstall {
                    label: format!("dependências {dependency_name}"),
                    program: process.program.clone(),
                    args: vec!["pub".into(), "get".into()],
                    working_directory: process.working_directory.clone(),
                });
            }
        }
    }

    missing_programs.sort();
    missing_dependencies.sort();
    let databases = detect_database_requirements(&project.path)
        .into_iter()
        .map(|database| {
            let available = if database.engine == DatabaseEngine::Sqlite {
                probe_sqlite_database(&database, &project.path)
            } else {
                probe_local_tcp(&database)
            };
            // Only a server that answers is logged into; nothing to test on a closed port.
            let connection = (available == Some(true))
                .then(|| {
                    test_connection(
                        &database,
                        &read_credentials(&database),
                        DATABASE_LOGIN_TIMEOUT,
                    )
                })
                .flatten();
            let managed = managed_info(&database);
            DatabaseStatus {
                requirement: database,
                available,
                connection,
                managed,
            }
        })
        .collect();
    let missing_compiler = project_processes
        .iter()
        .any(|process| compiles_cpp(process))
        && !localcodepilot_runtime::build_languages::cpp_compiler_available(environment_path);
    if missing_compiler && cfg!(target_os = "macos") {
        // make comes with the Apple command line tools.
        missing_programs.retain(|program| program != "make");
    }
    ProjectEnvironment {
        command_count: project_processes.len(),
        missing_programs,
        missing_compiler,
        missing_dependencies,
        install_steps,
        runtime_requirements,
        missing_php_extensions,
        laravel_constraint,
        databases,
        manifest_errors: manifest_errors(&project.path),
        ambiguous_entry: localcodepilot_runtime::entry_languages::ambiguous_entries(project),
        detected_commands: project_processes
            .iter()
            .map(|process| {
                let folder = process
                    .working_directory
                    .strip_prefix(&project.path)
                    .ok()
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .map_or_else(
                        || "pasta do projeto".to_owned(),
                        |relative| relative.display().to_string(),
                    );
                // Compile steps come first, as the confirmation shows them (T028.02).
                let command = process
                    .prepare
                    .iter()
                    .map(localcodepilot_core::processes::PrepareStep::command_line)
                    .chain(std::iter::once(process.command_line()))
                    .collect::<Vec<_>>()
                    .join(" → ");
                (process.name.clone(), command, folder)
            })
            .collect(),
    }
}

/// JSON manifests in the project folder that exist but are not valid JSON. Without this the
/// project only shows that nothing can be started, with no reason.
fn manifest_errors(project_path: &Path) -> Vec<String> {
    ["package.json", "composer.json"]
        .into_iter()
        .filter_map(|name| {
            let contents = fs::read_to_string(project_path.join(name)).ok()?;
            let error = serde_json::from_str::<serde_json::Value>(&contents).err()?;
            Some(format!("{name} inválido (linha {})", error.line()))
        })
        .collect()
}

fn inspect_php_extensions(environment_path: Option<&OsStr>) -> HashSet<String> {
    let args = vec!["-m".to_owned()];
    let Ok(output) = installation_command("php", &args, environment_path)
        .and_then(|mut command| command.output().map_err(|error| error.to_string()))
    else {
        return HashSet::new();
    };
    if !output.status.success() {
        return HashSet::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('['))
        .map(|line| line.to_ascii_lowercase().replace('-', "_"))
        .collect()
}

fn runtime_requirement_status(
    requirement: &RuntimeRequirement,
    inventory: Option<&ProgramInventory>,
) -> RequirementStatus {
    match inventory {
        Some(inventory)
            if requirement.version.recommended
                || runtime_version_matches(&requirement.version.value, &inventory.version)
                    != Some(false) =>
        {
            RequirementStatus::Available {
                version: inventory.version.clone(),
                path: inventory.path.clone(),
            }
        }
        Some(inventory) => RequirementStatus::Incompatible {
            installed_version: inventory.version.clone(),
            required_version: requirement.version.value.clone(),
            path: inventory.path.clone(),
        },
        None => RequirementStatus::Missing,
    }
}

fn read_laravel_constraint(directory: &Path) -> Option<String> {
    let contents = fs::read_to_string(directory.join("composer.json")).ok()?;
    let document: serde_json::Value = serde_json::from_str(&contents).ok()?;
    document
        .get("require")?
        .get("laravel/framework")?
        .as_str()
        .map(str::to_owned)
}

fn laravel_major(constraint: &str) -> Option<u64> {
    constraint
        .split(|character: char| !character.is_ascii_digit())
        .find(|part| !part.is_empty())?
        .parse()
        .ok()
}

fn node_project_needs_dependencies(directory: &Path) -> bool {
    let manifest = directory.join("package.json");
    if !manifest.is_file() {
        return false;
    }
    let Some(document) = fs::read_to_string(manifest)
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
    else {
        return true;
    };
    document.get("workspaces").is_some()
        || [
            "dependencies",
            "devDependencies",
            "optionalDependencies",
            "peerDependencies",
        ]
        .iter()
        .any(|key| {
            document
                .get(*key)
                .and_then(serde_json::Value::as_object)
                .is_some_and(|items| !items.is_empty())
        })
}

fn has_node_dependencies(directory: &Path, project_root: &Path) -> bool {
    has_dependency_directory(directory, project_root, "node_modules")
        || has_dependency_file(directory, project_root, ".pnp.cjs")
        || has_dependency_file(directory, project_root, ".pnp.js")
        || has_dependency_file(directory, project_root, ".yarn/install-state.gz")
}

fn has_dependency_directory(directory: &Path, project_root: &Path, name: &str) -> bool {
    has_project_ancestor_entry(directory, project_root, name, Path::is_dir)
}

fn has_dependency_file(directory: &Path, project_root: &Path, name: &str) -> bool {
    has_project_ancestor_entry(directory, project_root, name, Path::is_file)
}

fn has_project_ancestor_entry(
    directory: &Path,
    project_root: &Path,
    name: &str,
    matches: impl Fn(&Path) -> bool,
) -> bool {
    let mut current = Some(directory);
    while let Some(path) = current {
        if !path.starts_with(project_root) {
            break;
        }
        if matches(&path.join(name)) {
            return true;
        }
        if path == project_root {
            break;
        }
        current = path.parent();
    }
    false
}

fn is_localcodepilot_qa_project(path: &Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    normalized.contains("/localcodepilot/qa/")
        || normalized.contains("/localcodepilot/tests/fixtures/")
}

fn current_username() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "Usuário".to_owned())
}

fn initials_from_username(username: &str) -> String {
    let username = username.trim();
    let words: Vec<_> = username.split_whitespace().collect();
    let initials: String = if words.len() >= 2 {
        words
            .iter()
            .take(2)
            .filter_map(|word| word.chars().next())
            .flat_map(char::to_uppercase)
            .collect()
    } else {
        username
            .chars()
            .take(2)
            .flat_map(char::to_uppercase)
            .collect()
    };

    if initials.is_empty() {
        "?".to_owned()
    } else {
        initials
    }
}

fn open_in_vscode(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let mut candidates = Vec::new();
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(local_app_data)
                    .join("Programs")
                    .join("Microsoft VS Code")
                    .join("Code.exe"),
            );
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            candidates.push(
                PathBuf::from(program_files)
                    .join("Microsoft VS Code")
                    .join("Code.exe"),
            );
        }
        if let Some(executable) = candidates.into_iter().find(|candidate| candidate.is_file()) {
            return Command::new(executable)
                .arg(plain_path(path))
                .spawn()
                .map(|_| ())
                .map_err(|error| format!("Não foi possível abrir o VS Code: {error}"));
        }
    }

    let mut command = Command::new(if cfg!(windows) { "code.cmd" } else { "code" });
    configure_background_command(&mut command);
    command
        .arg(plain_path(path))
        .spawn()
        .map(|_| ())
        .map_err(|_| {
            "VS Code não encontrado. Instale-o ou adicione o comando 'code' ao PATH.".into()
        })
}

fn runtime_badge(ui: &mut egui::Ui, label: &str, color: Color32) {
    Frame::new()
        .fill(color.gamma_multiply(0.14))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.55)))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label).color(color).strong().size(12.0));
        });
}

fn runtime_color(runtime: RuntimeKind) -> Color32 {
    match runtime {
        RuntimeKind::Rust => Color32::from_rgb(244, 125, 76),
        RuntimeKind::Node => Color32::from_rgb(104, 190, 101),
        RuntimeKind::Php => Color32::from_rgb(137, 147, 210),
        RuntimeKind::Python => Color32::from_rgb(255, 205, 75),
        RuntimeKind::Java => Color32::from_rgb(237, 79, 63),
        RuntimeKind::Go => Color32::from_rgb(0, 173, 216),
        RuntimeKind::Dart => Color32::from_rgb(0, 180, 171),
        RuntimeKind::Flutter => Color32::from_rgb(84, 197, 248),
        RuntimeKind::Dotnet => Color32::from_rgb(162, 125, 230),
        RuntimeKind::Ruby => Color32::from_rgb(224, 76, 76),
        RuntimeKind::Kotlin => Color32::from_rgb(169, 123, 255),
        RuntimeKind::Elixir => Color32::from_rgb(155, 111, 191),
        RuntimeKind::Deno => Color32::from_rgb(112, 255, 175),
        RuntimeKind::Cpp => Color32::from_rgb(101, 154, 210),
        RuntimeKind::Swift => Color32::from_rgb(240, 81, 56),
        RuntimeKind::Scala => Color32::from_rgb(220, 50, 47),
        RuntimeKind::Clojure => Color32::from_rgb(99, 177, 50),
        RuntimeKind::Haskell => Color32::from_rgb(140, 120, 200),
        RuntimeKind::Perl => Color32::from_rgb(57, 69, 126).gamma_multiply(1.8),
        RuntimeKind::Lua => Color32::from_rgb(80, 110, 230),
        RuntimeKind::R => Color32::from_rgb(39, 109, 195).gamma_multiply(1.4),
        RuntimeKind::Julia => Color32::from_rgb(149, 88, 178),
        RuntimeKind::Zig => Color32::from_rgb(247, 164, 29),
    }
}

fn project_count_text(count: usize) -> String {
    if count == 1 {
        "1 projeto".into()
    } else {
        format!("{count} projetos")
    }
}

fn scan_roots_text(count: usize) -> String {
    if count == 1 {
        "Ver 1 pasta examinada".into()
    } else {
        format!("Ver {count} pastas examinadas")
    }
}

/// Who listens on the port reserved for a running project.
#[derive(Debug, PartialEq, Eq)]
enum PortListener {
    /// The project's process or a child. `None` when the system did not report the owner
    /// (macOS, Linux, or `netstat` failing on Windows): the port is then taken at face value,
    /// without confirmation.
    Project(Option<u32>),
    Foreign(PortOwner),
    None,
}

/// A listener already confirmed as the project's. The start time tells it apart from a later
/// process that got the same PID after it exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VerifiedListener {
    process_id: u32,
    started_at: Option<u64>,
}

/// Whether `process_id` is still the confirmed listener: same PID and same start time. Without
/// a start time, the earlier confirmation is not reused.
fn still_verified(
    verified: Option<VerifiedListener>,
    process_id: u32,
    started_at: impl Fn(u32) -> Option<u64>,
) -> bool {
    verified.is_some_and(|verified| {
        verified.process_id == process_id
            && verified.started_at.is_some()
            && started_at(process_id) == verified.started_at
    })
}

fn port_listener(
    availability: PortAvailability,
    verified: Option<VerifiedListener>,
    project_process: u32,
    descends_from: impl Fn(u32, u32) -> bool,
    started_at: impl Fn(u32) -> Option<u64>,
) -> PortListener {
    match availability {
        PortAvailability::Available => PortListener::None,
        PortAvailability::Occupied { owner: None } => {
            PortListener::Project(verified.map(|verified| verified.process_id))
        }
        PortAvailability::Occupied { owner: Some(owner) } => {
            if still_verified(verified, owner.process_id, started_at)
                || descends_from(owner.process_id, project_process)
            {
                PortListener::Project(Some(owner.process_id))
            } else {
                PortListener::Foreign(owner)
            }
        }
    }
}

/// Log line naming the program that holds the port reserved for a project.
fn port_owner_detail(port: Port, owner: &PortOwner) -> String {
    match &owner.process_name {
        Some(name) => format!(
            "A porta {} está em uso por {name} (PID {}), que não pertence a este projeto.",
            port.value(),
            owner.process_id
        ),
        None => format!(
            "A porta {} está em uso pelo PID {}, que não pertence a este projeto.",
            port.value(),
            owner.process_id
        ),
    }
}

/// Words of a label that starts with an icon glyph, for assistive technologies.
pub(crate) fn words(text: &str) -> &str {
    text.trim_start_matches(|c: char| !c.is_alphanumeric())
}

/// Announces a button or link by its words only; the icon glyph would be read as noise.
pub(crate) fn announced(response: egui::Response, role: egui::Role, text: &str) -> egui::Response {
    let enabled = response.enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(role, enabled, words(text)));
    response
}

/// Width a button needs for `text`, to lay buttons out right-aligned in reading order.
fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    galley.size().x + 2.0 * ui.spacing().button_padding.x
}

/// Why "Adicionar pasta" and "Atualizar" wait (T039.03, CA-9): up to three running
/// projects by name, then how many more.
fn folder_lock_sentence(names: &[String]) -> String {
    let shown = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    match names.len() {
        0 => "Não é possível alterar as pastas enquanto houver projetos rodando.".to_owned(),
        1..=3 => {
            format!("Não é possível alterar as pastas enquanto houver projetos rodando: {shown}.")
        }
        count => format!(
            "Não é possível alterar as pastas enquanto houver projetos rodando: {shown}, e mais {}.",
            count - 3
        ),
    }
}

/// Gives a search field without a visible label a name for assistive technologies.
pub(crate) fn name_search_field(response: &egui::Response, name: &str, value: &str) {
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::text_edit(true, value, value, "");
        info.label = Some(name.to_owned());
        info
    });
}

/// Sidebar group title; collapsed, only a separator remains.
fn nav_group(ui: &mut egui::Ui, collapsed: bool, title: &str) {
    if collapsed {
        ui.separator();
    } else {
        ui.label(RichText::new(title).color(theme::MUTED).strong().size(11.0));
    }
    ui.add_space(4.0);
}

/// Name read by assistive technologies and shown as the tooltip of a collapsed item.
fn nav_name(label: &str, projects: Option<usize>) -> String {
    match projects {
        Some(1) => format!("{label}, 1 projeto"),
        Some(count) => format!("{label}, {count} projetos"),
        None => label.to_owned(),
    }
}

fn technology_color(technology: TechnologyKind) -> Color32 {
    match technology {
        TechnologyKind::Laravel => Color32::from_rgb(255, 70, 70),
        TechnologyKind::Lumen => Color32::from_rgb(239, 110, 80),
        TechnologyKind::WordPress => Color32::from_rgb(50, 130, 190),
        TechnologyKind::Vue => Color32::from_rgb(66, 184, 131),
        TechnologyKind::React => Color32::from_rgb(97, 218, 251),
        TechnologyKind::TypeScript => Color32::from_rgb(49, 120, 198),
        TechnologyKind::JavaScript => Color32::from_rgb(240, 219, 79),
        TechnologyKind::Flutter => Color32::from_rgb(84, 197, 248),
        TechnologyKind::StaticSite => Color32::from_rgb(70, 180, 140),
        TechnologyKind::Containers => Color32::from_rgb(90, 160, 240),
    }
}

#[cfg(test)]
mod output_tests {
    #[test]
    fn output_lines_keep_utf8_and_do_not_stop_at_other_encodings() {
        assert_eq!(super::decode_output("versão".as_bytes()), "versão");
        // "versão" in code page 850 (Windows consoles in Portuguese) is not UTF-8.
        let oem = super::decode_output(b"vers\xc6o");
        assert!(oem.starts_with("vers") && oem.ends_with('o'));

        let (sender, receiver) = std::sync::mpsc::channel();
        super::spawn_output_reader(
            std::io::Cursor::new(b"um\r\nvers\xc6o\ntr\xc3\xaas\n".to_vec()),
            sender,
        );
        let lines = receiver.iter().collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "um");
        assert_eq!(lines[2], "três");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DependencyInstall, DependencyInstallEvent, HttpHealthStatus, Page, ProjectEnvironment,
        classify_http_health, dependency_error_suggestion, find_executable, initials_from_username,
        inspect_project_environment, is_localcodepilot_qa_project, laravel_major, lowercase_hex,
        nav_name, prepare_laravel_migration_files, probe_http_health, process_error_suggestion,
        process_exposes_application_url, process_matches_search, project_count_text,
        project_process_ids, resolve_environment_plan,
        resolve_environment_plan_with_tools_directory, run_dependency_installs,
        runtime_requirement_status, scan_roots_text, update_versions_line, words,
    };
    use super::{Notification, UPDATE_RESULT_NOTIFICATION_DURATION};
    use super::{PROJECT_MANIFEST_SOURCE, plan_confirm_label, plan_window_title};

    use localcodepilot_core::{
        environments::{
            InstallationPlan, InstallationStep, InstallationStepKind, RequirementSource,
            RequirementStatus, RuntimeRequirement, VersionConstraint,
        },
        ports::Port,
        processes::{ProcessState, ProjectProcess},
        projects::Project,
        runtimes::RuntimeKind,
        technologies::TechnologyKind,
    };
    use localcodepilot_platform::installations::ProgramInventory;

    use std::{
        ffi::OsStr,
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        sync::mpsc,
        thread,
        time::SystemTime,
    };

    #[test]
    fn a_found_port_prefers_the_printed_address_and_skips_debuggers() {
        let port = |value| Port::new(value).unwrap();
        let printed =
            localcodepilot_core::ports::extract_local_urls("ready at http://localhost:8080");
        assert_eq!(
            super::choose_listening_port(&[port(3000), port(8080)], &printed),
            Some(port(8080))
        );
        // Node inspector first, then the server.
        assert_eq!(
            super::choose_listening_port(&[port(3000), port(9229)], &[]),
            Some(port(3000))
        );
        assert_eq!(super::choose_listening_port(&[port(9229)], &[]), None);
        assert_eq!(super::choose_listening_port(&[], &printed), None);
    }

    #[test]
    fn the_folder_lock_names_up_to_three_running_projects() {
        let names = |count: usize| {
            (1..=count)
                .map(|index| format!("p{index}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            super::folder_lock_sentence(&names(1)),
            "Não é possível alterar as pastas enquanto houver projetos rodando: p1."
        );
        assert_eq!(
            super::folder_lock_sentence(&names(3)),
            "Não é possível alterar as pastas enquanto houver projetos rodando: p1, p2, p3."
        );
        assert_eq!(
            super::folder_lock_sentence(&names(5)),
            "Não é possível alterar as pastas enquanto houver projetos rodando: p1, p2, p3, e mais 2."
        );
    }

    #[test]
    fn a_saved_tab_of_servicos_opens_projetos() {
        // Serviços left the sidebar (T038.04): its saved tabs open the start page.
        for key in ["processes", "containers", "databases"] {
            assert!(Page::from_key(key) == Page::Projects);
        }
    }

    #[test]
    fn a_project_page_saved_on_close_opens_projetos() {
        // The project may be gone on the next start, so the saved page is the list.
        assert!(Page::from_key(Page::Project.key()) == Page::Projects);
    }

    #[test]
    fn a_saved_plugins_page_opens_the_overview() {
        // Projetos is the start page: the old overview and unknown pages open it.
        assert!(Page::from_key("plugins") == Page::Projects);
        assert!(Page::from_key("overview") == Page::Projects);
        for page in [
            Page::Projects,
            Page::Environment,
            Page::Help,
            Page::Settings,
        ] {
            assert!(Page::from_key(page.key()) == page);
        }
    }

    #[test]
    fn sidebar_and_projects_header_read_naturally() {
        assert_eq!(nav_name("Ajuda", None), "Ajuda");
        assert_eq!(nav_name("Projetos", Some(1)), "Projetos, 1 projeto");
        assert_eq!(nav_name("Projetos", Some(5)), "Projetos, 5 projetos");
        assert_eq!(project_count_text(1), "1 projeto");
        assert_eq!(project_count_text(0), "0 projetos");
        assert_eq!(scan_roots_text(1), "Ver 1 pasta examinada");
        assert_eq!(scan_roots_text(2), "Ver 2 pastas examinadas");
    }

    #[test]
    fn icon_buttons_are_announced_by_their_words() {
        use egui_phosphor::regular::{ARROW_SQUARE_OUT, PLUS};
        assert_eq!(
            words(&format!("{ARROW_SQUARE_OUT}  Abrir site")),
            "Abrir site"
        );
        assert_eq!(
            words(&format!("{PLUS}  Adicionar pasta")),
            "Adicionar pasta"
        );
        assert_eq!(words("Atualizar"), "Atualizar");
    }

    #[test]
    fn the_port_counts_only_when_the_project_listens_on_it() {
        use super::{PortAvailability, PortListener, PortOwner, VerifiedListener, port_listener};
        let owner = |process_id| PortAvailability::Occupied {
            owner: Some(PortOwner {
                process_id,
                process_name: Some("node.exe".into()),
            }),
        };
        let child_of_10 = |process: u32, ancestor: u32| process == 11 && ancestor == 10;
        let started_at_100 = |_: u32| Some(100);
        assert_eq!(
            port_listener(owner(11), None, 10, child_of_10, started_at_100),
            PortListener::Project(Some(11))
        );
        assert!(matches!(
            port_listener(owner(99), None, 10, child_of_10, started_at_100),
            PortListener::Foreign(PortOwner { process_id: 99, .. })
        ));
        // A listener whose relationship to the project was rejected (for example, its
        // registered parent PID now belongs to the project but the project is newer) is
        // another program's: no Rodando, no address, no Abrir site.
        let relationship_rejected = |_: u32, _: u32| false;
        assert!(matches!(
            port_listener(owner(11), None, 10, relationship_rejected, started_at_100),
            PortListener::Foreign(PortOwner { process_id: 11, .. })
        ));
        // A listener confirmed earlier is reused only while it is the same process: same PID
        // and same start time.
        let verified = Some(VerifiedListener {
            process_id: 11,
            started_at: Some(100),
        });
        assert_eq!(
            port_listener(
                owner(11),
                verified,
                10,
                relationship_rejected,
                started_at_100
            ),
            PortListener::Project(Some(11))
        );
        // The confirmed listener exited and another program got its PID: it started later, so
        // the earlier confirmation does not count and the relationship is checked again.
        let started_later = |_: u32| Some(500);
        assert!(matches!(
            port_listener(
                owner(11),
                verified,
                10,
                relationship_rejected,
                started_later
            ),
            PortListener::Foreign(PortOwner { process_id: 11, .. })
        ));
        // Without a start time, the earlier confirmation is not reused either.
        let unknown_start = |_: u32| None;
        let verified_without_time = Some(VerifiedListener {
            process_id: 11,
            started_at: None,
        });
        assert!(matches!(
            port_listener(
                owner(11),
                verified_without_time,
                10,
                relationship_rejected,
                unknown_start
            ),
            PortListener::Foreign(PortOwner { process_id: 11, .. })
        ));
        assert_eq!(
            port_listener(
                PortAvailability::Available,
                verified,
                10,
                child_of_10,
                started_at_100
            ),
            PortListener::None
        );
        // Known limitation: without the owner (macOS, Linux, or netstat failing on Windows)
        // the port is attributed to the project without confirmation.
        assert_eq!(
            port_listener(
                PortAvailability::Occupied { owner: None },
                None,
                10,
                child_of_10,
                started_at_100
            ),
            PortListener::Project(None)
        );
    }
    #[test]
    fn a_foreign_port_owner_is_named_only_in_the_log() {
        let port = localcodepilot_core::ports::Port::new(5173).unwrap();
        let owner = super::PortOwner {
            process_id: 1234,
            process_name: Some("node.exe".into()),
        };
        assert_eq!(
            super::port_owner_detail(port, &owner),
            "A porta 5173 está em uso por node.exe (PID 1234), que não pertence a este projeto."
        );
        let unnamed = super::PortOwner {
            process_name: None,
            ..owner
        };
        assert!(super::port_owner_detail(port, &unnamed).contains("pelo PID 1234"));
    }

    #[test]
    fn restores_help_page_from_workspace_key() {
        assert_eq!(Page::from_key(Page::Help.key()), Page::Help);
    }

    #[test]
    fn excludes_only_localcodepilot_test_projects_from_recents() {
        assert!(is_localcodepilot_qa_project(&PathBuf::from(
            r"C:\projects\LocalCodePilot\qa\manual-container-build"
        )));
        assert!(is_localcodepilot_qa_project(&PathBuf::from(
            r"C:\projects\LocalCodePilot\tests\fixtures\containers-smoke"
        )));
        assert!(!is_localcodepilot_qa_project(&PathBuf::from(
            r"C:\projects\client\qa\dashboard"
        )));
    }

    #[test]
    fn encodes_checksum_bytes_as_lowercase_hex() {
        assert_eq!(lowercase_hex(&[0x00, 0x09, 0xaf, 0xff]), "0009afff");
    }

    #[test]
    fn an_update_result_is_not_replaced_while_it_is_visible() {
        let result = Notification::update_result("LocalCodePilot atualizado para a versão 0.5.0.");
        assert!(result.blocks_replacement());
        assert!(result.duration > super::NOTIFICATION_DURATION);
        assert!(!Notification::new("Projetos e ambientes atualizados").blocks_replacement());

        let expired = Notification {
            created_at: std::time::Instant::now()
                .checked_sub(UPDATE_RESULT_NOTIFICATION_DURATION)
                .unwrap(),
            ..Notification::update_result("LocalCodePilot atualizado para a versão 0.5.0.")
        };
        assert!(!expired.blocks_replacement());
    }

    #[test]
    fn the_update_notice_shows_the_current_and_the_new_version() {
        assert_eq!(
            update_versions_line("9.9.9"),
            format!(
                "Versão atual: {} · Nova versão: 9.9.9",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn the_plan_title_says_whether_the_machine_changes_too() {
        let step = |source: Option<&str>| InstallationStep {
            label: "Instalar dependências Node.js".into(),
            detail: String::new(),
            source: source.map(str::to_owned),
            requires_elevation: false,
            kind: InstallationStepKind::RefreshEnvironment,
        };
        let mut plan = InstallationPlan {
            project_path: PathBuf::from("C:/projetos/loja"),
            title: String::new(),
            requirements: Vec::new(),
            steps: vec![step(Some(PROJECT_MANIFEST_SOURCE))],
        };
        assert_eq!(
            plan_window_title(&plan),
            "Instalar dependências deste projeto?"
        );
        assert_eq!(plan_confirm_label(&plan), "Instalar dependências");
        plan.steps.insert(0, step(Some("winget")));
        assert_eq!(plan_window_title(&plan), "Preparar este projeto?");
        assert_eq!(plan_confirm_label(&plan), "Preparar ambiente");
        plan.steps = vec![step(None), step(Some(PROJECT_MANIFEST_SOURCE))];
        assert_eq!(plan_window_title(&plan), "Preparar este projeto?");
    }

    /// T001: the card's verb is kept from the confirmation to the last notice.
    #[test]
    fn each_flow_keeps_the_card_verb_until_the_end() {
        use super::PlanVerb;
        let install = PlanVerb::InstallDependencies;
        let prepare = PlanVerb::PrepareEnvironment;
        assert_eq!(install.action(), "Instalar dependências");
        assert_eq!(prepare.action(), "Preparar ambiente");
        assert_eq!(
            install.cancel_button(),
            "Cancelar instalação de dependências"
        );
        assert_eq!(prepare.cancel_button(), "Cancelar preparação do ambiente");
        assert_eq!(
            install.cancelled_before_running(),
            "Instalação de dependências cancelada antes da execução."
        );
        assert_eq!(
            prepare.cancelled_before_running(),
            "Preparação do ambiente cancelada antes da execução."
        );
        assert_eq!(
            install.finished(true),
            "Dependências instaladas. O projeto está pronto."
        );
        assert_eq!(
            prepare.finished(true),
            "Ambiente preparado. O projeto está pronto."
        );
        assert_eq!(
            install.finished(false),
            "As dependências foram instaladas, mas o projeto ainda precisa de atenção."
        );
        assert_eq!(
            prepare.finished(false),
            "O ambiente foi preparado, mas o projeto ainda precisa de atenção."
        );
        for verb in [install, prepare] {
            let texts = [
                verb.action(),
                verb.started(),
                verb.cancel_button(),
                verb.cancelled_before_running(),
                verb.cancelled_while_running(),
                verb.finished(true),
                verb.finished(false),
            ];
            let (wanted, other) = match verb {
                PlanVerb::InstallDependencies => ("depend", "prepar"),
                PlanVerb::PrepareEnvironment => ("prepar", "depend"),
            };
            for text in texts {
                let lower = text.to_lowercase();
                assert!(lower.contains(wanted), "{text}");
                assert!(!lower.contains(other), "{text}");
            }
            assert!(verb.failed("x").to_lowercase().contains(wanted));
        }
    }

    #[test]
    fn classifies_success_and_error_http_statuses() {
        assert_eq!(
            classify_http_health(204, 12),
            HttpHealthStatus::Healthy {
                status_code: 204,
                response_time_ms: 12,
            }
        );
        assert_eq!(
            classify_http_health(503, 25),
            HttpHealthStatus::Attention {
                status_code: 503,
                response_time_ms: 25,
            }
        );
    }

    #[test]
    fn probes_a_local_http_service() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1_024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        let result = probe_http_health(format!("http://{address}"));
        server.join().unwrap();

        assert_eq!(result.url, format!("http://{address}"));
        assert!(matches!(
            result.status,
            HttpHealthStatus::Healthy {
                status_code: 204,
                ..
            }
        ));
    }

    #[test]
    fn builds_a_dependency_only_plan_when_node_is_compatible() {
        let project_path = PathBuf::from("project");
        let environment = ProjectEnvironment {
            command_count: 1,
            install_steps: vec![DependencyInstall {
                label: "dependências Node.js com npm".into(),
                program: "npm".into(),
                args: vec!["install".into()],
                working_directory: project_path.clone(),
            }],
            runtime_requirements: vec![RuntimeRequirement {
                runtime: RuntimeKind::Node,
                command: "node".into(),
                version: VersionConstraint::declared(">=20"),
                source: RequirementSource::Recommended,
                status: RequirementStatus::Available {
                    version: "v22.12.0".into(),
                    path: PathBuf::from("node"),
                },
            }],
            ..ProjectEnvironment::default()
        };

        let plan = resolve_environment_plan(&project_path, &environment).unwrap();

        assert_eq!(plan.steps.len(), 1);
        assert!(matches!(
            &plan.steps[0].kind,
            InstallationStepKind::Command { program, args, .. }
                if program == "npm" && args == &["install"]
        ));
    }

    #[test]
    fn accepts_an_installed_runtime_when_the_version_is_only_recommended() {
        let requirement = RuntimeRequirement {
            runtime: RuntimeKind::Python,
            command: "python".into(),
            version: VersionConstraint::recommended("3.14"),
            source: RequirementSource::Recommended,
            status: RequirementStatus::Pending,
        };
        let inventory = ProgramInventory {
            command: "python".into(),
            path: PathBuf::from("python.exe"),
            version: "Python 3.13.5".into(),
        };

        assert!(matches!(
            runtime_requirement_status(&requirement, Some(&inventory)),
            RequirementStatus::Available { .. }
        ));
    }

    #[test]
    fn a_missing_cmake_or_meson_is_prepared_with_the_environment() {
        let environment = ProjectEnvironment {
            command_count: 1,
            missing_programs: vec!["cmake".into()],
            ..ProjectEnvironment::default()
        };
        assert!(environment.needs_environment_preparation());
        assert_eq!(
            environment.can_prepare_environment(),
            cfg!(target_os = "windows")
        );
        assert!(super::is_build_tool("cmake") && super::is_build_tool("meson"));
        assert!(!super::is_build_tool("gcc"));
        let packages = localcodepilot_platform::installations::BUILD_TOOL_PACKAGES;
        assert!(packages.contains(&("cmake", "Kitware.CMake", "CMake")));
        assert!(packages.contains(&("meson", "mesonbuild.meson", "Meson")));
    }

    #[test]
    fn a_missing_cpp_compiler_is_prepared_where_the_system_allows() {
        use localcodepilot_core::processes::PrepareStep;
        let process = |program: &str, args: &[&str], prepare: &[&str]| ProjectProcess {
            id: "p".into(),
            project_name: "loja".into(),
            project_path: PathBuf::from("loja"),
            working_directory: PathBuf::from("loja"),
            name: "loja".into(),
            program: program.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            state: ProcessState::Stopped,
            process_id: None,
            exit_code: None,
            expected_port: None,
            port_override: None,
            prepare: prepare
                .iter()
                .map(|program| PrepareStep {
                    program: (*program).into(),
                    args: vec![],
                    skip_if_exists: None,
                })
                .collect(),
        };
        // What compiles C/C++: CMake, Meson, MSBuild (any path), or make run.
        assert!(super::compiles_cpp(&process(
            "B/bin/loja.exe",
            &[],
            &["cmake", "cmake"]
        )));
        assert!(super::compiles_cpp(&process(
            "B/loja",
            &[],
            &["meson", "meson"]
        )));
        assert!(super::compiles_cpp(&process(
            "B/bin/loja.exe",
            &[],
            &[r"C:\VS\MSBuild\Current\Bin\MSBuild.exe"]
        )));
        assert!(super::compiles_cpp(&process("make", &["run"], &[])));
        assert!(!super::compiles_cpp(&process("make", &["test"], &[])));
        assert!(!super::compiles_cpp(&process("npm", &["run", "dev"], &[])));
        assert!(super::is_msbuild(r"C:\VS\MSBuild\Current\Bin\MSBuild.exe"));

        // The card: stopped, not a problem, with the action this system allows.
        let environment = ProjectEnvironment {
            command_count: 1,
            missing_compiler: true,
            ..ProjectEnvironment::default()
        };
        assert!(!environment.is_ready());
        assert_eq!(
            environment.can_prepare_environment(),
            cfg!(any(target_os = "windows", target_os = "macos"))
        );
        let reason = environment.problem_details().unwrap();
        if cfg!(any(target_os = "windows", target_os = "macos")) {
            assert_eq!(
                reason,
                "Falta um compilador C/C++ para este projeto. Use Preparar ambiente."
            );
        } else {
            assert!(reason.starts_with("Falta um compilador C/C++. Instale o pacote "));
            assert!(!reason.contains("Preparar ambiente"));
        }
        // On macOS the Apple installer prepares only the compiler: with CMake missing too,
        // the app does not offer a preparation it cannot finish.
        let with_cmake = ProjectEnvironment {
            missing_programs: vec!["cmake".into()],
            ..environment.clone()
        };
        assert_eq!(
            with_cmake.can_prepare_environment(),
            cfg!(target_os = "windows")
        );

        // The plan: install, then verify, before anything else. Linux has none.
        if !cfg!(any(target_os = "windows", target_os = "macos")) {
            return;
        }
        let plan = super::resolve_environment_plan_with_tools_directory(
            std::path::Path::new("loja"),
            &environment,
            None,
        )
        .unwrap();
        assert!(super::prepares_compiler(&plan));
        let commands = plan
            .steps
            .iter()
            .map(super::installation_step_command)
            .collect::<Vec<_>>();
        if cfg!(target_os = "macos") {
            assert_eq!(commands[0], "xcode-select --install");
            assert!(!plan.requires_elevation());
            assert!(matches!(
                plan.steps[1].kind,
                InstallationStepKind::VerifyCppCompiler { wait: true }
            ));
            assert!(
                super::compiler_notice(&plan)
                    .unwrap()
                    .contains("instalador do macOS")
            );
        } else {
            // A Build Tools 2022 without C++ gets the workload from its own installer;
            // otherwise winget installs it with the workload.
            if localcodepilot_runtime::build_languages::build_tools_without_cpp().is_some() {
                assert!(commands[0].starts_with("powershell -NoProfile -NonInteractive -Command"));
                assert!(commands[0].contains("Start-Process -Verb RunAs"));
                assert!(commands[0].contains(
                    "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive --norestart"
                ));
            } else {
                assert!(commands[0].starts_with(
                    "winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --source winget"
                ));
                assert!(commands[0].ends_with(
                    "--override \"--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended\""
                ));
            }
            assert!(plan.requires_elevation());
            assert_eq!(plan.steps[0].label, "Instalar ferramentas C/C++");
            assert_eq!(plan.steps[1].label, "Verificar compilador");
            assert!(matches!(
                plan.steps[1].kind,
                InstallationStepKind::VerifyCppCompiler { wait: false }
            ));
            assert_eq!(
                super::compiler_notice(&plan),
                Some(super::COMPILER_NOTICE_WINDOWS)
            );
        }
    }

    #[test]
    fn the_workload_is_added_by_the_installer_running_as_administrator() {
        let command = super::elevated_installer_command(
            std::path::Path::new(r"C:\VS\Installer\setup.exe"),
            std::path::Path::new(r"C:\Program Files (x86)\Equipe d'Ana\BuildTools"),
        );
        assert_eq!(
            command,
            r#"$p = Start-Process -Verb RunAs -Wait -PassThru -FilePath 'C:\VS\Installer\setup.exe' -ArgumentList 'modify --installPath "C:\Program Files (x86)\Equipe d''Ana\BuildTools" --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive --norestart'; exit $p.ExitCode"#
        );
    }

    #[test]
    fn the_linux_card_names_the_package_of_the_distribution() {
        assert_eq!(
            super::linux_compiler_sentence(Some("build-essential")),
            "Falta um compilador C/C++. Instale o pacote build-essential da sua distribuição e use Atualizar."
        );
        assert_eq!(
            super::linux_compiler_sentence(None),
            "Falta um compilador C/C++. Instale o pacote de desenvolvimento da sua distribuição (por exemplo, build-essential ou gcc) e use Atualizar."
        );
    }

    #[test]
    fn prepares_pnpm_with_corepack_before_dependencies() {
        let project_path = PathBuf::from("project");
        let environment = ProjectEnvironment {
            command_count: 1,
            missing_programs: vec!["pnpm".into()],
            install_steps: vec![DependencyInstall {
                label: "dependências Node.js com pnpm".into(),
                program: "pnpm".into(),
                args: vec!["install".into()],
                working_directory: project_path.clone(),
            }],
            runtime_requirements: vec![RuntimeRequirement {
                runtime: RuntimeKind::Node,
                command: "node".into(),
                version: VersionConstraint::declared("22"),
                source: RequirementSource::Recommended,
                status: RequirementStatus::Available {
                    version: "v22.12.0".into(),
                    path: PathBuf::from("node"),
                },
            }],
            ..ProjectEnvironment::default()
        };

        let tools_directory = project_path.join("managed-tools");
        let plan = resolve_environment_plan_with_tools_directory(
            &project_path,
            &environment,
            Some(tools_directory.clone()),
        )
        .unwrap();
        assert!(
            resolve_environment_plan_with_tools_directory(&project_path, &environment, None,)
                .unwrap_err()
                .contains("pasta local de ferramentas")
        );
        assert!(plan.steps.iter().any(|step| matches!(
            &step.kind,
            InstallationStepKind::CreateCorepackShim { manager, install_directory }
                if manager == "pnpm" && install_directory == &tools_directory
        )));
        assert!(matches!(
            &plan.steps.last().unwrap().kind,
            InstallationStepKind::Command { program, .. } if program == "pnpm"
        ));
    }

    #[test]
    fn creates_a_controlled_php_extension_plan() {
        let environment = ProjectEnvironment {
            command_count: 1,
            missing_php_extensions: vec!["mbstring".into()],
            runtime_requirements: vec![RuntimeRequirement {
                runtime: RuntimeKind::Php,
                command: "php".into(),
                version: VersionConstraint::declared("^8.2"),
                source: RequirementSource::Recommended,
                status: RequirementStatus::Available {
                    version: "8.2.20".into(),
                    path: PathBuf::from("php"),
                },
            }],
            ..ProjectEnvironment::default()
        };
        let plan =
            resolve_environment_plan(PathBuf::from("project").as_path(), &environment).unwrap();
        assert!(matches!(
            &plan.steps[0].kind,
            InstallationStepKind::ConfigurePhpExtension { extension } if extension == "mbstring"
        ));
    }

    #[test]
    fn reports_a_missing_node_runtime_with_an_empty_path() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-node-status-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("package.json"), r#"{"engines":{"node":"22"}}"#).unwrap();
        let project = Project::new(path.clone(), vec![RuntimeKind::Node]);
        let process = ProjectProcess {
            id: "node-status".into(),
            project_name: project.name.clone(),
            project_path: path.clone(),
            working_directory: path.clone(),
            name: "Frontend".into(),
            program: "npm".into(),
            args: vec!["run".into(), "dev".into()],
            state: ProcessState::Stopped,
            process_id: None,
            exit_code: None,
            expected_port: None,
            port_override: None,
            prepare: Vec::new(),
        };

        let environment = inspect_project_environment(&project, &[process], Some(OsStr::new("")));

        assert!(environment.needs_environment_preparation());
        assert_eq!(
            environment.can_prepare_environment(),
            cfg!(target_os = "windows")
        );
        assert!(matches!(
            environment.runtime_requirements[0].status,
            RequirementStatus::Missing
        ));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn finds_an_executable_by_its_full_path() {
        let current_executable = std::env::current_exe().expect("current executable should exist");

        assert!(find_executable(&current_executable.to_string_lossy(), None).is_some());
    }

    #[test]
    fn rejects_a_program_that_does_not_exist() {
        assert!(
            find_executable("localcodepilot-program-that-does-not-exist-7d2bcfd8", None).is_none()
        );
    }

    #[test]
    fn creates_initials_from_the_computer_username() {
        assert_eq!(initials_from_username("Julio Cesar"), "JC");
        assert_eq!(initials_from_username("marcos"), "MA");
        assert_eq!(initials_from_username("  "), "?");
    }

    #[test]
    fn suggests_corrections_from_process_logs() {
        assert!(
            process_error_suggestion(&["Error: Cannot find module 'vite'".into()])
                .unwrap()
                .contains("dependência")
        );
        assert!(
            process_error_suggestion(&[
                "[LocalCodePilot] Falha: a porta 5173 deixou de responder por 6 segundos.".into()
            ])
            .unwrap()
            .contains("parou de escutar")
        );
        assert!(process_error_suggestion(&[]).is_none());
    }

    #[test]
    fn reports_missing_runtime_and_dependencies_before_starting() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-readiness-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("package.json"), "{}").unwrap();
        let project = Project::new(path.clone(), vec![RuntimeKind::Node]);
        let process = ProjectProcess {
            id: "frontend".into(),
            project_name: project.name.clone(),
            project_path: project.path.clone(),
            working_directory: project.path.clone(),
            name: "Frontend".into(),
            program: "localcodepilot-missing-runtime-427".into(),
            args: Vec::new(),
            state: ProcessState::Stopped,
            process_id: None,
            exit_code: None,
            expected_port: None,
            port_override: None,
            prepare: Vec::new(),
        };
        let missing_runtime = inspect_project_environment(&project, &[process], None);
        assert_eq!(
            missing_runtime.missing_programs,
            ["localcodepilot-missing-runtime-427"]
        );

        let node_process = ProjectProcess {
            id: "frontend".into(),
            project_name: project.name.clone(),
            project_path: project.path.clone(),
            working_directory: project.path.clone(),
            name: "Frontend".into(),
            program: "npm".into(),
            args: vec!["run".into(), "dev".into()],
            state: ProcessState::Stopped,
            process_id: None,
            exit_code: None,
            expected_port: Some(Port::new(5173).unwrap()),
            port_override: None,
            prepare: Vec::new(),
        };
        let no_dependencies =
            inspect_project_environment(&project, std::slice::from_ref(&node_process), None);
        assert!(no_dependencies.missing_dependencies.is_empty());
        assert!(no_dependencies.install_steps.is_empty());
        fs::write(
            path.join("package.json"),
            r#"{"devDependencies":{"vite":"^6.0.0"}}"#,
        )
        .unwrap();
        let missing_dependencies =
            inspect_project_environment(&project, std::slice::from_ref(&node_process), None);
        assert_eq!(missing_dependencies.missing_dependencies, ["Node.js"]);
        assert_eq!(missing_dependencies.install_steps.len(), 1);
        assert_eq!(missing_dependencies.install_steps[0].program, "npm");
        assert_eq!(missing_dependencies.install_steps[0].args, ["install"]);

        fs::create_dir(path.join("node_modules")).unwrap();
        let installed = inspect_project_environment(&project, &[node_process], None);
        assert!(installed.missing_dependencies.is_empty());
        assert!(installed.install_steps.is_empty());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn runs_a_dependency_install_step_in_the_project_directory() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-install-{nonce}"));
        fs::create_dir(&path).unwrap();

        #[cfg(windows)]
        let (program, args) = (
            "cmd.exe".to_owned(),
            vec!["/D".into(), "/C".into(), "mkdir node_modules".into()],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            "sh".to_owned(),
            vec!["-c".into(), "mkdir node_modules".into()],
        );
        let step = DependencyInstall {
            label: "dependências de teste".into(),
            program,
            args,
            working_directory: path.clone(),
        };

        let (sender, receiver) = mpsc::channel();
        run_dependency_installs(&[step], &sender).unwrap();
        drop(sender);
        let logs = receiver
            .into_iter()
            .filter_map(|event| match event {
                DependencyInstallEvent::Log(line) => Some(line),
                DependencyInstallEvent::Finished(_) => None,
            })
            .collect::<Vec<_>>();

        assert!(path.join("node_modules").is_dir());
        assert!(logs.iter().any(|line| line.starts_with('$')));
        assert!(logs.iter().any(|line| line.starts_with("[concluído]")));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn suggests_corrections_from_dependency_install_errors() {
        assert!(
            dependency_error_suggestion(
                "laravel/framework was not loaded because it is affected by security advisories (PKSA-test)",
                "composer install failed",
            )
            .contains("Laravel 12 requer PHP 8.2")
        );
        assert!(
            dependency_error_suggestion("npm ERR! code ERESOLVE", "installation failed")
                .contains("versões de pacotes incompatíveis")
        );
        assert!(
            dependency_error_suggestion("request failed: ENOTFOUND registry", "failed")
                .contains("conexão")
        );
        assert!(
            dependency_error_suggestion("Your PHP version does not satisfy that requirement", "")
                .contains("versão do runtime")
        );
        assert!(
            dependency_error_suggestion("EACCES: permission denied", "failed")
                .contains("permissão de escrita")
        );
    }

    #[test]
    fn prepares_laravel_migration_without_changing_unrelated_dependencies() {
        let source = r#"{
    "name": "example/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^10.0",
        "laravel/tinker": "^2.8"
    }
}"#;
        let lock = vec![1, 2, 3];

        let files = prepare_laravel_migration_files(source, Some(lock.clone()), "^13.0").unwrap();
        let document: serde_json::Value = serde_json::from_str(&files.composer_json).unwrap();

        assert_eq!(document["require"]["laravel/framework"], "^13.0");
        assert_eq!(document["require"]["laravel/tinker"], "^2.8");
        assert_eq!(files.composer_lock, Some(lock));
    }

    #[test]
    fn recognizes_legacy_laravel_constraints_without_waiting_for_composer_to_fail() {
        assert_eq!(laravel_major("^10.0"), Some(10));
        assert_eq!(laravel_major(">=11.0 <12.0"), Some(11));
        assert_eq!(laravel_major("dev-main"), None);

        let legacy = ProjectEnvironment {
            laravel_constraint: Some("^10.0".into()),
            ..ProjectEnvironment::default()
        };
        let supported = ProjectEnvironment {
            laravel_constraint: Some("^12.0".into()),
            ..ProjectEnvironment::default()
        };

        assert_eq!(legacy.legacy_laravel_constraint(), Some("^10.0"));
        assert_eq!(supported.legacy_laravel_constraint(), None);
    }

    #[test]
    fn selects_all_stopped_or_active_processes_for_a_project() {
        let project_path = PathBuf::from("app-gestao");
        let other_path = PathBuf::from("outro-projeto");
        let process = |id: &str, path: &PathBuf, state| ProjectProcess {
            id: id.into(),
            project_name: "Projeto".into(),
            project_path: path.clone(),
            working_directory: path.clone(),
            name: id.into(),
            program: "program".into(),
            args: Vec::new(),
            state,
            process_id: None,
            exit_code: None,
            expected_port: None,
            port_override: None,
            prepare: Vec::new(),
        };
        let processes = vec![
            process("backend", &project_path, ProcessState::Stopped),
            process("frontend", &project_path, ProcessState::Running),
            process("outro", &other_path, ProcessState::Stopped),
        ];

        assert_eq!(
            project_process_ids(&processes, &project_path, false),
            ["backend"]
        );
        assert_eq!(
            project_process_ids(&processes, &project_path, true),
            ["frontend"]
        );
    }

    #[test]
    fn searches_processes_by_project_technology_and_command() {
        let project = Project::new(PathBuf::from("app-gestao"), vec![RuntimeKind::Php])
            .with_technologies(vec![TechnologyKind::Laravel, TechnologyKind::Vue]);
        let process = ProjectProcess {
            id: "app-gestao::npm run dev".into(),
            project_name: project.name.clone(),
            project_path: project.path.clone(),
            working_directory: project.path.clone(),
            name: "Desenvolvimento".into(),
            program: "npm".into(),
            args: vec!["run".into(), "dev".into()],
            state: ProcessState::Stopped,
            process_id: None,
            exit_code: None,
            expected_port: Some(Port::new(5173).unwrap()),
            port_override: None,
            prepare: Vec::new(),
        };

        assert!(process_matches_search(&process, Some(&project), "laravel"));
        assert!(process_matches_search(&process, Some(&project), "npm run"));
        assert!(process_matches_search(
            &process,
            Some(&project),
            "app-gestao"
        ));
        assert!(!process_matches_search(&process, Some(&project), "django"));
    }

    #[test]
    fn exposes_only_the_backend_url_for_laravel_and_lumen() {
        let project = Project::new(PathBuf::from("app-gestao"), vec![RuntimeKind::Php])
            .with_technologies(vec![TechnologyKind::Laravel, TechnologyKind::Vue]);
        let mut process = ProjectProcess {
            id: "app-gestao::npm run dev".into(),
            project_name: project.name.clone(),
            project_path: project.path.clone(),
            working_directory: project.path.clone(),
            name: "Frontend".into(),
            program: "npm".into(),
            args: vec!["run".into(), "dev".into()],
            state: ProcessState::Running,
            process_id: Some(123),
            exit_code: None,
            expected_port: Some(Port::new(5173).unwrap()),
            port_override: None,
            prepare: Vec::new(),
        };

        assert!(!process_exposes_application_url(&project, &process));

        process.program = "php".into();
        process.args = vec!["artisan".into(), "serve".into()];
        assert!(process_exposes_application_url(&project, &process));

        let lumen = Project::new(PathBuf::from("lumen-api"), vec![RuntimeKind::Php])
            .with_technologies(vec![TechnologyKind::Lumen]);
        process.args = vec![
            "-S".into(),
            "localhost:8000".into(),
            "-t".into(),
            "public".into(),
        ];
        assert!(process_exposes_application_url(&lumen, &process));

        process.program = "npm".into();
        process.args = vec!["run".into(), "dev".into()];
        assert!(!process_exposes_application_url(&lumen, &process));

        // Flutter: the web server is a site; a desktop app is a window.
        let flutter = Project::new(PathBuf::from("app"), vec![RuntimeKind::Flutter]);
        process.program = "flutter".into();
        process.args = ["run", "-d", "web-server", "--web-port", "4180"]
            .map(String::from)
            .to_vec();
        assert!(process_exposes_application_url(&flutter, &process));
        process.args = ["run", "-d", "windows"].map(String::from).to_vec();
        assert!(!process_exposes_application_url(&flutter, &process));
    }
}
