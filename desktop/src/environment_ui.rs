//! Ambiente (T038.03): what this computer has for the projects, in two groups. "Controlado
//! pelo LocalCodePilot": the container engines, which turn on here with the usual
//! confirmation, and the databases of the projects. "Encontrado neste computador": containers
//! no project of the list owns, which only offer their logs, and the languages.

use super::*;

impl LocalCodePilot {
    pub(super) fn environment_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Ambiente").strong().size(24.0));
        ui.label(
            RichText::new("O que este computador tem para os seus projetos.")
                .color(theme::MUTED)
                .size(theme::TEXT_BODY),
        );
        ui.add_space(10.0);
        self.environment_summary(ui);
        ui.add_space(16.0);
        group_title(
            ui,
            "Controlado pelo LocalCodePilot",
            "O LocalCodePilot liga, para e prepara estes recursos, sempre com confirmação.",
        );
        self.engines_section(ui);
        ui.add_space(20.0);
        self.databases_page(ui);
        ui.add_space(28.0);
        group_title(
            ui,
            "Encontrado neste computador",
            "O LocalCodePilot mostra o que encontrou, mas não muda nada aqui.",
        );
        self.loose_containers_section(ui);
        ui.add_space(20.0);
        self.languages_section(ui);
    }

    /// One line to triage at a glance (T039.03, CA-8): each engine's state, the databases
    /// with a problem or not verified, and the other containers.
    fn environment_summary(&self, ui: &mut egui::Ui) {
        let mut parts = self
            .container_state
            .engines
            .iter()
            .map(|diagnosis| {
                format!(
                    "{}: {}",
                    diagnosis.engine,
                    container_ui::engine_summary(diagnosis).0
                )
            })
            .collect::<Vec<_>>();
        let (problems, unverified, total) = self.database_counts();
        parts.push(match (total, problems, unverified) {
            (0, _, _) => "Bancos: nenhum".to_owned(),
            (_, 0, 0) => format!("Bancos: {total}, sem problema encontrado"),
            _ => format!(
                "Bancos: {problems} com problema, {unverified} {}",
                if unverified == 1 {
                    "não verificado"
                } else {
                    "não verificados"
                }
            ),
        });
        parts.push(format!(
            "Outros containers: {}",
            self.loose_containers().len()
        ));
        Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(RichText::new(parts.join(" · ")).size(theme::TEXT_BODY))
                        .wrap(),
                );
            });
    }

    /// Docker and Podman: one row each, with the state in words and the reason, and Iniciar
    /// right there when the engine is installed and stopped.
    fn engines_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Motores de containers");
        ui.label(
            RichText::new("Docker e Podman rodam os projetos com containers.").color(theme::MUTED),
        );
        ui.add_space(10.0);
        let checking = self.container_state.checking();
        if self.container_state.engines.is_empty() {
            if checking {
                components::loading_state(
                    ui,
                    "Verificando os motores...",
                    "O LocalCodePilot consulta o Docker e o Podman em segundo plano.",
                );
            } else {
                components::empty_hint(ui, "Os motores ainda não foram verificados.");
            }
            return;
        }
        let busy = self.engine_start_busy();
        let mut start = None;
        let mut check = false;
        for diagnosis in &self.container_state.engines {
            let engine = diagnosis.engine;
            let (state, color, reason) = container_ui::engine_summary(diagnosis);
            let starting = self.engine_starting(engine);
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(160.0, 24.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.set_min_width(160.0);
                                ui.label(
                                    RichText::new(engine.to_string())
                                        .strong()
                                        .size(theme::TEXT_BODY),
                                );
                            },
                        );
                        runtime_badge(ui, state, color);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if starting {
                                ui.label("Ligando...");
                                ui.spinner();
                            } else if diagnosis.start.is_some() && state == "Parado" {
                                let name = format!("Iniciar {engine}");
                                let button = ui.add_enabled(
                                    !busy,
                                    egui::Button::new(
                                        RichText::new(format!("{PLAY}  Iniciar"))
                                            .color(Color32::WHITE)
                                            .strong(),
                                    )
                                    .fill(theme::PRIMARY_FILL),
                                );
                                if announced(button, egui::Role::Button, &name).clicked() {
                                    start = Some(engine);
                                }
                            } else if state == "Com problema"
                                && ui
                                    .add_enabled(!checking, egui::Button::new("Verificar de novo"))
                                    .clicked()
                            {
                                check = true;
                            }
                        });
                    });
                    if let Some(reason) = &reason {
                        ui.label(
                            RichText::new(reason)
                                .color(theme::MUTED)
                                .size(theme::TEXT_DETAIL),
                        );
                    }
                });
            ui.add_space(4.0);
        }
        // Turning an engine on asks first ("Ligar o Docker?") and starts no project.
        if let Some(engine) = start {
            self.request_engine_start(vec![engine], None, vec![], None);
        }
        if check {
            self.refresh_containers(ui.ctx().clone());
        }
    }

    /// Containers no project of the list owns, or whose origin was not confirmed: shown with
    /// their state, and only their logs (T038, CA-8). Nothing here starts or stops them.
    fn loose_containers_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Outros containers");
        ui.label(
            RichText::new(
                "Containers que não são de um projeto da lista ou cuja origem não foi confirmada.",
            )
            .color(theme::MUTED),
        );
        ui.add_space(10.0);
        self.container_execution_panel(ui);
        let containers = self.loose_containers();
        if containers.is_empty() {
            let engine_running = self
                .container_state
                .engines
                .iter()
                .any(|diagnosis| container_ui::engine_summary(diagnosis).0 == "Rodando");
            components::empty_hint(
                ui,
                if engine_running {
                    "Nenhum outro container neste computador."
                } else {
                    "Com os motores parados, o LocalCodePilot não vê os containers deles."
                },
            );
            return;
        }
        let mut logs = None;
        for container in &containers {
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&container.name)
                                .strong()
                                .size(theme::TEXT_BODY),
                        );
                        ui.label(
                            RichText::new(format!("{} · {}", container.image, container.engine))
                                .color(theme::MUTED)
                                .size(theme::TEXT_DETAIL),
                        );
                        ui.label(
                            RichText::new(&container.status)
                                .color(theme::MUTED)
                                .size(theme::TEXT_DETAIL),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let name = format!("Ver logs de {}", container.name);
                            if announced(ui.button("Ver logs"), egui::Role::Button, &name).clicked()
                            {
                                logs = Some(container.clone());
                            }
                        });
                    });
                });
            ui.add_space(4.0);
        }
        if let Some(container) = logs {
            self.show_container_logs(&container);
        }
    }
}

/// The title of one of the two groups of Ambiente, with what the app does there.
fn group_title(ui: &mut egui::Ui, title: &str, explanation: &str) {
    ui.label(RichText::new(title).strong().size(theme::TEXT_TITLE + 2.0));
    ui.label(
        RichText::new(explanation)
            .color(theme::MUTED)
            .size(theme::TEXT_BODY),
    );
    ui.add_space(12.0);
}
