use super::*;
use localcodepilot_core::containers::{
    ContainerCommand, ContainerEngine, ContainerProject, EngineDiagnosis, EngineState,
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

pub(super) struct ContainerState {
    projects: Vec<ContainerProject>,
    engines: Vec<EngineDiagnosis>,
    pending: Option<Receiver<(Vec<ContainerProject>, Vec<EngineDiagnosis>)>>,
    selected_engine: ContainerEngine,
    confirmation: Option<ContainerCommand>,
    execution: Option<ContainerSession>,
    error: Option<String>,
}

impl Default for ContainerState {
    fn default() -> Self {
        Self {
            projects: vec![],
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
    pub(super) fn refresh_containers(&mut self, ctx: egui::Context) {
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
            let _ = sender.send((projects, engines));
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_containers(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.container_state.pending {
            match receiver.try_recv() {
                Ok((projects, engines)) => {
                    if let Some(engine) = engines.iter().find(|engine| {
                        engine.state == EngineState::Ready && engine.compose_available
                    }) {
                        self.container_state.selected_engine = engine.engine;
                    }
                    self.container_state.projects = projects;
                    self.container_state.engines = engines;
                    self.container_state.pending = None;
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
        loop {
            let event = session
                .execution
                .as_ref()
                .map(|execution| execution.events.try_recv());
            match event {
                Some(Ok(ContainerExecutionEvent::Output(line))) => {
                    session.logs.push_back(line);
                    if session.logs.len() > 1_000 {
                        session.logs.pop_front();
                    }
                }
                Some(Ok(ContainerExecutionEvent::Finished(outcome))) => {
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
        if self.container_state.projects.is_empty() && !checking {
            ui.label(
                "Nenhum Dockerfile, Containerfile ou manifesto Compose encontrado nos projetos.",
            );
        }

        let mut requested_command = None;
        for project in &self.container_state.projects {
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
                egui::ScrollArea::vertical()
                    .id_salt("container-execution-output")
                    .max_height(220.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &session.logs {
                            ui.label(RichText::new(line).monospace().size(10.0));
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
                ui.set_width(520.0);
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
