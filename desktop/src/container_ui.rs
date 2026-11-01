use super::*;
use localcodepilot_core::compose_safety::{
    BlockedAction, ComposeEnvironment, ComposeName, ComposeSafety, ComposeVolume,
    classify_environment, explain, guard_container_action, guard_environment_action,
};
use localcodepilot_core::container_setup::{ContainerHostFacts, EngineGuidance, engine_guidance};
use localcodepilot_core::containers::{
    ContainerAction, ContainerCommand, ContainerEngine, ContainerProject, EngineDiagnosis,
    EngineState, RuntimeContainer, same_folder,
};
use localcodepilot_platform::containers::{
    ContainerExecution, ContainerExecutionEvent, ContainerExecutionOutcome, Precheck, compose_name,
    list_compose_volumes, query_engine_containers, verify_compose_action,
};
use std::hash::{DefaultHasher, Hash, Hasher};

struct ContainerSession {
    command: ContainerCommand,
    execution: Option<ContainerExecution>,
    logs: std::collections::VecDeque<String>,
    outcome: Option<ContainerExecutionOutcome>,
}

/// `projects` and `engines` are `None` after a runtime-only refresh, which keeps the
/// previous Compose scan and engine diagnosis.
struct ContainerSnapshot {
    projects: Option<Vec<ContainerProject>>,
    engines: Option<(Vec<EngineDiagnosis>, ContainerHostFacts)>,
    runtime: Vec<EngineRuntime>,
    /// Volumes are only listed on full refreshes; otherwise the previous lists are kept.
    volumes_refreshed: bool,
    compose_names: Option<ComposeNames>,
    /// Registry PATH re-read while an engine was missing.
    refreshed_path: Option<OsString>,
}

/// Containers and Compose volumes of one engine; `None` when the query failed, so the
/// Compose protection can fail closed.
#[derive(Debug, Clone)]
struct EngineRuntime {
    engine: ContainerEngine,
    containers: Option<Vec<RuntimeContainer>>,
    volumes: Option<Vec<ComposeVolume>>,
}

/// Compose project name per environment folder, with the fingerprint of the inputs that
/// decide it (manifests, `.env`, `COMPOSE_PROJECT_NAME`).
type ComposeNames = HashMap<PathBuf, (u64, ComposeName)>;

struct VolumeConfirmation {
    engine: ContainerEngine,
    directory: PathBuf,
    volumes: Vec<ComposeVolume>,
}

/// Queries both engines in parallel. Engines whose CLI is missing are skipped and reported
/// as failed queries, which keeps the Compose protection closed for them. Volumes are only
/// listed when `with_volumes` is set.
fn query_engine_runtime(
    path: Option<&OsStr>,
    missing: &[ContainerEngine],
    with_volumes: bool,
) -> Vec<EngineRuntime> {
    std::thread::scope(|scope| {
        [ContainerEngine::Docker, ContainerEngine::Podman]
            .map(|engine| {
                let query = scope.spawn(move || {
                    if missing.contains(&engine) {
                        return (None, None);
                    }
                    (
                        query_engine_containers(engine, path).ok(),
                        with_volumes
                            .then(|| list_compose_volumes(engine, path).ok())
                            .flatten(),
                    )
                });
                (engine, query)
            })
            .into_iter()
            .map(|(engine, query)| {
                let (containers, volumes) = query.join().unwrap_or((None, None));
                EngineRuntime {
                    engine,
                    containers,
                    volumes,
                }
            })
            .collect()
    })
}

/// Identities of the volumes the user confirmed for this engine and Compose folder.
fn confirmed_identities(
    confirmations: &[config::ComposeDataConfirmation],
    engine: ContainerEngine,
    directory: &Path,
) -> Vec<String> {
    confirmations
        .iter()
        .filter(|confirmation| {
            confirmation.engine == engine.program()
                && same_folder(&confirmation.directory, directory)
        })
        .flat_map(|confirmation| confirmation.volumes.iter().cloned())
        .collect()
}

fn compose_name_fingerprint(project: &ContainerProject) -> u64 {
    let mut hasher = DefaultHasher::new();
    for file in project
        .compose_file
        .iter()
        .chain(&project.compose_overrides)
        .cloned()
        .chain(std::iter::once(project.directory.join(".env")))
    {
        file.hash(&mut hasher);
        fs::metadata(&file)
            .and_then(|metadata| metadata.modified())
            .ok()
            .hash(&mut hasher);
    }
    std::env::var_os("COMPOSE_PROJECT_NAME").hash(&mut hasher);
    hasher.finish()
}

/// Asks the Compose provider for each environment's project name, reusing names whose
/// inputs did not change. Falls back to an estimate that only serves to find conflicts.
fn resolve_compose_names(
    projects: &[ContainerProject],
    cache: &ComposeNames,
    engines: &[EngineDiagnosis],
    path: Option<&OsStr>,
) -> ComposeNames {
    projects
        .iter()
        .filter(|project| project.compose_file.is_some())
        .map(|project| {
            let fingerprint = compose_name_fingerprint(project);
            let cached = cache
                .get(&project.directory)
                .filter(|(cached, name)| {
                    *cached == fingerprint && matches!(name, ComposeName::Resolved(_))
                })
                .map(|(_, name)| name.clone());
            let name = cached.unwrap_or_else(|| compose_name(engines, project, path));
            (project.directory.clone(), (fingerprint, name))
        })
        .collect()
}

fn engine_of(command: &ContainerCommand) -> ContainerEngine {
    match command.program {
        "podman" => ContainerEngine::Podman,
        _ => ContainerEngine::Docker,
    }
}

enum BlockedPanelAction {
    Copy(String),
    ConfirmVolumes,
}

