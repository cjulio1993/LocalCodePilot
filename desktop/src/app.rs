use crate::{config, theme};
use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, RichText, Sense, Stroke};
use egui_phosphor::regular::{
    BELL, CARET_RIGHT, CIRCLE, CODE_SIMPLE, FOLDER_OPEN, GEAR, LAYOUT, MEMORY, PLAY, PLUS,
    TERMINAL, TERMINAL_WINDOW,
};
use localcodepilot_core::{
    discovery::DiscoveryService,
    processes::{ProcessState, ProjectProcess},
    projects::Project,
    runtimes::RuntimeKind,
};
use localcodepilot_platform::{NativePlatform, Platform, filesystem::FilesystemProjectSource};
use localcodepilot_runtime::{ManifestRuntimeDetector, detect_processes};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Projects,
    Processes,
    Plugins,
    Settings,
}

struct RunningProcess {
    child: Child,
    output: Receiver<String>,
    logs: VecDeque<String>,
    finished: bool,
}

enum ProcessAction {
    Start(String),
    Stop(String),
}

pub struct LocalCodePilot {
    page: Page,
    projects: Vec<Project>,
    processes: Vec<ProjectProcess>,
    running_processes: HashMap<String, RunningProcess>,
    platform: NativePlatform,
    search: String,
    status: Option<String>,
    scan_roots: Vec<PathBuf>,
    discovery: Option<Receiver<Result<Vec<Project>, String>>>,
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
        Self {
            page: Page::Overview,
            projects: Vec::new(),
            processes: Vec::new(),
            running_processes: HashMap::new(),
            platform: NativePlatform::default(),
            search: String::new(),
            status: Some("Procurando projetos na máquina...".into()),
            scan_roots,
            discovery: Some(receiver),
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(248.0)
            .frame(
                Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(Margin::same(14))
                    .stroke(Stroke::new(1.0_f32, theme::BORDER)),
            )
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(theme::PRIMARY)
                        .corner_radius(10)
                        .inner_margin(Margin::same(9))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(TERMINAL_WINDOW)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                        });
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

    fn topbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("topbar")
            .exact_height(64.0)
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(Margin::symmetric(26, 14))
                    .stroke(Stroke::new(1.0_f32, theme::BORDER)),
            )
            .show(ctx, |ui| {
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
                                ui.label(RichText::new("JC").strong().size(10.0));
                            });

                        ui.label(RichText::new(BELL).color(theme::PRIMARY).size(16.0));

                        ui.add_sized(
                            [220.0, 32.0],
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Buscar projeto...")
                                .horizontal_align(Align::Center),
                        );
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
                self.status = Some(format!("{} projeto(s) encontrado(s)", projects.len()));
                self.processes = projects.iter().flat_map(detect_processes).collect();
                self.projects = projects;
                self.discovery = None;
            }
            Ok(Err(error)) => {
                self.status = Some(format!("Não foi possível concluir a varredura: {error}"));
                self.discovery = None;
            }
            Err(TryRecvError::Disconnected) => {
                self.status = Some("A varredura foi interrompida".into());
                self.discovery = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn refresh_projects(&mut self, ctx: &egui::Context) {
        if self.discovery.is_some() {
            return;
        }
        self.status = Some("Atualizando a lista de projetos...".into());
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            ctx.clone(),
        ));
    }

    fn add_scan_root(&mut self, ctx: &egui::Context, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        if self.scan_roots.iter().any(|root| root == &path) {
            self.status = Some("Essa pasta já faz parte da varredura".into());
            return;
        }
        self.scan_roots.push(path);
        self.status = Some(match config::save_scan_roots(&self.scan_roots) {
            Ok(()) => "Pasta adicionada. Procurando projetos...".into(),
            Err(error) => format!("Pasta adicionada, mas não foi possível salvar: {error}"),
        });
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            ctx.clone(),
        ));
    }

    fn remove_scan_root(&mut self, ctx: &egui::Context, path: &Path) {
        self.scan_roots.retain(|root| root != path);
        self.status = Some(match config::save_scan_roots(&self.scan_roots) {
            Ok(()) => "Pasta removida. Atualizando os projetos...".into(),
            Err(error) => format!("Pasta removida, mas não foi possível salvar: {error}"),
        });
        self.discovery = Some(spawn_discovery(
            FilesystemProjectSource::new(self.scan_roots.clone()),
            ctx.clone(),
        ));
    }

    fn reset_scan_roots(&mut self, ctx: &egui::Context) {
        self.scan_roots = FilesystemProjectSource::common_locations().roots().to_vec();
        self.status = Some(match config::save_scan_roots(&self.scan_roots) {
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
                    self.status = Some(
                        "O assistente de criação de projetos será adicionado futuramente".into(),
                    );
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

        projects.sort_by_key(|project| std::cmp::Reverse(project.modified_at));
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
        let grid_spacing = 14.0 * (columns.saturating_sub(1)) as f32;
        let frame_margin = 34.0;
        let card_width =
            ((available_width - grid_spacing) / columns as f32 - frame_margin).max(180.0);

        egui::Grid::new("project_grid")
            .num_columns(columns)
            .spacing([14.0, 14.0])
            .show(ui, |ui| {
                for (index, project) in projects.iter().enumerate() {
                    if let Some(path) = project_card(ui, project, card_width) {
                        self.status = Some(match open_in_vscode(&path) {
                            Ok(()) => format!("Abrindo {} no VS Code...", project.name),
                            Err(error) => error,
                        });
                    }
                    if (index + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
    }

    fn projects_page(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
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
                if ui
                    .add_enabled(!scanning, egui::Button::new("Atualizar"))
                    .clicked()
                {
                    self.refresh_projects(ctx);
                }
                if ui
                    .add_enabled(
                        !scanning,
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
                                    self.discovery.is_none(),
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
                        self.discovery.is_none(),
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

    fn start_process(&mut self, id: &str) {
        let Some(index) = self.processes.iter().position(|process| process.id == id) else {
            return;
        };
        if self
            .running_processes
            .get(id)
            .is_some_and(|running| !running.finished)
        {
            return;
        }
        let process = self.processes[index].clone();
        self.processes[index].state = ProcessState::Starting;
        let mut command = process_command(&process);
        command
            .current_dir(&process.working_directory)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        match command.spawn() {
            Ok(mut child) => {
                let process_id = child.id();
                let (sender, output) = mpsc::channel();
                if let Some(stdout) = child.stdout.take() {
                    spawn_output_reader(stdout, sender.clone());
                }
                if let Some(stderr) = child.stderr.take() {
                    spawn_output_reader(stderr, sender);
                }
                self.running_processes.insert(
                    id.to_owned(),
                    RunningProcess {
                        child,
                        output,
                        logs: VecDeque::new(),
                        finished: false,
                    },
                );
                self.processes[index].state = ProcessState::Running;
                self.processes[index].process_id = Some(process_id);
                self.status = Some(format!("{} iniciado (PID {process_id})", process.name));
            }
            Err(error) => {
                self.processes[index].state = ProcessState::Failed;
                self.processes[index].process_id = None;
                self.status = Some(format!(
                    "Não foi possível iniciar {}: {error}",
                    process.name
                ));
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
        if let Some(process) = self.processes.iter_mut().find(|process| process.id == id) {
            process.state = ProcessState::Stopped;
            process.process_id = None;
            self.status = Some(format!("{} interrompido", process.name));
        }
    }

    fn poll_processes(&mut self, ctx: &egui::Context) {
        let mut state_updates = Vec::new();
        let mut has_running_process = false;
        for (id, running) in &mut self.running_processes {
            while let Ok(line) = running.output.try_recv() {
                running.logs.push_back(line);
                if running.logs.len() > 500 {
                    running.logs.pop_front();
                }
            }
            if running.finished {
                continue;
            }
            match running.child.try_wait() {
                Ok(Some(status)) => {
                    running.finished = true;
                    state_updates.push((
                        id.clone(),
                        if status.success() {
                            ProcessState::Stopped
                        } else {
                            ProcessState::Failed
                        },
                    ));
                }
                Ok(None) => has_running_process = true,
                Err(_) => {
                    running.finished = true;
                    state_updates.push((id.clone(), ProcessState::Failed));
                }
            }
        }
        for (id, state) in state_updates {
            if let Some(process) = self.processes.iter_mut().find(|process| process.id == id) {
                process.state = state;
                process.process_id = None;
            }
        }
        if has_running_process {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn processes_page(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        ui.heading("Processos");
        ui.label(
            RichText::new(format!(
                "{} comando(s) detectado(s) em seus projetos",
                self.processes.len()
            ))
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

        for project in &self.projects {
            let commands: Vec<_> = self
                .processes
                .iter()
                .filter(|process| process.project_path == project.path)
                .cloned()
                .collect();
            if commands.is_empty() {
                continue;
            }
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(12)
                .inner_margin(Margin::same(18))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&project.name).strong().size(15.0));
                    ui.add_space(8.0);
                    for command in commands {
                        let (state_label, state_color) = process_state_display(command.state);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&command.name).size(12.0));
                                ui.label(
                                    RichText::new(command.command_line())
                                        .color(theme::MUTED)
                                        .monospace()
                                        .size(10.0),
                                );
                                if command.working_directory != command.project_path
                                    && let Ok(relative) = command
                                        .working_directory
                                        .strip_prefix(&command.project_path)
                                {
                                    ui.label(
                                        RichText::new(format!("em {}", relative.display()))
                                            .color(theme::MUTED)
                                            .size(9.0),
                                    );
                                }
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                match command.state {
                                    ProcessState::Running => {
                                        if ui.button("Parar").clicked() {
                                            action = Some(ProcessAction::Stop(command.id.clone()));
                                        }
                                    }
                                    ProcessState::Starting => {
                                        ui.add_enabled(false, egui::Button::new("Iniciando..."));
                                    }
                                    ProcessState::Stopped | ProcessState::Failed => {
                                        if ui
                                            .button(RichText::new(format!("{PLAY}  Iniciar")))
                                            .clicked()
                                        {
                                            action = Some(ProcessAction::Start(command.id.clone()));
                                        }
                                    }
                                }
                                runtime_badge(ui, state_label, state_color);
                                if let Some(process_id) = command.process_id {
                                    ui.label(
                                        RichText::new(format!("PID {process_id}"))
                                            .color(theme::MUTED)
                                            .monospace()
                                            .size(9.0),
                                    );
                                }
                            });
                        });
                        if let Some(running) = self.running_processes.get(&command.id)
                            && !running.logs.is_empty()
                        {
                            egui::CollapsingHeader::new(format!("Saída ({})", running.logs.len()))
                                .id_salt(format!("logs-{}", command.id))
                                .show(ui, |ui| {
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
                        ui.add_space(6.0);
                    }
                });
            ui.add_space(12.0);
        }
        match action {
            Some(ProcessAction::Start(id)) => self.start_process(&id),
            Some(ProcessAction::Stop(id)) => self.stop_process(&id),
            None => {}
        }
    }

    fn content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(Margin::same(36)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                    Page::Overview => self.overview(ui),
                    Page::Projects => self.projects_page(ctx, ui),
                    Page::Processes => self.processes_page(ui),
                    page => {
                        ui.heading(self.page_title());
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!(
                                "A área de {} está pronta para a próxima etapa.",
                                match page {
                                    Page::Plugins => "plugins",
                                    Page::Settings => "configurações",
                                    _ => "workspace",
                                }
                            ))
                            .color(theme::MUTED),
                        );
                    }
                });
            });
    }
}

