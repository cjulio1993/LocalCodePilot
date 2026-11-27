//! Ambiente (T037.05): what this computer has for the projects. The container engines
//! (whether Docker and Podman are available; starting one stays in Serviços > Containers,
//! next to the containers) and the languages, which moved here from Configurações.

use super::*;

impl LocalCodePilot {
    pub(super) fn environment_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Ambiente").strong().size(24.0));
        ui.label(
            RichText::new("Linguagens e motores neste computador")
                .color(theme::MUTED)
                .size(theme::TEXT_BODY),
        );
        ui.add_space(16.0);
        self.engines_section(ui);
        ui.add_space(24.0);
        self.languages_section(ui);
    }

    /// Docker and Podman: one row each, with their state. "Iniciar" lives in Serviços >
    /// Containers, where the person sees what the engine runs.
    fn engines_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Motores de containers");
        ui.label(
            RichText::new(
                "Docker e Podman rodam os projetos com containers. Para ligar um motor, use Serviços > Containers.",
            )
            .color(theme::MUTED),
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
        let mut open_containers = false;
        for diagnosis in &self.container_state.engines {
            let (status, color) = container_ui::engine_short_status(diagnosis);
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
                                    RichText::new(diagnosis.engine.to_string())
                                        .strong()
                                        .size(theme::TEXT_BODY),
                                );
                            },
                        );
                        runtime_badge(ui, status, color);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Abrir Serviços > Containers").clicked() {
                                open_containers = true;
                            }
                        });
                    });
                });
            ui.add_space(4.0);
        }
        if open_containers {
            self.open_page(Page::Containers, ui.ctx());
        }
    }
}
