use super::*;
use localcodepilot_core::compose_safety::{
    BlockedAction, ComposeEnvironment, ComposeName, ComposeSafety, ComposeVolume,
    classify_environment, explain, guard_container_action, guard_environment_action,
};
use localcodepilot_core::container_groups::{
    EnvironmentGroup, EnvironmentState, ProjectGroup, ServiceRow, ServiceState, StopScope,
    group_containers,
};
use localcodepilot_core::container_setup::{ContainerHostFacts, EngineGuidance, engine_guidance};
use localcodepilot_core::containers::{
    ContainerAction, ContainerCommand, ContainerEngine, ContainerProject, EngineDiagnosis,
    EngineState, RuntimeContainer, path_is_within, same_folder,
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
    /// Whether the result panel already scrolled into view.
    shown: bool,
    /// Told when the command succeeds, such as what a stop left running.
    done_message: Option<String>,
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
    CheckAgain,
}

/// Identifies the Compose notice of an environment, so other screens can open it.
fn compose_notice_id(directory: &Path) -> egui::Id {
    egui::Id::unique(("compose-notice", directory))
}

/// Why Compose actions are blocked: one sentence first, the steps under "Como resolver" and
/// names, folders and volumes under "Detalhes técnicos".
fn blocked_panel(
    ui: &mut egui::Ui,
    directory: &Path,
    blocked: &BlockedAction,
    can_check: bool,
) -> Option<BlockedPanelAction> {
    let mut action = None;
    let sentence = format!("{} Iniciar e parar estão bloqueados.", blocked.summary);
    // Nothing to change in the file when the environment could not be verified: check again.
    let unverifiable = blocked.snippet.is_none() && !blocked.can_confirm_volumes;
    let mut notice = notice::Notice::new(compose_notice_id(directory), &sentence);
    if unverifiable {
        notice = notice.action("Verificar de novo", can_check);
    }
    let mut resolve = |ui: &mut egui::Ui| {
        for (index, step) in blocked.steps.iter().enumerate() {
            ui.label(RichText::new(format!("{}. {step}", index + 1)).size(12.0));
        }
        if let Some(snippet) = &blocked.snippet {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(snippet).monospace());
                if ui.small_button("Copiar configuração sugerida").clicked() {
                    ui.ctx().copy_text(snippet.clone());
                    action = Some(BlockedPanelAction::Copy(snippet.clone()));
                }
            });
        }
        if blocked.can_confirm_volumes && ui.button("Estes dados são deste projeto").clicked() {
            action = Some(BlockedPanelAction::ConfirmVolumes);
        }
    };
    let mut details = |ui: &mut egui::Ui| {
        ui.label(RichText::new(&blocked.title).strong().size(12.0));
        notice::detail_line(ui, &blocked.explanation);
    };
    let check = notice.show(
        ui,
        notice::NoticeParts {
            resolve: Some(&mut resolve),
            details: Some(&mut details),
        },
    );
    if check {
        Some(BlockedPanelAction::CheckAgain)
    } else {
        action
    }
}

/// Identifies a project group in Containers, so links can open it.
fn container_group_id(root: &Path) -> egui::Id {
    egui::Id::unique(("container-group", root))
}

/// Identifies the "Outras ações" part of an environment, so the card can open it.
fn environment_more_id(directory: &Path) -> egui::Id {
    egui::Id::unique(("environment-more", directory))
}

/// "web", "web e db", "web, db e cache".
fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [name] => name.clone(),
        [rest @ .., last] => format!("{} e {last}", rest.join(", ")),
    }
}

/// What the confirmation of a stop says will happen, and what to tell afterwards.
fn stop_summary(scope: &StopScope) -> String {
    let mut text = match scope.will_stop.as_slice() {
        [] => String::new(),
        [name] => format!("{name} foi parado."),
        names => format!("{} foram parados.", join_names(names)),
    };
    let keeps = scope
        .keeps_running
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    match keeps.as_slice() {
        [] => {}
        [name] => text.push_str(&format!(" {name} continua rodando.")),
        names => text.push_str(&format!(" {} continuam rodando.", join_names(names))),
    }
    text
}

/// What the Containers page asks for after drawing a frame.
#[derive(Default)]
struct PageRequests {
    command: Option<ContainerCommand>,
    snippet: Option<String>,
    volumes: Option<VolumeConfirmation>,
    check_again: bool,
}

fn engine_short_status(diagnosis: &EngineDiagnosis) -> (&'static str, Color32) {
    match diagnosis.state {
        EngineState::Ready if diagnosis.compose_available => ("pronto", theme::SUCCESS),
        EngineState::Ready => ("Compose não confirmado", theme::WARNING),
        EngineState::Missing => ("não encontrado", theme::MUTED),
        EngineState::Unavailable => ("sem acesso", theme::WARNING),
        EngineState::TimedOut => ("demorou para responder", theme::WARNING),
        EngineState::QueryFailed => ("respondeu com erro", theme::WARNING),
    }
}

fn environment_state_text(state: EnvironmentState, paused: usize) -> (String, Color32) {
    let (mut text, color) = match state {
        EnvironmentState::Problem => ("Com problema".to_owned(), theme::DANGER),
        EnvironmentState::Running { active, total } => {
            (format!("{active} de {total} rodando"), theme::SUCCESS)
        }
        EnvironmentState::Stopped => ("Parado".to_owned(), theme::MUTED),
        EnvironmentState::ImageOnly => ("Imagem para construir".to_owned(), theme::MUTED),
    };
    match paused {
        0 => {}
        1 => text.push_str(" · 1 pausado"),
        paused => text.push_str(&format!(" · {paused} pausados")),
    }
    (text, color)
}

fn service_state_color(state: ServiceState) -> Color32 {
    match state {
        ServiceState::Running => theme::SUCCESS,
        ServiceState::Restarting | ServiceState::Paused => theme::WARNING,
        ServiceState::Problem => theme::DANGER,
        ServiceState::Stopped | ServiceState::NotCreated => theme::MUTED,
    }
}

fn paused_note(ui: &mut egui::Ui) {
    ui.label(
        RichText::new("Pausado: este container está temporariamente suspenso.")
            .color(theme::MUTED)
            .size(12.0),
    );
}

