use crate::theme;
use eframe::egui::{
    self, Align, Color32, FontId, Frame, Layout, Margin, RichText, Sense, Stroke, Vec2,
};
use localcodepilot_core::{projects::Project, runtimes::RuntimeKind};
use localcodepilot_platform::{NativePlatform, Platform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Projects,
    Processes,
    Plugins,
    Settings,
}

pub struct LocalCodePilot {
    page: Page,
    projects: Vec<Project>,
    platform: NativePlatform,
    search: String,
    status: Option<String>,
}

impl LocalCodePilot {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::configure(&cc.egui_ctx);
        let sample = Project::new(
            std::env::current_dir().unwrap_or_else(|_| "LocalCodePilot".into()),
            vec![RuntimeKind::Rust],
        );
        Self {
            page: Page::Overview,
            projects: vec![sample],
            platform: NativePlatform::default(),
            search: String::new(),
            status: None,
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
                            ui.label(RichText::new(">_").strong().color(Color32::WHITE));
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
                self.nav_button(ui, Page::Overview, "▦", "Visão geral", None);
                self.nav_button(
                    ui,
                    Page::Projects,
                    "▰",
                    "Projetos",
                    Some(self.projects.len()),
                );
                self.nav_button(ui, Page::Processes, ">_", "Processos", None);
                ui.add_space(18.0);
                ui.label(
                    RichText::new("SISTEMA")
                        .color(Color32::from_rgb(111, 119, 135))
                        .strong()
                        .size(10.0),
                );
                ui.add_space(4.0);
                self.nav_button(ui, Page::Plugins, "◇", "Plugins", None);
                self.nav_button(ui, Page::Settings, "⚙", "Configurações", None);

                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.colored_label(theme::SUCCESS, "●");
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
                    ui.label(RichText::new("Workspace  ›").color(theme::MUTED).size(12.0));
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
                        ui.label(RichText::new("●").color(theme::PRIMARY).size(10.0));
                        ui.add_sized(
                            [220.0, 32.0],
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Buscar projeto..."),
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

    fn add_project(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Selecione a pasta do projeto")
            .pick_folder()
        {
            let runtimes = localcodepilot_runtime::detect(&path);
            let project = Project::new(path, runtimes);
            self.status = Some(format!("Projeto {} adicionado", project.name));
            self.projects.push(project);
        }
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
                            RichText::new("＋  Novo projeto")
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(theme::PRIMARY)
                        .corner_radius(8),
                    )
                    .clicked()
                {
                    self.add_project();
                }
            });
        });
        ui.add_space(22.0);
        let snapshot = self.platform.snapshot();
        let used_gb = snapshot.used_memory_bytes as f64 / 1_073_741_824.0;
        ui.columns(3, |columns| {
            stat_card(
                &mut columns[0],
                "▰",
                "Projetos",
                &self.projects.len().to_string(),
                "disponíveis",
                theme::PRIMARY,
            );
            stat_card(
                &mut columns[1],
                "▶",
                "Processos ativos",
                "0",
                "pronto para iniciar",
                theme::SUCCESS,
            );
            stat_card(
                &mut columns[2],
                "⌁",
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
                if ui.link("Ver todos  →").clicked() {
                    self.page = Page::Projects;
                }
            });
        });
        ui.add_space(8.0);
        self.project_grid(ui);
    }

    fn project_grid(&mut self, ui: &mut egui::Ui) {
        let query = self.search.to_lowercase();
        let projects: Vec<_> = self
            .projects
            .iter()
            .filter(|p| {
                query.is_empty()
                    || p.name.to_lowercase().contains(&query)
                    || p.path.to_string_lossy().to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        let columns = if ui.available_width() > 900.0 {
            3
        } else if ui.available_width() > 560.0 {
            2
        } else {
            1
        };
        egui::Grid::new("project_grid")
            .num_columns(columns)
            .spacing([14.0, 14.0])
            .show(ui, |ui| {
                for (index, project) in projects.iter().enumerate() {
                    project_card(
                        ui,
                        project,
                        ui.available_width().max(220.0) / columns as f32 - 10.0,
                    );
                    if (index + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
                if ui
                    .add_sized(
                        [220.0, 150.0],
                        egui::Button::new("＋\n\nAdicionar projeto\nSelecione uma pasta local")
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0_f32, theme::BORDER))
                            .corner_radius(12),
                    )
                    .clicked()
                {
                    self.add_project();
                }
            });
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
                    Page::Projects => {
                        ui.heading("Projetos");
                        ui.label(
                            RichText::new("Todos os seus projetos locais").color(theme::MUTED),
                        );
                        ui.add_space(20.0);
                        self.project_grid(ui);
                    }
                    page => {
                        ui.heading(self.page_title());
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!(
                                "A área de {} está pronta para a próxima etapa.",
                                match page {
                                    Page::Processes => "processos",
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

fn project_card(ui: &mut egui::Ui, project: &Project, width: f32) {
    let color = theme::PRIMARY;
    Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(12)
        .inner_margin(Margin::same(17))
        .show(ui, |ui| {
            ui.set_min_size(Vec2::new(width, 116.0));
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("▰")
                        .color(color)
                        .font(FontId::proportional(24.0)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("•••").color(theme::MUTED));
                });
            });
            ui.add_space(10.0);
            ui.label(RichText::new(&project.name).strong().size(13.0));
            ui.label(
                RichText::new(project.path.to_string_lossy())
                    .color(Color32::from_rgb(116, 125, 141))
                    .monospace()
                    .size(9.0),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.colored_label(color, "●");
                ui.label(
                    RichText::new(project.display_stack())
                        .color(theme::MUTED)
                        .size(9.0),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("◷  Agora").color(theme::MUTED).size(9.0));
                });
            });
        });
}