/// Explains why Compose actions are blocked and how to resolve it.
fn blocked_panel(ui: &mut egui::Ui, blocked: &BlockedAction) -> Option<BlockedPanelAction> {
    let warning = Color32::from_rgb(255, 205, 75);
    let mut action = None;
    Frame::new()
        .fill(warning.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, warning.gamma_multiply(0.6)))
        .corner_radius(8)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&blocked.title).strong().color(warning));
            ui.label(&blocked.explanation);
            ui.label(
                RichText::new("Iniciar e parar o ambiente ficam bloqueados até resolver.")
                    .color(theme::MUTED)
                    .size(11.0),
            );
            for (index, step) in blocked.steps.iter().enumerate() {
                ui.label(RichText::new(format!("{}. {step}", index + 1)).size(12.0));
            }
            if let Some(snippet) = &blocked.snippet {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(snippet).monospace());
                    if ui.small_button("Copiar trecho").clicked() {
                        ui.ctx().copy_text(snippet.clone());
                        action = Some(BlockedPanelAction::Copy(snippet.clone()));
                    }
                });
            }
            if blocked.can_confirm_volumes && ui.button("Estes dados são deste projeto").clicked()
            {
                action = Some(BlockedPanelAction::ConfirmVolumes);
            }
        });
    action
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerRefresh {
    /// Compose scan of every catalogued project, engine diagnosis and container list.
    Full,
    /// Only `ps`, cheap enough to keep project cards current.
    RuntimeOnly,
}

/// Pages that show container state and how often they refresh it.
fn container_refresh_schedule(page: Page) -> Option<(Duration, ContainerRefresh)> {
    match page {
        Page::Processes | Page::Containers => {
            Some((Duration::from_secs(15), ContainerRefresh::Full))
        }
        Page::Overview | Page::Projects => {
            Some((Duration::from_secs(30), ContainerRefresh::RuntimeOnly))
        }
        Page::Plugins | Page::Help | Page::Settings => None,
    }
}

/// An engine installed from a terminal after launch is only on the registry PATH, so a full
/// check re-reads it while an engine is missing. Re-reading spawns PowerShell, so it is
/// skipped otherwise.
fn rereads_registry_path(scope: ContainerRefresh, engines: &[EngineDiagnosis]) -> bool {
    scope == ContainerRefresh::Full
        && engines
            .iter()
            .any(|engine| engine.state == EngineState::Missing)
}

/// Podman leaves `Status` empty, so it is only shown when the engine provides it.
fn container_summary(container: &RuntimeContainer) -> String {
    let mut summary = format!("{} · {}", container.engine, container.state);
    if !container.status.is_empty() {
        summary.push_str(" · ");
        summary.push_str(&container.status);
    }
    summary
}

/// Numbered preparation steps for an engine. Commands are only copied for the user to run.
/// Returns the command copied in this frame.
fn engine_guidance_panel(ui: &mut egui::Ui, guidance: &EngineGuidance) -> Option<&'static str> {
    let mut copied = None;
    ui.add_space(8.0);
    ui.label(RichText::new(guidance.summary).strong().size(12.0));
    for (index, step) in guidance.steps.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("{}.", index + 1))
                    .color(theme::MUTED)
                    .size(11.0),
            );
            match step.link {
                Some(link) => {
                    ui.hyperlink_to(RichText::new(step.text).size(11.0), link);
                }
                None => {
                    ui.label(RichText::new(step.text).size(11.0));
                }
            }
        });
        if let Some(command) = step.command {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(18.0);
                ui.label(RichText::new(command).monospace().size(11.0));
                if ui.small_button("Copiar").clicked() {
                    ui.ctx().copy_text(command.to_owned());
                    copied = Some(command);
                }
            });
        }
    }
    ui.add_space(4.0);
    for note in &guidance.notes {
        ui.label(RichText::new(*note).color(theme::MUTED).size(10.5));
    }
    copied
}

pub(super) struct ContainerState {
    projects: Vec<ContainerProject>,
    runtime_containers: Vec<RuntimeContainer>,
    last_refresh: Option<std::time::Instant>,
    engines: Vec<EngineDiagnosis>,
    host: Option<ContainerHostFacts>,
    engine_runtime: Vec<EngineRuntime>,
    compose_names: ComposeNames,
    /// Built from `compose_names` and the engine state whenever either changes, so screens
    /// read classifications instead of recomputing them on every frame.
    compose_catalog: Vec<ComposeEnvironment>,
    compose_safety: HashMap<(ContainerEngine, PathBuf), ComposeSafety>,
    volume_confirmation: Option<VolumeConfirmation>,
    pending: Option<Receiver<ContainerSnapshot>>,
    selected_engine: ContainerEngine,
    confirmation: Option<ContainerCommand>,
    execution: Option<ContainerSession>,
    error: Option<String>,
}

impl Default for ContainerState {
    fn default() -> Self {
        Self {
            projects: vec![],
            runtime_containers: vec![],
            last_refresh: None,
            engines: vec![],
            host: None,
            engine_runtime: vec![],
            compose_names: HashMap::new(),
            compose_catalog: vec![],
            compose_safety: HashMap::new(),
            volume_confirmation: None,
            pending: None,
            selected_engine: ContainerEngine::Docker,
            confirmation: None,
            execution: None,
            error: None,
        }
    }
}

impl LocalCodePilot {
    fn confirmed_volumes(&self, engine: ContainerEngine, directory: &Path) -> Vec<String> {
        confirmed_identities(
            &self.workspace_state.compose_data_confirmations,
            engine,
            directory,
        )
    }