/// Published ports only suggest an address; nothing here checks that a site answers.
fn container_addresses(ui: &mut egui::Ui, container: &RuntimeContainer) {
    let urls = container.browser_urls();
    let ports = container.other_local_ports();
    for url in &urls {
        if ui.button("Abrir site").clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
        ui.label(
            RichText::new(format!(
                "Endereço provável: {}",
                url.trim_start_matches("http://")
                    .trim_start_matches("https://")
            ))
            .color(theme::MUTED)
            .size(12.0),
        );
    }
    for port in &ports {
        ui.label(
            RichText::new(format!("Porta {port} disponível neste computador"))
                .color(theme::MUTED)
                .size(12.0),
        );
    }
    if urls.is_empty() && ports.is_empty() {
        ui.label(
            RichText::new("Sem endereço para abrir")
                .color(theme::MUTED)
                .size(12.0),
        );
    }
}

fn service_row(ui: &mut egui::Ui, row: &ServiceRow) {
    let state = row.state();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&row.name).strong());
        ui.label(RichText::new(state.label()).color(service_state_color(state)));
        if row.declared.is_none() {
            ui.label(
                RichText::new("não está no arquivo")
                    .color(theme::MUTED)
                    .size(12.0),
            );
        }
        if state.is_active()
            && let Some(container) = row.container
        {
            container_addresses(ui, container);
        }
    });
    if state == ServiceState::Paused {
        paused_note(ui);
    }
}

/// `within_environment` leaves out the service, Compose name and folder, which the
/// environment details already show.
fn container_detail_lines(
    ui: &mut egui::Ui,
    container: &RuntimeContainer,
    within_environment: bool,
) {
    ui.spacing_mut().item_spacing.y = 2.0;
    let id = container.id.get(..12).unwrap_or(&container.id);
    notice::detail_line(
        ui,
        &format!(
            "Container: {} · ID {id} · {}",
            container.name, container.engine
        ),
    );
    notice::detail_line(
        ui,
        &format!("Estado no motor: {}", container_summary(container)),
    );
    if !container.image.is_empty() {
        notice::detail_line(ui, &format!("Imagem: {}", container.image));
    }
    if !within_environment {
        if let Some(service) = &container.service {
            notice::detail_line(ui, &format!("Serviço: {service}"));
        }
        if let Some(project) = &container.compose_project {
            notice::detail_line(ui, &format!("Ambiente Compose: \"{project}\""));
        }
        if let Some(directory) = &container.project_directory {
            notice::detail_line(ui, &format!("Pasta: {}", display_path(directory)));
        }
    }
    if !container.ports.is_empty() {
        notice::detail_line(ui, &format!("Portas publicadas: {}", container.ports));
    }
    if let Some(running_for) = &container.running_for {
        notice::detail_line(ui, &format!("Tempo de execução: {running_for}"));
    }
}

fn environment_details(
    ui: &mut egui::Ui,
    group: &ProjectGroup,
    environment: &EnvironmentGroup,
    engine: ContainerEngine,
) {
    let project = environment.project;
    let folder = group.relative_folder(environment);
    ui.spacing_mut().item_spacing.y = 2.0;
    notice::detail_line(ui, &format!("Pasta: {}", display_path(&project.directory)));
    if !folder.is_empty() {
        notice::detail_line(ui, &format!("Subpasta do projeto: {folder}/"));
    }
    for file in project.dockerfile.iter().chain(project.compose_file.iter()) {
        notice::detail_line(
            ui,
            &format!(
                "Arquivo: {}",
                file.file_name().unwrap_or_default().to_string_lossy()
            ),
        );
    }
    for file in &project.compose_overrides {
        notice::detail_line(
            ui,
            &format!(
                "Override: {}",
                file.file_name().unwrap_or_default().to_string_lossy()
            ),
        );
    }
    match environment.name {
        Some(ComposeName::Resolved(name)) => {
            notice::detail_line(ui, &format!("Ambiente Compose: \"{name}\""));
        }
        Some(ComposeName::Estimated(name)) => notice::detail_line(
            ui,
            &format!("Ambiente Compose: \"{name}\" (estimado; o provedor Compose não respondeu)"),
        ),
        None => {}
    }
    for warning in &project.warnings {
        notice::detail_line(ui, warning);
    }
    for row in &environment.services {
        ui.add_space(6.0);
        ui.label(RichText::new(&row.name).strong().size(12.0));
        match row.declared {
            Some(service) => {
                if let Some(image) = &service.image {
                    notice::detail_line(ui, &format!("Imagem declarada: {image}"));
                }
                if service.builds_image {
                    notice::detail_line(ui, "Imagem construída a partir do projeto");
                }
                if !service.ports.is_empty() {
                    notice::detail_line(
                        ui,
                        &format!("Portas declaradas: {}", service.ports.join(", ")),
                    );
                }
                if !service.profiles.is_empty() {
                    notice::detail_line(
                        ui,
                        &format!("Perfil opcional: {}", service.profiles.join(", ")),
                    );
                }
            }
            None => notice::detail_line(
                ui,
                "Não está no arquivo atual; pode ter sobrado de uma versão anterior do ambiente.",
            ),
        }
        match row.container {
            Some(container) => container_detail_lines(ui, container, true),
            None => notice::detail_line(
                ui,
                "Ainda não criado: o motor não tem um container para este serviço. Ele é criado quando o ambiente é iniciado.",
            ),
        }
    }
    let commands = project.proposed_commands(engine);
    if !commands.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new("Comandos").strong().size(12.0));
        for command in commands {
            notice::detail_line(ui, command.label);
            ui.label(RichText::new(command.display()).monospace().size(11.0));
        }
    }
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
        Page::Containers => Some((Duration::from_secs(15), ContainerRefresh::Full)),
        Page::Overview | Page::Projects | Page::Processes => {
            Some((Duration::from_secs(30), ContainerRefresh::RuntimeOnly))
        }
        Page::Help | Page::Settings => None,
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

/// First layer of an engine that is not ready to use.
#[derive(Debug, PartialEq, Eq)]
struct EngineMessage {
    sentence: String,
    action: Option<&'static str>,
    resolve: &'static str,
    /// Shown above the preparation steps.
    hint: Option<&'static str>,
}

