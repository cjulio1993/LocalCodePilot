use crate::{
    config, theme,
    updates::{self, UpdateInfo},
};
use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, RichText, Sense, Stroke};
use egui_phosphor::regular::{
    ARROW_SQUARE_OUT, BELL, CARET_RIGHT, CIRCLE, CODE_SIMPLE, FOLDER_OPEN, GEAR, LAYOUT, MEMORY,
    PLAY, PLUS, TERMINAL,
};
use localcodepilot_core::{
    discovery::DiscoveryService,
    environments::{
        InstallationPlan, InstallationStep, InstallationStepKind, RequirementStatus,
        RuntimeRequirement,
    },
    ports::{LocalServerUrl, Port, extract_local_urls, strip_terminal_sequences},
    processes::{ProcessState, ProjectProcess},
    projects::Project,
    runtimes::RuntimeKind,
    technologies::TechnologyKind,
};
use localcodepilot_platform::{
    NativePlatform, Platform,
    filesystem::FilesystemProjectSource,
    installations::{
        ProgramInventory, WindowsWingetProvider, find_executable, inspect_program,
        managed_bin_directory, refreshed_environment_path,
    },
    ports::{PortAvailability, PortOwner, inspect_tcp_port, terminate_process_tree},
};
use localcodepilot_runtime::{
    ManifestRuntimeDetector, detect_dart_requirement, detect_go_requirement,
    detect_java_requirement, detect_node_requirement, detect_php_extensions,
    detect_php_requirement, detect_processes, detect_python_requirement, node_package_manager_spec,
    node_version_matches, runtime_version_matches,
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
const NOTIFICATION_FADE_START: Duration = Duration::from_millis(3_200);
const PROCESS_START_TIMEOUT: Duration = Duration::from_secs(30);
const PROCESS_START_CHECK_INTERVAL: Duration = Duration::from_millis(500);
const PROCESS_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const PROCESS_PORT_LOSS_GRACE: Duration = Duration::from_secs(6);
const INSTALLATION_STEP_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Projects,
    Processes,
    Plugins,
    Settings,
}

impl Page {
    fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Projects => "projects",
            Self::Processes => "processes",
            Self::Plugins => "plugins",
            Self::Settings => "settings",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "projects" => Self::Projects,
            "processes" => Self::Processes,
            "plugins" => Self::Plugins,
            "settings" => Self::Settings,
            _ => Self::Overview,
        }
    }
}

struct RunningProcess {
    child: Child,
    output: Receiver<String>,
    logs: VecDeque<String>,
    urls: Vec<LocalServerUrl>,
    finished: bool,
    started_at: Instant,
    last_health_check: Instant,
    port_unavailable_since: Option<Instant>,
}

#[derive(Debug, Clone)]
struct PortConflict {
    port: Port,
    owner: Option<PortOwner>,
}

#[derive(Debug, Clone)]
struct PortTerminationRequest {
    process_id: String,
    conflict: PortConflict,
}

struct Notification {
    message: String,
    created_at: Instant,
}

enum ProcessAction {
    Start(String),
    Stop(String),
    Restart(String),
    StartProject(PathBuf),
    StopProject(PathBuf),
    PrepareEnvironment(PathBuf),
    InstallDependencies(PathBuf),
    ViewDependencyLogs(PathBuf),
    ClearLogs(String),
    ViewLogs(String),
    OpenUrl(String),
    ResolvePortConflict(String),
}

#[derive(Debug, Clone, Default)]
struct ProjectEnvironment {
    command_count: usize,
    missing_programs: Vec<String>,
    missing_dependencies: Vec<String>,
    install_steps: Vec<DependencyInstall>,
    runtime_requirements: Vec<RuntimeRequirement>,
    missing_php_extensions: Vec<String>,
    laravel_constraint: Option<String>,
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
            && self.missing_programs.is_empty()
            && self.missing_dependencies.is_empty()
            && self.runtime_requirements.iter().all(|requirement| {
                matches!(requirement.status, RequirementStatus::Available { .. })
            })
            && self.missing_php_extensions.is_empty()
    }

    fn status(&self) -> (&'static str, Color32) {
        if self.command_count == 0 {
            ("Nenhum serviço detectado", theme::MUTED)
        } else if self
            .runtime_requirements
            .iter()
            .any(|requirement| matches!(requirement.status, RequirementStatus::Incompatible { .. }))
        {
            ("Versão incompatível", Color32::from_rgb(255, 205, 75))
        } else if !self.missing_programs.is_empty()
            || self
                .runtime_requirements
                .iter()
                .any(|requirement| matches!(requirement.status, RequirementStatus::Missing))
        {
            ("Runtime não encontrado", Color32::from_rgb(255, 205, 75))
        } else if !self.missing_php_extensions.is_empty() {
            ("Extensões PHP ausentes", Color32::from_rgb(255, 205, 75))
        } else if !self.missing_dependencies.is_empty() {
            ("Dependências ausentes", Color32::from_rgb(255, 205, 75))
        } else {
            ("Pronto", theme::SUCCESS)
        }
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
        } else if let Some(requirement) = self
            .runtime_requirements
            .iter()
            .find(|requirement| matches!(requirement.status, RequirementStatus::Missing))
        {
            Some(format!(
                "Instale {} {}",
                requirement.runtime, requirement.version.value
            ))
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
        cfg!(target_os = "windows") && self.needs_environment_preparation()
    }

    fn needs_environment_preparation(&self) -> bool {
        self.runtime_requirements.iter().any(|requirement| {
            matches!(
                requirement.status,
                RequirementStatus::Missing | RequirementStatus::Incompatible { .. }
            )
        }) || self
            .missing_programs
            .iter()
            .any(|program| matches!(program.as_str(), "composer" | "pnpm" | "yarn" | "bun"))
            || !self.missing_php_extensions.is_empty()
    }

    fn legacy_laravel_constraint(&self) -> Option<&str> {
        self.laravel_constraint
            .as_deref()
            .filter(|constraint| laravel_major(constraint).is_some_and(|major| major <= 11))
    }
}

enum ProjectCardAction {
    Open(PathBuf),
    OpenUrl(String),
    Start(PathBuf),
    Stop(PathBuf),
    ViewLogs(PathBuf),
    PrepareEnvironment(PathBuf),
    InstallDependencies(PathBuf),
    ViewDependencyLogs(PathBuf),
    PrepareLaravelMigration(PathBuf),
    ToggleFavorite(PathBuf),
}

struct ProjectCardView<'a> {
    environment: &'a ProjectEnvironment,
    active_processes: usize,
    dependency_install: Option<DependencyInstallStatus>,
    application_url: Option<&'a str>,
    history: Option<&'a config::ProjectHistory>,
    favorite: bool,
    resolving_plan: bool,
    width: f32,
}

pub struct LocalCodePilot {
    page: Page,
    projects: Vec<Project>,
    processes: Vec<ProjectProcess>,
    running_processes: HashMap<String, RunningProcess>,
    assigned_ports: HashMap<String, Port>,
    port_conflicts: HashMap<String, PortConflict>,
    port_termination: Option<PortTerminationRequest>,
    project_environments: HashMap<PathBuf, ProjectEnvironment>,
    focused_project: Option<PathBuf>,
    terminal_process: Option<String>,
    dependency_installs: HashMap<PathBuf, DependencyInstallSession>,
    dependency_terminal: Option<PathBuf>,
    environment_plan_request: Option<(PathBuf, Receiver<Result<InstallationPlan, String>>)>,
    environment_plan_confirmation: Option<InstallationPlan>,
    environment_path: Option<OsString>,
    laravel_migration: Option<LaravelMigration>,
    platform: NativePlatform,
    logo_texture: egui::TextureHandle,
    username: String,
    user_initials: String,
    search: String,
    notification: Option<Notification>,
    scan_roots: Vec<PathBuf>,
    discovery: Option<Receiver<Result<Vec<Project>, String>>>,
    workspace_state: config::WorkspaceState,
    workspace_dirty: bool,
    last_workspace_save: Instant,
    workspace_save_error_reported: bool,
    update_check: Option<Receiver<Result<Option<UpdateInfo>, String>>>,
    available_update: Option<UpdateInfo>,
}