    /// Classifies every catalogued environment on both engines. Runs when a snapshot arrives
    /// or a confirmation changes, not on every frame.
    fn recompute_compose_safety(&mut self) {
        let catalog = self
            .container_state
            .compose_names
            .iter()
            .map(|(directory, (_, name))| ComposeEnvironment {
                directory: directory.clone(),
                name: name.clone(),
            })
            .collect::<Vec<_>>();
        let mut safety = HashMap::new();
        for runtime in &self.container_state.engine_runtime {
            for environment in &catalog {
                let classified = classify_environment(
                    environment,
                    &catalog,
                    runtime.engine,
                    runtime.containers.as_deref(),
                    runtime.volumes.as_deref(),
                    &self.confirmed_volumes(runtime.engine, &environment.directory),
                );
                safety.insert((runtime.engine, environment.directory.clone()), classified);
            }
        }
        self.container_state.compose_catalog = catalog;
        self.container_state.compose_safety = safety;
    }

    fn compose_environment_safety(
        &self,
        directory: &Path,
        engine: ContainerEngine,
    ) -> ComposeSafety {
        self.container_state
            .compose_safety
            .get(&(engine, directory.to_path_buf()))
            .or_else(|| {
                self.container_state
                    .compose_safety
                    .iter()
                    .find(|((cached, folder), _)| {
                        *cached == engine && same_folder(folder, directory)
                    })
                    .map(|(_, safety)| safety)
            })
            .cloned()
            .unwrap_or_else(|| ComposeSafety::Unverifiable {
                reason: "A verificação deste ambiente ainda não terminou.".into(),
            })
    }

    fn container_action_guard(
        &self,
        action: ContainerAction,
        container: &RuntimeContainer,
    ) -> Result<(), BlockedAction> {
        if !matches!(
            action,
            ContainerAction::StartExisting
                | ContainerAction::StopExisting
                | ContainerAction::RestartExisting
        ) {
            return Ok(());
        }
        let environment_safety = container
            .project_directory
            .as_deref()
            .filter(|folder| {
                self.container_state
                    .compose_catalog
                    .iter()
                    .any(|environment| same_folder(&environment.directory, folder))
            })
            .map(|folder| self.compose_environment_safety(folder, container.engine));
        guard_container_action(
            action,
            container,
            &self.container_state.runtime_containers,
            &self.container_state.compose_catalog,
            environment_safety.as_ref(),
        )
    }

    /// Fresh verification run on the worker thread right before a command that changes a
    /// Compose environment or one of its containers. `None` for read-only commands.
    fn container_precheck(&self, command: &ContainerCommand) -> Option<Precheck> {
        let engine = engine_of(command);
        let path = self.environment_path.clone();
        let catalog = self.container_state.compose_catalog.clone();
        match command.action {
            ContainerAction::Start | ContainerAction::Stop => {
                let action = command.action;
                let Some(project) = self
                    .container_state
                    .projects
                    .iter()
                    .find(|project| same_folder(&project.directory, &command.working_directory))
                    .cloned()
                else {
                    return Some(Box::new(|| {
                        Err("O ambiente Compose não foi encontrado; verifique novamente.".into())
                    }));
                };
                let confirmed = self.confirmed_volumes(engine, &project.directory);
                Some(Box::new(move || {
                    verify_compose_action(
                        action,
                        engine,
                        &project,
                        &catalog,
                        &confirmed,
                        path.as_deref(),
                    )
                    .map_err(|blocked| format!("{}. {}", blocked.title, blocked.explanation))
                }))
            }
            ContainerAction::StartExisting
            | ContainerAction::StopExisting
            | ContainerAction::RestartExisting => {
                let action = command.action;
                let id = command.args.last()?.clone();
                let confirmations = self.workspace_state.compose_data_confirmations.clone();
                Some(Box::new(move || {
                    let containers =
                        query_engine_containers(engine, path.as_deref()).map_err(|error| {
                            format!("Não foi possível consultar os containers: {error}")
                        })?;
                    let container = containers
                        .iter()
                        .find(|container| container.id == id)
                        .ok_or("O container não existe mais.")?;
                    let environment = container.project_directory.as_deref().and_then(|folder| {
                        catalog
                            .iter()
                            .find(|environment| same_folder(&environment.directory, folder))
                    });
                    let safety = environment.map(|environment| {
                        let volumes = list_compose_volumes(engine, path.as_deref()).ok();
                        let confirmed =
                            confirmed_identities(&confirmations, engine, &environment.directory);
                        classify_environment(
                            environment,
                            &catalog,
                            engine,
                            Some(&containers),
                            volumes.as_deref(),
                            &confirmed,
                        )
                    });
                    guard_container_action(
                        action,
                        container,
                        &containers,
                        &catalog,
                        safety.as_ref(),
                    )
                    .map_err(|blocked| format!("{}. {}", blocked.title, blocked.explanation))
                }))
            }
            _ => None,
        }
    }

