//! Progress of an in-app update: the steps Baixar › Verificar › Reiniciar, the
//! download bar and one status line. Shared by the update banner and
//! Configurações > Atualizações, so both always say the same thing.

use eframe::egui::{self, RichText};
use egui_phosphor::regular::{CARET_RIGHT, CHECK_CIRCLE, CIRCLE, RECORD};

use crate::{theme, updates::UpdatePhase};

/// The steps the person sees. Installing happens after the app closes, in the
/// updater, so it is not a step of this screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum UpdateStep {
    Download,
    Verify,
    Restart,
}

impl UpdateStep {
    const ALL: [UpdateStep; 3] = [
        UpdateStep::Download,
        UpdateStep::Verify,
        UpdateStep::Restart,
    ];

    fn label(self) -> &'static str {
        match self {
            UpdateStep::Download => "Baixar",
            UpdateStep::Verify => "Verificar",
            UpdateStep::Restart => "Reiniciar",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UpdateProgress {
    pub step: UpdateStep,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

impl UpdateProgress {
    pub(super) fn new(
        phase: Option<UpdatePhase>,
        progress: Option<(u64, Option<u64>)>,
        restarting: bool,
    ) -> Self {
        let step = if restarting {
            UpdateStep::Restart
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
        }
    }

    /// Share of the download, only when the server sent the total size.
    pub(super) fn fraction(&self) -> Option<f32> {
        if self.step != UpdateStep::Download {
            return None;
        }
        let total = self.total_bytes?;
        Some((self.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0) as f32)
    }

    pub(super) fn status(&self) -> String {
        match self.step {
            UpdateStep::Restart => "Reiniciando o LocalCodePilot...".into(),
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
    pub(super) fn steps_description(&self) -> String {
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

pub(super) fn show(ui: &mut egui::Ui, progress: &UpdateProgress) {
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
        assert_eq!(progress.steps_description(), "Etapa 1 de 3: Baixar");
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
    fn verifying_and_restarting_replace_the_bar() {
        let verifying = UpdateProgress::new(
            Some(UpdatePhase::Verifying),
            Some((15 * MIB, Some(15 * MIB))),
            false,
        );
        assert_eq!(verifying.step, UpdateStep::Verify);
        assert_eq!(verifying.status(), "Verificando a atualização...");
        assert_eq!(verifying.fraction(), None);
        assert_eq!(verifying.steps_description(), "Etapa 2 de 3: Verificar");

        let restarting = UpdateProgress::new(Some(UpdatePhase::Verifying), None, true);
        assert_eq!(restarting.step, UpdateStep::Restart);
        assert_eq!(restarting.status(), "Reiniciando o LocalCodePilot...");
        assert_eq!(restarting.steps_description(), "Etapa 3 de 3: Reiniciar");
    }
}