impl LocalCodePilot {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::configure(&cc.egui_ctx);
        let mut fonts = eframe::egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);
        let default_source = FilesystemProjectSource::common_locations();
        let scan_roots =
            config::load_scan_roots().unwrap_or_else(|| default_source.roots().to_vec());
        let source = FilesystemProjectSource::new(scan_roots.clone());
        let receiver = spawn_discovery(source, cc.egui_ctx.clone());
        let logo_texture = load_logo_texture(&cc.egui_ctx);
        let username = current_username();
        let user_initials = initials_from_username(&username);
        let mut workspace_state = config::load_workspace_state();
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
        let available_update = workspace_state
            .cached_update
            .clone()
            .filter(updates::cached_update_is_newer)
            .filter(|_| workspace_state.update_remind_after <= now);
        let update_check = (now.saturating_sub(workspace_state.last_update_check)
            >= updates::CHECK_INTERVAL_SECONDS)
            .then(|| updates::spawn_update_check(cc.egui_ctx.clone()));
        Self {
            page,
            projects: Vec::new(),
            processes: Vec::new(),
            running_processes: HashMap::new(),
            assigned_ports: HashMap::new(),
            port_conflicts: HashMap::new(),
            port_termination: None,
            project_environments: HashMap::new(),
            focused_project,
            terminal_process: None,
            dependency_installs: HashMap::new(),
            dependency_terminal: None,
            environment_plan_request: None,
            environment_plan_confirmation: None,
            environment_path: refreshed_environment_path().ok(),
            laravel_migration: None,
            platform: NativePlatform::default(),
            logo_texture,
            username,
            user_initials,
            search: String::new(),
            notification: Some(Notification {
                message: "Procurando projetos na máquina...".into(),
                created_at: Instant::now(),
            }),
            scan_roots,
            discovery: Some(receiver),
            workspace_state,
            workspace_dirty: recovered_preparation,
            last_workspace_save: Instant::now(),
            workspace_save_error_reported: false,
            update_check,
            available_update,
        }
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.notification = Some(Notification {
            message: message.into(),
            created_at: Instant::now(),
        });
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

    fn record_process_history(&mut self, id: &str) {
        let Some(process) = self
            .processes
            .iter()
            .find(|process| process.id == id)
            .cloned()
        else {
            return;
        };
        let running = self.running_processes.get(id);
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
        let Some(receiver) = &self.update_check else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(update)) => {
                let now = unix_timestamp();
                self.workspace_state.last_update_check = now;
                self.workspace_state.cached_update = update.clone();
                self.available_update = update
                    .filter(updates::cached_update_is_newer)
                    .filter(|_| self.workspace_state.update_remind_after <= now);
                self.workspace_dirty = true;
                self.update_check = None;
            }
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                self.update_check = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn show_update_banner(&mut self, ui: &mut egui::Ui) {
        let Some(update) = self.available_update.clone() else {
            return;
        };
        let mut remind_later = false;
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
                            RichText::new(format!(
                                "Nova versão disponível: {}",
                                update.version
                            ))
                            .strong()
                            .color(Color32::WHITE),
                        );
                        ui.label(
                            RichText::new(format!(
                                "Você está usando a versão {}. O download será aberto no site oficial.",
                                updates::current_version()
                            ))
                            .color(theme::MUTED)
                            .size(10.0),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Lembrar depois").clicked() {
                            remind_later = true;
                        }
                        if ui
                            .add(
                                egui::Button::new("Baixar atualização")
                                    .fill(theme::PRIMARY.gamma_multiply(0.72)),
                            )
                            .clicked()
                        {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(&update.download_url));
                        }
                        if ui.link("Ver novidades").clicked() {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(&update.release_url));
                        }
                    });
                });
            });
        ui.add_space(14.0);
        if remind_later {
            self.workspace_state.update_remind_after =
                unix_timestamp().saturating_add(updates::CHECK_INTERVAL_SECONDS);
            self.available_update = None;
            self.workspace_dirty = true;
            self.notify("O lembrete de atualização voltará amanhã");
        }
    }

    fn show_notification(&mut self, root_ui: &mut egui::Ui) {
        let Some(notification) = &self.notification else {
            return;
        };
        let elapsed = notification.created_at.elapsed();
        if elapsed >= NOTIFICATION_DURATION {
            self.notification = None;
            return;
        }

        let opacity = if elapsed < NOTIFICATION_FADE_START {
            1.0
        } else {
            let fade_elapsed = elapsed - NOTIFICATION_FADE_START;
            let fade_duration = NOTIFICATION_DURATION - NOTIFICATION_FADE_START;
            1.0 - fade_elapsed.as_secs_f32() / fade_duration.as_secs_f32()
        };
        let message = notification.message.clone();
        root_ui
            .ctx()
            .request_repaint_after(Duration::from_millis(50));

        egui::Area::new("notification".into())
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
                        ui.set_max_width(360.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(CIRCLE)
                                    .color(theme::SUCCESS.gamma_multiply(opacity))
                                    .size(8.0),
                            );
                            ui.label(
                                RichText::new(message)
                                    .color(theme::TEXT.gamma_multiply(opacity))
                                    .size(11.0),
                            );
                        });
                    });
            });
    }

    fn sidebar(&mut self, root_ui: &mut egui::Ui) {
        egui::Panel::left("sidebar")
            .exact_size(248.0)
            .frame(
                Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(Margin::same(14))
                    .stroke(Stroke::new(1.0_f32, theme::BORDER)),
            )
            .show(root_ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::new(&self.logo_texture)
                            .fit_to_exact_size(egui::vec2(42.0, 42.0))
                            .alt_text("Logo do LocalCodePilot"),
                    );
                    ui.vertical(|ui| {
                        ui.label(RichText::new("LocalCodePilot").strong().size(14.0));
                        ui.label(
                            RichText::new("Workspace manager")
                                .color(theme::MUTED)
                                .size(11.0),
                        );
                    });
                });
                ui.add_space(24.0);
                ui.label(
                    RichText::new("WORKSPACE")
                        .color(Color32::from_rgb(111, 119, 135))
                        .strong()
                        .size(10.0),
                );
                ui.add_space(4.0);
                self.nav_button(ui, Page::Overview, LAYOUT, "Visão geral", None);
                self.nav_button(
                    ui,
                    Page::Projects,
                    FOLDER_OPEN,
                    "Projetos",
                    Some(self.projects.len()),
                );
                self.nav_button(ui, Page::Processes, TERMINAL, "Processos", None);
                ui.add_space(18.0);
                ui.label(
                    RichText::new("SISTEMA")
                        .color(Color32::from_rgb(111, 119, 135))
                        .strong()
                        .size(10.0),
                );
                ui.add_space(4.0);
                self.nav_button(ui, Page::Plugins, CODE_SIMPLE, "Plugins", None);
                self.nav_button(ui, Page::Settings, GEAR, "Configurações", None);

                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.colored_label(theme::SUCCESS, CIRCLE);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Ambiente local").strong().size(11.0));
                            ui.label(
                                RichText::new("Todos os serviços online")
                                    .color(theme::MUTED)
                                    .size(10.0),
                            );
                        });
                    });
                    ui.separator();
                });
            });
    }

    fn nav_button(
        &mut self,
        ui: &mut egui::Ui,
        page: Page,
        icon: &str,
        label: &str,
        count: Option<usize>,
    ) {
        let selected = self.page == page;
        let fill = if selected {
            theme::PRIMARY.gamma_multiply(0.13)
        } else {
            Color32::TRANSPARENT
        };
        let response = Frame::new()
            .fill(fill)
            .corner_radius(8)
            .inner_margin(Margin::symmetric(10, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(icon)
                            .color(if selected {
                                Color32::WHITE
                            } else {
                                theme::MUTED
                            })
                            .size(16.0),
                    );
                    ui.label(
                        RichText::new(label)
                            .color(if selected {
                                Color32::WHITE
                            } else {
                                theme::MUTED
                            })
                            .size(13.0),
                    );
                    if let Some(count) = count {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(count.to_string())
                                    .color(theme::MUTED)
                                    .size(10.0),
                            );
                        });
                    }
                });
            })
            .response
            .interact(Sense::click());
        if response.clicked() {
            self.page = page;
        }
    }

    fn topbar(&mut self, root_ui: &mut egui::Ui) {
        egui::Panel::top("topbar")
            .exact_size(64.0)
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(Margin::symmetric(26, 14))
                    .stroke(Stroke::new(1.0_f32, theme::BORDER)),
            )
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Workspace  {CARET_RIGHT}"))
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                    ui.label(RichText::new(self.page_title()).size(12.0));
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

                        ui.label(RichText::new(BELL).color(theme::PRIMARY).size(16.0));

                        let search_hint = match self.page {
                            Page::Overview | Page::Projects => Some("Buscar projeto..."),
                            Page::Processes => Some("Buscar processo..."),
                            Page::Plugins | Page::Settings => None,
                        };
                        if let Some(search_hint) = search_hint {
                            ui.add_sized(
                                [220.0, 32.0],
                                egui::TextEdit::singleline(&mut self.search)
                                    .hint_text(search_hint)
                                    .horizontal_align(Align::Center),
                            );
                        }
                    });
                });
            });
    }

    fn page_title(&self) -> &'static str {
        match self.page {
            Page::Overview => "Visão geral",
            Page::Projects => "Projetos",
            Page::Processes => "Processos",
            Page::Plugins => "Plugins",
            Page::Settings => "Configurações",
        }
    }

    fn poll_discovery(&mut self) {
        let Some(receiver) = &self.discovery else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(projects)) => {
                self.notify(format!("{} projeto(s) encontrado(s)", projects.len()));
                self.processes = projects.iter().flat_map(detect_processes).collect();
                self.assigned_ports.clear();
                self.port_conflicts.clear();
                self.project_environments = projects
                    .iter()
                    .map(|project| {
                        (
                            project.path.clone(),
                            inspect_project_environment(
                                project,
                                &self.processes,
                                self.environment_path.as_deref(),
                            ),
                        )
                    })
                    .collect();
                self.projects = projects;
                self.discovery = None;
            }
            Ok(Err(error)) => {
                self.notify(format!("Não foi possível concluir a varredura: {error}"));
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
        if self.has_active_processes() {
            self.notify("Pare os processos ativos antes de atualizar os projetos");
            return;
        }
        if self.discovery.is_some() {
            return;
        }
        self.notify("Atualizando a lista de projetos...");
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            ctx.clone(),
        ));
    }

    fn add_scan_root(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.has_active_processes() {
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
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            ctx.clone(),
        ));
    }

    fn remove_scan_root(&mut self, ctx: &egui::Context, path: &Path) {
        if self.has_active_processes() {
            self.notify("Pare os processos ativos antes de remover uma pasta");
            return;
        }
        self.scan_roots.retain(|root| root != path);
        self.notify(match config::save_scan_roots(&self.scan_roots) {
            Ok(()) => "Pasta removida. Atualizando os projetos...".into(),
            Err(error) => format!("Pasta removida, mas não foi possível salvar: {error}"),
        });
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            ctx.clone(),
        ));
    }

    fn reset_scan_roots(&mut self, ctx: &egui::Context) {
        if self.has_active_processes() {
            self.notify("Pare os processos ativos antes de restaurar as pastas");
            return;
        }
        self.scan_roots = FilesystemProjectSource::common_locations().roots().to_vec();
        self.notify(match config::save_scan_roots(&self.scan_roots) {
            Ok(()) => "Pastas padrão restauradas. Atualizando os projetos...".into(),
            Err(error) => format!("Pastas restauradas, mas não foi possível salvar: {error}"),
        });
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            ctx.clone(),
        ));
    }

    fn overview(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("BOM TE VER NOVAMENTE")
                        .color(theme::PRIMARY)
                        .strong()
                        .size(11.0),
                );
                ui.label(RichText::new("Seus projetos").strong().size(32.0));
                ui.label(
                    RichText::new("Gerencie ambientes, processos e atalhos em um só lugar.")
                        .color(theme::MUTED)
                        .size(13.0),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(format!("{PLUS}  Novo projeto"))
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(theme::PRIMARY)
                        .corner_radius(8),
                    )
                    .clicked()
                {
                    self.notify("O assistente de criação de projetos será adicionado futuramente");
                }
            });
        });
        ui.add_space(22.0);
        let snapshot = self.platform.snapshot();
        let used_gb = snapshot.used_memory_bytes as f64 / 1_073_741_824.0;
        let active_processes = self
            .running_processes
            .values()
            .filter(|process| !process.finished)
            .count();
        ui.columns(3, |columns| {
            stat_card(
                &mut columns[0],
                FOLDER_OPEN,
                "Projetos",
                &self.projects.len().to_string(),
                "disponíveis",
                theme::PRIMARY,
            );
            stat_card(
                &mut columns[1],
                PLAY,
                "Processos ativos",
                &active_processes.to_string(),
                "em execução",
                theme::SUCCESS,
            );
            stat_card(
                &mut columns[2],
                MEMORY,
                "Uso de memória",
                &format!("{used_gb:.1} GB"),
                "no sistema",
                Color32::from_rgb(170, 132, 255),
            );
        });
        ui.add_space(26.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Projetos recentes").strong().size(16.0));
                ui.label(
                    RichText::new("Continue de onde parou")
                        .color(theme::MUTED)
                        .size(11.0),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let link_ver_todos =
                    format!("{} Ver todos os projetos", egui_phosphor::regular::LIST);

                if ui.link(link_ver_todos).clicked() {
                    self.page = Page::Projects;
                }
            });
        });
        ui.add_space(8.0);
        self.project_grid(ui, Some(3));
    }

    fn project_grid(&mut self, ui: &mut egui::Ui, max_projects: Option<usize>) {
        let query = self.search.trim().to_lowercase();
        let mut projects: Vec<_> = self
            .projects
            .iter()
            .filter(|p| {
                query.is_empty()
                    || p.name.to_lowercase().contains(&query)
                    || p.path.to_string_lossy().to_lowercase().contains(&query)
            })
            .cloned()
            .collect();

        projects.sort_by_key(|project| {
            let last_activity = self
                .workspace_state
                .project(&project.path)
                .map(|history| history.last_activity)
                .unwrap_or_else(|| system_time_timestamp(project.modified_at));
            std::cmp::Reverse((self.is_favorite(&project.path), last_activity))
        });
        if let Some(max_projects) = max_projects {
            projects.truncate(max_projects);
        }

        if projects.is_empty() {
            let (title, detail) = if self.discovery.is_some() {
                (
                    "Procurando projetos...",
                    "Aguarde enquanto examinamos as pastas mais comuns da sua máquina.",
                )
            } else if !query.is_empty() {
                (
                    "Nenhum resultado para esta busca",
                    "Tente buscar pelo nome do projeto ou por parte do caminho.",
                )
            } else {
                (
                    "Nenhum projeto encontrado",
                    "A descoberta procura manifestos Rust, Node.js, PHP e Python.",
                )
            };
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(12)
                .inner_margin(Margin::same(24))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(title).strong().size(14.0));
                    ui.label(RichText::new(detail).color(theme::MUTED).size(11.0));
                });
            return;
        }

        let available_width = ui.available_width().max(220.0);
        let columns: usize = if available_width > 900.0 {
            3
        } else if available_width > 560.0 {
            2
        } else {
            1
        };
        for row in projects.chunks(columns) {
            ui.columns(columns, |column_uis| {
                for (column, project) in row.iter().enumerate() {
                    let column_ui = &mut column_uis[column];
                    let card_width = (column_ui.available_width() - 36.0).max(180.0);
                    let environment = self
                        .project_environments
                        .get(&project.path)
                        .cloned()
                        .unwrap_or_default();
                    let active_processes =
                        project_process_ids(&self.processes, &project.path, true).len();
                    let dependency_install = self
                        .dependency_installs
                        .get(&project.path)
                        .map(|session| session.status.clone());
                    let application_url = self
                        .processes
                        .iter()
                        .filter(|process| process.project_path == project.path)
                        .filter(|process| process_exposes_application_url(project, process))
                        .filter_map(|process| {
                            let running = self.running_processes.get(&process.id)?;
                            running
                                .urls
                                .last()
                                .map(|url| url.address().to_owned())
                                .or_else(|| {
                                    self.assigned_ports
                                        .get(&process.id)
                                        .map(|port| format!("http://localhost:{}", port.value()))
                                })
                        })
                        .next();
                    let project_history = self.workspace_state.project(&project.path).cloned();
                    let favorite = self.is_favorite(&project.path);
                    let resolving_plan = self
                        .environment_plan_request
                        .as_ref()
                        .is_some_and(|(path, _)| path == &project.path);
                    match project_card(
                        column_ui,
                        project,
                        ProjectCardView {
                            environment: &environment,
                            active_processes,
                            dependency_install,
                            application_url: application_url.as_deref(),
                            history: project_history.as_ref(),
                            favorite,
                            resolving_plan,
                            width: card_width,
                        },
                    ) {
                        Some(ProjectCardAction::Open(path)) => {
                            self.notify(match open_in_vscode(&path) {
                                Ok(()) => format!("Abrindo {} no VS Code...", project.name),
                                Err(error) => error,
                            });
                        }
                        Some(ProjectCardAction::OpenUrl(url)) => {
                            column_ui.ctx().open_url(egui::OpenUrl::new_tab(&url));
                            self.notify(format!("Abrindo {url}"));
                        }
                        Some(ProjectCardAction::Start(path)) => self.start_project(&path),
                        Some(ProjectCardAction::Stop(path)) => self.stop_project(&path),
                        Some(ProjectCardAction::ViewLogs(path)) => {
                            let active_processes =
                                project_process_ids(&self.processes, &path, true);
                            self.focused_project = Some(path);
                            if let [process_id] = active_processes.as_slice() {
                                self.terminal_process = Some(process_id.clone());
                            } else {
                                self.search.clear();
                                self.page = Page::Processes;
                            }
                        }
                        Some(ProjectCardAction::PrepareEnvironment(path)) => {
                            self.request_environment_plan(&path, column_ui.ctx().clone());
                        }
                        Some(ProjectCardAction::InstallDependencies(path)) => {
                            self.install_dependencies(&path, column_ui.ctx().clone());
                        }
                        Some(ProjectCardAction::ViewDependencyLogs(path)) => {
                            self.dependency_terminal = Some(path);
                        }
                        Some(ProjectCardAction::PrepareLaravelMigration(path)) => {
                            self.prepare_laravel_migration(&path);
                        }
                        Some(ProjectCardAction::ToggleFavorite(path)) => {
                            self.toggle_favorite(&path);
                        }
                        None => {}
                    }
                }
            });
            ui.add_space(14.0);
        }
    }

    fn projects_page(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let has_active_processes = self.has_active_processes();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading("Projetos");
                ui.label(
                    RichText::new(format!(
                        "{} projeto(s) em {} pasta(s) de busca",
                        self.projects.len(),
                        self.scan_roots.len()
                    ))
                    .color(theme::MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let scanning = self.discovery.is_some();
                let can_change_roots = !scanning && !has_active_processes;
                if ui
                    .add_enabled(can_change_roots, egui::Button::new("Atualizar"))
                    .clicked()
                {
                    self.refresh_projects(ctx);
                }
                if ui
                    .add_enabled(
                        can_change_roots,
                        egui::Button::new(format!("{PLUS}  Adicionar pasta")).fill(theme::PRIMARY),
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title("Escolha uma pasta com projetos")
                        .pick_folder()
                {
                    self.add_scan_root(ctx, path);
                }
                if scanning {
                    ui.spinner();
                    ui.label(RichText::new("Procurando...").color(theme::MUTED));
                }
            });
        });
        if has_active_processes {
            ui.label(
                RichText::new("Pare os processos ativos para alterar ou atualizar os projetos.")
                    .color(Color32::from_rgb(255, 205, 75))
                    .size(10.0),
            );
        }
        let mut root_to_remove = None;
        ui.collapsing(
            format!("Pastas examinadas ({})", self.scan_roots.len()),
            |ui| {
                for root in &self.scan_roots {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(root.to_string_lossy())
                                .color(theme::MUTED)
                                .monospace()
                                .size(10.0),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    self.discovery.is_none() && !has_active_processes,
                                    egui::Button::new("Remover").small(),
                                )
                                .clicked()
                            {
                                root_to_remove = Some(root.clone());
                            }
                        });
                    });
                }
                ui.add_space(6.0);
                if ui
                    .add_enabled(
                        self.discovery.is_none() && !has_active_processes,
                        egui::Button::new("Restaurar pastas padrão"),
                    )
                    .clicked()
                {
                    self.reset_scan_roots(ctx);
                }
            },
        );
        if let Some(root) = root_to_remove {
            self.remove_scan_root(ctx, &root);
        }
        ui.add_space(20.0);
        self.project_grid(ui, None);
    }

    fn port_is_available(&self, port: Port) -> bool {
        !self
            .assigned_ports
            .values()
            .any(|assigned| *assigned == port)
            && matches!(inspect_tcp_port(port), PortAvailability::Available)
    }

    fn find_available_port_after(&self, preferred: Port) -> Option<Port> {
        (1..=20).find_map(|offset| {
            let value = preferred.value().checked_add(offset)?;
            let candidate = Port::new(value)?;
            self.port_is_available(candidate).then_some(candidate)
        })
    }

    fn start_process(&mut self, id: &str) {
        let Some(index) = self.processes.iter().position(|process| process.id == id) else {
            self.notify("O comando selecionado não está mais disponível");
            return;
        };
        if self.discovery.is_some() {
            self.notify("Aguarde a varredura terminar antes de iniciar um processo");
            return;
        }
        if self
            .running_processes
            .get(id)
            .is_some_and(|running| !running.finished)
        {
            self.notify(format!(
                "{} já está em execução",
                self.processes[index].name
            ));
            return;
        }
        let process = self.processes[index].clone();
        if !process.working_directory.is_dir() {
            self.processes[index].state = ProcessState::Failed;
            self.processes[index].process_id = None;
            self.processes[index].exit_code = None;
            self.notify(format!(
                "Não foi possível iniciar {}: a pasta '{}' não existe mais",
                process.name,
                process.working_directory.display()
            ));
            return;
        }
        if !executable_available(&process.program) {
            self.processes[index].state = ProcessState::Failed;
            self.processes[index].process_id = None;
            self.processes[index].exit_code = None;
            self.notify(format!(
                "Não foi possível iniciar {}: o programa '{}' não foi encontrado no PATH",
                process.name, process.program
            ));
            return;
        }
        let mut assigned_port = None;
        let mut alternative_port = None;
        if let Some(port) = process.expected_port {
            if self.port_is_available(port) {
                assigned_port = Some(port);
                self.port_conflicts.remove(id);
            } else if process.port_override.is_some()
                && let Some(available_port) = self.find_available_port_after(port)
            {
                assigned_port = Some(available_port);
                alternative_port = Some(available_port);
                self.port_conflicts.remove(id);
            } else {
                let owner = match inspect_tcp_port(port) {
                    PortAvailability::Occupied { owner } => owner,
                    PortAvailability::Available => None,
                };
                let conflict = PortConflict { port, owner };
                self.processes[index].state = ProcessState::Failed;
                self.processes[index].process_id = None;
                self.processes[index].exit_code = None;
                self.notify(port_conflict_message(&process.name, &conflict));
                self.port_conflicts.insert(id.to_owned(), conflict);
                self.record_process_history(id);
                return;
            }
        }
        self.processes[index].state = ProcessState::Starting;
        self.processes[index].exit_code = None;
        if let Some(port) = assigned_port {
            self.assigned_ports.insert(id.to_owned(), port);
        }
        let mut command =
            process_command_on_port(&process, alternative_port, self.environment_path.as_deref());
        command
            .current_dir(&process.working_directory)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        match command.spawn() {
            Ok(mut child) => {
                self.port_conflicts.remove(id);
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
                self.running_processes.insert(
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
                    },
                );
                self.processes[index].state = if assigned_port.is_some() {
                    ProcessState::Starting
                } else {
                    ProcessState::Running
                };
                self.processes[index].process_id = Some(process_id);
                self.processes[index].exit_code = None;
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
                self.assigned_ports.remove(id);
                self.processes[index].state = ProcessState::Failed;
                self.processes[index].process_id = None;
                self.processes[index].exit_code = None;
                self.notify(format!(
                    "Não foi possível iniciar {}: {error}",
                    process.name
                ));
                self.record_process_history(id);
            }
        }
    }

    fn stop_process(&mut self, id: &str) {
        let Some(running) = self.running_processes.get_mut(id) else {
            return;
        };
        if running.finished {
            return;
        }
        terminate_process(&mut running.child);
        running.finished = true;
        self.assigned_ports.remove(id);
        if let Some(process) = self.processes.iter_mut().find(|process| process.id == id) {
            process.state = ProcessState::Stopped;
            process.process_id = None;
            process.exit_code = None;
            let process_name = process.name.clone();
            self.notify(format!("{process_name} interrompido"));
        }
        self.record_process_history(id);
    }

    fn start_project(&mut self, project_path: &Path) {
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
                            &self.processes,
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
        let process_ids = project_process_ids(&self.processes, project_path, false);
        let pending_processes: Vec<_> = process_ids
            .iter()
            .filter_map(|id| self.processes.iter().find(|process| process.id == *id))
            .cloned()
            .collect();
        let mut first_conflict = None;
        for process in &pending_processes {
            let Some(port) = process.expected_port else {
                continue;
            };
            if !self.port_is_available(port) && process.port_override.is_none() {
                let owner = match inspect_tcp_port(port) {
                    PortAvailability::Occupied { owner } => owner,
                    PortAvailability::Available => None,
                };
                let conflict = PortConflict { port, owner };
                if let Some(process) = self
                    .processes
                    .iter_mut()
                    .find(|candidate| candidate.id == process.id)
                {
                    process.state = ProcessState::Failed;
                    process.process_id = None;
                    process.exit_code = None;
                }
                self.port_conflicts
                    .insert(process.id.clone(), conflict.clone());
                first_conflict.get_or_insert_with(|| (process.name.clone(), conflict));
            } else {
                self.port_conflicts.remove(&process.id);
            }
        }
        if let Some((process_name, conflict)) = first_conflict {
            self.focused_project = Some(project_path.to_path_buf());
            self.page = Page::Processes;
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
        let process_ids = project_process_ids(&self.processes, project_path, true);
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
        self.notify("Preparando o ambiente em segundo plano...");
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
                        let logs = session.logs.iter().cloned().collect::<Vec<_>>().join("\n");
                        DependencyInstallStatus::Failed {
                            error: error.clone(),
                            suggestion: dependency_error_suggestion(&logs, error),
                        }
                    }
                };
                completed.push((project_path.clone(), result));
            }
        }

        for (project_path, result) in completed {
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
                        let refreshed_processes = detect_processes(project);
                        self.processes
                            .retain(|process| process.project_path != project_path);
                        self.processes.extend(refreshed_processes);
                        let environment = inspect_project_environment(
                            project,
                            &self.processes,
                            self.environment_path.as_deref(),
                        );
                        let ready = environment.is_ready();
                        self.project_environments
                            .insert(project_path.clone(), environment);
                        self.notify(if ready {
                            "Preparação concluída. O ambiente está pronto."
                        } else {
                            "A preparação terminou, mas o ambiente ainda precisa de atenção."
                        });
                    }
                }
                Err(error) => {
                    self.dependency_terminal = Some(project_path);
                    self.notify(format!("Não foi possível instalar: {error}"));
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
                        &self.processes,
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
        let Some(conflict) = self.port_conflicts.get(id).cloned() else {
            self.notify("A porta já está livre. Tente iniciar o serviço novamente.");
            return;
        };
        if conflict.owner.is_none() {
            self.notify(
                "A porta está ocupada, mas o Windows não informou qual processo a utiliza.",
            );
            return;
        }
        self.port_termination = Some(PortTerminationRequest {
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
                self.port_conflicts.remove(&request.process_id);
                self.notify("A porta já foi liberada. Tentando iniciar o serviço...");
                self.start_process(&request.process_id);
            }
            PortAvailability::Occupied {
                owner: Some(current_owner),
            } if current_owner.process_id != owner.process_id => {
                let conflict = PortConflict {
                    port: request.conflict.port,
                    owner: Some(current_owner),
                };
                self.port_conflicts
                    .insert(request.process_id.clone(), conflict.clone());
                self.notify(port_conflict_message("O serviço", &conflict));
            }
            PortAvailability::Occupied { .. } => match terminate_process_tree(owner.process_id) {
                Ok(()) => {
                    self.port_conflicts.remove(&request.process_id);
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
        let Some(request) = self.port_termination.clone() else {
            return;
        };
        let Some(owner) = request.conflict.owner.as_ref() else {
            self.port_termination = None;
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
            .id(egui::Id::new("port-termination-confirmation"))
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
            self.port_termination = None;
            self.terminate_port_owner_and_retry(&request);
        } else if cancel || !open {
            self.port_termination = None;
        }
    }

    fn clear_process_logs(&mut self, id: &str) {
        if let Some(running) = self.running_processes.get_mut(id) {
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
        let Some(process_id) = self.terminal_process.clone() else {
            return;
        };
        let Some(process) = self
            .processes
            .iter()
            .find(|process| process.id == process_id)
            .cloned()
        else {
            self.terminal_process = None;
            return;
        };
        let process_history = self.workspace_state.service(&process_id).cloned();
        let (logs, finished) = self
            .running_processes
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
        let assigned_port = self.assigned_ports.get(&process_id).copied().or_else(|| {
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
            .id(egui::Id::new("process-terminal"))
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
                    RichText::new(process.working_directory.to_string_lossy())
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
            self.terminal_process = None;
        }
    }

    fn show_environment_plan_confirmation(&mut self, root_ui: &mut egui::Ui) {
        let Some(plan) = self.environment_plan_confirmation.clone() else {
            return;
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Preparar ambiente")
            .id(egui::Id::new(("environment-plan", &plan.project_path)))
            .open(&mut open)
            .default_width(620.0)
            .resizable(true)
            .collapsible(false)
            .show(root_ui, |ui| {
                ui.label(RichText::new(&plan.title).strong().size(16.0));
                ui.label(
                    RichText::new(plan.project_path.to_string_lossy())
                        .color(theme::MUTED)
                        .monospace()
                        .size(10.0),
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
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add(
                            egui::Button::new("Preparar")
                                .fill(theme::PRIMARY.gamma_multiply(0.72)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });

        if confirm {
            self.environment_plan_confirmation = None;
            self.start_environment_plan(plan, root_ui.ctx().clone());
        } else if cancel || !open {
            self.environment_plan_confirmation = None;
            self.notify("Preparação cancelada antes da execução");
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
        let offers_laravel_migration = matches!(&status, DependencyInstallStatus::Failed { .. })
            && composer_security_advisory(&logs.join("\n"))
            && project_path.join("composer.json").is_file();
        let mut open = true;
        let mut clear_logs = false;
        let mut retry = false;
        let mut cancel_installation = false;
        let mut prepare_migration = false;

        egui::Window::new(format!("{TERMINAL}  Preparação · {project_name}"))
            .id(egui::Id::new(("dependency-terminal", &project_path)))
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
                    RichText::new(project_path.to_string_lossy())
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
                        && ui.button("Cancelar preparação").clicked()
                    {
                        cancel_installation = true;
                    }
                    if matches!(&status, DependencyInstallStatus::Failed { .. })
                        && ui
                            .add(
                                egui::Button::new("Tentar novamente")
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
            .id(egui::Id::new("laravel-migration"))
            .open(&mut open)
            .default_size([720.0, 520.0])
            .min_size([500.0, 360.0])
            .resizable(true)
            .collapsible(false)
            .show(root_ui, |ui| {
                ui.heading(&project_name);
                ui.label(
                    RichText::new(project_path.to_string_lossy())
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

    fn has_active_processes(&self) -> bool {
        self.running_processes
            .values()
            .any(|running| !running.finished)
    }

    fn poll_processes(&mut self, ctx: &egui::Context) {
        let process_states: HashMap<_, _> = self
            .processes
            .iter()
            .map(|process| (process.id.clone(), process.state))
            .collect();
        let assigned_ports = self.assigned_ports.clone();
        let mut state_updates = Vec::new();
        let mut ready_updates = Vec::new();
        let mut history_updates = Vec::new();
        let mut has_running_process = false;
        let now = Instant::now();
        for (id, running) in &mut self.running_processes {
            while let Ok(line) = running.output.try_recv() {
                if !history_updates.contains(id) {
                    history_updates.push(id.clone());
                }
                for url in extract_local_urls(&line) {
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
                    let stopped_during_startup = state == ProcessState::Starting;
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
                        Some(reason),
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
                        Some(error.to_string()),
                    ));
                    continue;
                }
            }

            let Some(port) = assigned_ports.get(id).copied() else {
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
            let port_is_listening =
                matches!(inspect_tcp_port(port), PortAvailability::Occupied { .. });
            let url_was_announced = running.urls.iter().any(|url| url.port() == port);

            if state == ProcessState::Starting {
                if port_is_listening || url_was_announced {
                    running.port_unavailable_since = None;
                    running.logs.push_back(format!(
                        "[LocalCodePilot] Serviço disponível na porta {}.",
                        port.value()
                    ));
                    ready_updates.push((id.clone(), port));
                } else if now.duration_since(running.started_at) >= PROCESS_START_TIMEOUT {
                    running.logs.push_back(format!(
                        "[LocalCodePilot] Falha: a porta {} não ficou disponível em {} segundos.",
                        port.value(),
                        PROCESS_START_TIMEOUT.as_secs()
                    ));
                    terminate_process(&mut running.child);
                    running.finished = true;
                    state_updates.push((
                        id.clone(),
                        ProcessState::Failed,
                        None,
                        Some(format!(
                            "a porta {} não respondeu dentro do tempo esperado",
                            port.value()
                        )),
                    ));
                }
                continue;
            }

            if state == ProcessState::Running {
                if port_is_listening {
                    running.port_unavailable_since = None;
                } else {
                    let unavailable_since = running.port_unavailable_since.get_or_insert(now);
                    if now.duration_since(*unavailable_since) >= PROCESS_PORT_LOSS_GRACE {
                        running.logs.push_back(format!(
                            "[LocalCodePilot] Falha: a porta {} deixou de responder por {} segundos.",
                            port.value(),
                            PROCESS_PORT_LOSS_GRACE.as_secs()
                        ));
                        terminate_process(&mut running.child);
                        running.finished = true;
                        state_updates.push((
                            id.clone(),
                            ProcessState::Failed,
                            None,
                            Some(format!("a porta {} deixou de responder", port.value())),
                        ));
                    }
                }
            }
        }
        for (id, port) in ready_updates {
            let notification = self
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
        for (id, state, exit_code, error) in state_updates {
            self.assigned_ports.remove(&id);
            if let Some(process) = self.processes.iter_mut().find(|process| process.id == id) {
                process.state = state;
                process.process_id = None;
                process.exit_code = exit_code;
                let message = match (error, exit_code) {
                    (Some(error), _) => format!("{}: {error}", process.name),
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

    fn processes_page(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let query = self.search.trim().to_lowercase();
        let matching_processes = self
            .processes
            .iter()
            .filter(|process| {
                let project = self
                    .projects
                    .iter()
                    .find(|project| project.path == process.project_path);
                process_matches_search(process, project, &query)
            })
            .count();
        ui.heading("Processos");
        ui.label(
            RichText::new(if query.is_empty() {
                format!(
                    "{} comando(s) detectado(s) em seus projetos",
                    self.processes.len()
                )
            } else {
                format!(
                    "{matching_processes} de {} comando(s) encontrado(s)",
                    self.processes.len()
                )
            })
            .color(theme::MUTED),
        );
        ui.add_space(20.0);

        if self.processes.is_empty() {
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(12)
                .inner_margin(Margin::same(24))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Nenhum comando detectado").strong());
                    ui.label(
                        RichText::new(
                            "Atualize os projetos para procurar scripts e comandos disponíveis.",
                        )
                        .color(theme::MUTED),
                    );
                });
            return;
        }

        if matching_processes == 0 {
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(12)
                .inner_margin(Margin::same(24))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Nenhum processo encontrado").strong());
                    ui.label(
                        RichText::new(
                            "Busque pelo projeto, tecnologia, comando ou caminho da pasta.",
                        )
                        .color(theme::MUTED),
                    );
                });
            return;
        }

        let mut projects = self.projects.clone();
        projects.sort_by_key(|project| {
            if self.focused_project.as_ref() == Some(&project.path) {
                0
            } else if !project_process_ids(&self.processes, &project.path, true).is_empty() {
                1
            } else {
                2
            }
        });

        for project in &projects {
            let project_matches = project.name.to_lowercase().contains(&query)
                || project
                    .path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&query)
                || project.display_stack().to_lowercase().contains(&query);
            let mut commands: Vec<_> = self
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
            let project_command_count = self
                .processes
                .iter()
                .filter(|process| process.project_path == project.path)
                .count();
            let active_command_count =
                project_process_ids(&self.processes, &project.path, true).len();
            let environment = self
                .project_environments
                .get(&project.path)
                .cloned()
                .unwrap_or_default();
            let dependency_install_status = self
                .dependency_installs
                .get(&project.path)
                .map(|session| session.status.clone());
            let installing_dependencies = matches!(
                &dependency_install_status,
                Some(DependencyInstallStatus::Running | DependencyInstallStatus::Cancelling)
            );
            let dependency_install_failed = matches!(
                &dependency_install_status,
                Some(DependencyInstallStatus::Failed { .. })
            );
            let resolving_plan = self
                .environment_plan_request
                .as_ref()
                .is_some_and(|(path, _)| path == &project.path);
            let has_dependency_logs = self.dependency_installs.contains_key(&project.path);
            let project_history = self.workspace_state.project(&project.path).cloned();
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(12)
                .inner_margin(Margin::same(18))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&project.name).strong().size(15.0));
                            let (status, status_color) = if resolving_plan {
                                ("Preparando plano", theme::PRIMARY)
                            } else if installing_dependencies {
                                ("Preparando ambiente", theme::PRIMARY)
                            } else if dependency_install_failed {
                                ("Falha na instalação", Color32::from_rgb(235, 87, 87))
                            } else if active_command_count > 0 {
                                ("Em execução", theme::SUCCESS)
                            } else {
                                environment.status()
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{status} · {project_command_count} serviço(s)"
                                ))
                                .color(status_color)
                                .size(10.0),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if active_command_count > 0 {
                                if ui
                                    .add(
                                        egui::Button::new("Parar ambiente").fill(
                                            Color32::from_rgb(235, 87, 87).gamma_multiply(0.18),
                                        ),
                                    )
                                    .clicked()
                                {
                                    action = Some(ProcessAction::StopProject(project.path.clone()));
                                }
                            } else if resolving_plan {
                                ui.spinner();
                                ui.label("Consultando versões...");
                            } else if installing_dependencies {
                                ui.spinner();
                                ui.label("Instalando...");
                            } else if environment.can_prepare_environment() {
                                if ui
                                    .add(
                                        egui::Button::new("Preparar ambiente")
                                            .fill(theme::PRIMARY.gamma_multiply(0.72)),
                                    )
                                    .clicked()
                                {
                                    action = Some(ProcessAction::PrepareEnvironment(
                                        project.path.clone(),
                                    ));
                                }
                            } else if environment.can_install_dependencies() {
                                if ui
                                    .add(
                                        egui::Button::new(if dependency_install_failed {
                                            "Tentar novamente"
                                        } else {
                                            "Instalar dependências"
                                        })
                                        .fill(theme::PRIMARY.gamma_multiply(0.72)),
                                    )
                                    .clicked()
                                {
                                    action = Some(ProcessAction::InstallDependencies(
                                        project.path.clone(),
                                    ));
                                }
                            } else {
                                let start = ui.add_enabled(
                                    environment.is_ready(),
                                    egui::Button::new(RichText::new(format!(
                                        "{PLAY}  {}",
                                        if project_history.is_some() {
                                            "Retomar ambiente"
                                        } else {
                                            "Iniciar ambiente"
                                        }
                                    )))
                                    .fill(theme::PRIMARY.gamma_multiply(0.72)),
                                );
                                let start = if let Some(details) = environment.problem_details() {
                                    start.on_hover_text(details)
                                } else {
                                    start
                                };
                                if start.clicked() {
                                    action =
                                        Some(ProcessAction::StartProject(project.path.clone()));
                                }
                            }
                        });
                    });
                    if !installing_dependencies && let Some(details) = environment.problem_details()
                    {
                        ui.label(
                            RichText::new(details)
                                .color(Color32::from_rgb(255, 205, 75))
                                .size(10.0),
                        );
                    }
                    if has_dependency_logs
                        && ui
                            .link(if dependency_install_failed {
                                format!("{TERMINAL}  Ver erro e sugestões")
                            } else {
                                format!("{TERMINAL}  Ver logs da instalação")
                            })
                            .clicked()
                    {
                        action = Some(ProcessAction::ViewDependencyLogs(project.path.clone()));
                    }
                    ui.add_space(10.0);
                    for command in commands {
                        let port_conflict = self.port_conflicts.get(&command.id).cloned();
                        let assigned_port = self.assigned_ports.get(&command.id).copied();
                        let process_history = self.workspace_state.service(&command.id).cloned();
                        let failure_suggestion = (command.state == ProcessState::Failed
                            && port_conflict.is_none())
                        .then(|| {
                            self.running_processes
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
                            self.running_processes
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
                        let (state_label, state_color) = if port_conflict.is_some() {
                            ("Porta ocupada", Color32::from_rgb(235, 87, 87))
                        } else if command.state == ProcessState::Running && server_url.is_some() {
                            ("Disponível", theme::SUCCESS)
                        } else {
                            process_state_display(command.state)
                        };
                        Frame::new()
                            .fill(theme::BACKGROUND)
                            .stroke(Stroke::new(1.0, theme::BORDER.gamma_multiply(0.75)))
                            .corner_radius(9)
                            .inner_margin(Margin::same(13))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(&command.name).strong().size(12.0));
                                        runtime_badge(ui, state_label, state_color);
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if (self.running_processes.contains_key(&command.id)
                                            || process_history
                                                .as_ref()
                                                .is_some_and(|history| !history.logs.is_empty()))
                                            && ui
                                                .button(RichText::new(format!(
                                                    "{TERMINAL}  {}",
                                                    if self.running_processes.contains_key(&command.id) {
                                                        "Abrir terminal"
                                                    } else {
                                                        "Últimos logs"
                                                    }
                                                )))
                                                .clicked()
                                        {
                                            action =
                                                Some(ProcessAction::ViewLogs(command.id.clone()));
                                        }
                                        match command.state {
                                            ProcessState::Running => {
                                                if ui
                                                    .add(
                                                        egui::Button::new("Parar")
                                                            .fill(
                                                                Color32::from_rgb(235, 87, 87)
                                                                    .gamma_multiply(0.18),
                                                            )
                                                            .stroke(Stroke::new(
                                                                1.0,
                                                                Color32::from_rgb(235, 87, 87)
                                                                    .gamma_multiply(0.65),
                                                            )),
                                                    )
                                                    .clicked()
                                                {
                                                    action = Some(ProcessAction::Stop(
                                                        command.id.clone(),
                                                    ));
                                                }
                                                if ui.button("Reiniciar").clicked() {
                                                    action = Some(ProcessAction::Restart(
                                                        command.id.clone(),
                                                    ));
                                                }
                                            }
                                            ProcessState::Starting => {
                                                ui.add_enabled(
                                                    false,
                                                    egui::Button::new("Iniciando..."),
                                                );
                                            }
                                            ProcessState::Stopped | ProcessState::Failed => {
                                                let start = ui.add_enabled(
                                                    environment.is_ready(),
                                                    egui::Button::new(RichText::new(format!(
                                                        "{PLAY}  {}",
                                                        if port_conflict.is_some() {
                                                            "Tentar novamente"
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
                                    });
                                });
                                if let Some(server_url) = &server_url {
                                    ui.add_space(3.0);
                                    if ui
                                        .link(
                                            RichText::new(format!(
                                                "Abrir {} no navegador {ARROW_SQUARE_OUT}",
                                                project.name
                                            ))
                                            .color(theme::SUCCESS)
                                            .size(10.0),
                                        )
                                        .on_hover_text(format!(
                                            "Abrir no navegador · porta {}",
                                            server_url.port().value()
                                        ))
                                        .clicked()
                                    {
                                        action = Some(ProcessAction::OpenUrl(
                                            server_url.address().to_owned(),
                                        ));
                                    }
                                    ui.add_space(2.0);
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
                                            .size(9.0),
                                        );
                                    }
                                } else if let Some(conflict) = &port_conflict {
                                    ui.add_space(4.0);
                                    Frame::new()
                                        .fill(Color32::from_rgb(235, 87, 87).gamma_multiply(0.08))
                                        .stroke(Stroke::new(
                                            1.0,
                                            Color32::from_rgb(235, 87, 87).gamma_multiply(0.4),
                                        ))
                                        .corner_radius(6)
                                        .inner_margin(Margin::same(8))
                                        .show(ui, |ui| {
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label(
                                                    RichText::new(port_conflict_details(conflict))
                                                        .color(Color32::from_rgb(245, 144, 144))
                                                        .size(10.0),
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
                                    ui.add_space(2.0);
                                } else if let Some(port) = command.expected_port {
                                    ui.add_space(3.0);
                                    ui.label(
                                        RichText::new(format!(
                                            "Porta prevista: {} · verificada antes de iniciar",
                                            port.value()
                                        ))
                                        .color(theme::MUTED)
                                        .size(9.0),
                                    );
                                }
                                if let Some(suggestion) = &failure_suggestion {
                                    ui.add_space(4.0);
                                    Frame::new()
                                        .fill(Color32::from_rgb(255, 205, 75).gamma_multiply(0.07))
                                        .stroke(Stroke::new(
                                            1.0,
                                            Color32::from_rgb(255, 205, 75).gamma_multiply(0.3),
                                        ))
                                        .corner_radius(6)
                                        .inner_margin(Margin::same(8))
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new(format!("Sugestão: {suggestion}"))
                                                    .color(Color32::from_rgb(255, 215, 112))
                                                    .size(10.0),
                                            );
                                        });
                                }
                                if !matches!(
                                    command.state,
                                    ProcessState::Running | ProcessState::Starting
                                ) && let Some(history) = &process_history
                                    && history.updated_at > 0
                                {
                                    ui.add_space(3.0);
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
                                        .size(9.0),
                                    );
                                }
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(command.command_line())
                                        .color(theme::MUTED)
                                        .monospace()
                                        .size(10.0),
                                );
                                ui.horizontal(|ui| {
                                    if command.working_directory != command.project_path
                                        && let Ok(relative) = command
                                            .working_directory
                                            .strip_prefix(&command.project_path)
                                    {
                                        ui.label(
                                            RichText::new(format!("Pasta: {}", relative.display()))
                                                .color(theme::MUTED)
                                                .size(9.0),
                                        );
                                    }
                                    if let Some(process_id) = command.process_id {
                                        ui.label(
                                            RichText::new(format!("PID {process_id}"))
                                                .color(theme::MUTED)
                                                .monospace()
                                                .size(9.0),
                                        );
                                    }
                                    if let Some(exit_code) = command.exit_code {
                                        ui.label(
                                            RichText::new(format!("Código de saída: {exit_code}"))
                                                .color(theme::MUTED)
                                                .monospace()
                                                .size(9.0),
                                        );
                                    }
                                });
                                if let Some(running) = self.running_processes.get(&command.id)
                                    && !running.logs.is_empty()
                                {
                                    ui.add_space(4.0);
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
                                                            .size(9.0),
                                                    );
                                                }
                                            });
                                    });
                                }
                            });
                        ui.add_space(8.0);
                    }
                });
            ui.add_space(12.0);
        }
        match action {
            Some(ProcessAction::Start(id)) => self.start_process(&id),
            Some(ProcessAction::Stop(id)) => self.stop_process(&id),
            Some(ProcessAction::Restart(id)) => self.restart_process(&id),
            Some(ProcessAction::StartProject(path)) => self.start_project(&path),
            Some(ProcessAction::StopProject(path)) => self.stop_project(&path),
            Some(ProcessAction::PrepareEnvironment(path)) => {
                self.request_environment_plan(&path, ui.ctx().clone());
            }
            Some(ProcessAction::InstallDependencies(path)) => {
                self.install_dependencies(&path, ui.ctx().clone());
            }
            Some(ProcessAction::ViewDependencyLogs(path)) => {
                self.dependency_terminal = Some(path);
            }
            Some(ProcessAction::ClearLogs(id)) => self.clear_process_logs(&id),
            Some(ProcessAction::ViewLogs(id)) => self.terminal_process = Some(id),
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

    fn settings_page(&self, ui: &mut egui::Ui) {
        ui.heading("Runtimes");
        ui.label(
            RichText::new("Inventário usado pelos projetos deste workspace").color(theme::MUTED),
        );
        ui.add_space(16.0);

        let mut installations: HashMap<(String, String, PathBuf), Vec<String>> = HashMap::new();
        for project in &self.projects {
            let Some(environment) = self.project_environments.get(&project.path) else {
                continue;
            };
            for requirement in &environment.runtime_requirements {
                if let RequirementStatus::Available { version, path } = &requirement.status {
                    installations
                        .entry((
                            requirement.runtime.to_string(),
                            version.clone(),
                            path.clone(),
                        ))
                        .or_default()
                        .push(project.name.clone());
                }
            }
        }
        let mut installations = installations.into_iter().collect::<Vec<_>>();
        installations.sort_by(|left, right| left.0.cmp(&right.0));

        if installations.is_empty() {
            ui.label(
                RichText::new("Nenhum runtime foi validado nos projetos encontrados.")
                    .color(theme::MUTED),
            );
        } else {
            for ((runtime, version, path), mut projects) in installations {
                projects.sort();
                projects.dedup();
                Frame::new()
                    .fill(theme::SURFACE)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .corner_radius(8)
                    .inner_margin(Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(runtime).strong().size(13.0));
                            ui.label(RichText::new(version).color(theme::SUCCESS).size(11.0));
                        });
                        ui.label(
                            RichText::new(path.display().to_string())
                                .color(theme::MUTED)
                                .monospace()
                                .size(10.0),
                        );
                        ui.label(
                            RichText::new(format!("Projetos: {}", projects.join(", ")))
                                .color(theme::MUTED)
                                .size(10.0),
                        );
                    });
                ui.add_space(8.0);
            }
        }

        ui.add_space(18.0);
        ui.heading("Histórico de preparação");
        ui.add_space(8.0);
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
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(Margin::same(36)),
            )
            .show(root_ui, |ui| {
                self.show_update_banner(ui);
                egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                    Page::Overview => self.overview(ui),
                    Page::Projects => self.projects_page(&ctx, ui),
                    Page::Processes => self.processes_page(ui),
                    Page::Settings => self.settings_page(ui),
                    Page::Plugins => {
                        ui.heading(self.page_title());
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!(
                                "A área de {} está pronta para a próxima etapa.",
                                "plugins"
                            ))
                            .color(theme::MUTED),
                        );
                    }
                });
            });
    }
}

