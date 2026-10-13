use super::*;
use localcodepilot_core::containers::{
    ContainerEngine, ContainerProject, EngineDiagnosis, EngineState,
};

pub(super) struct ContainerState {
    projects: Vec<ContainerProject>,
    engines: Vec<EngineDiagnosis>,
    pending: Option<Receiver<(Vec<ContainerProject>, Vec<EngineDiagnosis>)>>,
    selected_engine: ContainerEngine,
    error: Option<String>,
}

impl Default for ContainerState {
    fn default() -> Self {
        Self {
            projects: vec![],
            engines: vec![],
            pending: None,
            selected_engine: ContainerEngine::Docker,
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

    pub(super) fn poll_containers(&mut self) {
        let Some(receiver) = &self.container_state.pending else {
            return;
        };
        match receiver.try_recv() {
            Ok((projects, engines)) => {
                if let Some(engine) = engines
                    .iter()
                    .find(|engine| engine.state == EngineState::Ready && engine.compose_available)
                {
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

    pub(super) fn containers_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Containers");
        ui.label(
            RichText::new("Dockerfile, Compose e disponibilidade de Docker/Podman")
                .color(theme::MUTED),
        );
        ui.add_space(12.0);
        let checking = self.container_state.pending.is_some();
        if ui
            .add_enabled(
                !checking && self.discovery.is_none(),
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
            ui.label("Comandos propostos para:");
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
        ui.label(RichText::new("Esta etapa oferece diagnóstico e prévia. Os comandos abaixo não são executados pelo app.").color(theme::MUTED).size(11.0));
        ui.add_space(16.0);
        if self.container_state.projects.is_empty() && !checking {
            ui.label(
                "Nenhum Dockerfile, Containerfile ou manifesto Compose encontrado nos projetos.",
            );
        }
        for project in &self.container_state.projects {
            Frame::new().fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(10).inner_margin(Margin::same(16)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(project.directory.file_name().unwrap_or_default().to_string_lossy()).strong().size(16.0));
                    ui.label(RichText::new(project.directory.to_string_lossy()).monospace().color(theme::MUTED).size(10.0));
                    for file in project.dockerfile.iter().chain(project.compose_file.iter()) {
                        ui.label(format!("Arquivo: {}", file.file_name().unwrap_or_default().to_string_lossy()));
                    }
                    if let Some(error) = &project.error { ui.colored_label(Color32::from_rgb(255, 112, 112), error); }
                    for service in &project.services {
                        ui.separator();
                        ui.label(RichText::new(&service.name).strong());
                        if let Some(image) = &service.image { ui.label(format!("Imagem: {image}")); }
                        if service.builds_image { ui.label("Imagem construída a partir do projeto"); }
                        if !service.ports.is_empty() { ui.label(format!("Portas declaradas: {}", service.ports.join(", "))); }
                        if !service.profiles.is_empty() { ui.label(format!("Perfil opcional: {}", service.profiles.join(", "))); }
                    }
                    if project.compose_file.is_none() { ui.label("Dockerfile isolado: ainda não há serviço ou porta de execução definidos."); }
                    for warning in &project.warnings { ui.label(RichText::new(warning).color(theme::MUTED).size(11.0)); }
                    egui::CollapsingHeader::new("Comandos propostos").id_salt(&project.directory).show(ui, |ui| {
                        for command in project.proposed_commands(self.container_state.selected_engine) {
                            ui.label(RichText::new(command.label).strong().size(12.0));
                            ui.label(RichText::new(command.display()).monospace().size(11.0));
                            ui.add_space(5.0);
                        }
                    });
                });
            ui.add_space(12.0);
        }
    }
}
