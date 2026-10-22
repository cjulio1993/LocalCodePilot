use super::*;
use localcodepilot_core::containers::{
    ContainerAction, ContainerCommand, ContainerEngine, ContainerProject, EngineDiagnosis,
    EngineState, RuntimeContainer,
};
use localcodepilot_platform::containers::{
    ContainerExecution, ContainerExecutionEvent, ContainerExecutionOutcome,
};

struct ContainerSession {
    command: ContainerCommand,
    execution: Option<ContainerExecution>,
    logs: std::collections::VecDeque<String>,
    outcome: Option<ContainerExecutionOutcome>,
}

struct ContainerSnapshot {
    projects: Vec<ContainerProject>,
    engines: Vec<EngineDiagnosis>,
    containers: Vec<RuntimeContainer>,
}

pub(super) struct ContainerState {
    projects: Vec<ContainerProject>,
    runtime_containers: Vec<RuntimeContainer>,
    last_refresh: Option<std::time::Instant>,
    engines: Vec<EngineDiagnosis>,
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
            pending: None,
            selected_engine: ContainerEngine::Docker,
            confirmation: None,
            execution: None,
            error: None,
        }
    }
}

impl LocalCodePilot {
    pub(super) fn project_container_status(&self, project: &Path) -> (bool, Option<String>) {
        let running = self
            .container_state
            .runtime_containers
            .iter()
            .filter(|container| {
                container.state.eq_ignore_ascii_case("running")
                    && container
                        .project_directory
                        .as_ref()
                        .is_some_and(|directory| directory.starts_with(project))
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

    pub(super) fn database_container_running(&self, project: &Path, service: &str) -> Option<bool> {
        self.container_state
            .runtime_containers
            .iter()
            .find(|container| {
                container.project_directory.as_deref() == Some(project)
                    && container.service.as_deref() == Some(service)
            })
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
                    self.focused_project.as_ref().is_none_or(|path| {
                        container
                            .project_directory
                            .as_ref()
                            .is_some_and(|directory| directory.starts_with(path))
                    })
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
                self.focused_project.as_ref().is_none_or(|path| {
                    container
                        .project_directory
                        .as_ref()
                        .is_some_and(|directory| directory.starts_with(path))
                })
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
                    ui.label(format!(
                        "{} · {} · {}",
                        container.engine, container.state, container.status
                    ));
                    ui.label(format!("Imagem: {}", container.image));
                    if let Some(service) = &container.service {
                        ui.label(format!("Serviço: {service}"));
                    }
                    if let Some(directory) = &container.project_directory {
                        ui.label(format!("Projeto: {}", directory.display()));
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
                    ui.horizontal_wrapped(|ui| {
                        for action in [
                            ContainerAction::StartExisting,
                            ContainerAction::StopExisting,
                            ContainerAction::RestartExisting,
                            ContainerAction::LogsExisting,
                        ] {
                            if let Some(command) = container.command(action)
                                && ui
                                    .add_enabled(
                                        self.container_state
                                            .execution
                                            .as_ref()
                                            .is_none_or(|session| session.execution.is_none()),
                                        egui::Button::new(command.label),
                                    )
                                    .clicked()
                            {
                                requested = Some(command);
                            }
                        }
                    });
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
        self.container_state.last_refresh = Some(std::time::Instant::now());
        let directories = self
            .projects
            .iter()
            .map(|project| project.path.clone())
            .collect::<Vec<_>>();
        let path = self.environment_path.clone();
        let (sender, receiver) = mpsc::channel();
        self.container_state.pending = Some(receiver);
        self.container_state.error = None;
        std::thread::spawn(move || {
            let mut seen = HashSet::new();
            let projects = directories
                .iter()
                .flat_map(|directory| {
                    localcodepilot_runtime::containers::detect_container_projects(directory)
                })
                .filter(|project| seen.insert(project.directory.clone()))
                .collect();
            let engines =
                localcodepilot_platform::containers::inspect_container_engines(path.as_deref());
            let containers =
                localcodepilot_platform::containers::list_runtime_containers(path.as_deref());
            let _ = sender.send(ContainerSnapshot {
                projects,
                engines,
                containers,
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_containers(&mut self, ctx: &egui::Context) {
        if matches!(self.page, Page::Processes | Page::Containers) {
            ctx.request_repaint_after(Duration::from_secs(15));
            if self.container_state.pending.is_none()
                && self
                    .container_state
                    .last_refresh
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(15))
            {
                self.refresh_containers(ctx.clone());
            }
        }
        if let Some(receiver) = &self.container_state.pending {
            match receiver.try_recv() {
                Ok(ContainerSnapshot {
                    projects,
                    engines,
                    containers,
                }) => {
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
                    if let Some(engine) = engines.iter().find(|engine| {
                        engine.state == EngineState::Ready && engine.compose_available
                    }) {
                        self.container_state.selected_engine = engine.engine;
                    }
                    self.container_state.projects = projects;
                    self.container_state.runtime_containers = containers;
                    self.container_state.engines = engines;
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
        let execution = localcodepilot_platform::containers::execute_container_command(
            command.clone(),
            self.environment_path.as_deref(),
        );
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
        for diagnosis in &self.container_state.engines {
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
                        } else {
                            "Compose não confirmado; verifique o plugin ou provedor"
                        });
                    }
                    if diagnosis.state == EngineState::Unavailable {
                        ui.label(
                            RichText::new(
                                "Abra o motor e confira sua conexão antes de iniciar um ambiente.",
                            )
                            .color(theme::MUTED)
                            .size(11.0),
                        );
                    }
                });
            ui.add_space(6.0);
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
                        RichText::new(project.directory.to_string_lossy())
                            .monospace()
                            .color(theme::MUTED)
                            .size(10.0),
                    );
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
                                if ui
                                    .add_enabled(available, egui::Button::new(command.label))
                                    .on_hover_text(command.action.description())
                                    .clicked()
                                {
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
        self.container_confirmation(ui.ctx());
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
                    RichText::new(session.command.working_directory.to_string_lossy())
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
                    RichText::new(command.working_directory.to_string_lossy())
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