impl eframe::App for LocalCodePilot {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_discovery();
        self.poll_environment_plan();
        self.poll_dependency_installs(ctx);
        self.poll_laravel_migration(ctx);
        self.poll_processes(ctx);
        self.poll_update_check();
        self.persist_workspace_state(false);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.sidebar(ui);
        self.topbar(ui);
        self.content(ui);
        self.show_terminal(ui);
        self.show_environment_plan_confirmation(ui);
        self.show_dependency_terminal(ui);
        self.show_laravel_migration(ui);
        self.show_port_termination_confirmation(ui);
        self.show_notification(ui);
    }
}

impl Drop for LocalCodePilot {
    fn drop(&mut self) {
        let active_ids: Vec<_> = self
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
            if let Some(process) = self.processes.iter_mut().find(|process| process.id == *id) {
                process.state = ProcessState::Stopped;
            }
            self.record_process_history(id);
        }
        self.persist_workspace_state(true);
        for running in self.running_processes.values_mut() {
            if !running.finished {
                terminate_process(&mut running.child);
            }
        }
    }
}

fn spawn_discovery(
    source: FilesystemProjectSource,
    repaint: egui::Context,
) -> Receiver<Result<Vec<Project>, String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let service = DiscoveryService::new(source, ManifestRuntimeDetector);
        let result = service
            .discover()
            .map(|catalog| catalog.into_projects())
            .map_err(|error| error.to_string());
        let _ = sender.send(result);
        repaint.request_repaint();
    });
    receiver
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
    let output = Command::new("php")
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
    };
    let mut child = process_command(&process)
        .current_dir(working_directory)
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
    let mut steps = Vec::new();
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
            RuntimeKind::Rust | RuntimeKind::Node | RuntimeKind::Flutter => continue,
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
            RuntimeKind::Rust | RuntimeKind::Node | RuntimeKind::Flutter => continue,
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

    if environment
        .missing_programs
        .iter()
        .any(|program| program == "composer")
    {
        let install_directory = managed_bin_directory().ok_or_else(|| {
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
            let install_directory = managed_bin_directory().ok_or_else(|| {
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
                label: format!("Instalar {}", step.label),
                detail: format!(
                    "Executar no projeto: {} {}",
                    step.program,
                    step.args.join(" ")
                ),
                source: Some("Manifesto do projeto".into()),
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
        InstallationStepKind::Command { program, args, .. } => std::iter::once(program.as_str())
            .chain(args.iter().map(String::as_str))
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
    let mut environment_path = None;
    for (index, step) in plan.steps.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("A preparação foi cancelada".into());
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
            InstallationStepKind::Command {
                program,
                args,
                working_directory,
            } => {
                let command_line = installation_step_command(step);
                let mut command = installation_command(program, args, environment_path.as_deref())?;
                let mut child = command
                    .current_dir(working_directory)
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
                let status = loop {
                    if cancel.load(Ordering::Relaxed) {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("A preparação foi cancelada".into());
                    }
                    if step_started.elapsed() >= INSTALLATION_STEP_TIMEOUT {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!(
                            "{} excedeu o limite de {} minutos",
                            step.label,
                            INSTALLATION_STEP_TIMEOUT.as_secs() / 60
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
                if !status.success() {
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

fn install_composer(
    install_directory: &Path,
    environment_path: Option<&OsStr>,
    sender: &mpsc::Sender<DependencyInstallEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    const SIGNATURE_URL: &str = "https://composer.github.io/installer.sig";
    const INSTALLER_URL: &str = "https://getcomposer.org/installer";
    if cancel.load(Ordering::Relaxed) {
        return Err("A preparação foi cancelada".into());
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
        .current_dir(install_directory)
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
    #[cfg(target_os = "windows")]
    let mut command = if executable
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat"))
    {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C"]).arg(&executable).args(args);
        command
    } else {
        let mut command = Command::new(&executable);
        command.args(args);
        command
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut command = Command::new(&executable);
        command.args(args);
        command
    };
    if let Some(path) = environment_path {
        command.env("PATH", path);
    }
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
        "Abra o terminal do processo e revise a primeira mensagem de erro antes do encerramento. Depois de corrigir a causa, use Tentar novamente."
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
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C"]).arg(&process.program).args(&args);
        if let Some(path) = environment_path {
            command.env("PATH", path);
        }
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

fn executable_available(program: &str) -> bool {
    let program_path = Path::new(program);
    if program_path.components().count() > 1 {
        return executable_candidate(program_path);
    }

    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        let candidate = directory.join(program);
        if executable_candidate(&candidate) {
            return true;
        }

        #[cfg(target_os = "windows")]
        if candidate.extension().is_none() {
            let path_extensions =
                std::env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
            return path_extensions
                .to_string_lossy()
                .split(';')
                .any(|extension| {
                    let extension = extension.trim().trim_start_matches('.');
                    !extension.is_empty()
                        && executable_candidate(&candidate.with_extension(extension))
                });
        }

        false
    })
}

fn executable_candidate(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn spawn_output_reader(reader: impl Read + Send + 'static, sender: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            let line = strip_terminal_sequences(&line);
            if sender.send(line).is_err() {
                break;
            }
        }
    });
}

fn terminate_process(child: &mut Child) {
    #[cfg(target_os = "windows")]
    {
        let process_id = child.id().to_string();
        if Command::new("taskkill")
            .args(["/PID", &process_id, "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            let _ = child.wait();
            return;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
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

fn process_exposes_application_url(project: &Project, process: &ProjectProcess) -> bool {
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

fn system_time_timestamp(time: Option<SystemTime>) -> u64 {
    time.and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
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

fn process_state_key(state: ProcessState) -> &'static str {
    match state {
        ProcessState::Stopped => "stopped",
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
        _ => "finalizado",
    }
}

fn project_history_status(history: &config::ProjectHistory) -> &'static str {
    history
        .services
        .iter()
        .max_by_key(|service| service.updated_at)
        .map(|service| service_history_status(&service.status))
        .unwrap_or("sem execução")
}

fn process_state_display(state: ProcessState) -> (&'static str, Color32) {
    match state {
        ProcessState::Stopped => ("Parado", theme::MUTED),
        ProcessState::Starting => ("Iniciando", Color32::from_rgb(255, 205, 75)),
        ProcessState::Running => ("Executando", theme::SUCCESS),
        ProcessState::Failed => ("Falhou", Color32::from_rgb(235, 87, 87)),
    }
}

fn stat_card(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    value: &str,
    detail: &str,
    color: Color32,
) {
    Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(12)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_min_height(62.0);
            ui.horizontal_wrapped(|ui| {
                Frame::new()
                    .fill(color.gamma_multiply(0.12))
                    .corner_radius(9)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.label(RichText::new(icon).color(color).size(18.0));
                    });
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).color(theme::MUTED).size(11.0));
                    ui.label(RichText::new(value).strong().size(19.0));
                });
                ui.with_layout(Layout::right_to_left(Align::BOTTOM), |ui| {
                    ui.label(
                        RichText::new(detail)
                            .color(Color32::from_rgb(105, 114, 130))
                            .size(9.0),
                    );
                });
            });
        });
}

fn inspect_project_environment(
    project: &Project,
    processes: &[ProjectProcess],
    environment_path: Option<&OsStr>,
) -> ProjectEnvironment {
    let project_processes: Vec<_> = processes
        .iter()
        .filter(|process| process.project_path == project.path)
        .collect();
    let mut missing_programs = Vec::new();
    let mut missing_dependencies = Vec::new();
    let mut install_steps = Vec::new();
    let node_inventory = inspect_program("node", &["--version"], environment_path);
    let python_inventory = inspect_program("python", &["--version"], environment_path);
    let php_inventory = inspect_program(
        "php",
        &["-r", "echo PHP_VERSION, PHP_EOL;"],
        environment_path,
    );
    let java_inventory = inspect_program("java", &["-version"], environment_path);
    let go_inventory = inspect_program("go", &["version"], environment_path);
    let dart_inventory = inspect_program("dart", &["--version"], environment_path);
    let installed_php_extensions = inspect_php_extensions(environment_path);
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
        .filter(|extension| !installed_php_extensions.contains(extension))
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
        requirement.status = runtime_requirement_status(&requirement, node_inventory.as_ref());
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
                python_inventory.as_ref(),
            ),
            (
                detect_php_requirement(directory, &project.path),
                php_inventory.as_ref(),
            ),
            (
                detect_java_requirement(directory, &project.path),
                java_inventory.as_ref(),
            ),
            (
                detect_go_requirement(directory, &project.path),
                go_inventory.as_ref(),
            ),
            (
                detect_dart_requirement(directory, &project.path),
                dart_inventory.as_ref(),
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
        if find_executable(&process.program, environment_path).is_none()
            && !missing_programs.contains(&process.program)
        {
            missing_programs.push(process.program.clone());
        }

        let uses_node = matches!(process.program.as_str(), "npm" | "pnpm" | "yarn" | "bun");
        let missing_node_dependencies = uses_node
            && process.working_directory.join("package.json").is_file()
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
    ProjectEnvironment {
        command_count: project_processes.len(),
        missing_programs,
        missing_dependencies,
        install_steps,
        runtime_requirements,
        missing_php_extensions,
        laravel_constraint,
    }
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

fn project_card(
    ui: &mut egui::Ui,
    project: &Project,
    view: ProjectCardView<'_>,
) -> Option<ProjectCardAction> {
    let ProjectCardView {
        environment,
        active_processes,
        dependency_install,
        application_url,
        history,
        favorite,
        resolving_plan,
        width,
    } = view;
    let mut action = None;
    let installing_dependencies = matches!(
        &dependency_install,
        Some(DependencyInstallStatus::Running | DependencyInstallStatus::Cancelling)
    );
    let dependency_install_failed = matches!(
        &dependency_install,
        Some(DependencyInstallStatus::Failed { .. })
    );
    let has_dependency_logs = dependency_install.is_some();
    Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(12)
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(width);
            ui.set_min_height(126.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&project.name).strong().size(15.0))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_ui(|ui| {
                        ui.set_max_width(420.0);
                        ui.label(RichText::new("Local do projeto").strong().size(11.0));
                        ui.label(
                            RichText::new(project.path.to_string_lossy())
                                .color(theme::MUTED)
                                .monospace()
                                .size(10.0),
                        );
                    });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .button(if favorite { "★" } else { "☆" })
                        .on_hover_text(if favorite {
                            "Remover dos favoritos"
                        } else {
                            "Adicionar aos favoritos"
                        })
                        .clicked()
                    {
                        action = Some(ProjectCardAction::ToggleFavorite(project.path.clone()));
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if project.runtimes.is_empty() && project.technologies.is_empty() {
                    runtime_badge(ui, "Projeto local", theme::MUTED);
                } else {
                    for technology in &project.technologies {
                        runtime_badge(ui, &technology.to_string(), technology_color(*technology));
                    }
                    for runtime in project
                        .runtimes
                        .iter()
                        .copied()
                        .filter(|runtime| project.shows_runtime(*runtime))
                    {
                        runtime_badge(ui, &runtime.to_string(), runtime_color(runtime));
                    }
                }
            });
            ui.add_space(10.0);
            let (status, status_color) = if resolving_plan {
                ("Preparando plano", theme::PRIMARY)
            } else if installing_dependencies {
                ("Preparando ambiente", theme::PRIMARY)
            } else if dependency_install_failed {
                ("Falha na instalação", Color32::from_rgb(235, 87, 87))
            } else if application_url.is_some() {
                ("Disponível", theme::SUCCESS)
            } else if active_processes > 0 {
                ("Em execução", theme::SUCCESS)
            } else {
                environment.status()
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(CIRCLE).color(status_color).size(7.0));
                ui.label(
                    RichText::new(status)
                        .color(status_color)
                        .strong()
                        .size(10.0),
                );
                if environment.command_count > 0 {
                    ui.label(
                        RichText::new(format!("· {} serviço(s)", environment.command_count))
                            .color(theme::MUTED)
                            .size(9.0),
                    );
                }
            });
            if !installing_dependencies && let Some(details) = environment.problem_details() {
                ui.label(RichText::new(details).color(theme::MUTED).size(9.0));
            }
            if active_processes == 0
                && let Some(history) = history
                && history.last_activity > 0
            {
                let last_status = project_history_status(history);
                ui.label(
                    RichText::new(format!(
                        "Última sessão: {} · {last_status}",
                        relative_time(history.last_activity)
                    ))
                    .color(theme::MUTED)
                    .size(9.0),
                );
            }
            ui.add_space(10.0);
            if let Some(url) = application_url
                && ui
                    .add_sized(
                        [ui.available_width(), 30.0],
                        egui::Button::new(RichText::new(format!(
                            "{ARROW_SQUARE_OUT}  Abrir aplicação"
                        )))
                        .fill(theme::SUCCESS.gamma_multiply(0.25)),
                    )
                    .clicked()
            {
                action = Some(ProjectCardAction::OpenUrl(url.to_owned()));
            }
            if active_processes > 0 {
                if ui
                    .add_sized(
                        [ui.available_width(), 30.0],
                        egui::Button::new("Parar ambiente")
                            .fill(Color32::from_rgb(235, 87, 87).gamma_multiply(0.18)),
                    )
                    .clicked()
                {
                    action = Some(ProjectCardAction::Stop(project.path.clone()));
                }
            } else if resolving_plan {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Consultando versões disponíveis...");
                });
            } else if installing_dependencies {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Preparando ambiente...");
                });
            } else if environment.can_prepare_environment() {
                if ui
                    .add_sized(
                        [ui.available_width(), 30.0],
                        egui::Button::new("Preparar ambiente")
                            .fill(theme::PRIMARY.gamma_multiply(0.72)),
                    )
                    .clicked()
                {
                    action = Some(ProjectCardAction::PrepareEnvironment(project.path.clone()));
                }
            } else if environment.can_install_dependencies() {
                if ui
                    .add_sized(
                        [ui.available_width(), 30.0],
                        egui::Button::new(if dependency_install_failed {
                            "Tentar novamente"
                        } else {
                            "Instalar dependências"
                        })
                        .fill(theme::PRIMARY.gamma_multiply(0.72)),
                    )
                    .clicked()
                {
                    action = Some(ProjectCardAction::InstallDependencies(project.path.clone()));
                }
            } else {
                let start = ui.add_enabled(
                    environment.is_ready(),
                    egui::Button::new(RichText::new(format!(
                        "{PLAY}  {}",
                        if history.is_some() {
                            "Retomar ambiente"
                        } else {
                            "Iniciar ambiente"
                        }
                    )))
                    .min_size(egui::vec2(ui.available_width(), 30.0))
                    .fill(theme::PRIMARY.gamma_multiply(0.72)),
                );
                let start = if let Some(details) = environment.problem_details() {
                    start.on_hover_text(details)
                } else {
                    start
                };
                if start.clicked() {
                    action = Some(ProjectCardAction::Start(project.path.clone()));
                }
            }
            if active_processes > 0
                && ui
                    .link(RichText::new(format!("{TERMINAL}  Ver logs")).size(10.0))
                    .clicked()
            {
                action = Some(ProjectCardAction::ViewLogs(project.path.clone()));
            }
            if has_dependency_logs
                && ui
                    .link(
                        RichText::new(if dependency_install_failed {
                            format!("{TERMINAL}  Ver erro e sugestões")
                        } else {
                            format!("{TERMINAL}  Ver logs da instalação")
                        })
                        .size(10.0),
                    )
                    .clicked()
            {
                action = Some(ProjectCardAction::ViewDependencyLogs(project.path.clone()));
            }
            if environment.legacy_laravel_constraint().is_some()
                && ui
                    .link(RichText::new("Planejar atualização do Laravel").size(10.0))
                    .clicked()
            {
                action = Some(ProjectCardAction::PrepareLaravelMigration(
                    project.path.clone(),
                ));
            }
            if ui
                .link(RichText::new(format!("{CODE_SIMPLE}  Abrir no VS Code")).size(10.0))
                .clicked()
            {
                action = Some(ProjectCardAction::Open(project.path.clone()));
            }
        });
    action
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
                .arg(path)
                .spawn()
                .map(|_| ())
                .map_err(|error| format!("Não foi possível abrir o VS Code: {error}"));
        }
    }

    Command::new(if cfg!(windows) { "code.cmd" } else { "code" })
        .arg(path)
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
            ui.label(RichText::new(label).color(color).strong().size(9.0));
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
    }
}

