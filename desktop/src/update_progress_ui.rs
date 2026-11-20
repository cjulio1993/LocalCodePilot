//! Progress of an update: the steps Baixar › Verificar › Instalar › Reiniciar,
//! the download bar and one status line. Shared by the update banner,
//! Configurações > Atualizações and the updater window, so all of them always
//! say the same thing.

use eframe::egui::{self, RichText};
use egui_phosphor::regular::{CARET_RIGHT, CHECK_CIRCLE, CIRCLE, RECORD};

use crate::{theme, updates::UpdatePhase};

/// The steps the person sees. The app shows the first three; once it closes,
/// the updater window goes on with Instalar and Reiniciar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UpdateStep {
    Download,
    Verify,
    Install,
    Restart,
}

impl UpdateStep {
    const ALL: [UpdateStep; 4] = [
        UpdateStep::Download,
        UpdateStep::Verify,
        UpdateStep::Install,
        UpdateStep::Restart,
    ];

    fn label(self) -> &'static str {
        match self {
            UpdateStep::Download => "Baixar",
            UpdateStep::Verify => "Verificar",
            UpdateStep::Install => "Instalar",
            UpdateStep::Restart => "Reiniciar",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpdateProgress {
    pub step: UpdateStep,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    /// The updater is still waiting for the app to close, for longer than usual.
    pub waiting_for_app: bool,
}

impl UpdateProgress {
    /// The app's progress, until it closes for the updater.
    pub(crate) fn new(
        phase: Option<UpdatePhase>,
        progress: Option<(u64, Option<u64>)>,
        installing: bool,
    ) -> Self {
        let step = if installing {
            UpdateStep::Install
        } else if phase == Some(UpdatePhase::Verifying) {
            UpdateStep::Verify
        } else {
            UpdateStep::Download
        };
        let (downloaded_bytes, total_bytes) = progress.unwrap_or((0, None));
        Self {
            step,
            downloaded_bytes,
            total_bytes: total_bytes.filter(|total| *total > 0),
            waiting_for_app: false,
        }
    }

    /// The updater window's progress, after the download was verified.
    pub(crate) fn after_close(step: UpdateStep, waiting_for_app: bool) -> Self {
        Self {
            step,
            downloaded_bytes: 0,
            total_bytes: None,
            waiting_for_app: waiting_for_app && step == UpdateStep::Install,
        }
    }

    /// Share of the download, only when the server sent the total size.
    pub(crate) fn fraction(&self) -> Option<f32> {
        if self.step != UpdateStep::Download {
            return None;
        }
        let total = self.total_bytes?;
        Some((self.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0) as f32)
    }

    pub(crate) fn status(&self) -> String {
        match self.step {
            UpdateStep::Restart => "Reiniciando o LocalCodePilot...".into(),
            UpdateStep::Install if self.waiting_for_app => {
                "Aguardando o LocalCodePilot fechar...".into()
            }
            UpdateStep::Install => "Instalando a atualização...".into(),
            UpdateStep::Verify => "Verificando a atualização...".into(),
            UpdateStep::Download if self.downloaded_bytes == 0 => {
                "Baixando a atualização...".into()
            }
            UpdateStep::Download => match (self.fraction(), self.total_bytes) {
                (Some(fraction), Some(total)) => format!(
                    "Baixando a atualização · {:.0}% · {} de {} MB",
                    fraction * 100.0,
                    megabytes(self.downloaded_bytes),
                    megabytes(total)
                ),
                _ => format!(
                    "Baixando a atualização · {} MB baixados",
                    megabytes(self.downloaded_bytes)
                ),
            },
        }
    }

    /// The steps as text, for screen readers and for the hover of the step row.
    pub(crate) fn steps_description(&self) -> String {
        let position = UpdateStep::ALL
            .iter()
            .position(|step| *step == self.step)
            .unwrap_or_default();
        format!(
            "Etapa {} de {}: {}",
            position + 1,
            UpdateStep::ALL.len(),
            self.step.label()
        )
    }
}

/// One decimal with a comma, as the person reads numbers in Portuguese.
fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0)).replace('.', ",")
}

