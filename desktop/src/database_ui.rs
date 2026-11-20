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

use super::*;
use localcodepilot_runtime::{
    database_connection::{loopback_address, test_remote_connection},
    database_packages::{self, DatabasePackage},
    database_servers::{self, ManagedServer, Recognition, ServerError, SystemVault},
};
use sha2::{Digest, Sha256};
use std::sync::mpsc::{Receiver, Sender};

/// Where the app keeps database packages and servers: `%LOCALAPPDATA%\LocalCodePilot\bancos`.
pub(super) fn database_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|folder| PathBuf::from(folder).join("LocalCodePilot").join("bancos"))
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
}

impl DatabaseFix {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::OtherPort => "Usar outra porta",
            Self::FixPassword => "Corrigir conexão",
            Self::NewData => "Criar banco novo",
        }
    }
}

/// Why the last start stopped at the project's database, and what can be done.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DatabaseFailure {
    pub reason: String,
    /// The card's main action, when a fix is safe here.
    pub fix: Option<DatabaseFix>,
    /// "Usar outra porta" in "Detalhes técnicos" only: a database on the port refuses access.
    pub other_port_option: bool,
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
            other_port_option: false,
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
    Downloading { received: u64, total: u64 },
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
    },
    Failed(JobFailure),
    Stopped(Result<(), String>),
    Tested(Option<ConnectionTest>),
}

struct JobFailure {
    message: String,
    outcome: Option<ConnectionOutcome>,
    server_id: Option<String>,
    /// The app's server did not start and its log shows damaged data.
    damaged: bool,
}

impl JobFailure {
    fn text(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            outcome: None,
            server_id: None,
            damaged: false,
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
            .filter(|requirement| {
                local_server(requirement) && engine_key(requirement.engine).is_some()
            })
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

    fn project_is_running(&self, project_path: &Path) -> bool {
        project_process_ids(&self.processes, project_path, false)
            .iter()
            .filter_map(|id| self.processes.iter().find(|process| process.id == *id))
            .any(|process| {
                matches!(
                    process.state,
                    ProcessState::Running | ProcessState::Starting
                )
            })
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
            let can_manage = cfg!(windows)
                && root.is_some()
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
                    other_port_option: false,
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
                    other_port_option: can_manage,
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
                    other_port_option: false,
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
            let result = work(&sender);
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
            while let Ok(event) = job.receiver.try_recv() {
                match event {
                    JobEvent::Step(step) => job.step = step,
                    JobEvent::Done(result) => finished.push((project.clone(), result)),
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
        // The app's server did not start: another try may pass (a locked file, an antivirus).
        result.retry = failure.server_id.is_some() && failure.outcome.is_none();
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
                &self.processes,
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
                && engine_key(requirement.engine).is_some()
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
                        .filter(|(path, _)| self.project_is_running(path))
                        .map(|(_, name)| name)
                        .collect();
                    extras.can_stop =
                        !self.project_is_running(project_path) && extras.stop_blocked_by.is_empty();
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
        let engine = engine_key(requirement.engine).unwrap_or_default();
        let server = database_root()
            .as_deref()
            .and_then(|root| database_servers::find_server(root, engine, port));
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
            DatabaseFix::FixPassword => server.map(|server| FixConfirmation::FixPassword {
                project: project_path.to_path_buf(),
                server_id: server.id,
                user: read_credentials(&requirement).user.unwrap_or_default(),
                requirement: requirement.clone(),
            }),
            DatabaseFix::NewData => server.map(|server| FixConfirmation::NewData {
                project: project_path.to_path_buf(),
                server_id: server.id,
                requirement: requirement.clone(),
            }),
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
        let Some(request) = self.database.confirmation.clone() else {
            return;
        };
        let mut open = true;
        let mut cancel = false;
        let mut confirm = false;
        let mut retry = false;
        egui::Window::new("Preparar banco de dados para este projeto?")
            .id(egui::Id::unique("database-prepare-confirmation"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(520.0)
            .show(root_ui, |ui| {
                let size = size_label(request.package.size_bytes);
                ui.label(format!(
                    "O LocalCodePilot vai baixar {} ({size}), criar o banco {} e o usuário {}. Os dados ficarão na pasta do LocalCodePilot. Ao fechar o aplicativo, o LocalCodePilot tentará encerrar o servidor. Os dados permanecerão.",
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
                    if ui.button("Agora não").clicked() {
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
                    if ui.button("Agora não").clicked() {
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
            "a pasta de dados do Windows não foi encontrada",
        ));
    };
    let package = request.package;
    if !package.is_prepared(&root) {
        let archive = match package.download(&root, |received, total| {
            let _ = sender.send(JobEvent::Step(DatabaseStep::Downloading {
                received,
                total,
            }));
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
            "a pasta de dados do Windows não foi encontrada",
        ));
    };
    let Some(server) = registered(&root, server_id) else {
        return JobResult::Failed(JobFailure::text("o servidor não está mais registrado"));
    };
    let password = read_credentials(requirement).password_for_setup();
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
            "a pasta de dados do Windows não foi encontrada",
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
            }
        }
        _ => login_failure(requirement, test, Some(server.id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
