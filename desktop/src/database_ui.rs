//! The project's database around "Iniciar" (T019.04) and its guided fixes (T019.05).
//!
//! Before any process of a project starts, a local PostgreSQL, MySQL or MariaDB it needs must
//! accept the project's own login. On Windows the app prepares (after a confirmation) or
//! starts its own server, gives the project its database and user, and tests the login again.
//! When something stands in the way, the card says why and, where it is safe, offers a fix
//! that is confirmed first and checked by the project's login afterwards: another port (with
//! a copy of `.env`), the user's password on the app's server, or a new data folder that
//! keeps the old one. Agreed with product in `.internal/conversas/2026-11-20-t019-04-*.md`
//! and `.internal/conversas/2026-11-20-t019-05-correcoes.md`.
//!
//! SQL Server (T019.06) runs in a container of Docker or Podman that the app creates after the
//! person accepts Microsoft's license terms; its ids carry the `mssql:` prefix. Agreed in
//! `.internal/conversas/2026-11-20-t019-06-sql-server.md`.

use super::*;
use localcodepilot_runtime::{
    database_connection::{loopback_address, test_remote_connection},
    database_packages::{self, DatabasePackage},
    database_servers::{self, ManagedServer, Recognition, ServerError, SystemVault},
    sql_server_container::{self, ContainerState, SqlServerContainer},
};
use sha2::{Digest, Sha256};
use std::sync::mpsc::{Receiver, Sender};

/// Where the app keeps database packages and servers: `%LOCALAPPDATA%\LocalCodePilot\bancos`
/// on Windows, `~/Library/Application Support/LocalCodePilot/bancos` on macOS and
/// `$XDG_DATA_HOME/localcodepilot/bancos` (or `~/.local/share/...`) on Linux (T029).
pub(super) fn database_root() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(|folder| PathBuf::from(folder).join("LocalCodePilot").join("bancos"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("LocalCodePilot")
                .join("bancos")
        });
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(".local").join("share")))
        .map(|data| data.join("localcodepilot").join("bancos"))
}

/// The app prepares the servers of `engine` on this computer: SQL Server as a container on
/// Windows, the others from a pinned package for this system (T029).
fn engine_preparable_here(engine: DatabaseEngine) -> bool {
    if engine == DatabaseEngine::SqlServer {
        cfg!(windows)
    } else {
        database_packages::package_for_this_system(engine).is_some()
    }
}

fn engine_key(engine: DatabaseEngine) -> Option<&'static str> {
    match engine {
        DatabaseEngine::PostgreSql => Some("postgresql"),
        DatabaseEngine::MySql => Some("mysql"),
        DatabaseEngine::MariaDb => Some("mariadb"),
        _ => None,
    }
}

/// The Visual C++ files each server imports and its package does not bring.
fn runtime_dlls(engine: &str) -> &'static [&'static str] {
    match engine {
        "postgresql" => &["vcruntime140.dll"],
        "mysql" => &["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"],
        _ => &[],
    }
}

/// The id of an app SQL Server container among the app's servers.
const SQL_PREFIX: &str = "mssql:";

fn sql_id(id: &str) -> Option<&str> {
    id.strip_prefix(SQL_PREFIX)
}

/// The engines whose local database the app looks after.
fn looked_after(engine: DatabaseEngine) -> bool {
    engine_key(engine).is_some() || engine == DatabaseEngine::SqlServer
}

const NO_CONTAINER_ENGINE: &str =
    "Para preparar o SQL Server, prepare o Docker ou o Podman na página Containers.";

/// The first container engine that answers (Docker first).
fn ready_container_engine(path: Option<&std::ffi::OsStr>) -> Option<&'static str> {
    localcodepilot_platform::containers::inspect_container_engines(path)
        .into_iter()
        .find(|diagnosis| diagnosis.state == localcodepilot_core::containers::EngineState::Ready)
        .map(|diagnosis| diagnosis.engine.program())
}

fn local_server(requirement: &DatabaseRequirement) -> bool {
    requirement.compose_service.is_none()
        && requirement
            .host
            .as_deref()
            .and_then(loopback_address)
            .is_some()
}

/// One decimal with a comma; GB from 1 GB on.
pub(super) fn size_label(bytes: u64) -> String {
    let megabytes = bytes as f64 / (1024.0 * 1024.0);
    if megabytes >= 1024.0 {
        format!("{:.1} GB", megabytes / 1024.0).replace('.', ",")
    } else {
        format!("{megabytes:.0} MB")
    }
}

const PORT_TAKEN: &str =
    "já está em uso. O LocalCodePilot não iniciou outro banco para proteger seus dados.";
const ENV_NOT_RESTORED: &str =
    "Não foi possível recuperar a configuração anterior. Uma cópia foi guardada.";
const NEW_DATA_NOTE: &str =
    "O projeto está conectado a um banco novo. Os dados anteriores foram guardados.";

/// A fix the card can offer, always confirmed first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DatabaseFix {
    OtherPort,
    FixPassword,
    NewData,
    /// SQL Server needs Docker or Podman ready: the page where they are prepared.
    OpenContainers,
    /// No administrator password the app keeps logs in to its SQL Server: a new container and
    /// volume; the old ones are kept.
    NewSqlServer,
}

impl DatabaseFix {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::OtherPort => "Usar outra porta",
            Self::FixPassword => "Corrigir conexão",
            Self::NewData => "Criar banco novo",
            Self::OpenContainers => "Abrir Containers",
            Self::NewSqlServer => "Criar SQL Server novo",
        }
    }
}

/// Why the last start stopped at the project's database, and what can be done.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DatabaseFailure {
    pub reason: String,
    /// The card's main action, when a fix is safe here.
    pub fix: Option<DatabaseFix>,
    /// A fix offered in "Detalhes técnicos" only ("Usar outra porta" when a database on the port
    /// refuses access, "Criar SQL Server novo").
    pub detail_fix: Option<DatabaseFix>,
    /// "Tentar de novo": the app's server did not start, for a reason that may pass.
    pub retry: bool,
    /// Lines for "Detalhes técnicos".
    pub details: Vec<String>,
}

impl DatabaseFailure {
    fn plain(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            fix: None,
            detail_fix: None,
            retry: false,
            details: vec![],
        }
    }
}

/// What the app needs to prepare or start the database of one project.
#[derive(Debug, Clone)]
pub(super) struct DatabaseRequest {
    pub project: PathBuf,
    pub requirement: DatabaseRequirement,
    pub package: &'static DatabasePackage,
    pub port: u16,
    pub database: String,
    pub user: String,
    password: String,
    /// The app has no server for this engine and port yet: it will be created.
    pub new_server: bool,
    /// Visual C++ files missing on this computer (MySQL, PostgreSQL).
    pub missing_runtime: Vec<String>,
}

impl DatabaseRequest {
    fn engine_label(&self) -> &'static str {
        self.requirement.engine.label()
    }
}

/// The SQL Server container the app would create for a project, waiting for the terms.
#[derive(Debug, Clone)]
pub(super) struct SqlServerRequest {
    pub project: PathBuf,
    pub requirement: DatabaseRequirement,
    /// `docker` or `podman`.
    pub engine: &'static str,
    pub port: u16,
    pub database: String,
    pub user: String,
    password: String,
    /// "Criar SQL Server novo": the container the app lets go of (kept, with its volume).
    pub replaces: Option<SqlServerContainer>,
    /// The terms checkbox; never checked by the app.
    pub accepted: bool,
    path: Option<std::ffi::OsString>,
}

/// A fix waiting for the person's confirmation.
#[derive(Debug, Clone)]
pub(super) enum FixConfirmation {
    OtherPort {
        project: PathBuf,
        env: PathBuf,
        from: u16,
        to: u16,
    },
    FixPassword {
        project: PathBuf,
        server_id: String,
        requirement: DatabaseRequirement,
        user: String,
    },
    NewData {
        project: PathBuf,
        server_id: String,
        requirement: DatabaseRequirement,
    },
}

/// A `.env` the app changed, until the project's login confirms the change.
#[derive(Debug, Clone)]
pub(super) struct PortChange {
    pub env: PathBuf,
    pub backup: PathBuf,
    /// SHA-256 of what the app wrote: restored only if the file is still exactly that.
    pub written: String,
    pub from: u16,
    pub to: u16,
}

/// The steps shown under "Iniciando".
#[derive(Debug, Clone, PartialEq)]
pub(super) enum DatabaseStep {
    Downloading {
        received: u64,
        total: u64,
    },
    /// The download is complete and its SHA-256 is being checked against the pinned one.
    Verifying,
    Preparing,
    StartingServer,
}

impl DatabaseStep {
    pub(super) fn text(&self, engine: &str) -> String {
        match self {
            Self::Downloading { received, total } if *total > 0 => format!(
                "Baixando {engine}: {:.0}% ({} de {} MB)",
                *received as f64 * 100.0 / *total as f64,
                received / (1024 * 1024),
                total / (1024 * 1024)
            ),
            Self::Downloading { .. } => format!("Baixando {engine}..."),
            Self::Verifying => format!("Verificando o pacote de {engine}…"),
            Self::Preparing => "Preparando o banco de dados…".into(),
            Self::StartingServer => "Iniciando o banco de dados…".into(),
        }
    }
}

/// What a background job ends with.
enum JobResult {
    /// The project's login was accepted: start the project.
    Ready {
        server_id: String,
        actions: Vec<String>,
        new_data: Option<PathBuf>,
        /// Shown with "Rodando" until the next start.
        note: Option<String>,
    },
    Failed(JobFailure),
    Stopped(Result<(), String>),
    Tested(Option<ConnectionTest>),
}

/// The card's sentence when a database job stops without a result.
const JOB_INTERRUPTED: &str =
    "Não foi possível concluir o preparo do banco de dados. Confira os detalhes e tente novamente.";

/// The message of a panic payload, for "Detalhes técnicos".
fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "erro interno sem mensagem".into())
}

struct JobFailure {
    message: String,
    outcome: Option<ConnectionOutcome>,
    server_id: Option<String>,
    /// The app's server did not start and its log shows damaged data.
    damaged: bool,
    /// SQL Server: the recovery that fits (no "Tentar de novo" then).
    recovery: Option<DatabaseFix>,
    details: Vec<String>,
}

impl JobFailure {
    fn text(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            outcome: None,
            server_id: None,
            damaged: false,
            recovery: None,
            details: vec![],
        }
    }

    /// The job stopped without finishing (a panic, a lost thread). No "Tentar de novo": the
    /// server and the data are not known to be in a state another try can safely resume;
    /// Iniciar checks them again from the start.
    fn interrupted(detail: String) -> Self {
        let mut failure = Self::text(JOB_INTERRUPTED);
        failure.details = vec![format!(
            "A tarefa do banco de dados foi interrompida: {detail}"
        )];
        failure
    }

    /// A SQL Server step failed: the sentence for the card, the rest in "Detalhes técnicos".
    fn sql(error: String, server_id: Option<String>) -> Self {
        let recovery = if error.starts_with(sql_server_container::STOP_NOT_CONFIRMED) {
            Some(DatabaseFix::OpenContainers)
        } else if sql_server_container::needs_new_container(&error) {
            Some(DatabaseFix::NewSqlServer)
        } else {
            None
        };
        let (message, details) = match error.strip_prefix(sql_server_container::PREPARATION_FAILED)
        {
            Some(rest) => (
                sql_server_container::PREPARATION_FAILED.to_owned(),
                vec![
                    rest.trim()
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .to_owned(),
                ],
            ),
            None => match error.find(" (") {
                Some(at) if recovery == Some(DatabaseFix::OpenContainers) => (
                    error[..at].to_owned(),
                    vec![
                        error[at..]
                            .trim()
                            .trim_start_matches('(')
                            .trim_end_matches(')')
                            .to_owned(),
                    ],
                ),
                _ => (error, vec![]),
            },
        };
        Self {
            message,
            outcome: None,
            server_id,
            damaged: false,
            recovery,
            details,
        }
    }
}