    fn volume_confirmation_dialog(&mut self, ctx: &egui::Context) {
        let Some(request) = &self.container_state.volume_confirmation else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Confirmar dados do ambiente")
            .id(egui::Id::unique("compose-volume-confirmation"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 520.0));
                ui.label(format!(
                    "Confirme que estes volumes do {} pertencem ao ambiente em {}:",
                    request.engine,
                    display_path(&request.directory)
                ));
                for volume in &request.volumes {
                    ui.label(RichText::new(&volume.name).monospace());
                }
                ui.label(
                    RichText::new(
                        "Ao iniciar, o ambiente vai usar esses dados. Confirme só se eles foram criados por este projeto.",
                    )
                    .color(Color32::from_rgb(255, 205, 75)),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add(egui::Button::new("Confirmar que são deste projeto").fill(theme::PRIMARY))
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.container_state.volume_confirmation = None;
        }
        if confirm && let Some(request) = self.container_state.volume_confirmation.take() {
            let engine = request.engine.program().to_owned();
            // Name and creation time: a volume recreated later under the same name is new data.
            let identities = request
                .volumes
                .iter()
                .filter_map(ComposeVolume::identity)
                .collect::<Vec<_>>();
            let confirmations = &mut self.workspace_state.compose_data_confirmations;
            match confirmations.iter_mut().find(|confirmation| {
                confirmation.engine == engine
                    && same_folder(&confirmation.directory, &request.directory)
            }) {
                Some(existing) => {
                    for identity in identities {
                        if !existing.volumes.contains(&identity) {
                            existing.volumes.push(identity);
                        }
                    }
                }
                None => confirmations.push(config::ComposeDataConfirmation {
                    engine,
                    directory: request.directory,
                    volumes: identities,
                }),
            }
            self.workspace_dirty = true;
            self.recompute_compose_safety();
            self.notify("Dados confirmados para este ambiente");
            self.refresh_containers(ctx.clone());
        }
    }

    pub(super) fn project_container_status(&self, project: &Path) -> (bool, Option<String>) {
        let running = self
            .container_state
            .runtime_containers
            .iter()
            .filter(|container| {
                container.state.eq_ignore_ascii_case("running") && container.belongs_to(project)
            });
        let mut found = false;
        let mut url = None;
        for container in running {
            found = true;
            if url.is_none() {
                url = container.browser_urls().into_iter().next();
            }
        }
        (found, url)
    }

