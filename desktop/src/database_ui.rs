//! "Iniciar" with the project's database (T019.04): before any process of the project starts,
//! a local PostgreSQL, MySQL or MariaDB it needs must accept the project's own login. On
//! Windows the app prepares (after a confirmation) or starts its own server, gives the project
//! its database and user, and tests the login again. Agreed with product in
//! `.internal/conversas/2026-11-20-t019-04-iniciar-com-preparo.md`.

use super::*;
use localcodepilot_runtime::{
    database_connection::loopback_address,
    database_packages::{self, DatabasePackage},
    database_servers::{self, Recognition, ServerError, SystemVault},
};
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

/// One decimal with a comma; GB from 1 GB on.
pub(super) fn size_label(bytes: u64) -> String {
    let megabytes = bytes as f64 / (1024.0 * 1024.0);
    if megabytes >= 1024.0 {
        format!("{:.1} GB", megabytes / 1024.0).replace('.', ",")
    } else {
        format!("{megabytes:.0} MB")
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

enum JobEvent {
    Step(DatabaseStep),
    Done(Result<(String, Vec<String>), String>),
}

pub(super) struct DatabaseJob {
    pub engine: &'static str,
    pub step: DatabaseStep,
    receiver: Receiver<JobEvent>,
}

/// The decision before starting a project's processes.
pub(super) enum DatabaseGate {
    Ready,
    /// A confirmation or a background job now owns the start.
    Waiting,
    Blocked(String),
}

#[derive(Default)]
pub(super) struct DatabaseState {
    pub confirmation: Option<DatabaseRequest>,
    pub jobs: HashMap<PathBuf, DatabaseJob>,
    /// Why the last start of a project stopped at its database (card: "Com problema").
    pub failures: HashMap<PathBuf, String>,
    /// Servers this app started (or found still running after a failed close); stopped on
    /// close.
    pub session_servers: Vec<String>,
    /// What was created for each project, for "Detalhes técnicos".
    pub actions: HashMap<PathBuf, Vec<String>>,
    /// Servers left running by the last close were looked at.
    checked_left_running: bool,
}

impl LocalCodePilot {
    /// Before the processes of a project start: every local database it needs must accept
    /// its login (CA-5). Returns `Waiting` when a confirmation or a preparation took over.
    pub(super) fn project_database_gate(
        &mut self,
        project_path: &Path,
        environment: &ProjectEnvironment,
    ) -> DatabaseGate {
        for status in &environment.databases {
            let requirement = &status.requirement;
            let local = requirement.compose_service.is_none()
                && requirement
                    .host
                    .as_deref()
                    .and_then(loopback_address)
                    .is_some();
            let Some(engine) = engine_key(requirement.engine).filter(|_| local) else {
                continue;
            };
            let credentials = read_credentials(requirement);
            // A fresh test: the card may be from an earlier check.
            let test = test_connection(requirement, &credentials, DATABASE_LOGIN_TIMEOUT);
            let outcome = test.as_ref().map(|test| test.outcome.clone());
            if outcome == Some(ConnectionOutcome::Connected) {
                continue;
            }
            let project = project_path.to_path_buf();
            let status_now = DatabaseStatus {
                requirement: requirement.clone(),
                available: status.available,
                connection: test.clone(),
            };
            let Some(port) = requirement.port else {
                continue;
            };
            let root = database_root();
            let managed = root
                .as_deref()
                .and_then(|root| database_servers::find_server(root, engine, port));
            let can_manage = cfg!(windows)
                && root.is_some()
                && database_packages::package_for_this_system(requirement.engine).is_some();
            let fixable = matches!(
                outcome,
                Some(ConnectionOutcome::NotResponding | ConnectionOutcome::DatabaseMissing)
            ) || (outcome == Some(ConnectionOutcome::AccessDenied)
                && managed.is_some());
            let port_taken_by_other = || {
                managed.is_none()
                    && localcodepilot_core::ports::Port::new(port)
                        .and_then(localcodepilot_platform::ports::listener_process_id)
                        .is_some()
            };
            if !can_manage || !fixable {
                // Something that is not a database (or not one the test can read) answers on
                // the port: say so, rather than "the test failed".
                let other_program = matches!(
                    outcome,
                    Some(ConnectionOutcome::Failed(_) | ConnectionOutcome::TimedOut)
                ) && port_taken_by_other();
                return DatabaseGate::Blocked(
                    status_now
                        .rejection()
                        .or_else(|| {
                            other_program.then(|| format!(
                                "A porta {port} já está em uso. O LocalCodePilot não iniciou outro banco para proteger seus dados."
                            ))
                        })
                        .or_else(|| {
                            status_now
                                .not_running()
                                .then(|| "O banco de dados deste projeto não está rodando.".into())
                        })
                        .unwrap_or_else(|| status_now.card_state(None)),
                );
            }
            // A server that answers but is not the app's is left alone.
            if outcome != Some(ConnectionOutcome::NotResponding) && managed.is_none() {
                return DatabaseGate::Blocked(status_now.rejection().unwrap_or_default());
            }
            let (Some(database), Some(user)) =
                (credentials.database.clone(), credentials.user.clone())
            else {
                return DatabaseGate::Blocked(status_now.card_state(None));
            };
            if !database_servers::creatable_name(&database)
                || !database_servers::creatable_name(&user)
            {
                return DatabaseGate::Blocked(ServerError::InvalidName.to_string());
            }
            if managed.is_none()
                && localcodepilot_core::ports::Port::new(port)
                    .and_then(localcodepilot_platform::ports::listener_process_id)
                    .is_some()
            {
                return DatabaseGate::Blocked(format!(
                    "A porta {port} já está em uso. O LocalCodePilot não iniciou outro banco para proteger seus dados."
                ));
            }
            let package = database_packages::package_for_this_system(requirement.engine)
                .expect("checked above");
            let root = root.expect("checked above");
            let missing_runtime = localcodepilot_platform::vc_runtime::missing_runtime(
                &package.folder(&root).join(&package.server_program),
                match engine {
                    "postgresql" => &["vcruntime140.dll"][..],
                    "mysql" => &["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"][..],
                    _ => &[][..],
                },
            );
            let request = DatabaseRequest {
                project,
                requirement: requirement.clone(),
                package,
                port,
                database,
                user,
                password: credentials.password_for_setup(),
                new_server: managed.is_none(),
                missing_runtime: if package.is_prepared(&root) {
                    missing_runtime
                } else {
                    vec![]
                },
            };
            if request.new_server {
                // First time: the person decides (CA-3).
                self.database.confirmation = Some(request);
            } else {
                self.start_database_job(request);
            }
            return DatabaseGate::Waiting;
        }
        DatabaseGate::Ready
    }

    fn start_database_job(&mut self, request: DatabaseRequest) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let engine = request.engine_label();
        let project = request.project.clone();
        self.database.failures.remove(&project);
        self.database.jobs.insert(
            project,
            DatabaseJob {
                engine,
                step: DatabaseStep::Preparing,
                receiver,
            },
        );
        std::thread::spawn(move || {
            let result = run_database_job(&request, &sender);
            let _ = sender.send(JobEvent::Done(result));
        });
    }

    /// Reads the progress of database preparations; a finished one starts the project again.
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
                Ok((server_id, actions)) => {
                    if !self.database.session_servers.contains(&server_id) {
                        self.database.session_servers.push(server_id);
                    }
                    self.database.actions.insert(project.clone(), actions);
                    self.refresh_project_environment(&project);
                    self.start_project(&project);
                }
                Err(reason) => {
                    self.notify(reason.clone());
                    self.database.failures.insert(project.clone(), reason);
                    self.refresh_project_environment(&project);
                }
            }
        }
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

    /// The first-time confirmation (CA-3), with the texts agreed with product.
    pub(super) fn show_database_confirmation(&mut self, root_ui: &mut egui::Ui) {
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
        if confirm {
            self.database.confirmation = None;
            self.start_database_job(request);
        } else if retry {
            self.database.confirmation = None;
            let project = request.project.clone();
            self.start_project(&project);
        } else if cancel || !open {
            self.database.confirmation = None;
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

/// The preparation, in the background: package, server, the project's database and user,
/// and the project's own login, which decides.
fn run_database_job(
    request: &DatabaseRequest,
    sender: &Sender<JobEvent>,
) -> Result<(String, Vec<String>), String> {
    let root = database_root().ok_or("a pasta de dados do Windows não foi encontrada")?;
    let package = request.package;
    if !package.is_prepared(&root) {
        let archive = package
            .download(&root, |received, total| {
                let _ = sender.send(JobEvent::Step(DatabaseStep::Downloading {
                    received,
                    total,
                }));
            })
            .map_err(|error| error.to_string())?;
        let _ = sender.send(JobEvent::Step(DatabaseStep::Preparing));
        package
            .prepare(&archive, &root)
            .map_err(|error| error.to_string())?;
        // The extracted package is what is kept; the zip is not needed any more.
        let _ = fs::remove_file(&archive);
    }
    let vault = SystemVault;
    let server = match database_servers::find_server(&root, &package.engine, request.port) {
        Some(mut server) => {
            match database_servers::recognize(&server) {
                Recognition::Managed => {}
                Recognition::NotRunning => {
                    let _ = sender.send(JobEvent::Step(DatabaseStep::StartingServer));
                    database_servers::start(&mut server, &root)
                        .map_err(|error| error.to_string())?;
                }
                Recognition::Existing { reason } => {
                    return Err(ServerError::NotManaged(reason).to_string());
                }
            }
            server
        }
        None => {
            let _ = sender.send(JobEvent::Step(DatabaseStep::Preparing));
            database_servers::prepare(package, &root, request.port, &vault)
                .map_err(|error| error.to_string())?
        }
    };
    let actions = database_servers::ensure_project_database(
        &server,
        &vault,
        &request.database,
        &request.user,
        &request.password,
    )
    .map_err(|error| error.to_string())?;
    let test = test_connection(
        &request.requirement,
        &read_credentials(&request.requirement),
        DATABASE_LOGIN_TIMEOUT,
    );
    let status = DatabaseStatus {
        requirement: request.requirement.clone(),
        available: Some(true),
        connection: test,
    };
    match status.connection.as_ref().map(|test| &test.outcome) {
        Some(ConnectionOutcome::Connected) => Ok((server.id, actions)),
        _ => Err(status
            .rejection()
            .unwrap_or_else(|| status.card_state(None))),
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
}