enum JobEvent {
    Step(DatabaseStep),
    Done(JobResult),
}

pub(super) struct DatabaseJob {
    pub engine: &'static str,
    pub step: DatabaseStep,
    /// Shown under "Iniciando" (stop and remote tests run without it).
    pub visible: bool,
    receiver: Receiver<JobEvent>,
}

/// The decision before starting a project's processes.
pub(super) enum DatabaseGate {
    Ready,
    /// A confirmation or a background job now owns the start.
    Waiting,
    Blocked(DatabaseFailure),
}

#[derive(Default)]
pub(super) struct DatabaseState {
    pub confirmation: Option<DatabaseRequest>,
    /// The SQL Server preparation waiting for the person (with the license terms).
    pub sql_confirmation: Option<SqlServerRequest>,
    pub fix_confirmation: Option<FixConfirmation>,
    pub jobs: HashMap<PathBuf, DatabaseJob>,
    pub failures: HashMap<PathBuf, DatabaseFailure>,
    /// Servers this app started (or found still running after a failed close); stopped on
    /// close.
    pub session_servers: Vec<String>,
    /// What was created or changed for each project, for "Detalhes técnicos".
    pub actions: HashMap<PathBuf, Vec<String>>,
    /// A `.env` changed by "Usar outra porta", until the login confirms it.
    pub port_changes: HashMap<PathBuf, PortChange>,
    /// Consecutive starts of the app's server that failed with damaged data.
    pub damaged_starts: HashMap<PathBuf, u32>,
    /// Shown with "Rodando" after "Criar banco novo", until the next start.
    pub notes: HashMap<PathBuf, String>,
    /// "Testar conexão" results of remote databases.
    pub remote_tests: HashMap<PathBuf, ConnectionTest>,
    /// Servers left running by the last close were looked at.
    checked_left_running: bool,
}

/// What the card shows about the project's database beyond its state.
#[derive(Debug, Clone, Default)]
pub(super) struct DatabaseCardExtras {
    pub failure: Option<DatabaseFailure>,
    /// The app's server of this project is running and may be stopped.
    pub can_stop: bool,
    /// Other running projects that use the same server.
    pub stop_blocked_by: Vec<String>,
    pub remote: bool,
    pub remote_test: Option<ConnectionTest>,
    pub lines: Vec<String>,
}

impl LocalCodePilot {
    /// The project's local database requirements the app looks after.
    fn managed_requirements(environment: &ProjectEnvironment) -> Vec<DatabaseRequirement> {
        environment
            .databases
            .iter()
            .map(|status| status.requirement.clone())
            .filter(|requirement| local_server(requirement) && looked_after(requirement.engine))
            .collect()
    }

    /// Other known projects whose local database uses `engine` on `port` (and, with
    /// `user`, that user).
    fn projects_sharing(
        &self,
        project_path: &Path,
        engine: DatabaseEngine,
        port: u16,
        user: Option<&str>,
    ) -> Vec<(PathBuf, String)> {
        self.project_environments
            .iter()
            .filter(|(path, _)| path.as_path() != project_path)
            .filter(|(_, environment)| {
                Self::managed_requirements(environment)
                    .iter()
                    .any(|requirement| {
                        requirement.engine == engine
                            && requirement.port == Some(port)
                            && user.is_none_or(|user| {
                                read_credentials(requirement).user.as_deref() == Some(user)
                            })
                    })
            })
            .map(|(path, _)| {
                let name = self
                    .projects
                    .iter()
                    .find(|project| project.path == *path)
                    .map_or_else(
                        || path.display().to_string(),
                        |project| project.name.clone(),
                    );
                (path.clone(), name)
            })
            .collect()
    }

    /// Before the processes of a project start: every local database it needs must accept
    /// its login (CA-5). Returns `Waiting` when a confirmation or a preparation took over.
    pub(super) fn project_database_gate(
        &mut self,
        project_path: &Path,
        environment: &ProjectEnvironment,
    ) -> DatabaseGate {
        for status in &environment.databases {
            let requirement = &status.requirement;
            if requirement.engine == DatabaseEngine::SqlServer && local_server(requirement) {
                match self.sql_server_gate(project_path, status) {
                    Some(gate) => return gate,
                    None => continue,
                }
            }
            let Some(engine) = engine_key(requirement.engine).filter(|_| local_server(requirement))
            else {
                continue;
            };
            let credentials = read_credentials(requirement);
            // A fresh test: the card may be from an earlier check.
            let test = test_connection(requirement, &credentials, DATABASE_LOGIN_TIMEOUT);
            let outcome = test.as_ref().map(|test| test.outcome.clone());
            if outcome == Some(ConnectionOutcome::Connected) {
                continue;
            }
            let Some(port) = requirement.port else {
                continue;
            };
            let status_now = DatabaseStatus {
                requirement: requirement.clone(),
                available: status.available,
                connection: test.clone(),
                managed: None,
            };
            let root = database_root();
            let managed = root
                .as_deref()
                .and_then(|root| database_servers::find_server(root, engine, port));
            let can_manage = root.is_some()
                && database_packages::package_for_this_system(requirement.engine).is_some();
            let listener = localcodepilot_core::ports::Port::new(port)
                .and_then(localcodepilot_platform::ports::listener_process_id)
                .is_some();
            let port_taken = format!("A porta {port} {PORT_TAKEN}");
            // Something that is not a database the test can read answers on the port.
            if managed.is_none()
                && listener
                && matches!(
                    outcome,
                    Some(ConnectionOutcome::Failed(_) | ConnectionOutcome::TimedOut)
                )
            {
                return DatabaseGate::Blocked(DatabaseFailure {
                    reason: port_taken,
                    fix: can_manage.then_some(DatabaseFix::OtherPort),
                    detail_fix: None,
                    retry: false,
                    details: vec![],
                });
            }
            // A database that is not the app's answers and refuses: explain; another port
            // is an explicit option in "Detalhes técnicos" only.
            if managed.is_none()
                && matches!(
                    outcome,
                    Some(ConnectionOutcome::AccessDenied | ConnectionOutcome::DatabaseMissing)
                )
            {
                return DatabaseGate::Blocked(DatabaseFailure {
                    reason: status_now.rejection().unwrap_or_default(),
                    fix: None,
                    detail_fix: can_manage.then_some(DatabaseFix::OtherPort),
                    retry: false,
                    details: vec![
                        "O servidor nesta porta não é o do LocalCodePilot; nada foi alterado nele."
                            .into(),
                    ],
                });
            }
            let fixable = can_manage
                && (outcome == Some(ConnectionOutcome::NotResponding)
                    || (managed.is_some()
                        && matches!(
                            outcome,
                            Some(
                                ConnectionOutcome::DatabaseMissing
                                    | ConnectionOutcome::AccessDenied
                            )
                        )));
            if !fixable {
                return DatabaseGate::Blocked(DatabaseFailure::plain(
                    status_now
                        .rejection()
                        .or_else(|| {
                            status_now
                                .not_running()
                                .then(|| "O banco de dados deste projeto não está rodando.".into())
                        })
                        .unwrap_or_else(|| status_now.card_state(None)),
                ));
            }
            let (Some(database), Some(user)) =
                (credentials.database.clone(), credentials.user.clone())
            else {
                return DatabaseGate::Blocked(DatabaseFailure::plain(status_now.card_state(None)));
            };
            if !database_servers::creatable_name(&database)
                || !database_servers::creatable_name(&user)
            {
                return DatabaseGate::Blocked(DatabaseFailure::plain(
                    ServerError::InvalidName.to_string(),
                ));
            }
            if managed.is_none() && listener {
                return DatabaseGate::Blocked(DatabaseFailure {
                    reason: port_taken,
                    fix: Some(DatabaseFix::OtherPort),
                    detail_fix: None,
                    retry: false,
                    details: vec![],
                });
            }
            let package = database_packages::package_for_this_system(requirement.engine)
                .expect("checked above");
            let root = root.expect("checked above");
            let missing_runtime = if package.is_prepared(&root) {
                localcodepilot_platform::vc_runtime::missing_runtime(
                    &package.folder(&root).join(&package.server_program),
                    runtime_dlls(engine),
                )
            } else {
                vec![]
            };
            let request = DatabaseRequest {
                project: project_path.to_path_buf(),
                requirement: requirement.clone(),
                package,
                port,
                database,
                user,
                password: credentials.password_for_setup(),
                new_server: managed.is_none(),
                missing_runtime,
            };
            if request.new_server {
                // First time: the person decides (CA-3).
                self.database.confirmation = Some(request);
            } else {
                self.spawn_job(project_path, request.engine_label(), true, move |sender| {
                    run_start_job(&request, sender)
                });
            }
            return DatabaseGate::Waiting;
        }
        DatabaseGate::Ready
    }

    /// SQL Server on this computer: `None` when the project's login is accepted. The app's
    /// container starts after a click on "Iniciar"; a new one needs the terms accepted.
    fn sql_server_gate(
        &mut self,
        project_path: &Path,
        status: &DatabaseStatus,
    ) -> Option<DatabaseGate> {
        let requirement = &status.requirement;
        let test = test_connection(
            requirement,
            &read_credentials(requirement),
            DATABASE_LOGIN_TIMEOUT,
        );
        let outcome = test.as_ref().map(|test| test.outcome.clone());
        if outcome == Some(ConnectionOutcome::Connected) {
            return None;
        }
        let port = requirement.port?;
        let status_now = DatabaseStatus {
            requirement: requirement.clone(),
            available: status.available,
            connection: test,
            managed: None,
        };
        let plain_reason = || {
            status_now
                .rejection()
                .or_else(|| {
                    status_now
                        .not_running()
                        .then(|| "O banco de dados deste projeto não está rodando.".into())
                })
                .unwrap_or_else(|| status_now.card_state(None))
        };
        let Some(root) = database_root().filter(|_| engine_preparable_here(requirement.engine))
        else {
            return Some(DatabaseGate::Blocked(
                DatabaseFailure::plain(plain_reason()),
            ));
        };
        if let Some(server) = sql_server_container::find(&root, port) {
            let credentials = read_credentials(requirement);
            let (Some(database), Some(user)) =
                (credentials.database.clone(), credentials.user.clone())
            else {
                return Some(DatabaseGate::Blocked(
                    DatabaseFailure::plain(plain_reason()),
                ));
            };
            let requirement = requirement.clone();
            let password = credentials.password_for_setup();
            let path = self.environment_path.clone();
            self.spawn_job(project_path, "SQL Server", true, move |sender| {
                run_sql_start_job(
                    &server,
                    &requirement,
                    &database,
                    &user,
                    &password,
                    path.as_deref(),
                    sender,
                )
            });
            return Some(DatabaseGate::Waiting);
        }
        if matches!(
            outcome,
            Some(ConnectionOutcome::AccessDenied | ConnectionOutcome::DatabaseMissing)
        ) {
            return Some(DatabaseGate::Blocked(DatabaseFailure {
                reason: plain_reason(),
                fix: None,
                detail_fix: Some(DatabaseFix::OtherPort),
                retry: false,
                details: vec![
                    "O servidor nesta porta não é o do LocalCodePilot; nada foi alterado nele."
                        .into(),
                ],
            }));
        }
        let listener = localcodepilot_core::ports::Port::new(port)
            .and_then(localcodepilot_platform::ports::listener_process_id)
            .is_some();
        if listener {
            return Some(DatabaseGate::Blocked(DatabaseFailure {
                reason: format!("A porta {port} {PORT_TAKEN}"),
                fix: Some(DatabaseFix::OtherPort),
                detail_fix: None,
                retry: false,
                details: vec![],
            }));
        }
        if outcome != Some(ConnectionOutcome::NotResponding) {
            return Some(DatabaseGate::Blocked(
                DatabaseFailure::plain(plain_reason()),
            ));
        }
        Some(
            match self.sql_server_request(project_path, requirement, None) {
                Ok(request) => {
                    // A new container: the person reads and accepts the terms first.
                    self.database.sql_confirmation = Some(request);
                    DatabaseGate::Waiting
                }
                Err(failure) => DatabaseGate::Blocked(failure),
            },
        )
    }