pub(crate) fn show(ui: &mut egui::Ui, progress: &UpdateProgress) {
    // Each step has its own symbol (done, current, next) besides the color, and
    // the current one is bold, so the row does not depend on color alone.
    let steps = ui
        .horizontal_wrapped(|ui| {
            for (index, step) in UpdateStep::ALL.into_iter().enumerate() {
                if index > 0 {
                    ui.label(RichText::new(CARET_RIGHT).color(theme::MUTED));
                }
                let (icon, color) = match step.cmp(&progress.step) {
                    std::cmp::Ordering::Less => (CHECK_CIRCLE, theme::SUCCESS),
                    std::cmp::Ordering::Equal => (RECORD, theme::PRIMARY),
                    std::cmp::Ordering::Greater => (CIRCLE, theme::MUTED),
                };
                let mut text = RichText::new(format!("{icon} {}", step.label())).color(color);
                if step == progress.step {
                    text = text.strong();
                }
                ui.label(text);
            }
        })
        .response;
    let description = progress.steps_description();
    steps.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &description));
    steps.on_hover_text(description);
    ui.add_space(4.0);
    let status = progress.status();
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(&status);
    });
    if let Some(fraction) = progress.fraction() {
        ui.add_space(4.0);
        ui.add(egui::ProgressBar::new(fraction).desired_height(8.0))
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, &status)
            });
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(100));
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn download_with_a_known_size_shows_percentage_sizes_and_the_bar() {
        let progress = UpdateProgress::new(
            Some(UpdatePhase::Downloading),
            Some((6 * MIB + MIB * 3 / 10, Some(15 * MIB))),
            false,
        );
        assert_eq!(progress.step, UpdateStep::Download);
        assert_eq!(
            progress.status(),
            "Baixando a atualização · 42% · 6,3 de 15,0 MB"
        );
        assert!((progress.fraction().unwrap() - 0.42).abs() < 0.001);
        assert_eq!(progress.steps_description(), "Etapa 1 de 4: Baixar");
    }

    #[test]
    fn without_the_total_there_is_no_percentage_nor_bar() {
        for total in [None, Some(0)] {
            let progress = UpdateProgress::new(
                Some(UpdatePhase::Downloading),
                Some((3 * MIB, total)),
                false,
            );
            assert_eq!(
                progress.status(),
                "Baixando a atualização · 3,0 MB baixados"
            );
            assert_eq!(progress.fraction(), None);
        }
    }

    #[test]
    fn before_the_first_bytes_the_download_step_has_no_numbers() {
        for phase in [
            None,
            Some(UpdatePhase::Checksums),
            Some(UpdatePhase::Downloading),
        ] {
            let progress = UpdateProgress::new(phase, Some((0, None)), false);
            assert_eq!(progress.status(), "Baixando a atualização...");
        }
    }

    #[test]
    fn verifying_and_installing_replace_the_bar() {
        let verifying = UpdateProgress::new(
            Some(UpdatePhase::Verifying),
            Some((15 * MIB, Some(15 * MIB))),
            false,
        );
        assert_eq!(verifying.step, UpdateStep::Verify);
        assert_eq!(verifying.status(), "Verificando a atualização...");
        assert_eq!(verifying.fraction(), None);
        assert_eq!(verifying.steps_description(), "Etapa 2 de 4: Verificar");

        // The app shows Instalar before closing, so the updater window goes on
        // from the same step and the sequence never goes back.
        let installing = UpdateProgress::new(Some(UpdatePhase::Verifying), None, true);
        assert_eq!(installing.step, UpdateStep::Install);
        assert_eq!(installing.status(), "Instalando a atualização...");
        assert_eq!(installing.fraction(), None);
        assert_eq!(installing.steps_description(), "Etapa 3 de 4: Instalar");
    }

    #[test]
    fn the_updater_window_installs_waits_and_restarts() {
        let installing = UpdateProgress::after_close(UpdateStep::Install, false);
        assert_eq!(installing.status(), "Instalando a atualização...");
        assert_eq!(installing.steps_description(), "Etapa 3 de 4: Instalar");

        let waiting = UpdateProgress::after_close(UpdateStep::Install, true);
        assert_eq!(waiting.status(), "Aguardando o LocalCodePilot fechar...");
        assert_eq!(waiting.steps_description(), "Etapa 3 de 4: Instalar");

        // Waiting only applies before installing; restarting says so.
        let restarting = UpdateProgress::after_close(UpdateStep::Restart, true);
        assert_eq!(restarting.status(), "Reiniciando o LocalCodePilot...");
        assert_eq!(restarting.steps_description(), "Etapa 4 de 4: Reiniciar");
        assert_eq!(restarting.fraction(), None);
    }
}