impl eframe::App for LocalCodePilot {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_discovery();
        self.poll_processes(ctx);
        self.sidebar(ctx);
        self.topbar(ctx);
        self.content(ctx);
        if let Some(message) = self.status.clone() {
            egui::Area::new("status".into())
                .anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -20.0])
                .show(ctx, |ui| {
                    Frame::new()
                        .fill(theme::SURFACE)
                        .stroke(Stroke::new(1.0_f32, theme::SUCCESS))
                        .corner_radius(8)
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.label(message);
                        });
                });
        }
    }
}

impl Drop for LocalCodePilot {
    fn drop(&mut self) {
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

fn process_command(process: &ProjectProcess) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("cmd.exe");
        command
            .args(["/D", "/C"])
            .arg(&process.program)
            .args(&process.args);
        command
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut command = Command::new(&process.program);
        command.args(&process.args);
        command
    }
}

fn spawn_output_reader(reader: impl Read + Send + 'static, sender: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
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
            return;
        }
    }
    let _ = child.kill();
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
            ui.horizontal(|ui| {
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

fn project_card(ui: &mut egui::Ui, project: &Project, width: f32) -> Option<PathBuf> {
    let mut open_path = None;
    Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(12)
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(width);
            ui.set_min_height(72.0);
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
                    ui.add_space(6.0);
                    if ui
                        .button(RichText::new(format!("{CODE_SIMPLE}  Abrir no VS Code")).strong())
                        .clicked()
                    {
                        open_path = Some(project.path.clone());
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if project.runtimes.is_empty() {
                    runtime_badge(ui, "Projeto local", theme::MUTED);
                } else {
                    for runtime in &project.runtimes {
                        runtime_badge(ui, &runtime.to_string(), runtime_color(*runtime));
                    }
                }
            });
        });
    open_path
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
    }
}