    /// What the SQL Server confirmation needs: the project's names and a ready engine.
    fn sql_server_request(
        &self,
        project_path: &Path,
        requirement: &DatabaseRequirement,
        replaces: Option<SqlServerContainer>,
    ) -> Result<SqlServerRequest, DatabaseFailure> {
        let credentials = read_credentials(requirement);
        let (Some(database), Some(user), Some(port)) = (
            credentials.database.clone(),
            credentials.user.clone(),
            requirement.port,
        ) else {
            return Err(DatabaseFailure::plain(
                "Faltam o nome do banco ou o usuário na configuração deste projeto.",
            ));
        };
        if !database_servers::creatable_name(&database) || !database_servers::creatable_name(&user)
        {
            return Err(DatabaseFailure::plain(ServerError::InvalidName.to_string()));
        }
        let path = self.environment_path.clone();
        let Some(engine) = ready_container_engine(path.as_deref()) else {
            return Err(DatabaseFailure {
                reason: NO_CONTAINER_ENGINE.into(),
                fix: Some(DatabaseFix::OpenContainers),
                detail_fix: None,
                retry: false,
                details: vec![],
            });
        };
        Ok(SqlServerRequest {
            project: project_path.to_path_buf(),
            requirement: requirement.clone(),
            engine,
            port,
            database,
            user,
            password: credentials.password_for_setup(),
            replaces,
            accepted: false,
            path,
        })
    }