    pub(super) fn database_container_running(&self, project: &Path, host: &str) -> Option<bool> {
        self.container_state
            .runtime_containers
            .iter()
            .find(|container| container.belongs_to(project) && container.answers_to_host(host))
            .map(|container| container.state.eq_ignore_ascii_case("running"))
    }
    pub(super) fn container_refresh_pending(&self) -> bool {
        self.container_state.pending.is_some()
    }
    pub(super) fn container_processes_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Containers Docker e Podman")
                    .strong()
                    .size(16.0),
            );
            if ui
                .add_enabled(
                    self.container_state.pending.is_none(),
                    egui::Button::new("Atualizar containers"),
                )
                .clicked()
            {
                self.refresh_containers(ui.ctx().clone());
            }
        });
        if self.container_state.pending.is_some() {
            ui.spinner();
        }
        if self.container_state.pending.is_none()
            && !self
                .container_state
                .runtime_containers
                .iter()
                .any(|container| {
                    self.focused_project
                        .as_ref()
                        .is_none_or(|path| container.belongs_to(path))
                })
        {
            ui.label(
                RichText::new(if self.focused_project.is_some() {
                    "Nenhum container deste projeto encontrado. Inicie os serviços em Containers."
                } else {
                    "Nenhum container retornado pelos motores disponíveis."
                })
                .color(theme::MUTED),
            );
        }
        self.container_execution_panel(ui);
        let mut requested = None;
        for container in self
            .container_state
            .runtime_containers
            .iter()
            .filter(|container| {
                self.focused_project
                    .as_ref()
                    .is_none_or(|path| container.belongs_to(path))
            })
        {
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(10)
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&container.name).strong());
                    ui.label(container_summary(container));
                    ui.label(format!("Imagem: {}", container.image));
                    if let Some(service) = &container.service {
                        ui.label(format!("Serviço: {service}"));
                    }
                    if let Some(directory) = &container.project_directory {
                        ui.label(format!("Projeto: {}", display_path(directory)));
                    }
                    if !container.ports.is_empty() {
                        ui.label(format!("Portas: {}", container.ports));
                    }
                    for url in container.browser_urls() {
                        if ui.link(format!("URL provável · abrir: {url}")).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&url));
                        }
                    }
                    if let Some(running_for) = &container.running_for {
                        ui.label(format!("Tempo de execução: {running_for}"));
                    }
                    let mut blocked_reason = None;
                    ui.horizontal_wrapped(|ui| {
                        for action in [
                            ContainerAction::StartExisting,
                            ContainerAction::StopExisting,
                            ContainerAction::RestartExisting,
                            ContainerAction::LogsExisting,
                        ] {
                            let Some(command) = container.command(action) else {
                                continue;
                            };
                            let guard = self.container_action_guard(action, container);
                            let idle = self
                                .container_state
                                .execution
                                .as_ref()
                                .is_none_or(|session| session.execution.is_none());
                            let response = ui.add_enabled(
                                idle && guard.is_ok(),
                                egui::Button::new(command.label),
                            );
                            if let Err(blocked) = guard {
                                response.on_disabled_hover_text(&blocked.explanation);
                                blocked_reason = Some(blocked.title);
                            } else if response.clicked() {
                                requested = Some(command);
                            }
                        }
                    });
                    if let Some(reason) = blocked_reason {
                        ui.label(
                            RichText::new(format!(
                                "{reason}: iniciar, parar e reiniciar estão bloqueados para não afetar outro projeto. Veja como resolver em Containers."
                            ))
                            .color(Color32::from_rgb(255, 205, 75))
                            .size(11.0),
                        );
                    }
                });
            ui.add_space(8.0);
        }
        if let Some(command) = requested {
            if command.action.changes_environment() {
                self.container_state.confirmation = Some(command);
            } else {
                self.start_container_execution(command);
            }
        }
        self.container_confirmation(ui.ctx());
        ui.add_space(8.0);
    }

    pub(super) fn refresh_containers(&mut self, ctx: egui::Context) {
        self.start_container_refresh(ctx, ContainerRefresh::Full);
    }

    fn start_container_refresh(&mut self, ctx: egui::Context, scope: ContainerRefresh) {
        self.container_state.last_refresh = Some(std::time::Instant::now());
        let directories = self
            .projects
            .iter()
            .map(|project| project.path.clone())
            .collect::<Vec<_>>();
        let path = self.environment_path.clone();
        let reread_path = rereads_registry_path(scope, &self.container_state.engines);
        let name_cache = self.container_state.compose_names.clone();
        let known_missing = self
            .container_state
            .engines
            .iter()
            .filter(|engine| engine.state == EngineState::Missing)
            .map(|engine| engine.engine)
            .collect::<Vec<_>>();
        let (sender, receiver) = mpsc::channel();
        self.container_state.pending = Some(receiver);
        self.container_state.error = None;
        std::thread::spawn(move || {
            let refreshed_path = reread_path
                .then(|| refreshed_environment_path().ok())
                .flatten();
            let path = refreshed_path.clone().or(path);
            let mut missing = known_missing;
            let (projects, engines, compose_names) = match scope {
                ContainerRefresh::Full => {
                    let mut seen = HashSet::new();
                    let projects: Vec<ContainerProject> = directories
                        .iter()
                        .flat_map(|directory| {
                            localcodepilot_runtime::containers::detect_container_projects(directory)
                        })
                        .filter(|project| seen.insert(project.directory.clone()))
                        .collect();
                    let engines = localcodepilot_platform::containers::inspect_container_engines(
                        path.as_deref(),
                    );
                    let host = localcodepilot_platform::containers::inspect_container_host(
                        path.as_deref(),
                    );
                    let names =
                        resolve_compose_names(&projects, &name_cache, &engines, path.as_deref());
                    missing = engines
                        .iter()
                        .filter(|engine| engine.state == EngineState::Missing)
                        .map(|engine| engine.engine)
                        .collect();
                    (Some(projects), Some((engines, host)), Some(names))
                }
                ContainerRefresh::RuntimeOnly => (None, None, None),
            };
            let volumes_refreshed = scope == ContainerRefresh::Full;
            let runtime = query_engine_runtime(path.as_deref(), &missing, volumes_refreshed);
            let _ = sender.send(ContainerSnapshot {
                projects,
                engines,
                runtime,
                volumes_refreshed,
                compose_names,
                refreshed_path,
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_containers(&mut self, ctx: &egui::Context) {
        if let Some((interval, scope)) = container_refresh_schedule(self.page) {
            ctx.request_repaint_after(interval);
            if self.container_state.pending.is_none()
                && self
                    .container_state
                    .last_refresh
                    .is_none_or(|last| last.elapsed() >= interval)
            {
                self.start_container_refresh(ctx.clone(), scope);
            }
        }
        if let Some(receiver) = &self.container_state.pending {
            match receiver.try_recv() {
                Ok(ContainerSnapshot {
                    projects,
                    engines,
                    mut runtime,
                    volumes_refreshed,
                    compose_names,
                    refreshed_path,
                }) => {
                    let containers = runtime
                        .iter()
                        .flat_map(|engine| engine.containers.iter().flatten().cloned())
                        .collect::<Vec<_>>();
                    if !volumes_refreshed {
                        for current in &mut runtime {
                            current.volumes = self
                                .container_state
                                .engine_runtime
                                .iter()
                                .find(|previous| previous.engine == current.engine)
                                .and_then(|previous| previous.volumes.clone());
                        }
                    }
                    self.container_state.engine_runtime = runtime;
                    if let Some(compose_names) = compose_names {
                        self.container_state.compose_names = compose_names;
                    }
                    self.recompute_compose_safety();
                    if refreshed_path.is_some() {
                        self.environment_path = refreshed_path;
                    }
                    let stopped = self
                        .container_state
                        .runtime_containers
                        .iter()
                        .filter(|old| old.state.eq_ignore_ascii_case("running"))
                        .filter_map(|old| {
                            containers
                                .iter()
                                .find(|new| new.engine == old.engine && new.id == old.id)
                                .filter(|new| !new.state.eq_ignore_ascii_case("running"))
                                .map(|new| new.name.clone())
                        })
                        .collect::<Vec<_>>();
                    if let Some((engines, host)) = engines {
                        if let Some(engine) = engines.iter().find(|engine| {
                            engine.state == EngineState::Ready && engine.compose_available
                        }) {
                            self.container_state.selected_engine = engine.engine;
                        }
                        self.container_state.engines = engines;
                        self.container_state.host = Some(host);
                    }
                    if let Some(projects) = projects {
                        self.container_state.projects = projects;
                    }
                    self.container_state.runtime_containers = containers;
                    self.container_state.pending = None;
                    if !self
                        .container_state
                        .execution
                        .as_ref()
                        .is_some_and(|session| {
                            matches!(
                                session.command.action,
                                ContainerAction::StopExisting | ContainerAction::RestartExisting
                            )
                        })
                    {
                        for name in stopped {
                            self.notify(format!("Container {name} não está mais em execução"));
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.container_state.pending = None;
                    self.container_state.error =
                        Some("O diagnóstico foi interrompido. Verifique novamente.".into());
                }
                Err(TryRecvError::Empty) => {}
            }
        }

        let Some(session) = &mut self.container_state.execution else {
            return;
        };
        let mut refresh_after_action = false;
        loop {
            let event = session
                .execution
                .as_ref()
                .map(|execution| execution.events.try_recv());
            match event {
                Some(Ok(ContainerExecutionEvent::Output(line))) => {
                    let line = if line.len() > 4_096 {
                        format!(
                            "{}… [linha truncada]",
                            line.chars().take(4_096).collect::<String>()
                        )
                    } else {
                        line
                    };
                    session.logs.push_back(line);
                    if session.logs.len() > 1_000 {
                        session.logs.pop_front();
                    }
                }
                Some(Ok(ContainerExecutionEvent::Finished(outcome))) => {
                    refresh_after_action = session.command.action.changes_environment();
                    session.outcome = Some(outcome);
                    session.execution = None;
                    ctx.request_repaint();
                    break;
                }
                Some(Err(TryRecvError::Disconnected)) => {
                    session.outcome = Some(ContainerExecutionOutcome::Failed(
                        "A execução foi interrompida sem resultado.".into(),
                    ));
                    session.execution = None;
                    break;
                }
                Some(Err(TryRecvError::Empty)) | None => break,
            }
        }
        if session.execution.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if refresh_after_action {
            self.refresh_containers(ctx.clone());
        }
    }

    fn start_container_execution(&mut self, command: ContainerCommand) {
        let display = command.display();
        let execution = match self.container_precheck(&command) {
            Some(precheck) => {
                localcodepilot_platform::containers::execute_container_command_checked(
                    command.clone(),
                    self.environment_path.as_deref(),
                    precheck,
                )
            }
            None => localcodepilot_platform::containers::execute_container_command(
                command.clone(),
                self.environment_path.as_deref(),
            ),
        };
        let mut logs = std::collections::VecDeque::new();
        logs.push_back(format!("$ {display}"));
        self.container_state.execution = Some(match execution {
            Ok(execution) => ContainerSession {
                command,
                execution: Some(execution),
                logs,
                outcome: None,
            },
            Err(error) => ContainerSession {
                command,
                execution: None,
                logs,
                outcome: Some(ContainerExecutionOutcome::Failed(error)),
            },
        });
    }

    pub(super) fn containers_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Containers");
        if let Some(project_path) = self.focused_project.clone() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Projeto: {}",
                    project_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
                if ui.button("Ver todos os projetos").clicked() {
                    self.focused_project = None;
                }
            });
        }
        ui.label(
            RichText::new("Execução controlada de ambientes Docker e Podman com Compose")
                .color(theme::MUTED),
        );
        ui.add_space(12.0);
        let checking = self.container_state.pending.is_some();
        let executing = self
            .container_state
            .execution
            .as_ref()
            .is_some_and(|session| session.execution.is_some());
        if ui
            .add_enabled(
                !checking && !executing && self.discovery.is_none(),
                egui::Button::new("Verificar motores e arquivos"),
            )
            .clicked()
        {
            self.refresh_containers(ui.ctx().clone());
        }
        if checking {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Verificando em segundo plano...");
            });
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if let Some(error) = &self.container_state.error {
            ui.colored_label(theme::MUTED, error);
        }

        self.container_execution_panel(ui);
        ui.add_space(12.0);
        let mut copied = None;
        for diagnosis in &self.container_state.engines {
            let guidance = self
                .container_state
                .host
                .as_ref()
                .and_then(|host| engine_guidance(diagnosis, host));
            Frame::new()
                .fill(theme::SURFACE)
                .corner_radius(8)
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(diagnosis.engine.to_string()).strong());
                    let (status, color) = match diagnosis.state {
                        EngineState::Ready => ("Motor disponível", theme::SUCCESS),
                        EngineState::Missing => ("CLI não encontrada no PATH", theme::MUTED),
                        EngineState::Unavailable => (
                            "CLI encontrada; motor parado ou inacessível",
                            Color32::from_rgb(255, 205, 75),
                        ),
                        EngineState::TimedOut => (
                            "Consulta excedeu o tempo limite; tente novamente",
                            Color32::from_rgb(255, 205, 75),
                        ),
                        EngineState::QueryFailed => (
                            "A CLI não respondeu corretamente",
                            Color32::from_rgb(255, 205, 75),
                        ),
                    };
                    ui.colored_label(color, status);
                    if diagnosis.state != EngineState::Missing {
                        ui.label(if diagnosis.compose_available {
                            "Compose disponível"
                        } else if diagnosis.state == EngineState::Ready {
                            "Compose não confirmado; verifique o plugin ou provedor"
                        } else {
                            "Compose será verificado quando o motor estiver ativo"
                        });
                    }
                    if let Some(guidance) = &guidance
                        && let Some(command) = engine_guidance_panel(ui, guidance)
                    {
                        copied = Some(command);
                    }
                });
            ui.add_space(6.0);
        }
        if let Some(command) = copied {
            self.notify(format!("Comando copiado: {command}"));
        }
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Executar com:");
            ui.selectable_value(
                &mut self.container_state.selected_engine,
                ContainerEngine::Docker,
                "Docker",
            );
            ui.selectable_value(
                &mut self.container_state.selected_engine,
                ContainerEngine::Podman,
                "Podman",
            );
        });
        let selected_diagnosis = self
            .container_state
            .engines
            .iter()
            .find(|diagnosis| diagnosis.engine == self.container_state.selected_engine)
            .cloned();
        let engine_ready = selected_diagnosis
            .as_ref()
            .is_some_and(|diagnosis| diagnosis.state == EngineState::Ready);
        let compose_ready = selected_diagnosis
            .as_ref()
            .is_some_and(|diagnosis| diagnosis.compose_available);
        if !engine_ready {
            ui.label(
                RichText::new("Verifique e inicie o motor selecionado para liberar as ações.")
                    .color(theme::MUTED)
                    .size(11.0),
            );
        }
        ui.add_space(16.0);
        if !checking
            && !self.container_state.projects.iter().any(|project| {
                self.focused_project
                    .as_ref()
                    .is_none_or(|path| project.directory.starts_with(path))
            })
        {
            ui.label(if self.focused_project.is_some() {
                "Nenhum arquivo Compose ou Dockerfile encontrado neste projeto."
            } else {
                "Nenhum Dockerfile, Containerfile ou manifesto Compose encontrado nos projetos."
            });
        }

        let mut requested_command = None;
        let mut copied_snippet = None;
        let mut volume_request = None;
        for project in self.container_state.projects.iter().filter(|project| {
            self.focused_project
                .as_ref()
                .is_none_or(|path| project.directory.starts_with(path))
        }) {
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(10)
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(
                            project
                                .directory
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                        )
                        .strong()
                        .size(16.0),
                    );
                    ui.label(
                        RichText::new(display_path(&project.directory))
                            .monospace()
                            .color(theme::MUTED)
                            .size(10.0),
                    );
                    let engine = self.container_state.selected_engine;
                    let safety = project
                        .compose_file
                        .is_some()
                        .then(|| self.compose_environment_safety(&project.directory, engine));
                    if let Some((_, name)) = self.container_state.compose_names.get(&project.directory) {
                        ui.label(match name {
                            ComposeName::Resolved(name) => format!("Ambiente Compose: \"{name}\""),
                            ComposeName::Estimated(name) => format!(
                                "Ambiente Compose: \"{name}\" (estimado; o provedor Compose não respondeu)"
                            ),
                        });
                    }
                    if let Some(safety) = &safety
                        && *safety != ComposeSafety::Safe
                    {
                        ui.add_space(6.0);
                        match blocked_panel(ui, &explain(safety, &project.directory)) {
                            Some(BlockedPanelAction::Copy(snippet)) => copied_snippet = Some(snippet),
                            Some(BlockedPanelAction::ConfirmVolumes) => {
                                if let ComposeSafety::UnconfirmedData { volumes, .. } = safety {
                                    volume_request = Some(VolumeConfirmation {
                                        engine,
                                        directory: project.directory.clone(),
                                        volumes: volumes.clone(),
                                    });
                                }
                            }
                            None => {}
                        }
                        ui.add_space(6.0);
                    }
                    for file in project.dockerfile.iter().chain(project.compose_file.iter()) {
                        ui.label(format!(
                            "Arquivo: {}",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                    for file in &project.compose_overrides {
                        ui.label(format!(
                            "Override: {}",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                    if let Some(error) = &project.error {
                        ui.colored_label(Color32::from_rgb(255, 112, 112), error);
                    }
                    for service in &project.services {
                        ui.separator();
                        ui.label(RichText::new(&service.name).strong());
                        if let Some(image) = &service.image {
                            ui.label(format!("Imagem: {image}"));
                        }
                        if service.builds_image {
                            ui.label("Imagem construída a partir do projeto");
                        }
                        if !service.ports.is_empty() {
                            ui.label(format!("Portas declaradas: {}", service.ports.join(", ")));
                        }
                        let urls = service.browser_urls();
                        for url in &urls {
                            if ui.link(format!("URL provável · abrir: {url}")).clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                            }
                        }
                        if !service.ports.is_empty() && urls.is_empty() {
                            ui.label(
                                RichText::new("URL web não confirmada para estas portas")
                                    .color(theme::MUTED)
                                    .size(11.0),
                            );
                        }
                        if !service.profiles.is_empty() {
                            ui.label(format!("Perfil opcional: {}", service.profiles.join(", ")));
                        }
                    }
                    if project.compose_file.is_none() {
                        ui.label(
                            "Dockerfile isolado: a ação disponível somente constrói a imagem.",
                        );
                    }
                    for warning in &project.warnings {
                        ui.label(RichText::new(warning).color(theme::MUTED).size(11.0));
                    }

                    let commands = project.proposed_commands(self.container_state.selected_engine);
                    if !commands.is_empty() {
                        ui.add_space(10.0);
                        ui.label(RichText::new("Ações").strong());
                        ui.horizontal_wrapped(|ui| {
                            for command in &commands {
                                let available = !executing
                                    && engine_ready
                                    && (project.compose_file.is_none() || compose_ready);
                                let guard = safety.as_ref().map(|safety| {
                                    guard_environment_action(command.action, safety, &project.directory)
                                });
                                let blocked = guard.as_ref().and_then(|guard| guard.as_ref().err());
                                let response = ui
                                    .add_enabled(
                                        available && blocked.is_none(),
                                        egui::Button::new(command.label),
                                    )
                                    .on_hover_text(command.action.description());
                                if let Some(blocked) = blocked {
                                    response.on_disabled_hover_text(format!(
                                        "Bloqueado: {}",
                                        blocked.title
                                    ));
                                } else if response.clicked() {
                                    requested_command = Some(command.clone());
                                }
                            }
                        });
                        egui::CollapsingHeader::new("Revisar comandos")
                            .id_salt(&project.directory)
                            .show(ui, |ui| {
                                for command in commands {
                                    ui.label(RichText::new(command.label).strong().size(12.0));
                                    ui.label(
                                        RichText::new(command.display()).monospace().size(11.0),
                                    );
                                    ui.add_space(5.0);
                                }
                            });
                    }
                });
            ui.add_space(12.0);
        }
        if let Some(command) = requested_command {
            self.container_state.confirmation = Some(command);
        }
        if let Some(snippet) = copied_snippet {
            self.notify(format!("Trecho copiado: {snippet}"));
        }
        if volume_request.is_some() {
            self.container_state.volume_confirmation = volume_request;
        }
        self.container_confirmation(ui.ctx());
        self.volume_confirmation_dialog(ui.ctx());
    }

    fn container_execution_panel(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &mut self.container_state.execution else {
            return;
        };
        let mut clear = false;
        Frame::new()
            .fill(theme::BACKGROUND)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(session.command.label).strong());
                    let (status, color) = match &session.outcome {
                        None => ("Em andamento", Color32::from_rgb(255, 205, 75)),
                        Some(ContainerExecutionOutcome::Succeeded) => ("Concluído", theme::SUCCESS),
                        Some(ContainerExecutionOutcome::Cancelled) => ("Cancelado", theme::MUTED),
                        Some(ContainerExecutionOutcome::TimedOut) => {
                            ("Tempo limite excedido", Color32::from_rgb(235, 87, 87))
                        }
                        Some(ContainerExecutionOutcome::Failed(_)) => {
                            ("Falhou", Color32::from_rgb(235, 87, 87))
                        }
                    };
                    runtime_badge(ui, status, color);
                });
                ui.label(
                    RichText::new(session.command.display())
                        .monospace()
                        .size(10.0),
                );
                ui.label(
                    RichText::new(display_path(&session.command.working_directory))
                        .color(theme::MUTED)
                        .size(10.0),
                );
                if let Some(ContainerExecutionOutcome::Failed(error)) = &session.outcome {
                    ui.colored_label(Color32::from_rgb(255, 112, 112), error);
                } else if matches!(session.outcome, Some(ContainerExecutionOutcome::TimedOut)) {
                    ui.colored_label(
                        Color32::from_rgb(255, 112, 112),
                        "A execução foi encerrada após 30 minutos.",
                    );
                }
                ui.add_space(8.0);
                if ui.button("Copiar logs").clicked() {
                    ui.ctx()
                        .copy_text(session.logs.iter().cloned().collect::<Vec<_>>().join("\n"));
                }
                egui::ScrollArea::vertical()
                    .id_salt("container-execution-output")
                    .max_height(220.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &session.logs {
                            let text = RichText::new(line).monospace().size(10.0);
                            if line.to_ascii_lowercase().contains("error")
                                || line.to_ascii_lowercase().contains("failed")
                            {
                                ui.label(text.color(Color32::from_rgb(255, 112, 112)));
                            } else {
                                ui.label(text);
                            }
                        }
                    });
                ui.add_space(8.0);
                if let Some(execution) = &session.execution {
                    let cancelling = execution.cancellation_requested();
                    if ui
                        .add_enabled(!cancelling, egui::Button::new("Cancelar execução"))
                        .clicked()
                    {
                        execution.cancel();
                    }
                    if cancelling {
                        ui.label(RichText::new("Cancelando...").color(theme::MUTED));
                    }
                } else if ui.button("Limpar resultado").clicked() {
                    clear = true;
                }
            });
        if clear {
            self.container_state.execution = None;
        }
    }

    fn container_confirmation(&mut self, ctx: &egui::Context) {
        let Some(command) = self.container_state.confirmation.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Confirmar ação de container")
            .id(egui::Id::unique("container-command-confirmation"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 520.0));
                ui.label(RichText::new(command.label).strong().size(16.0));
                ui.label(command.action.description());
                if command.action.changes_environment() {
                    ui.label(
                        RichText::new("Esta ação altera o ambiente de containers do projeto.")
                            .color(Color32::from_rgb(255, 205, 75)),
                    );
                }
                ui.add_space(8.0);
                ui.label("Comando:");
                ui.label(RichText::new(command.display()).monospace());
                ui.label("Pasta:");
                ui.label(
                    RichText::new(display_path(&command.working_directory))
                        .monospace()
                        .size(10.0),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add(egui::Button::new("Confirmar e executar").fill(theme::PRIMARY))
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.container_state.confirmation = None;
        }
        if confirm {
            self.container_state.confirmation = None;
            self.start_container_execution(command);
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_cards_keep_container_state_current_with_a_light_refresh() {
        for page in [Page::Overview, Page::Projects] {
            assert_eq!(
                container_refresh_schedule(page),
                Some((Duration::from_secs(30), ContainerRefresh::RuntimeOnly))
            );
        }
        for page in [Page::Processes, Page::Containers] {
            assert_eq!(
                container_refresh_schedule(page),
                Some((Duration::from_secs(15), ContainerRefresh::Full))
            );
        }
        for page in [Page::Plugins, Page::Help, Page::Settings] {
            assert_eq!(container_refresh_schedule(page), None);
        }
    }

    #[test]
    fn rereads_the_path_only_on_full_checks_with_a_missing_engine() {
        let engine = |state| EngineDiagnosis {
            engine: ContainerEngine::Docker,
            state,
            compose_available: false,
        };
        let missing = [engine(EngineState::Ready), engine(EngineState::Missing)];
        assert!(rereads_registry_path(ContainerRefresh::Full, &missing));
        assert!(!rereads_registry_path(
            ContainerRefresh::RuntimeOnly,
            &missing
        ));
        assert!(!rereads_registry_path(
            ContainerRefresh::Full,
            &[engine(EngineState::Ready), engine(EngineState::Unavailable)]
        ));
        assert!(!rereads_registry_path(ContainerRefresh::Full, &[]));
    }

    #[test]
    fn container_summary_omits_an_empty_engine_status() {
        let mut container = RuntimeContainer {
            engine: ContainerEngine::Podman,
            id: "abcdef123456".into(),
            name: "blog-db-1".into(),
            image: "mariadb".into(),
            state: "running".into(),
            status: String::new(),
            ports: String::new(),
            service: None,
            project_directory: None,
            running_for: None,
            compose_project: None,
        };
        assert_eq!(container_summary(&container), "Podman · running");
        container.engine = ContainerEngine::Docker;
        container.status = "Up 2 minutes".into();
        assert_eq!(
            container_summary(&container),
            "Docker · running · Up 2 minutes"
        );
    }
}
