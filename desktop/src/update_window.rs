//! The small window the updater shows while the app is closed for an update,
//! so the person does not think the app froze. It has no buttons and never
//! shows the outcome: the version that opens next shows it. Closing it only
//! hides it; the update goes on.

use std::{sync::Arc, time::Duration};

use eframe::egui::{self, Frame, Margin, RichText};

use crate::{
    theme,
    update_progress_ui::{self, UpdateProgress, UpdateStep},
    updates::{HELPER_SLOW_CLOSE, HelperProgress, HelperStage},
};

const TITLE: &str = "Atualizando o LocalCodePilot";
const CLOSE_NOTE: &str = "Você pode fechar esta janela. A atualização continuará.";
const WIDTH: f32 = 460.0;
const MARGIN: i8 = 20;

/// Shows the window until the update finishes or the person closes it. Returns
/// right away when the window cannot open; the update does not depend on it.
pub fn show(progress: Arc<HelperProgress>) {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_inner_size([WIDTH, 170.0])
        .with_resizable(false)
        .with_maximize_button(false);
    if let Some(icon) = crate::app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };
    let _ = eframe::run_native(
        "LocalCodePilot",
        options,
        Box::new(|cc| {
            theme::configure(&cc.egui_ctx);
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(UpdaterWindow { progress }))
        }),
    );
}

struct UpdaterWindow {
    progress: Arc<HelperProgress>,
}

/// What the window shows for a stage; `None` once there is nothing to show.
fn window_progress(stage: HelperStage) -> Option<UpdateProgress> {
    match stage {
        HelperStage::WaitingForApp { since } => Some(UpdateProgress::after_close(
            UpdateStep::Install,
            since.elapsed() >= HELPER_SLOW_CLOSE,
        )),
        HelperStage::Installing => Some(UpdateProgress::after_close(UpdateStep::Install, false)),
        HelperStage::Restarting => Some(UpdateProgress::after_close(UpdateStep::Restart, false)),
        HelperStage::Finished => None,
    }
}

impl eframe::App for UpdaterWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let Some(progress) = window_progress(self.progress.stage()) else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        };
        let content_height = egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(Margin::same(MARGIN)),
            )
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    update_progress_ui::show(ui, &progress);
                    ui.add_space(2.0);
                    ui.add(egui::Label::new(RichText::new(CLOSE_NOTE).color(theme::MUTED)).wrap());
                })
                .response
                .rect
                .height()
            })
            .inner;
        // The window follows its content, so wrapped lines (larger text,
        // narrower screens) are never cut.
        let wanted = content_height + 2.0 * f32::from(MARGIN);
        let current = ctx.content_rect().height();
        if (wanted - current).abs() > 1.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(WIDTH, wanted)));
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn the_window_installs_then_restarts_and_closes_when_finished() {
        let just_started = HelperStage::WaitingForApp {
            since: Instant::now(),
        };
        assert_eq!(
            window_progress(just_started).unwrap().status(),
            "Instalando a atualização..."
        );
        assert_eq!(
            window_progress(HelperStage::Installing).unwrap().status(),
            "Instalando a atualização..."
        );
        assert_eq!(
            window_progress(HelperStage::Restarting).unwrap().status(),
            "Reiniciando o LocalCodePilot..."
        );
        assert_eq!(window_progress(HelperStage::Finished), None);
    }

    #[test]
    fn a_slow_close_says_the_updater_is_waiting() {
        let Some(since) = Instant::now().checked_sub(HELPER_SLOW_CLOSE) else {
            return;
        };
        let progress = window_progress(HelperStage::WaitingForApp { since }).unwrap();
        assert_eq!(progress.status(), "Aguardando o LocalCodePilot fechar...");
        assert_eq!(progress.steps_description(), "Etapa 3 de 4: Instalar");
    }
}