/// `None` when the engine and Compose are ready. The diagnosis only knows whether the CLI ran
/// and whether `info` answered, so a failed `info` is not called "stopped": a missing
/// permission or a wrong connection fail the same way.
fn engine_message(diagnosis: &EngineDiagnosis) -> Option<EngineMessage> {
    let engine = diagnosis.engine;
    let check = Some("Verificar de novo");
    let (sentence, action, resolve, hint) = match diagnosis.state {
        EngineState::Ready if diagnosis.compose_available => return None,
        EngineState::Ready => (
            format!("O {engine} está pronto, mas o Compose não foi confirmado."),
            check,
            "Como resolver",
            None,
        ),
        EngineState::Missing => (
            format!("Não foi possível encontrar o {engine} neste computador."),
            None,
            "Como instalar",
            None,
        ),
        EngineState::Unavailable => (
            format!("O {engine} foi encontrado, mas não foi possível acessá-lo."),
            check,
            "Como resolver",
            Some("Ele pode estar parado. Siga os passos e clique em Verificar de novo."),
        ),
        EngineState::TimedOut => (
            format!("O {engine} demorou para responder."),
            check,
            "Como resolver",
            None,
        ),
        EngineState::QueryFailed => (
            format!("O {engine} respondeu com erro."),
            check,
            "Como resolver",
            None,
        ),
    };
    Some(EngineMessage {
        sentence,
        action,
        resolve,
        hint,
    })
}

/// What the diagnosis found, for "Detalhes técnicos".
fn engine_status(state: EngineState) -> &'static str {
    match state {
        EngineState::Ready => "Motor disponível",
        EngineState::Missing => "CLI não encontrada no PATH",
        EngineState::Unavailable => "CLI encontrada; a consulta de informações do motor falhou",
        EngineState::TimedOut => "Consulta excedeu o tempo limite",
        EngineState::QueryFailed => "A CLI não respondeu corretamente",
    }
}

fn compose_status(diagnosis: &EngineDiagnosis) -> &'static str {
    if diagnosis.compose_available {
        "Compose disponível"
    } else if diagnosis.state == EngineState::Ready {
        "Compose não confirmado; verifique o plugin ou provedor"
    } else {
        "Compose será verificado quando o motor estiver ativo"
    }
}