fn technology_color(technology: TechnologyKind) -> Color32 {
    match technology {
        TechnologyKind::Laravel => Color32::from_rgb(255, 70, 70),
        TechnologyKind::Lumen => Color32::from_rgb(239, 110, 80),
        TechnologyKind::Vue => Color32::from_rgb(66, 184, 131),
        TechnologyKind::React => Color32::from_rgb(97, 218, 251),
        TechnologyKind::TypeScript => Color32::from_rgb(49, 120, 198),
        TechnologyKind::JavaScript => Color32::from_rgb(240, 219, 79),
        TechnologyKind::Flutter => Color32::from_rgb(84, 197, 248),
        TechnologyKind::StaticSite => Color32::from_rgb(70, 180, 140),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DependencyInstall, DependencyInstallEvent, ProjectEnvironment, dependency_error_suggestion,
        executable_available, initials_from_username, inspect_project_environment, laravel_major,
        lowercase_hex, prepare_laravel_migration_files, process_error_suggestion,
        process_exposes_application_url, process_matches_search, project_process_ids,
        resolve_environment_plan, run_dependency_installs, runtime_requirement_status,
    };
    use localcodepilot_core::{
        environments::{
            InstallationStepKind, RequirementSource, RequirementStatus, RuntimeRequirement,
            VersionConstraint,
        },
        ports::Port,
        processes::{ProcessState, ProjectProcess},
        projects::Project,
        runtimes::RuntimeKind,
        technologies::TechnologyKind,
    };
    use localcodepilot_platform::installations::ProgramInventory;
    use std::{ffi::OsStr, fs, path::PathBuf, sync::mpsc, time::SystemTime};

    #[test]
    fn encodes_checksum_bytes_as_lowercase_hex() {
        assert_eq!(lowercase_hex(&[0x00, 0x09, 0xaf, 0xff]), "0009afff");
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

        let plan = resolve_environment_plan(&project_path, &environment).unwrap();
        assert!(plan.steps.iter().any(|step| matches!(
            &step.kind,
            InstallationStepKind::CreateCorepackShim { manager, .. } if manager == "pnpm"
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

        assert!(executable_available(&current_executable.to_string_lossy()));
    }

    #[test]
    fn rejects_a_program_that_does_not_exist() {
        assert!(!executable_available(
            "localcodepilot-program-that-does-not-exist-7d2bcfd8"
        ));
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
        };
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
    }
}