    fn spawn_job(
        &mut self,
        project: &Path,
        engine: &'static str,
        visible: bool,
        work: impl FnOnce(&Sender<JobEvent>) -> JobResult + Send + 'static,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        if visible {
            self.database.failures.remove(project);
            self.database.notes.remove(project);
        }
        self.database.jobs.insert(
            project.to_path_buf(),
            DatabaseJob {
                engine,
                step: DatabaseStep::Preparing,
                visible,
                receiver,
            },
        );
        std::thread::spawn(move || {
            // A panic in a library (a TLS backend, a driver) must not leave the card on
            // "Preparando…" forever: it becomes a failure the card can show.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&sender)))
                .unwrap_or_else(|panic| {
                    JobResult::Failed(JobFailure::interrupted(panic_text(panic.as_ref())))
                });
            let _ = sender.send(JobEvent::Done(result));
        });
    }

    /// A start stopped at the database: remembered on the card. A `.env` the app changed for
    /// this start goes back first, only if it is still what the app wrote.
    pub(super) fn record_database_failure(&mut self, project: &Path, mut failure: DatabaseFailure) {
        if let Some(change) = self.database.port_changes.remove(project) {
            match restore_env(&change) {
                Ok(()) => failure.details.push(format!(
                    "A porta voltou para {} em {}; a cópia continua em {}.",
                    change.from,
                    display_path(&change.env),
                    display_path(&change.backup)
                )),
                Err(why) => {
                    failure.details.insert(0, why);
                    failure
                        .details
                        .push(format!("Cópia: {}", display_path(&change.backup)));
                    failure.reason = ENV_NOT_RESTORED.into();
                    failure.fix = None;
                }
            }
        }
        self.notify(failure.reason.clone());
        self.database
            .failures
            .insert(project.to_path_buf(), failure);
    }

    /// Reads the progress of database jobs; a finished preparation starts the project again.
    pub(super) fn poll_database_jobs(&mut self, ctx: &egui::Context) {
        if !self.database.checked_left_running {
            self.database.checked_left_running = true;
            self.check_databases_left_running();
        }
        let mut finished = vec![];
        for (project, job) in &mut self.database.jobs {
            loop {
                match job.receiver.try_recv() {
                    Ok(JobEvent::Step(step)) => job.step = step,
                    Ok(JobEvent::Done(result)) => {
                        finished.push((project.clone(), result));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    // The job's thread ended without a result: never "Preparando…" forever.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished.push((
                            project.clone(),
                            JobResult::Failed(JobFailure::interrupted(
                                "a tarefa do banco de dados terminou sem resultado".into(),
                            )),
                        ));
                        break;
                    }
                }
            }
        }
        if !self.database.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        for (project, result) in finished {
            self.database.jobs.remove(&project);
            match result {
                JobResult::Ready {
                    server_id,
                    actions,
                    new_data,
                    note,
                } => {
                    if !self.database.session_servers.contains(&server_id) {
                        self.database.session_servers.push(server_id);
                    }
                    self.database.damaged_starts.remove(&project);
                    let mut lines = actions;
                    if let Some(change) = self.database.port_changes.remove(&project) {
                        lines.push(format!(
                            "porta alterada de {} para {} em {}; cópia em {}",
                            change.from,
                            change.to,
                            display_path(&change.env),
                            display_path(&change.backup)
                        ));
                    }
                    if let Some(kept) = new_data {
                        lines.push(format!(
                            "banco novo criado; os dados anteriores estão em {}",
                            display_path(&kept)
                        ));
                        self.database
                            .notes
                            .insert(project.clone(), NEW_DATA_NOTE.into());
                    }
                    if let Some(note) = note {
                        self.database.notes.insert(project.clone(), note);
                    }
                    self.database.actions.insert(project.clone(), lines);
                    self.refresh_project_environment(&project);
                    self.start_project(&project);
                }
                JobResult::Failed(failure) => {
                    let failure = self.failure_from_job(&project, failure);
                    self.record_database_failure(&project, failure);
                    self.refresh_project_environment(&project);
                }
                JobResult::Stopped(result) => {
                    match result {
                        Ok(()) => self.notify("Banco de dados parado."),
                        Err(error) => self.notify(error),
                    }
                    self.refresh_project_environment(&project);
                }
                JobResult::Tested(test) => {
                    if let Some(test) = test {
                        self.database.remote_tests.insert(project, test);
                    }
                }
            }
        }
    }

    /// Turns a job failure into the card's reason and, where safe, its fix.
    fn failure_from_job(&mut self, project: &Path, failure: JobFailure) -> DatabaseFailure {
        let mut result = DatabaseFailure::plain(failure.message.clone());
        result.details.extend(failure.details.iter().cloned());
        // The app's server did not start: another try may pass (a locked file, an antivirus).
        result.retry = failure.server_id.is_some() && failure.outcome.is_none();
        match failure.recovery {
            Some(DatabaseFix::OpenContainers) => {
                result.retry = false;
                result.fix = Some(DatabaseFix::OpenContainers);
            }
            Some(DatabaseFix::NewSqlServer) => {
                result.retry = false;
                result.detail_fix = Some(DatabaseFix::NewSqlServer);
                result.details.push(
                    "Criar SQL Server novo cria outro container e outro volume; o volume atual é mantido, com os dados."
                        .into(),
                );
            }
            _ => {}
        }
        if failure.damaged {
            let count = self
                .database
                .damaged_starts
                .entry(project.to_path_buf())
                .or_default();
            *count += 1;
            // Only after two starts that failed with recognized damage.
            if *count >= 2 {
                result.fix = Some(DatabaseFix::NewData);
            }
            result
                .details
                .push("O registro do servidor indica dados danificados.".into());
        } else {
            self.database.damaged_starts.remove(project);
        }
        if failure.outcome == Some(ConnectionOutcome::AccessDenied) && failure.server_id.is_some() {
            result.reason = "O banco não aceitou os dados de acesso deste projeto.".into();
            let shared = self
                .project_environments
                .get(project)
                .map(Self::managed_requirements)
                .unwrap_or_default()
                .first()
                .map(|requirement| {
                    let user = read_credentials(requirement).user;
                    self.projects_sharing(
                        project,
                        requirement.engine,
                        requirement.port.unwrap_or_default(),
                        user.as_deref(),
                    )
                })
                .unwrap_or_default();
            if shared.is_empty() {
                result.fix = Some(DatabaseFix::FixPassword);
            } else {
                let names: Vec<_> = shared.into_iter().map(|(_, name)| name).collect();
                result.details.push(format!(
                    "O usuário também é usado por: {}. Corrija a senha na configuração deste projeto.",
                    names.join(", ")
                ));
            }
        }
        result
    }

    fn refresh_project_environment(&mut self, project_path: &Path) {
        if let Some(project) = self
            .projects
            .iter()
            .find(|project| project.path == project_path)
        {
            let environment = inspect_project_environment(
                project,
                &self.managed.processes,
                self.environment_path.as_deref(),
            );
            self.project_environments
                .insert(project_path.to_path_buf(), environment);
        }
    }

    /// The card's database extras, for the grid.
    pub(super) fn database_card_extras(&self, project_path: &Path) -> DatabaseCardExtras {
        let mut extras = DatabaseCardExtras {
            failure: self.database.failures.get(project_path).cloned(),
            remote_test: self.database.remote_tests.get(project_path).cloned(),
            lines: self
                .database
                .actions
                .get(project_path)
                .cloned()
                .unwrap_or_default(),
            ..DatabaseCardExtras::default()
        };
        let Some(environment) = self.project_environments.get(project_path) else {
            return extras;
        };
        for status in &environment.databases {
            let requirement = &status.requirement;
            if !local_server(requirement)
                && requirement.compose_service.is_none()
                && looked_after(requirement.engine)
                && requirement.host.is_some()
            {
                extras.remote = true;
            }
            if let Some(managed) = &status.managed {
                extras.lines.push(managed.summary.clone());
                if managed.running {
                    extras.stop_blocked_by = self
                        .projects_sharing(
                            project_path,
                            requirement.engine,
                            requirement.port.unwrap_or_default(),
                            None,
                        )
                        .into_iter()
                        .filter(|(path, _)| self.managed.project_is_running(path))
                        .map(|(_, name)| name)
                        .collect();
                    extras.can_stop = !self.managed.project_is_running(project_path)
                        && extras.stop_blocked_by.is_empty();
                }
            }
        }
        extras
    }

    /// A fix button of the card or of "Detalhes técnicos": opens its confirmation.
    pub(super) fn ask_database_fix(&mut self, project_path: &Path, fix: DatabaseFix) {
        let Some(requirement) = self
            .project_environments
            .get(project_path)
            .map(Self::managed_requirements)
            .and_then(|requirements| requirements.into_iter().next())
        else {
            return;
        };
        let Some(port) = requirement.port else {
            return;
        };
        let sql = requirement.engine == DatabaseEngine::SqlServer;
        let sql_server = database_root()
            .filter(|_| sql)
            .and_then(|root| sql_server_container::find(&root, port));
        match fix {
            DatabaseFix::OpenContainers => {
                self.touch_project(project_path);
                self.focused_project = Some(project_path.to_path_buf());
                self.page = Page::Containers;
                return;
            }
            DatabaseFix::NewSqlServer => {
                match self.sql_server_request(project_path, &requirement, sql_server) {
                    Ok(request) => self.database.sql_confirmation = Some(request),
                    Err(failure) => self.record_database_failure(project_path, failure),
                }
                return;
            }
            _ => {}
        }
        let engine = engine_key(requirement.engine).unwrap_or_default();
        let server = database_root()
            .as_deref()
            .and_then(|root| database_servers::find_server(root, engine, port))
            .map(|server| server.id)
            .or_else(|| sql_server.map(|server| format!("{SQL_PREFIX}{}", server.id)));
        self.database.fix_confirmation = match fix {
            DatabaseFix::OtherPort => {
                let Some(to) = free_port_after(requirement.engine.default_port().unwrap_or(port))
                else {
                    self.notify("Não foi encontrada uma porta livre para o banco deste projeto.");
                    return;
                };
                Some(FixConfirmation::OtherPort {
                    project: project_path.to_path_buf(),
                    env: requirement.source.clone(),
                    from: port,
                    to,
                })
            }
            DatabaseFix::FixPassword => server.map(|server_id| FixConfirmation::FixPassword {
                project: project_path.to_path_buf(),
                server_id,
                user: read_credentials(&requirement).user.unwrap_or_default(),
                requirement: requirement.clone(),
            }),
            DatabaseFix::NewData => {
                server
                    .filter(|server_id| sql_id(server_id).is_none())
                    .map(|server_id| FixConfirmation::NewData {
                        project: project_path.to_path_buf(),
                        server_id,
                        requirement: requirement.clone(),
                    })
            }
            DatabaseFix::OpenContainers | DatabaseFix::NewSqlServer => None,
        };
    }

    /// "Parar o banco de dados" in "Detalhes técnicos": only the app's server, only when no
    /// project that uses it is running.
    pub(super) fn stop_project_database(&mut self, project_path: &Path) {
        let extras = self.database_card_extras(project_path);
        if !extras.can_stop {
            return;
        }
        let Some(server) = self
            .project_environments
            .get(project_path)
            .and_then(|environment| {
                environment
                    .databases
                    .iter()
                    .find_map(|status| status.managed.clone())
            })
        else {
            return;
        };
        if let Some(id) = sql_id(&server.id).map(str::to_owned) {
            let path = self.environment_path.clone();
            self.spawn_job(project_path, "", false, move |_| {
                let Some(server) = database_root().and_then(|root| {
                    sql_server_container::load_registry(&root)
                        .into_iter()
                        .find(|known| known.id == id)
                }) else {
                    return JobResult::Stopped(Err("o servidor não está mais registrado".into()));
                };
                JobResult::Stopped(sql_server_container::stop(&server, path.as_deref()))
            });
            return;
        }
        self.spawn_job(project_path, "", false, move |_| {
            let Some(root) = database_root() else {
                return JobResult::Stopped(Err(
                    "a pasta do LocalCodePilot não foi encontrada".into()
                ));
            };
            let Some(mut server) = database_servers::load_registry(&root)
                .into_iter()
                .find(|known| known.id == server.id)
            else {
                return JobResult::Stopped(Err("o servidor não está mais registrado".into()));
            };
            JobResult::Stopped(
                database_servers::stop(&mut server, &root, &SystemVault)
                    .map_err(|error| error.to_string()),
            )
        });
    }

    /// "Testar conexão" of a remote database, in "Detalhes técnicos".
    pub(super) fn test_remote_database(&mut self, project_path: &Path) {
        let Some(requirement) =
            self.project_environments
                .get(project_path)
                .and_then(|environment| {
                    environment
                        .databases
                        .iter()
                        .map(|status| status.requirement.clone())
                        .find(|requirement| {
                            !local_server(requirement) && requirement.compose_service.is_none()
                        })
                })
        else {
            return;
        };
        self.spawn_job(project_path, "", false, move |_| {
            JobResult::Tested(test_remote_connection(
                &requirement,
                &read_credentials(&requirement),
                Duration::from_secs(10),
            ))
        });
    }

    /// The first-time confirmation (CA-3), with the texts agreed with product.
    pub(super) fn show_database_confirmation(&mut self, root_ui: &mut egui::Ui) {
        self.show_fix_confirmation(root_ui);
        self.show_sql_server_confirmation(root_ui);
        let Some(request) = self.database.confirmation.clone() else {
            return;
        };
        let mut open = true;
        let mut cancel = false;
        let mut confirm = false;
        let mut retry = false;
        let shown = egui::Window::new("Preparar banco de dados para este projeto?")
            .id(egui::Id::unique("database-prepare-confirmation"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(520.0)
            .show(root_ui, |ui| {
                let size = size_label(request.package.size_bytes);
                ui.label(format!(
                    "O LocalCodePilot vai baixar {} ({size}), criar o banco {} e o usuário {}. O servidor aceitará conexões só deste computador. O programa e os dados ficarão na pasta do LocalCodePilot. Ao fechar o aplicativo, tentaremos encerrar o servidor; os dados permanecerão.",
                    request.engine_label(),
                    request.database,
                    request.user
                ));
                ui.add_space(6.0);
                ui.label(RichText::new(format!("Download: {size}")).color(theme::MUTED));
                ui.label(
                    RichText::new(format!(
                        "Espaço necessário após o preparo: {}",
                        size_label(request.package.space_bytes)
                    ))
                    .color(theme::MUTED),
                );
                egui::CollapsingHeader::new("Detalhes técnicos")
                    .id_salt("database-prepare-details")
                    .show(ui, |ui| {
                        let folder = database_root().unwrap_or_default();
                        for line in [
                            format!(
                                "{} {} em 127.0.0.1:{}",
                                request.engine_label(),
                                request.package.version,
                                request.port
                            ),
                            format!("Pasta: {}", display_path(&folder)),
                            format!("Configuração: {}", display_path(&request.requirement.source)),
                            format!("Origem do pacote: {}", request.package.url),
                            format!("SHA-256 conferido: {}", request.package.sha256),
                        ] {
                            ui.label(RichText::new(line).color(theme::MUTED).size(12.0));
                        }
                    });
                ui.add_space(10.0);
                if !request.missing_runtime.is_empty() {
                    ui.colored_label(
                        theme::WARNING,
                        format!(
                            "Para preparar {}, este computador precisa do Microsoft Visual C++ Redistributable. Instale-o pelo site oficial e depois escolha Tentar de novo.",
                            request.engine_label()
                        ),
                    );
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    let not_now = ui.button("Agora não");
                    focus_safe_choice_once(ui, PREPARE_FOCUS, &not_now);
                    if not_now.clicked() {
                        cancel = true;
                    }
                    if request.missing_runtime.is_empty() {
                        if ui
                            .add(egui::Button::new("Preparar e iniciar").fill(theme::PRIMARY))
                            .clicked()
                        {
                            confirm = true;
                        }
                    } else if ui.button("Tentar de novo").clicked() {
                        retry = true;
                    }
                });
            });
        if escape_pressed(root_ui) {
            cancel = true;
        }
        if !(confirm || retry || cancel || !open) {
            super::notice::keep_focus_inside(
                root_ui.ctx(),
                shown.as_ref().map(|shown| &shown.response),
            );
        }
        if confirm || retry || cancel || !open {
            forget_first_focus(root_ui, PREPARE_FOCUS);
        }
        let project = request.project.clone();
        if confirm {
            self.database.confirmation = None;
            self.spawn_job(&project, request.engine_label(), true, move |sender| {
                run_start_job(&request, sender)
            });
        } else if retry {
            self.database.confirmation = None;
            self.start_project(&project);
        } else if cancel || !open {
            self.database.confirmation = None;
            // A `.env` changed for this start goes back when the person gives up.
            if let Some(change) = self.database.port_changes.remove(&project)
                && let Err(why) = restore_env(&change)
            {
                let mut failure = DatabaseFailure::plain(ENV_NOT_RESTORED);
                failure.details = vec![why, format!("Cópia: {}", display_path(&change.backup))];
                self.database.failures.insert(project, failure);
            }
        }
    }

    /// A new SQL Server container: the person reads and accepts Microsoft's license terms
    /// (never accepted by the app) and sees where the data stays.
    fn show_sql_server_confirmation(&mut self, root_ui: &mut egui::Ui) {
        let Some(mut request) = self.database.sql_confirmation.clone() else {
            return;
        };
        match sql_server_confirmation_window(root_ui, &mut request) {
            SqlChoice::Cancel => self.database.sql_confirmation = None,
            SqlChoice::Waiting => self.database.sql_confirmation = Some(request),
            SqlChoice::Confirm => {
                self.database.sql_confirmation = None;
                let project = request.project.clone();
                self.spawn_job(&project, "SQL Server", true, move |sender| {
                    run_sql_prepare_job(&request, sender)
                });
            }
        }
    }
    fn show_fix_confirmation(&mut self, root_ui: &mut egui::Ui) {
        let Some(confirmation) = self.database.fix_confirmation.clone() else {
            return;
        };
        let (title, text, button) = match &confirmation {
            FixConfirmation::OtherPort { from, to, .. } => (
                "Usar outra porta?",
                format!(
                    "Para conectar este projeto ao banco do LocalCodePilot, vamos mudar a porta de {from} para {to} em .env. Guardaremos uma cópia da configuração atual. Isso pode fazer o projeto usar outro banco de dados."
                ),
                "Alterar e iniciar",
            ),
            FixConfirmation::FixPassword { user, .. } => (
                "Corrigir conexão?",
                format!(
                    "Vamos trocar, no banco gerenciado pelo LocalCodePilot, a senha do usuário {user} pela senha configurada neste projeto. Outros projetos que usam esse usuário podem precisar atualizar a conexão."
                ),
                "Corrigir e iniciar",
            ),
            FixConfirmation::NewData { .. } => (
                "Criar banco novo?",
                "Não foi possível iniciar o banco com os dados atuais. Podemos guardar a pasta atual e criar outra. O projeto poderá abrir com um banco vazio; seus dados anteriores continuarão guardados.".to_owned(),
                "Criar banco novo",
            ),
        };
        let mut open = true;
        let mut cancel = false;
        let mut confirm = false;
        egui::Window::new(title)
            .id(egui::Id::unique("database-fix-confirmation"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(500.0)
            .show(root_ui, |ui| {
                ui.label(text);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let not_now = ui.button("Agora não");
                    focus_safe_choice_once(ui, FIX_FOCUS, &not_now);
                    if not_now.clicked() {
                        cancel = true;
                    }
                    if ui
                        .add(egui::Button::new(button).fill(theme::PRIMARY))
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if escape_pressed(root_ui) {
            cancel = true;
        }
        if confirm || cancel || !open {
            forget_first_focus(root_ui, FIX_FOCUS);
        }
        if cancel || !open {
            self.database.fix_confirmation = None;
            return;
        }
        if !confirm {
            return;
        }
        self.database.fix_confirmation = None;
        match confirmation {
            FixConfirmation::OtherPort {
                project,
                env,
                from,
                to,
            } => match change_env_port(&env, from, to) {
                Ok(change) => {
                    self.database.failures.remove(&project);
                    self.database.port_changes.insert(project.clone(), change);
                    self.refresh_project_environment(&project);
                    self.start_project(&project);
                }
                Err(why) => self.record_database_failure(&project, DatabaseFailure::plain(why)),
            },
            FixConfirmation::FixPassword {
                project,
                server_id,
                requirement,
                user,
            } => {
                let engine = requirement.engine.label();
                self.spawn_job(&project, engine, true, move |_| {
                    run_fix_password_job(&server_id, &requirement, &user)
                });
            }
            FixConfirmation::NewData {
                project,
                server_id,
                requirement,
            } => {
                let engine = requirement.engine.label();
                self.spawn_job(&project, engine, true, move |sender| {
                    run_new_data_job(&server_id, &requirement, sender)
                });
            }
        }
    }

    /// On close: the clean shutdown of every server the app started. One that does not stop
    /// in time is left running and remembered, never forced.
    pub(super) fn stop_session_databases(&mut self) {
        let Some(root) = database_root() else {
            return;
        };
        for id in std::mem::take(&mut self.database.session_servers) {
            if let Some(id) = sql_id(&id) {
                if let Some(server) = sql_server_container::load_registry(&root)
                    .into_iter()
                    .find(|server| server.id == id)
                    && sql_server_container::stop(&server, self.environment_path.as_deref())
                        .is_err()
                {
                    let _ = sql_server_container::mark_stop_failed(&root, &server, true);
                }
                continue;
            }
            let Some(mut server) = database_servers::load_registry(&root)
                .into_iter()
                .find(|server| server.id == id)
            else {
                continue;
            };
            if database_servers::stop(&mut server, &root, &SystemVault).is_err() {
                let _ = database_servers::mark_stop_failed(&root, &server, true);
            }
        }
    }

    /// At start: a server that did not stop when the app last closed is reported only if it
    /// is still the app's and still running; otherwise the mark is cleared.
    pub(super) fn check_databases_left_running(&mut self) {
        let Some(root) = database_root() else {
            return;
        };
        for server in database_servers::load_registry(&root)
            .into_iter()
            .filter(|server| server.stop_failed)
        {
            match database_servers::recognize(&server) {
                Recognition::Managed => {
                    self.database.session_servers.push(server.id.clone());
                    self.notify(
                        "O banco de dados não parou quando o aplicativo foi fechado e ainda está rodando.",
                    );
                }
                Recognition::NotRunning | Recognition::Existing { .. } => {
                    let _ = database_servers::mark_stop_failed(&root, &server, false);
                }
            }
        }
        for server in sql_server_container::load_registry(&root)
            .into_iter()
            .filter(|server| server.stop_failed)
        {
            if sql_server_container::state(&server, self.environment_path.as_deref())
                == Ok(ContainerState::Running)
            {
                self.database
                    .session_servers
                    .push(format!("{SQL_PREFIX}{}", server.id));
                self.notify(
                    "O SQL Server não parou quando o aplicativo foi fechado e ainda está rodando.",
                );
            } else {
                let _ = sql_server_container::mark_stop_failed(&root, &server, false);
            }
        }
    }
}

const PREPARE_FOCUS: &str = "database-prepare-confirmation";
const FIX_FOCUS: &str = "database-fix-confirmation";
const SQL_FOCUS: &str = "sql-server-confirmation";

fn first_focus_id(key: &'static str) -> egui::Id {
    egui::Id::unique(("first-focus", key))
}

/// The database confirmations open with the focus on the safe choice, as the app's other
/// confirmations do: opened from the keyboard, it would stay on the button behind the
/// window. Tab reaches the main action.
fn focus_safe_choice_once(ui: &egui::Ui, key: &'static str, button: &egui::Response) {
    let id = first_focus_id(key);
    if !ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id))
        .unwrap_or(false)
    {
        button.request_focus();
        ui.ctx().data_mut(|data| data.insert_temp(id, true));
    }
}

/// A closed confirmation forgets it was focused, so the next one starts on the safe choice.
fn forget_first_focus(ui: &egui::Ui, key: &'static str) {
    ui.ctx()
        .data_mut(|data| data.remove::<bool>(first_focus_id(key)));
}

/// Escape cancels a database confirmation, as "Agora não" does.
fn escape_pressed(ui: &egui::Ui) -> bool {
    ui.input(|input| input.key_pressed(egui::Key::Escape))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlChoice {
    Waiting,
    Cancel,
    /// Only with the terms accepted.
    Confirm,
}

/// "Preparar SQL Server para este projeto?": what is downloaded, where the data stays, the
/// terms checkbox (unchecked) and "Preparar e iniciar", enabled only once it is checked.
fn sql_server_confirmation_window(
    root_ui: &mut egui::Ui,
    request: &mut SqlServerRequest,
) -> SqlChoice {
    let engine_name = if request.engine == "podman" {
        "Podman"
    } else {
        "Docker"
    };
    let mut open = true;
    let mut cancel = false;
    let mut confirm = false;
    egui::Window::new("Preparar SQL Server para este projeto?")
        .id(egui::Id::unique("sql-server-confirmation"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_width(540.0)
        .show(root_ui, |ui| {
            ui.label(format!(
                "O LocalCodePilot vai baixar a imagem oficial do SQL Server ({}) e criar um container com o {engine_name}. O SQL Server Developer é gratuito para desenvolvimento e teste, nos termos da Microsoft. Os dados ficarão num volume do {engine_name}. Ao fechar o aplicativo, o LocalCodePilot tentará parar o container.",
                sql_server_container::APPROXIMATE_DOWNLOAD
            ));
            if let Some(old) = &request.replaces {
                ui.add_space(6.0);
                ui.label(format!(
                    "O projeto passará a usar um banco vazio. Os dados anteriores continuam no volume {}.",
                    old.volume()
                ));
            }
            ui.label(
                RichText::new(format!(
                    "Espaço em disco depois do preparo: {}",
                    sql_server_container::APPROXIMATE_SPACE
                ))
                .color(theme::MUTED),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "Banco {} e usuário {}, com acesso só a esse banco.",
                    request.database, request.user
                ))
                .color(theme::MUTED),
            );
            egui::CollapsingHeader::new("Detalhes técnicos")
                .id_salt("sql-server-confirmation-details")
                .show(ui, |ui| {
                    for line in [
                        format!(
                            "{} em 127.0.0.1:{}",
                            sql_server_container::VERSION_LABEL,
                            request.port
                        ),
                        format!("Imagem: {}", sql_server_container::IMAGE),
                        "Container e volume: localcodepilot-mssql-<identificador>".to_owned(),
                        format!("Configuração: {}", display_path(&request.requirement.source)),
                        "Quem administra o motor de containers consegue ver a senha inicial do administrador, que o LocalCodePilot troca logo depois do preparo.".to_owned(),
                    ] {
                        ui.label(RichText::new(line).color(theme::MUTED).size(12.0));
                    }
                });
            ui.add_space(8.0);
            ui.checkbox(
                &mut request.accepted,
                "Li e aceito os termos de licença do Microsoft SQL Server",
            );
            ui.hyperlink_to(
                "Ler os termos de licença (site da Microsoft)",
                sql_server_container::TERMS_URL,
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let not_now = ui.button("Agora não");
                focus_safe_choice_once(ui, SQL_FOCUS, &not_now);
                if not_now.clicked() {
                    cancel = true;
                }
                if ui
                    .add_enabled(
                        request.accepted,
                        egui::Button::new("Preparar e iniciar").fill(theme::PRIMARY),
                    )
                    .clicked()
                {
                    confirm = true;
                }
            });
        });
    if escape_pressed(root_ui) {
        cancel = true;
    }
    let confirm = confirm && request.accepted;
    if confirm || cancel || !open {
        forget_first_focus(root_ui, SQL_FOCUS);
    }
    if cancel || !open {
        SqlChoice::Cancel
    } else if confirm {
        SqlChoice::Confirm
    } else {
        SqlChoice::Waiting
    }
}
/// The first free port after `default` that no server of the app uses either.
fn free_port_after(default: u16) -> Option<u16> {
    let registered: Vec<u16> = database_root()
        .map(|root| {
            database_servers::load_registry(&root)
                .into_iter()
                .map(|server| server.port)
                .collect()
        })
        .unwrap_or_default();
    (default.saturating_add(1)..default.saturating_add(100)).find(|port| {
        !registered.contains(port)
            && std::net::TcpListener::bind(("127.0.0.1", *port)).is_ok()
            && localcodepilot_core::ports::Port::new(*port)
                .and_then(localcodepilot_platform::ports::listener_process_id)
                .is_none()
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `contents` with `DB_PORT` set to `to` (the line is added when missing). Nothing else
/// changes, line endings included.
fn with_port(contents: &str, to: u16) -> String {
    let newline = if contents.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut found = false;
    let mut lines: Vec<String> = contents
        .split(newline)
        .map(|line| {
            let key = line
                .trim_start()
                .strip_prefix("export ")
                .unwrap_or(line.trim_start());
            if key.split('=').next().map(str::trim) == Some("DB_PORT") && !found {
                found = true;
                format!("DB_PORT={to}")
            } else {
                line.to_owned()
            }
        })
        .collect();
    if !found {
        if lines.last().is_some_and(String::is_empty) {
            lines.pop();
            lines.push(format!("DB_PORT={to}"));
            lines.push(String::new());
        } else {
            lines.push(format!("DB_PORT={to}"));
        }
    }
    lines.join(newline)
}

/// "Usar outra porta": a copy of `.env` next to it, then only `DB_PORT` changes.
fn change_env_port(env: &Path, from: u16, to: u16) -> Result<PortChange, String> {
    let original = fs::read(env)
        .map_err(|error| format!("não foi possível ler {}: {error}", display_path(env)))?;
    let moment = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let name = env
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".env".into());
    let backup = env.with_file_name(format!("{name}.localcodepilot-{moment}"));
    fs::write(&backup, &original)
        .map_err(|error| format!("não foi possível guardar a cópia: {error}"))?;
    let changed = with_port(&String::from_utf8_lossy(&original), to);
    fs::write(env, changed.as_bytes())
        .map_err(|error| format!("não foi possível alterar {}: {error}", display_path(env)))?;
    Ok(PortChange {
        env: env.to_path_buf(),
        backup,
        written: sha256_hex(changed.as_bytes()),
        from,
        to,
    })
}

/// Puts the original `.env` back, only if the file is still exactly what the app wrote: a
/// later edit by the person is never overwritten.
fn restore_env(change: &PortChange) -> Result<(), String> {
    let current = fs::read(&change.env)
        .map_err(|error| format!("não foi possível ler a configuração: {error}"))?;
    if sha256_hex(&current) != change.written {
        return Err("A configuração foi alterada depois da troca de porta; o LocalCodePilot não a sobrescreveu.".into());
    }
    let original = fs::read(&change.backup)
        .map_err(|error| format!("não foi possível ler a cópia: {error}"))?;
    fs::write(&change.env, &original)
        .map_err(|error| format!("não foi possível restaurar a configuração: {error}"))?;
    let restored = fs::read(&change.env)
        .map_err(|error| format!("não foi possível conferir a configuração: {error}"))?;
    if restored == original {
        Ok(())
    } else {
        Err("a configuração restaurada não confere com a cópia".into())
    }
}

/// The project's login, which decides whether it can start.
fn project_login(requirement: &DatabaseRequirement) -> Option<ConnectionTest> {
    test_connection(
        requirement,
        &read_credentials(requirement),
        DATABASE_LOGIN_TIMEOUT,
    )
}

fn login_failure(
    requirement: &DatabaseRequirement,
    test: Option<ConnectionTest>,
    server_id: Option<String>,
) -> JobResult {
    let outcome = test.as_ref().map(|test| test.outcome.clone());
    let status = DatabaseStatus {
        requirement: requirement.clone(),
        available: Some(true),
        connection: test,
        managed: None,
    };
    JobResult::Failed(JobFailure {
        message: status
            .rejection()
            .unwrap_or_else(|| status.card_state(None)),
        outcome,
        server_id,
        damaged: false,
        recovery: None,
        details: vec![],
    })
}

fn registered(root: &Path, id: &str) -> Option<ManagedServer> {
    database_servers::load_registry(root)
        .into_iter()
        .find(|server| server.id == id)
}

/// Prepare or start: package, server, the project's database and user, and the project's
/// own login, which decides.
fn run_start_job(request: &DatabaseRequest, sender: &Sender<JobEvent>) -> JobResult {
    let Some(root) = database_root() else {
        return JobResult::Failed(JobFailure::text(
            "a pasta de dados do aplicativo não foi encontrada",
        ));
    };
    let package = request.package;
    if !package.is_prepared(&root) {
        let archive = match package.download(&root, |received, total| {
            // The last bytes are followed by the SHA-256 check of the whole file.
            let step = if total > 0 && received >= total {
                DatabaseStep::Verifying
            } else {
                DatabaseStep::Downloading { received, total }
            };
            let _ = sender.send(JobEvent::Step(step));
        }) {
            Ok(archive) => archive,
            Err(error) => return JobResult::Failed(JobFailure::text(error.to_string())),
        };
        let _ = sender.send(JobEvent::Step(DatabaseStep::Preparing));
        if let Err(error) = package.prepare(&archive, &root) {
            return JobResult::Failed(JobFailure::text(error.to_string()));
        }
        // The extracted package is what is kept; the zip is not needed any more.
        let _ = fs::remove_file(&archive);
        let missing = localcodepilot_platform::vc_runtime::missing_runtime(
            &package.folder(&root).join(&package.server_program),
            runtime_dlls(&package.engine),
        );
        if !missing.is_empty() {
            return JobResult::Failed(JobFailure::text(format!(
                "Para preparar {}, este computador precisa do Microsoft Visual C++ Redistributable. Instale-o pelo site oficial e depois escolha Tentar de novo.",
                request.engine_label()
            )));
        }
    }
    let vault = SystemVault;
    let server = match database_servers::find_server(&root, &package.engine, request.port) {
        Some(mut server) => {
            match database_servers::recognize(&server) {
                Recognition::Managed => {}
                Recognition::NotRunning => {
                    let _ = sender.send(JobEvent::Step(DatabaseStep::StartingServer));
                    if let Err(error) = database_servers::start(&mut server, &root) {
                        let damaged = matches!(&error, ServerError::StartFailed { log, .. }
                            if database_servers::log_shows_damage(&server.engine, log));
                        return JobResult::Failed(JobFailure {
                            message: error.to_string(),
                            outcome: None,
                            server_id: Some(server.id.clone()),
                            damaged,
                            recovery: None,
                            details: vec![],
                        });
                    }
                }
                Recognition::Existing { reason } => {
                    return JobResult::Failed(JobFailure::text(
                        ServerError::NotManaged(reason).to_string(),
                    ));
                }
            }
            server
        }
        None => {
            let _ = sender.send(JobEvent::Step(DatabaseStep::Preparing));
            match database_servers::prepare(package, &root, request.port, &vault) {
                Ok(server) => server,
                Err(error) => return JobResult::Failed(JobFailure::text(error.to_string())),
            }
        }
    };
    let actions = match database_servers::ensure_project_database(
        &server,
        &vault,
        &request.database,
        &request.user,
        &request.password,
    ) {
        Ok(actions) => actions,
        Err(error) => {
            let mut failure = JobFailure::text(error.to_string());
            failure.server_id = Some(server.id.clone());
            return JobResult::Failed(failure);
        }
    };
    let test = project_login(&request.requirement);
    match test.as_ref().map(|test| &test.outcome) {
        Some(ConnectionOutcome::Connected) => JobResult::Ready {
            server_id: server.id,
            actions,
            new_data: None,
            note: None,
        },
        _ => login_failure(&request.requirement, test, Some(server.id)),
    }
}

/// "Corrigir conexão": the user's password on the app's server becomes the project's.
fn run_fix_password_job(
    server_id: &str,
    requirement: &DatabaseRequirement,
    user: &str,
) -> JobResult {
    let Some(root) = database_root() else {
        return JobResult::Failed(JobFailure::text(
            "a pasta de dados do aplicativo não foi encontrada",
        ));
    };
    let password = read_credentials(requirement).password_for_setup();
    if let Some(id) = sql_id(server_id) {
        let Some(server) = sql_server_container::load_registry(&root)
            .into_iter()
            .find(|server| server.id == id)
        else {
            return JobResult::Failed(JobFailure::text("o servidor não está mais registrado"));
        };
        if let Err(error) =
            sql_server_container::set_project_password(&server, &SystemVault, user, &password)
        {
            return JobResult::Failed(JobFailure::sql(error, Some(server_id.to_owned())));
        }
        let test = project_login(requirement);
        return match test.as_ref().map(|test| &test.outcome) {
            Some(ConnectionOutcome::Connected) => JobResult::Ready {
                server_id: server_id.to_owned(),
                actions: vec![format!(
                    "senha do login {user} atualizada no SQL Server do LocalCodePilot"
                )],
                new_data: None,
                note: None,
            },
            _ => login_failure(requirement, test, Some(server_id.to_owned())),
        };
    }
    let Some(server) = registered(&root, server_id) else {
        return JobResult::Failed(JobFailure::text("o servidor não está mais registrado"));
    };
    if let Err(error) =
        database_servers::set_project_password(&server, &SystemVault, user, &password)
    {
        return JobResult::Failed(JobFailure::text(error.to_string()));
    }
    let test = project_login(requirement);
    match test.as_ref().map(|test| &test.outcome) {
        Some(ConnectionOutcome::Connected) => JobResult::Ready {
            server_id: server.id,
            actions: vec![format!(
                "senha do usuário {user} atualizada no servidor do LocalCodePilot"
            )],
            new_data: None,
            note: None,
        },
        _ => login_failure(requirement, test, Some(server.id)),
    }
}

/// "Criar banco novo": keeps the damaged folder, creates the server again, then the
/// project's database and user, and tests the login.
fn run_new_data_job(
    server_id: &str,
    requirement: &DatabaseRequirement,
    sender: &Sender<JobEvent>,
) -> JobResult {
    let Some(root) = database_root() else {
        return JobResult::Failed(JobFailure::text(
            "a pasta de dados do aplicativo não foi encontrada",
        ));
    };
    let Some(mut server) = registered(&root, server_id) else {
        return JobResult::Failed(JobFailure::text("o servidor não está mais registrado"));
    };
    let _ = sender.send(JobEvent::Step(DatabaseStep::Preparing));
    let vault = SystemVault;
    let kept = match database_servers::recreate_with_new_data(&mut server, &root, &vault) {
        Ok(kept) => kept,
        Err(error) => return JobResult::Failed(JobFailure::text(error.to_string())),
    };
    let credentials = read_credentials(requirement);
    let (Some(database), Some(user)) = (credentials.database.clone(), credentials.user.clone())
    else {
        return login_failure(requirement, project_login(requirement), Some(server.id));
    };
    let mut actions = match database_servers::ensure_project_database(
        &server,
        &vault,
        &database,
        &user,
        &credentials.password_for_setup(),
    ) {
        Ok(actions) => actions,
        Err(error) => return JobResult::Failed(JobFailure::text(error.to_string())),
    };
    let test = project_login(requirement);
    match test.as_ref().map(|test| &test.outcome) {
        Some(ConnectionOutcome::Connected) => {
            actions.insert(0, "pasta de dados nova criada".into());
            JobResult::Ready {
                server_id: server.id,
                actions,
                new_data: Some(kept),
                note: None,
            }
        }
        _ => login_failure(requirement, test, Some(server.id)),
    }
}

/// The project's database and login on the app's container, then the project's own login.
fn sql_project_ready(
    server: &SqlServerContainer,
    requirement: &DatabaseRequirement,
    database: &str,
    user: &str,
    password: &str,
    mut actions: Vec<String>,
    note: Option<String>,
) -> JobResult {
    let server_id = format!("{SQL_PREFIX}{}", server.id);
    if let Err(error) = sql_server_container::ensure_project_database(
        server,
        &SystemVault,
        database,
        user,
        password,
    ) {
        return JobResult::Failed(JobFailure::sql(error, Some(server_id)));
    }
    let test = project_login(requirement);
    match test.as_ref().map(|test| &test.outcome) {
        Some(ConnectionOutcome::Connected) => {
            actions.push(format!(
                "banco {database} e login {user} conferidos no SQL Server; o login do projeto tem acesso só ao banco {database}"
            ));
            JobResult::Ready {
                server_id,
                actions,
                new_data: None,
                note,
            }
        }
        _ => login_failure(requirement, test, Some(server_id)),
    }
}

/// "Iniciar" with the app's SQL Server container already created.
fn run_sql_start_job(
    server: &SqlServerContainer,
    requirement: &DatabaseRequirement,
    database: &str,
    user: &str,
    password: &str,
    path: Option<&std::ffi::OsStr>,
    sender: &Sender<JobEvent>,
) -> JobResult {
    let _ = sender.send(JobEvent::Step(DatabaseStep::StartingServer));
    if let Err(error) = sql_server_container::start(server, &SystemVault, path) {
        return JobResult::Failed(JobFailure::sql(
            error,
            Some(format!("{SQL_PREFIX}{}", server.id)),
        ));
    }
    sql_project_ready(server, requirement, database, user, password, vec![], None)
}

/// After the terms were accepted: pull, create the container, change the initial password,
/// then the project's database and login. "Criar SQL Server novo" first stops the old
/// container (confirmed) and lets go of it; its volume keeps the data.
fn run_sql_prepare_job(request: &SqlServerRequest, sender: &Sender<JobEvent>) -> JobResult {
    let Some(root) = database_root() else {
        return JobResult::Failed(JobFailure::text(
            "a pasta de dados do aplicativo não foi encontrada",
        ));
    };
    let path = request.path.as_deref();
    let mut actions = vec![];
    let mut note = None;
    if let Some(old) = &request.replaces {
        if let Err(error) = sql_server_container::stop(old, path) {
            return JobResult::Failed(JobFailure::sql(error, None));
        }
        if let Err(error) = sql_server_container::release(&root, &old.id) {
            return JobResult::Failed(JobFailure::text(error));
        }
        actions.push(format!(
            "SQL Server novo criado; os dados anteriores continuam no volume {} do {}",
            old.volume(),
            old.engine
        ));
        note = Some(format!(
            "O projeto está conectado a um SQL Server novo. Os dados anteriores continuam no volume {}.",
            old.volume()
        ));
    }
    let _ = sender.send(JobEvent::Step(DatabaseStep::Downloading {
        received: 0,
        total: 0,
    }));
    let server = match sql_server_container::prepare(
        &root,
        sql_server_container::ContainerKind::SqlServer,
        request.engine,
        request.port,
        &SystemVault,
        path,
        || {
            let _ = sender.send(JobEvent::Step(DatabaseStep::StartingServer));
        },
    ) {
        Ok(server) => server,
        Err(error) => {
            // A container that was created stays registered: "Tentar de novo" resumes it.
            let created = sql_server_container::find(&root, request.port)
                .map(|server| format!("{SQL_PREFIX}{}", server.id));
            return JobResult::Failed(JobFailure::sql(error, created));
        }
    };
    actions.insert(
        0,
        format!(
            "container {} e volume {} criados no {}",
            server.container(),
            server.volume(),
            server.engine
        ),
    );
    sql_project_ready(
        &server,
        &request.requirement,
        &request.database,
        &request.user,
        &request.password,
        actions,
        note,
    )
}

/// A project's database, as the last check of the project saw it.
#[derive(Debug, Clone)]
pub(super) struct DatabaseStatus {
    pub requirement: DatabaseRequirement,
    /// SQLite: the file is fine. Servers: something answers on the configured local port.
    /// `None`: not checked (remote host, Compose service, unknown port).
    pub available: Option<bool>,
    /// The authenticated login with the project's own access data, when the app tests it.
    pub connection: Option<ConnectionTest>,
    /// The app's own server for this engine and port, when there is one.
    pub managed: Option<ManagedInfo>,
}

/// The app's server behind a project's database, as the last check saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManagedInfo {
    pub id: String,
    /// Running and recognized as the app's (PID, start time, program, data, PID file).
    pub running: bool,
    /// For "Detalhes técnicos".
    pub summary: String,
}

impl DatabaseStatus {
    fn is_server(&self) -> bool {
        self.requirement.engine != DatabaseEngine::Sqlite
    }

    /// The server does not answer: before any start the project stays "Parado".
    pub(super) fn not_running(&self) -> bool {
        self.is_server()
            && (self.available == Some(false)
                || matches!(
                    self.connection.as_ref().map(|test| &test.outcome),
                    Some(ConnectionOutcome::NotResponding)
                ))
    }

    /// A server on this computer that is not a Compose service: the kind the app prepares.
    fn local_server(&self) -> bool {
        self.requirement.compose_service.is_none()
            && self
                .requirement
                .host
                .as_deref()
                .and_then(localcodepilot_runtime::database_connection::loopback_address)
                .is_some()
    }

    /// The app can prepare and start this database itself (a package for the engine on this
    /// system, a host on this computer, not a Compose service): "Iniciar" leads there.
    pub(super) fn app_can_prepare(&self) -> bool {
        self.local_server() && engine_preparable_here(self.requirement.engine)
    }

    /// A local server the app prepares elsewhere but not on this system (PostgreSQL on macOS
    /// and Linux, MariaDB on macOS...): the card names the engine and says so, with the reason
    /// in "Detalhes técnicos" (T029, CA-6). Never another engine in its place.
    pub(super) fn not_preparable_here(&self) -> Option<String> {
        let engine = self.requirement.engine;
        if !self.not_running()
            || !self.local_server()
            || engine == DatabaseEngine::SqlServer
            || engine_preparable_here(engine)
        {
            return None;
        }
        Some(format!(
            "O {} deste projeto não está rodando. O LocalCodePilot ainda não consegue prepará-lo neste sistema.",
            engine.label()
        ))
    }

    /// The server answered but did not let the project in: "Com problema", with the reason.
    pub(super) fn rejection(&self) -> Option<String> {
        let test = self.connection.as_ref()?;
        match &test.outcome {
            ConnectionOutcome::AccessDenied => Some(
                "O banco respondeu, mas não aceitou os dados de acesso deste projeto. Confira a configuração de conexão.".into(),
            ),
            ConnectionOutcome::DatabaseMissing => Some(format!(
                "O banco respondeu, mas o banco de dados {} não foi encontrado. Confira a configuração de conexão.",
                test.database.as_deref().unwrap_or_default()
            )),
            _ => None,
        }
    }

    /// What follows "Banco: {motor} · " on the card. Only an authenticated login says
    /// "Banco de dados conectado."; a port or a running container proves nothing.
    pub(super) fn card_state(&self, container_running: Option<bool>) -> String {
        if !self.is_server() {
            return match self.available {
                Some(true) => "arquivo íntegro",
                Some(false) => "arquivo será preparado ao iniciar",
                None => "arquivo não verificado",
            }
            .into();
        }
        if self.requirement.compose_service.is_some() || container_running.is_some() {
            return match container_running {
                Some(true) => "Container ativo; conexão não verificada.",
                Some(false) => "container do banco parado",
                None => "serviço do Compose; inicie os containers",
            }
            .into();
        }
        match self.connection.as_ref().map(|test| &test.outcome) {
            Some(ConnectionOutcome::Connected) => "Banco de dados conectado.".into(),
            Some(ConnectionOutcome::AccessDenied | ConnectionOutcome::DatabaseMissing) => {
                self.rejection().unwrap_or_default()
            }
            Some(ConnectionOutcome::NotResponding) => {
                "O banco de dados deste projeto não está rodando.".into()
            }
            Some(ConnectionOutcome::InsufficientData) => {
                "Conexão não verificada: faltam dados de acesso que o aplicativo consiga ler."
                    .into()
            }
            Some(ConnectionOutcome::TimedOut) => {
                "Conexão não verificada: o banco não respondeu a tempo.".into()
            }
            Some(ConnectionOutcome::Failed(_)) => {
                "Conexão não verificada: o teste falhou (veja Detalhes técnicos).".into()
            }
            None if self.available == Some(false) => {
                "O banco de dados deste projeto não está rodando.".into()
            }
            None => "conexão não verificada".into(),
        }
    }

    /// "Detalhes técnicos": configuration, address actually tested, result and when, never the
    /// password.
    pub(super) fn technical(&self) -> Vec<String> {
        let requirement = &self.requirement;
        let mut lines = vec![format!(
            "Banco {}: configurado em {} como {}:{}",
            requirement.engine.label(),
            display_path(&requirement.source),
            requirement.host.as_deref().unwrap_or("?"),
            requirement
                .port
                .map_or_else(|| "?".to_owned(), |port| port.to_string()),
        )];
        if let Some(test) = &self.connection {
            let when = test
                .at
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| relative_time(elapsed.as_secs()))
                .unwrap_or_default();
            let result = match &test.outcome {
                ConnectionOutcome::Connected => "conectado".to_owned(),
                ConnectionOutcome::AccessDenied => "acesso recusado pelo servidor".to_owned(),
                ConnectionOutcome::DatabaseMissing => "o banco de dados não existe".to_owned(),
                ConnectionOutcome::NotResponding => "nada respondeu no endereço".to_owned(),
                ConnectionOutcome::TimedOut => "tempo esgotado".to_owned(),
                ConnectionOutcome::InsufficientData => {
                    format!("não testado: {}", test.insufficient.join(", "))
                }
                ConnectionOutcome::Failed(message) => format!("falhou: {message}"),
            };
            lines.push(format!(
                "Conexão testada em {} com o usuário {} no banco {} ({when}): {result}",
                test.target,
                test.user.as_deref().unwrap_or("?"),
                test.database.as_deref().unwrap_or("?"),
            ));
        }
        if self.not_preparable_here().is_some()
            && let Err(reason) = database_packages::availability(requirement.engine)
        {
            lines.push(format!("Preparo neste sistema: {reason}"));
        }
        lines
    }
}

/// The app's server for a local database, looked up during the project check.
pub(super) fn managed_info(requirement: &DatabaseRequirement) -> Option<ManagedInfo> {
    if requirement.engine == DatabaseEngine::SqlServer {
        use localcodepilot_runtime::sql_server_container::{self, ContainerState};
        let root = database_ui::database_root()?;
        let server = sql_server_container::find(&root, requirement.port?)?;
        let state = sql_server_container::state(&server, None);
        let running = state == Ok(ContainerState::Running);
        let state_text = match &state {
            Ok(ContainerState::Running) => "rodando".to_owned(),
            Ok(ContainerState::Stopped) => "parado".to_owned(),
            Ok(ContainerState::Missing) => "container não encontrado".to_owned(),
            Ok(ContainerState::NotOurs(reason)) => reason.clone(),
            Err(_) => "estado não confirmado".to_owned(),
        };
        return Some(ManagedInfo {
            summary: format!(
                "SQL Server gerenciado pelo LocalCodePilot: {} em 127.0.0.1:{} ({state_text}); container {} e volume {} no {}",
                sql_server_container::VERSION_LABEL,
                server.port,
                server.container(),
                server.volume(),
                server.engine
            ),
            id: format!("mssql:{}", server.id),
            running,
        });
    }
    let engine = match requirement.engine {
        DatabaseEngine::PostgreSql => "postgresql",
        DatabaseEngine::MySql => "mysql",
        DatabaseEngine::MariaDb => "mariadb",
        _ => return None,
    };
    let root = database_ui::database_root()?;
    let server =
        localcodepilot_runtime::database_servers::find_server(&root, engine, requirement.port?)?;
    let running = localcodepilot_runtime::database_servers::recognize(&server)
        == localcodepilot_runtime::database_servers::Recognition::Managed;
    Some(ManagedInfo {
        summary: format!(
            "Servidor gerenciado pelo LocalCodePilot: {} {} em 127.0.0.1:{} ({}); dados em {}",
            requirement.engine.label(),
            server.version,
            server.port,
            if running { "rodando" } else { "parado" },
            display_path(&server.folder)
        ),
        id: server.id,
        running,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders the SQL Server confirmation without a screen and reads its accessibility tree:
    /// (choice, [(name, disabled)]).
    fn sql_confirmation_tree(request: &mut SqlServerRequest) -> (SqlChoice, Vec<(String, bool)>) {
        let (choice, nodes, _) = sql_confirmation_frames(request, &[]);
        (choice, nodes)
    }

    /// Three frames of the SQL Server confirmation (a window settles its size in the first
    /// ones); `last_events` reach the last frame. Returns the accessible names, whether each
    /// is disabled, and the name of the focused widget.
    fn sql_confirmation_frames(
        request: &mut SqlServerRequest,
        last_events: &[egui::Event],
    ) -> (SqlChoice, Vec<(String, bool)>, Option<String>) {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut choice = SqlChoice::Waiting;
        let mut nodes = vec![];
        let mut focused = None;
        for frame in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 900.0),
                    )),
                    events: if frame == 2 {
                        last_events.to_vec()
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ui| choice = sql_server_confirmation_window(ui, request),
            );
            output.textures_delta.clear();
            if let Some(update) = output.platform_output.accesskit_update {
                focused = update
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == update.focus)
                    .and_then(|(_, node)| node.label().map(str::to_owned));
                nodes = update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        node.label()
                            .or(node.value())
                            .map(|name| (name.to_owned(), node.is_disabled()))
                    })
                    .collect();
            }
            if choice != SqlChoice::Waiting {
                break;
            }
        }
        (choice, nodes, focused)
    }

    #[test]
    fn a_database_confirmation_starts_on_the_safe_choice_and_escape_cancels_it() {
        let mut request = SqlServerRequest {
            project: PathBuf::from("loja"),
            requirement: DatabaseRequirement {
                source: PathBuf::from("loja").join(".env"),
                engine: DatabaseEngine::SqlServer,
                host: Some("127.0.0.1".into()),
                port: Some(14339),
                compose_service: None,
            },
            engine: "podman",
            port: 14339,
            database: "loja".into(),
            user: "app_loja".into(),
            password: "x".into(),
            replaces: None,
            accepted: true,
            path: None,
        };
        let (choice, _, focused) = sql_confirmation_frames(&mut request, &[]);
        assert_eq!(choice, SqlChoice::Waiting);
        assert_eq!(focused.as_deref(), Some("Agora não"));
        // Even with the terms accepted, Escape gives up instead of preparing.
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (choice, _, _) = sql_confirmation_frames(&mut request, &[escape]);
        assert_eq!(choice, SqlChoice::Cancel);
    }

    #[test]
    fn an_interrupted_job_becomes_a_failure_without_retry() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("no process-level CryptoProvider");
        let failure = JobFailure::interrupted(panic_text(payload.as_ref()));
        assert_eq!(failure.message, JOB_INTERRUPTED);
        assert_eq!(
            failure.details,
            vec!["A tarefa do banco de dados foi interrompida: no process-level CryptoProvider"]
        );
        // No server known to be safe to resume: no "Tentar de novo", Iniciar checks again.
        assert_eq!(failure.server_id, None);
        let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("outro"));
        assert_eq!(panic_text(payload.as_ref()), "outro");
        let payload: Box<dyn std::any::Any + Send> = Box::new(7);
        assert_eq!(panic_text(payload.as_ref()), "erro interno sem mensagem");
    }

    #[test]
    fn the_sql_server_terms_are_never_accepted_for_the_person() {
        let mut request = SqlServerRequest {
            project: PathBuf::from("loja"),
            requirement: DatabaseRequirement {
                source: PathBuf::from("loja").join(".env"),
                engine: DatabaseEngine::SqlServer,
                host: Some("127.0.0.1".into()),
                port: Some(14339),
                compose_service: None,
            },
            engine: "docker",
            port: 14339,
            database: "loja".into(),
            user: "app_loja".into(),
            password: "x".into(),
            replaces: None,
            accepted: false,
            path: None,
        };
        let (choice, nodes) = sql_confirmation_tree(&mut request);
        let find = |nodes: &[(String, bool)], name: &str| {
            nodes
                .iter()
                .find(|(label, _)| label.contains(name))
                .cloned()
                .unwrap_or_else(|| panic!("{name} não está na janela: {nodes:?}"))
        };
        assert_eq!(choice, SqlChoice::Waiting);
        // What is downloaded and where the data stays, before any choice.
        find(
            &nodes,
            "baixar a imagem oficial do SQL Server (cerca de 500 MB)",
        );
        find(&nodes, "Os dados ficarão num volume do Docker");
        find(
            &nodes,
            "Banco loja e usuário app_loja, com acesso só a esse banco.",
        );
        find(&nodes, "Ler os termos de licença");
        // The box starts unchecked and "Preparar e iniciar" is disabled until it is checked.
        assert!(!request.accepted);
        find(
            &nodes,
            "Li e aceito os termos de licença do Microsoft SQL Server",
        );
        assert!(
            find(&nodes, "Preparar e iniciar").1,
            "deve estar desabilitado"
        );
        assert!(!find(&nodes, "Agora não").1);
        request.accepted = true;
        let (choice, nodes) = sql_confirmation_tree(&mut request);
        assert_eq!(choice, SqlChoice::Waiting, "marcar não inicia nada sozinho");
        assert!(
            !find(&nodes, "Preparar e iniciar").1,
            "habilitado depois do aceite"
        );
        // "Criar SQL Server novo" says the project starts empty and where the old data is.
        request.replaces = Some(SqlServerContainer {
            kind: Default::default(),
            id: "abc123".into(),
            engine: "docker".into(),
            port: 14339,
            stop_failed: false,
        });
        let (_, nodes) = sql_confirmation_tree(&mut request);
        find(
            &nodes,
            "Os dados anteriores continuam no volume localcodepilot-mssql-abc123.",
        );
    }

    #[test]
    fn sql_server_failures_offer_only_the_recovery_that_helps() {
        // The preparation failed after the container was created: the agreed sentence on the
        // card, the cause in "Detalhes técnicos", "Tentar de novo" resumes it.
        let failure = JobFailure::sql(
            format!(
                "{} (o SQL Server não ficou pronto: tempo esgotado)",
                sql_server_container::PREPARATION_FAILED
            ),
            Some("mssql:abc".into()),
        );
        assert_eq!(failure.message, sql_server_container::PREPARATION_FAILED);
        assert_eq!(
            failure.details,
            vec!["o SQL Server não ficou pronto: tempo esgotado"]
        );
        assert_eq!(failure.recovery, None);
        // The stop was not confirmed: "Abrir Containers", never "Tentar de novo".
        let failure = JobFailure::sql(
            format!(
                "{} (tempo esgotado)",
                sql_server_container::STOP_NOT_CONFIRMED
            ),
            None,
        );
        assert_eq!(failure.message, sql_server_container::STOP_NOT_CONFIRMED);
        assert_eq!(failure.recovery, Some(DatabaseFix::OpenContainers));
        // No kept password logs in: "Criar SQL Server novo" in "Detalhes técnicos".
        let failure = JobFailure::sql(
            "Nenhuma senha de administração guardada pelo LocalCodePilot entra neste SQL Server."
                .into(),
            Some("mssql:abc".into()),
        );
        assert_eq!(failure.recovery, Some(DatabaseFix::NewSqlServer));
    }

    fn stopped_server(engine: DatabaseEngine, host: &str, compose: Option<&str>) -> DatabaseStatus {
        DatabaseStatus {
            requirement: DatabaseRequirement {
                source: PathBuf::from(".env"),
                engine,
                host: Some(host.into()),
                port: Some(5432),
                compose_service: compose.map(str::to_owned),
            },
            available: Some(false),
            connection: None,
            managed: None,
        }
    }

    #[test]
    fn a_local_engine_without_a_package_here_is_named_and_never_offered() {
        for engine in [
            DatabaseEngine::PostgreSql,
            DatabaseEngine::MySql,
            DatabaseEngine::MariaDb,
        ] {
            let status = stopped_server(engine, "127.0.0.1", None);
            let here = database_packages::package_for_this_system(engine).is_some();
            assert_eq!(status.app_can_prepare(), here);
            match status.not_preparable_here() {
                None => assert!(here),
                Some(reason) => {
                    assert!(!here);
                    assert_eq!(
                        reason,
                        format!(
                            "O {} deste projeto não está rodando. O LocalCodePilot ainda não consegue prepará-lo neste sistema.",
                            engine.label()
                        )
                    );
                    assert!(
                        status
                            .technical()
                            .iter()
                            .any(|line| line.starts_with("Preparo neste sistema: "))
                    );
                }
            }
        }
        // Remote hosts and Compose services are not the app's to prepare anywhere.
        for status in [
            stopped_server(DatabaseEngine::PostgreSql, "db.empresa.example", None),
            stopped_server(DatabaseEngine::PostgreSql, "db", Some("db")),
        ] {
            assert!(!status.app_can_prepare());
            assert_eq!(status.not_preparable_here(), None);
        }
        // On Windows every engine has a package; PostgreSQL has none on macOS and Linux.
        assert_eq!(
            stopped_server(DatabaseEngine::PostgreSql, "localhost", None)
                .not_preparable_here()
                .is_some(),
            !cfg!(windows)
        );
    }

    #[test]
    fn sizes_read_like_the_confirmation_says_them() {
        assert_eq!(size_label(68_546_993), "65 MB");
        assert_eq!(size_label(1_650_000_000), "1,5 GB");
    }

    #[test]
    fn progress_shows_percentage_only_with_a_total() {
        assert_eq!(
            DatabaseStep::Downloading {
                received: 120 * 1024 * 1024,
                total: 290 * 1024 * 1024
            }
            .text("MySQL"),
            "Baixando MySQL: 41% (120 de 290 MB)"
        );
        assert_eq!(
            DatabaseStep::Downloading {
                received: 5,
                total: 0
            }
            .text("MySQL"),
            "Baixando MySQL..."
        );
        assert_eq!(
            DatabaseStep::Verifying.text("MariaDB"),
            "Verificando o pacote de MariaDB…"
        );
        assert_eq!(
            DatabaseStep::Preparing.text("MySQL"),
            "Preparando o banco de dados…"
        );
        assert_eq!(
            DatabaseStep::StartingServer.text("MySQL"),
            "Iniciando o banco de dados…"
        );
    }

    #[test]
    fn only_db_port_changes_and_line_endings_stay() {
        assert_eq!(
            with_port("APP=x\r\nDB_PORT=3306\r\nDB_USERNAME=app\r\n", 3307),
            "APP=x\r\nDB_PORT=3307\r\nDB_USERNAME=app\r\n"
        );
        assert_eq!(
            with_port("DB_HOST=127.0.0.1\n", 3307),
            "DB_HOST=127.0.0.1\nDB_PORT=3307\n"
        );
        assert_eq!(
            with_port("DB_HOST=127.0.0.1", 3307),
            "DB_HOST=127.0.0.1\nDB_PORT=3307"
        );
    }

    #[test]
    fn the_env_goes_back_only_if_it_is_still_what_the_app_wrote() {
        let folder = std::env::temp_dir().join(format!("lcp-env-port-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let env = folder.join(".env");
        fs::write(&env, "DB_PORT=3306\nDB_PASSWORD=x\n").unwrap();
        let change = change_env_port(&env, 3306, 3307).unwrap();
        assert_eq!(
            fs::read_to_string(&env).unwrap(),
            "DB_PORT=3307\nDB_PASSWORD=x\n"
        );
        assert_eq!(
            fs::read_to_string(&change.backup).unwrap(),
            "DB_PORT=3306\nDB_PASSWORD=x\n"
        );
        restore_env(&change).unwrap();
        assert_eq!(
            fs::read_to_string(&env).unwrap(),
            "DB_PORT=3306\nDB_PASSWORD=x\n"
        );
        // An edit after the change is never overwritten; the copy stays.
        let change = change_env_port(&env, 3306, 3307).unwrap();
        fs::write(&env, "DB_PORT=3307\nDB_PASSWORD=nova\n").unwrap();
        assert!(restore_env(&change).is_err());
        assert_eq!(
            fs::read_to_string(&env).unwrap(),
            "DB_PORT=3307\nDB_PASSWORD=nova\n"
        );
        assert!(change.backup.is_file());
        let _ = fs::remove_dir_all(&folder);
    }
}