/// Numbered preparation steps for an engine. Commands are only copied for the user to run.
/// Returns the command copied in this frame.
fn engine_guidance_panel(ui: &mut egui::Ui, guidance: &EngineGuidance) -> Option<&'static str> {
    let mut copied = None;
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
    /// Text of "Buscar projeto ou serviço" in Containers.
    filter: String,
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
            filter: String::new(),
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
    /// Containers live in Containers; Processes only says how many are up and leads there.
    pub(super) fn container_processes_summary(&mut self, ui: &mut egui::Ui) {
        let running = self
            .container_state
            .runtime_containers
            .iter()
            .filter(|container| {
                ServiceState::of(container).is_active()
                    && self
                        .focused_project
                        .as_ref()
                        .is_none_or(|path| container.belongs_to(path))
            })
            .count();
        if running == 0 {
            return;
        }
        let text = match (running, self.focused_project.is_some()) {
            (1, true) => "1 container rodando neste projeto".to_owned(),
            (count, true) => format!("{count} containers rodando neste projeto"),
            (1, false) => "1 container rodando".to_owned(),
            (count, false) => format!("{count} containers rodando"),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(text).color(theme::MUTED));
            if ui.link("Ver em Containers").clicked() {
                self.page = Page::Containers;
            }
        });
        ui.add_space(8.0);
    }

    /// Opens Containers on the environment created from `folder`, with "Como resolver" open.
    fn show_compose_notice(&mut self, ctx: &egui::Context, folder: &Path) {
        let directory = self
            .container_state
            .projects
            .iter()
            .find(|project| same_folder(&project.directory, folder))
            .map_or_else(|| folder.to_path_buf(), |project| project.directory.clone());
        notice::open_resolve(ctx, compose_notice_id(&directory));
        // The project the Compose folder belongs to (`wp-blog`, not its `docker/`), chosen
        // the same way Containers groups it.
        let root = self
            .projects
            .iter()
            .map(|project| &project.path)
            .filter(|path| path_is_within(&directory, path))
            .max_by_key(|path| path.components().count())
            .cloned()
            .unwrap_or(directory);
        notice::open(ctx, container_group_id(&root));
        // A search typed earlier could hide the notice.
        self.container_state.filter.clear();
        if self.focused_project.is_none() {
            self.focused_project = Some(root);
        }
        self.page = Page::Containers;
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
                            // A stop the person asked for is told once, not per container.
                            matches!(
                                session.command.action,
                                ContainerAction::Stop
                                    | ContainerAction::StopExisting
                                    | ContainerAction::RestartExisting
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
        let mut finished = None;
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
                    finished = Some(match &outcome {
                        ContainerExecutionOutcome::Succeeded => session.done_message.clone(),
                        _ => Some(format!(
                            "{}: não foi concluído. Veja o resultado em Containers.",
                            session.command.label
                        )),
                    });
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
        // Outside Containers the result panel is not visible, so failures are told here too.
        if let Some(message) = finished.flatten().filter(|message| {
            !message.ends_with("Veja o resultado em Containers.") || self.page != Page::Containers
        }) {
            self.notify(message);
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
                shown: false,
                done_message: None,
            },
            Err(error) => ContainerSession {
                command,
                execution: None,
                logs,
                outcome: Some(ContainerExecutionOutcome::Failed(error)),
                shown: false,
                done_message: None,
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
            RichText::new(
                "Os ambientes Docker e Podman dos seus projetos e o estado de cada serviço",
            )
            .color(theme::MUTED),
        );
        ui.add_space(12.0);
        let checking = self.container_state.pending.is_some();
        let executing = self
            .container_state
            .execution
            .as_ref()
            .is_some_and(|session| session.execution.is_some());
        // The progress sits beside the button: a line of its own would push the page down
        // every 15 seconds and move the buttons under the pointer.
        let check = ui
            .horizontal(|ui| {
                let clicked = ui
                    .add_enabled(
                        !checking && !executing && self.discovery.is_none(),
                        egui::Button::new("Verificar motores e arquivos"),
                    )
                    .clicked();
                if checking {
                    ui.spinner();
                    ui.label("Verificando em segundo plano...");
                }
                clicked
            })
            .inner;
        if check {
            self.refresh_containers(ui.ctx().clone());
        }
        if checking {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if let Some(error) = &self.container_state.error {
            ui.colored_label(theme::MUTED, error);
        }

        self.container_execution_panel(ui);
        ui.add_space(8.0);
        self.engine_overview(ui, checking || executing);
        ui.add_space(12.0);
        self.container_groups(ui, checking, executing);
        self.volume_confirmation_dialog(ui.ctx());
    }

    fn engine_readiness(&self, engine: ContainerEngine) -> (bool, bool) {
        self.container_state
            .engines
            .iter()
            .find(|diagnosis| diagnosis.engine == engine)
            .map_or((false, false), |diagnosis| {
                (
                    diagnosis.state == EngineState::Ready,
                    diagnosis.compose_available,
                )
            })
    }

    /// One line with the state of each engine; the notice of an engine that is not ready
    /// opens on its own when the app would use it, and on request otherwise.
    fn engine_overview(&mut self, ui: &mut egui::Ui, busy: bool) {
        let selected = self.container_state.selected_engine;
        let usable = |diagnosis: &EngineDiagnosis| {
            diagnosis.state == EngineState::Ready && diagnosis.compose_available
        };
        let none_usable = !self.container_state.engines.iter().any(usable);
        let mut shown = Vec::new();
        ui.horizontal_wrapped(|ui| {
            for diagnosis in &self.container_state.engines {
                let (status, color) = engine_short_status(diagnosis);
                ui.label(RichText::new(format!("{}:", diagnosis.engine)).strong());
                ui.label(RichText::new(status).color(color));
                if engine_message(diagnosis).is_some()
                    && (diagnosis.engine == selected
                        || none_usable
                        || notice::disclosure(
                            ui,
                            egui::Id::unique(("engine-notice-open", diagnosis.engine.to_string())),
                            &format!("Aviso do {}", diagnosis.engine),
                            false,
                        ))
                {
                    shown.push(diagnosis.engine);
                }
                ui.add_space(12.0);
            }
        });
        let mut copied = None;
        let mut check_engines = false;
        for diagnosis in &self.container_state.engines {
            if !shown.contains(&diagnosis.engine) {
                continue;
            }
            let Some(message) = engine_message(diagnosis) else {
                continue;
            };
            let guidance = self
                .container_state
                .host
                .as_ref()
                .and_then(|host| engine_guidance(diagnosis, host));
            ui.add_space(4.0);
            let mut notice = notice::Notice::new(
                egui::Id::unique(("engine-notice", diagnosis.engine.to_string())),
                &message.sentence,
            )
            .resolve_label(message.resolve);
            if let Some(action) = message.action {
                notice = notice.action(action, !busy);
            }
            let mut resolve = |ui: &mut egui::Ui| {
                if let Some(hint) = message.hint {
                    ui.label(RichText::new(hint).size(12.0));
                }
                if let Some(guidance) = &guidance
                    && let Some(command) = engine_guidance_panel(ui, guidance)
                {
                    copied = Some(command);
                }
            };
            let mut details = |ui: &mut egui::Ui| {
                notice::detail_line(ui, engine_status(diagnosis.state));
                if diagnosis.state != EngineState::Missing {
                    notice::detail_line(ui, compose_status(diagnosis));
                }
            };
            if notice.show(
                ui,
                notice::NoticeParts {
                    resolve: if guidance.is_some() {
                        Some(&mut resolve)
                    } else {
                        None
                    },
                    details: Some(&mut details),
                },
            ) {
                check_engines = true;
            }
        }
        if check_engines {
            self.refresh_containers(ui.ctx().clone());
        }
        if let Some(command) = copied {
            self.notify(format!("Comando copiado: {command}"));
        }
        // With a single usable engine there is nothing to choose.
        if self
            .container_state
            .engines
            .iter()
            .filter(|diagnosis| usable(diagnosis))
            .count()
            > 1
        {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Iniciar com:");
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
        }
    }

    fn compose_name_map(&self) -> HashMap<PathBuf, ComposeName> {
        self.container_state
            .compose_names
            .iter()
            .map(|(folder, (_, name))| (folder.clone(), name.clone()))
            .collect()
    }

    /// Runs `f` on the environment created from `directory`, grouped as Containers shows it.
    fn with_environment<R>(
        &self,
        directory: &Path,
        f: impl FnOnce(&ProjectGroup, &EnvironmentGroup) -> R,
    ) -> Option<R> {
        let catalog = self
            .projects
            .iter()
            .map(|project| (project.name.as_str(), project.path.as_path()))
            .collect::<Vec<_>>();
        let names = self.compose_name_map();
        let overview = group_containers(
            &catalog,
            &self.container_state.projects,
            &names,
            &self.container_state.runtime_containers,
        );
        let (group, environment) = overview.projects.iter().find_map(|group| {
            group
                .environments
                .iter()
                .find(|environment| same_folder(&environment.project.directory, directory))
                .map(|environment| (group, environment))
        })?;
        Some(f(group, environment))
    }

    /// What a confirmation acts on: the project (and folder) of a Compose command, or the
    /// container of a per-container command.
    fn confirmation_target(&self, command: &ContainerCommand) -> Option<String> {
        if matches!(
            command.action,
            ContainerAction::StartExisting
                | ContainerAction::StopExisting
                | ContainerAction::RestartExisting
                | ContainerAction::LogsExisting
        ) {
            let id = command.args.last()?;
            let container = self
                .container_state
                .runtime_containers
                .iter()
                .find(|container| &container.id == id)?;
            return Some(match &container.service {
                Some(service) => format!("Container: {service} ({})", container.name),
                None => format!("Container: {}", container.name),
            });
        }
        self.with_environment(&command.working_directory, |group, environment| match group
            .relative_folder(environment)
        {
            folder if folder.is_empty() => format!("Projeto: {}", group.name),
            folder => format!("Projeto: {} · pasta {folder}/", group.name),
        })
    }

    /// Asks to confirm a container command, wherever it was requested.
    pub(super) fn confirm_container_command(&mut self, command: ContainerCommand) {
        self.container_state.confirmation = Some(command);
    }

    /// What stopping the environment reaches. The reach was tested with Docker Compose as the
    /// provider, so Podman with another provider cannot be stated.
    fn environment_stop_scope(
        &self,
        environment: &EnvironmentGroup,
        engine: ContainerEngine,
    ) -> Option<StopScope> {
        if engine == ContainerEngine::Podman
            && self
                .container_state
                .host
                .as_ref()
                .and_then(|host| host.podman_uses_docker_compose)
                != Some(true)
        {
            return None;
        }
        environment.stop_scope(engine)
    }

    /// The stop a project card can confirm on its own: one environment up on one engine,
    /// allowed by the Compose checks, whose stop reaches at least one service.
    pub(super) fn card_container_stop(&self, root: &Path) -> Option<ContainerCommand> {
        let catalog = self
            .projects
            .iter()
            .map(|project| (project.name.as_str(), project.path.as_path()))
            .collect::<Vec<_>>();
        let names = self.compose_name_map();
        let overview = group_containers(
            &catalog,
            &self.container_state.projects,
            &names,
            &self.container_state.runtime_containers,
        );
        let group = overview
            .projects
            .iter()
            .find(|group| same_folder(group.root, root))?;
        let mut up = group
            .environments
            .iter()
            .filter(|environment| !environment.engines_in_use().is_empty());
        let environment = up.next()?;
        if up.next().is_some() {
            return None;
        }
        let [engine] = environment.engines_in_use()[..] else {
            return None;
        };
        let scope = self.environment_stop_scope(environment, engine)?;
        let (ready, compose) = self.engine_readiness(engine);
        let directory = &environment.project.directory;
        let safety = self.compose_environment_safety(directory, engine);
        if scope.will_stop.is_empty()
            || !ready
            || !compose
            || guard_environment_action(ContainerAction::Stop, &safety, directory).is_err()
        {
            return None;
        }
        environment
            .project
            .proposed_commands(engine)
            .into_iter()
            .find(|command| command.action == ContainerAction::Stop)
    }

    /// Opens a project in Containers with the parts that need a choice: the blocked notice,
    /// or "Outras ações" of each environment that is up.
    pub(super) fn choose_containers_to_stop(&mut self, ctx: &egui::Context, root: &Path) {
        let environments = self
            .container_state
            .projects
            .iter()
            .filter(|project| path_is_within(&project.directory, root))
            .map(|project| project.directory.clone())
            .collect::<Vec<_>>();
        if let Some(blocked) = environments.iter().find(|directory| {
            [ContainerEngine::Docker, ContainerEngine::Podman]
                .into_iter()
                .any(|engine| {
                    self.container_state
                        .compose_safety
                        .get(&(engine, (*directory).clone()))
                        .is_some_and(|safety| *safety != ComposeSafety::Safe)
                })
        }) {
            let blocked = blocked.clone();
            self.show_compose_notice(ctx, &blocked);
            return;
        }
        for directory in &environments {
            notice::open(ctx, environment_more_id(directory));
        }
        notice::reveal(ctx, container_group_id(root));
        self.container_state.filter.clear();
        self.focused_project = Some(root.to_path_buf());
        self.page = Page::Containers;
        self.notify("Escolha em Containers o que parar.");
    }

    /// Engine an environment's actions use: the one already running it, or the selected one.
    fn action_engine(&self, environment: &EnvironmentGroup) -> ContainerEngine {
        match environment.engines_in_use().as_slice() {
            [engine] => *engine,
            _ => self.container_state.selected_engine,
        }
    }

    fn environment_safety(&self, environment: &EnvironmentGroup) -> Option<ComposeSafety> {
        environment.project.compose_file.is_some().then(|| {
            self.compose_environment_safety(
                &environment.project.directory,
                self.action_engine(environment),
            )
        })
    }

    /// Blocked groups first, then those with a problem, running and stopped.
    fn group_rank(&self, group: &ProjectGroup) -> u8 {
        let environments = &group.environments;
        if environments.iter().any(|environment| {
            self.environment_safety(environment)
                .is_some_and(|safety| safety != ComposeSafety::Safe)
        }) {
            0
        } else if environments
            .iter()
            .any(|environment| environment.state() == EnvironmentState::Problem)
        {
            1
        } else if environments
            .iter()
            .any(|environment| matches!(environment.state(), EnvironmentState::Running { .. }))
        {
            2
        } else {
            3
        }
    }

    fn container_groups(&mut self, ui: &mut egui::Ui, checking: bool, executing: bool) {
        if !self.container_state.projects.is_empty()
            || !self.container_state.runtime_containers.is_empty()
        {
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.container_state.filter)
                    .hint_text("Buscar projeto ou serviço")
                    .desired_width(320.0),
            );
            announced(search, egui::Role::SearchInput, "Buscar projeto ou serviço");
            ui.add_space(8.0);
        }
        let filter = self.container_state.filter.trim().to_lowercase();
        let catalog = self
            .projects
            .iter()
            .map(|project| (project.name.as_str(), project.path.as_path()))
            .collect::<Vec<_>>();
        let names = self.compose_name_map();
        let overview = group_containers(
            &catalog,
            &self.container_state.projects,
            &names,
            &self.container_state.runtime_containers,
        );
        let focus = self.focused_project.as_deref();
        let in_focus = |folder: &Path| focus.is_none_or(|focus| path_is_within(folder, focus));
        let focused_groups = overview
            .projects
            .iter()
            .filter(|group| in_focus(group.root))
            .collect::<Vec<_>>();
        let focused_others = overview
            .others
            .iter()
            .copied()
            .filter(|container| focus.is_none_or(|focus| container.belongs_to(focus)))
            .collect::<Vec<_>>();
        let mut groups = focused_groups
            .into_iter()
            .filter(|group| group.matches(&filter))
            .map(|group| (self.group_rank(group), group))
            .collect::<Vec<_>>();
        groups.sort_by_key(|(rank, group)| (*rank, group.name.to_lowercase()));
        let others = focused_others
            .iter()
            .copied()
            .filter(|container| {
                filter.is_empty()
                    || [
                        container.name.as_str(),
                        container.image.as_str(),
                        container.service.as_deref().unwrap_or_default(),
                    ]
                    .iter()
                    .any(|text| text.to_lowercase().contains(&filter))
            })
            .collect::<Vec<_>>();

        let nothing_found = overview.projects.iter().all(|group| !in_focus(group.root))
            && focused_others.is_empty();
        if nothing_found {
            if !checking {
                ui.label(if self.focused_project.is_some() {
                    "Nenhum arquivo Compose, Dockerfile ou container encontrado neste projeto."
                } else {
                    "Nenhum Dockerfile, Containerfile, manifesto Compose ou container encontrado."
                });
            }
        } else if groups.is_empty() && others.is_empty() {
            ui.label(
                RichText::new("Nenhum projeto ou serviço encontrado. Tente outro termo.")
                    .color(theme::MUTED),
            );
        }

        let mut requests = PageRequests::default();
        for (rank, group) in &groups {
            self.project_group(
                ui,
                group,
                *rank,
                !filter.is_empty(),
                checking,
                executing,
                &mut requests,
            );
            ui.add_space(10.0);
        }
        if !others.is_empty() {
            self.other_containers(ui, &others, !filter.is_empty(), executing, &mut requests);
        }

        if let Some(command) = requests.command {
            if command.action.changes_environment() {
                self.container_state.confirmation = Some(command);
            } else {
                self.start_container_execution(command);
            }
        }
        if requests.check_again {
            self.refresh_containers(ui.ctx().clone());
        }
        if let Some(snippet) = requests.snippet {
            self.notify(format!("Trecho copiado: {snippet}"));
        }
        if requests.volumes.is_some() {
            self.container_state.volume_confirmation = requests.volumes;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn project_group(
        &self,
        ui: &mut egui::Ui,
        group: &ProjectGroup,
        rank: u8,
        filtering: bool,
        checking: bool,
        executing: bool,
        requests: &mut PageRequests,
    ) {
        Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Stopped projects start closed; the rest need attention or are in use.
                let default_open = rank < 3 || filtering || self.focused_project.is_some();
                let single = match group.environments.as_slice() {
                    [environment] => Some(environment),
                    _ => None,
                };
                let open = ui
                    .horizontal_wrapped(|ui| {
                        let open = notice::heading_disclosure(
                            ui,
                            container_group_id(group.root),
                            group.name,
                            default_open,
                        );
                        if let Some(environment) = single {
                            self.environment_header(ui, group, environment, executing, requests);
                        }
                        open
                    })
                    .inner;
                if !open {
                    return;
                }
                for environment in &group.environments {
                    if single.is_none() {
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            let folder = group.relative_folder(environment);
                            ui.label(
                                RichText::new(if folder.is_empty() {
                                    "Pasta do projeto".to_owned()
                                } else {
                                    format!("{folder}/")
                                })
                                .strong(),
                            );
                            self.environment_header(ui, group, environment, executing, requests);
                        });
                    }
                    self.environment_body(ui, group, environment, checking, executing, requests);
                }
            });
    }

    /// State, the one main action and "Ver logs" of an environment.
    fn environment_header(
        &self,
        ui: &mut egui::Ui,
        group: &ProjectGroup,
        environment: &EnvironmentGroup,
        executing: bool,
        requests: &mut PageRequests,
    ) {
        // Screen readers hear "Iniciar wp-blog", not one "Iniciar" per project.
        let context = match group.relative_folder(environment) {
            folder if group.environments.len() > 1 && !folder.is_empty() => {
                format!("{} {folder}", group.name)
            }
            _ => group.name.to_owned(),
        };
        // The group stays open after an action, even if its new state would close it.
        let mut request = |ui: &egui::Ui, command: ContainerCommand| {
            notice::open(ui.ctx(), container_group_id(group.root));
            requests.command = Some(command);
        };
        let (state, color) = environment_state_text(environment.state(), environment.paused());
        ui.label(RichText::new(state).color(color));
        let safety = self.environment_safety(environment);
        if safety
            .as_ref()
            .is_some_and(|safety| *safety != ComposeSafety::Safe)
        {
            ui.label(RichText::new("Bloqueado").color(theme::WARNING));
        }
        let project = environment.project;
        let active = environment
            .services
            .iter()
            .any(|row| row.state().is_active());
        // Only optional-profile services or containers no longer in the file are up: a stop
        // of the environment would not reach them.
        let out_of_reach = active
            && self
                .environment_stop_scope(environment, self.action_engine(environment))
                .is_some_and(|scope| scope.will_stop.is_empty());
        let main = if project.compose_file.is_none() {
            ContainerAction::Build
        } else if active && !out_of_reach {
            ContainerAction::Stop
        } else {
            ContainerAction::Start
        };
        if out_of_reach {
            ui.label(
                RichText::new("O que continua rodando para em Outras ações")
                    .color(theme::MUTED)
                    .size(12.0),
            );
        }
        // Containers of one environment running on both engines get one stop per engine.
        let engines = match environment.engines_in_use() {
            engines if main == ContainerAction::Stop && engines.len() > 1 => engines,
            _ => vec![self.action_engine(environment)],
        };
        for engine in &engines {
            let Some(command) = project
                .proposed_commands(*engine)
                .into_iter()
                .find(|command| command.action == main)
            else {
                continue;
            };
            let label = match main {
                ContainerAction::Start => "Iniciar".to_owned(),
                ContainerAction::Stop if engines.len() > 1 => format!("Parar no {engine}"),
                ContainerAction::Stop => "Parar".to_owned(),
                _ => command.label.to_owned(),
            };
            let (ready, compose) = self.engine_readiness(*engine);
            let engine_ok = ready && (project.compose_file.is_none() || compose);
            let blocked = project.compose_file.as_ref().and_then(|_| {
                let safety = self.compose_environment_safety(&project.directory, *engine);
                guard_environment_action(main, &safety, &project.directory).err()
            });
            let button = if main == ContainerAction::Start {
                egui::Button::new(RichText::new(&label).color(Color32::WHITE).strong())
                    .fill(theme::PRIMARY)
            } else {
                egui::Button::new(&label)
            };
            let response = announced(
                ui.add_enabled(!executing && engine_ok && blocked.is_none(), button),
                egui::Role::Button,
                &format!("{label} {context}"),
            );
            if let Some(blocked) = blocked {
                response.on_disabled_hover_text(format!("Bloqueado: {}", blocked.summary));
            } else if !engine_ok {
                response.on_disabled_hover_text(format!(
                    "O {engine} não está pronto. Veja o aviso no topo da página."
                ));
            } else if response
                .on_hover_text(command.action.description())
                .clicked()
            {
                request(ui, command);
            }
        }
        if project.compose_file.is_some() {
            let engine = self.action_engine(environment);
            let (ready, compose) = self.engine_readiness(engine);
            if let Some(command) = project
                .proposed_commands(engine)
                .into_iter()
                .find(|command| command.action == ContainerAction::Logs)
                && announced(
                    ui.add_enabled(
                        !executing && ready && compose,
                        egui::Button::new("Ver logs"),
                    ),
                    egui::Role::Button,
                    &format!("Ver logs {context}"),
                )
                .on_hover_text(command.action.description())
                .clicked()
            {
                request(ui, command);
            }
        }
    }

    fn environment_body(
        &self,
        ui: &mut egui::Ui,
        group: &ProjectGroup,
        environment: &EnvironmentGroup,
        checking: bool,
        executing: bool,
        requests: &mut PageRequests,
    ) {
        let project = environment.project;
        if let Some(safety) = self.environment_safety(environment)
            && safety != ComposeSafety::Safe
        {
            ui.add_space(6.0);
            match blocked_panel(
                ui,
                &project.directory,
                &explain(&safety, &project.directory),
                !checking && !executing,
            ) {
                Some(BlockedPanelAction::Copy(snippet)) => requests.snippet = Some(snippet),
                Some(BlockedPanelAction::CheckAgain) => requests.check_again = true,
                Some(BlockedPanelAction::ConfirmVolumes) => {
                    if let ComposeSafety::UnconfirmedData { volumes, .. } = &safety {
                        requests.volumes = Some(VolumeConfirmation {
                            engine: self.action_engine(environment),
                            directory: project.directory.clone(),
                            volumes: volumes.clone(),
                        });
                    }
                }
                None => {}
            }
        }
        if let Some(error) = &project.error {
            ui.colored_label(theme::DANGER, error);
        }
        ui.add_space(4.0);
        for row in &environment.services {
            service_row(ui, row);
        }
        if project.compose_file.is_none() {
            ui.label(
                RichText::new("Só há um Dockerfile: dá para construir a imagem, mas não há serviços para iniciar.")
                    .color(theme::MUTED)
                    .size(12.0),
            );
        }
        let directory = &project.directory;
        let (more, details) = ui
            .horizontal_wrapped(|ui| {
                (
                    notice::disclosure(ui, environment_more_id(directory), "Outras ações", false),
                    notice::disclosure(
                        ui,
                        egui::Id::unique(("environment-details", directory)),
                        "Detalhes técnicos",
                        false,
                    ),
                )
            })
            .inner;
        if more {
            self.more_actions(ui, environment, executing, requests);
        }
        if details {
            environment_details(ui, group, environment, self.action_engine(environment));
        }
    }

    /// Checks and per-container actions that most people never need.
    fn more_actions(
        &self,
        ui: &mut egui::Ui,
        environment: &EnvironmentGroup,
        executing: bool,
        requests: &mut PageRequests,
    ) {
        let engine = self.action_engine(environment);
        let (ready, compose) = self.engine_readiness(engine);
        let checks = environment
            .project
            .proposed_commands(engine)
            .into_iter()
            .filter(|command| {
                matches!(
                    command.action,
                    ContainerAction::Validate | ContainerAction::Status
                )
            })
            .collect::<Vec<_>>();
        if !checks.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for command in checks {
                    if ui
                        .add_enabled(
                            !executing && ready && compose,
                            egui::Button::new(command.label),
                        )
                        .on_hover_text(command.action.description())
                        .clicked()
                    {
                        requests.command = Some(command);
                    }
                }
            });
        }
        for row in &environment.services {
            let Some(container) = row.container else {
                continue;
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(&row.name).strong().size(12.0));
                self.container_buttons(ui, container, executing, requests);
            });
        }
    }

    fn container_buttons(
        &self,
        ui: &mut egui::Ui,
        container: &RuntimeContainer,
        executing: bool,
        requests: &mut PageRequests,
    ) {
        for action in [
            ContainerAction::StartExisting,
            ContainerAction::StopExisting,
            ContainerAction::RestartExisting,
            ContainerAction::LogsExisting,
        ] {
            let Some(command) = container.command(action) else {
                continue;
            };
            let label = if action == ContainerAction::LogsExisting {
                "Ver logs"
            } else {
                command.label
            };
            let guard = self.container_action_guard(action, container);
            let response = ui.add_enabled(!executing && guard.is_ok(), egui::Button::new(label));
            match guard {
                Err(blocked) => {
                    response.on_disabled_hover_text(blocked.summary);
                }
                Ok(()) => {
                    if response.on_hover_text(action.description()).clicked() {
                        requests.command = Some(command);
                    }
                }
            }
        }
    }

    fn other_containers(
        &self,
        ui: &mut egui::Ui,
        others: &[&RuntimeContainer],
        filtering: bool,
        executing: bool,
        requests: &mut PageRequests,
    ) {
        Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !notice::heading_disclosure(
                    ui,
                    egui::Id::unique("other-containers"),
                    &format!("Outros containers ({})", others.len()),
                    filtering,
                ) {
                    return;
                }
                ui.label(
                    RichText::new(
                        "Containers que não são de um projeto da lista ou cuja origem não foi confirmada.",
                    )
                    .color(theme::MUTED)
                    .size(12.0),
                );
                for container in others {
                    ui.separator();
                    let state = ServiceState::of(container);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&container.name).strong());
                        ui.label(RichText::new(state.label()).color(service_state_color(state)));
                        if state.is_active() {
                            container_addresses(ui, container);
                        }
                    });
                    if state == ServiceState::Paused {
                        paused_note(ui);
                    }
                    let details = ui
                        .horizontal_wrapped(|ui| {
                            self.container_buttons(ui, container, executing, requests);
                            notice::disclosure(
                                ui,
                                egui::Id::unique((
                                    "other-container-details",
                                    container.engine.to_string(),
                                    &container.id,
                                )),
                                "Detalhes técnicos",
                                false,
                            )
                        })
                        .inner;
                    if details {
                        container_detail_lines(ui, container, false);
                    }
                }
            });
    }

    fn container_execution_panel(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &mut self.container_state.execution else {
            return;
        };
        let mut clear = false;
        let frame = Frame::new()
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
        // A result appears at the top of the page; bring it into view once.
        if !session.shown {
            session.shown = true;
            frame.response.scroll_to_me(Some(Align::TOP));
        }
        if clear {
            self.container_state.execution = None;
        }
    }

    pub(super) fn container_confirmation(&mut self, ctx: &egui::Context) {
        let Some(command) = self.container_state.confirmation.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        let scope = (command.action == ContainerAction::Stop)
            .then(|| {
                self.with_environment(&command.working_directory, |_, environment| {
                    self.environment_stop_scope(environment, engine_of(&command))
                })
                .flatten()
            })
            .flatten();
        let target = self.confirmation_target(&command);
        let confirm_label = match scope.as_ref().map(|scope| scope.will_stop.as_slice()) {
            Some(names @ [_, ..]) if names.len() <= 3 => format!("Parar {}", join_names(names)),
            Some([_, ..]) => "Parar serviços".to_owned(),
            _ => "Confirmar e executar".to_owned(),
        };
        egui::Window::new("Confirmar ação de container")
            .id(egui::Id::unique("container-command-confirmation"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 520.0));
                ui.label(RichText::new(command.label).strong().size(16.0));
                if let Some(target) = &target {
                    ui.label(RichText::new(target).strong());
                }
                ui.label(command.action.description());
                if command.action.changes_environment() {
                    ui.label(
                        RichText::new("Esta ação altera o ambiente de containers do projeto.")
                            .color(theme::WARNING),
                    );
                }
                if command.action == ContainerAction::Stop {
                    ui.add_space(4.0);
                    match &scope {
                        Some(scope) => {
                            if !scope.will_stop.is_empty() {
                                ui.label(format!(
                                    "Serão parados: {}",
                                    join_names(&scope.will_stop)
                                ));
                            }
                            if !scope.already_stopped.is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "Já estão parados: {}",
                                        join_names(&scope.already_stopped)
                                    ))
                                    .color(theme::MUTED),
                                );
                            }
                            if !scope.keeps_running.is_empty() {
                                let kept = scope
                                    .keeps_running
                                    .iter()
                                    .map(|(name, reason)| format!("{name} ({})", reason.label()))
                                    .collect::<Vec<_>>();
                                ui.label(
                                    RichText::new(format!(
                                        "Continuarão rodando: {}. Para parar, use Outras ações em Containers.",
                                        join_names(&kept)
                                    ))
                                    .color(theme::WARNING),
                                );
                            }
                        }
                        None => {
                            ui.label(
                                RichText::new(
                                    "Não foi possível determinar quais containers serão parados.",
                                )
                                .color(theme::WARNING),
                            );
                        }
                    }
                }
                ui.add_space(4.0);
                if notice::disclosure(
                    ui,
                    egui::Id::unique("container-confirmation-details"),
                    "Detalhes técnicos",
                    false,
                ) {
                    notice::detail_line(ui, "Comando:");
                    ui.label(RichText::new(command.display()).monospace());
                    notice::detail_line(ui, "Pasta:");
                    ui.label(
                        RichText::new(display_path(&command.working_directory))
                            .monospace()
                            .size(10.0),
                    );
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(&confirm_label).color(Color32::WHITE).strong(),
                            )
                            .fill(theme::PRIMARY),
                        )
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
            if let (Some(scope), Some(session)) = (&scope, &mut self.container_state.execution) {
                session.done_message = Some(stop_summary(scope));
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_cards_keep_container_state_current_with_a_light_refresh() {
        for page in [Page::Overview, Page::Projects, Page::Processes] {
            assert_eq!(
                container_refresh_schedule(page),
                Some((Duration::from_secs(30), ContainerRefresh::RuntimeOnly))
            );
        }
        assert_eq!(
            container_refresh_schedule(Page::Containers),
            Some((Duration::from_secs(15), ContainerRefresh::Full))
        );
        for page in [Page::Help, Page::Settings] {
            assert_eq!(container_refresh_schedule(page), None);
        }
    }

    #[test]
    fn engine_messages_do_not_claim_more_than_the_diagnosis() {
        let diagnosis = |state, compose_available| EngineDiagnosis {
            engine: ContainerEngine::Docker,
            state,
            compose_available,
        };
        assert_eq!(engine_message(&diagnosis(EngineState::Ready, true)), None);
        let missing = engine_message(&diagnosis(EngineState::Missing, false)).unwrap();
        assert_eq!(
            missing.sentence,
            "Não foi possível encontrar o Docker neste computador."
        );
        // The app does not install engines: no action, only the steps.
        assert_eq!((missing.action, missing.resolve), (None, "Como instalar"));
        let unavailable = engine_message(&diagnosis(EngineState::Unavailable, false)).unwrap();
        assert_eq!(
            unavailable.sentence,
            "O Docker foi encontrado, mas não foi possível acessá-lo."
        );
        assert_eq!(unavailable.action, Some("Verificar de novo"));
        for state in [
            EngineState::Ready,
            EngineState::Missing,
            EngineState::Unavailable,
            EngineState::TimedOut,
            EngineState::QueryFailed,
        ] {
            if let Some(message) = engine_message(&diagnosis(state, false)) {
                assert!(!message.sentence.contains("parado"), "{state:?}");
                assert!(!message.sentence.contains("CLI"), "{state:?}");
            }
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
