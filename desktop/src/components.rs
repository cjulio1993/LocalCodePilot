//! Shared pieces of the screens (T036): the states of a page (loading, empty, error) and the
//! confirmation of an action that changes data. Each one keeps the same hierarchy (title,
//! explanation, one action), the text sizes of `theme`, and names for screen readers.

use super::*;

/// The frame of a page state: a surface with the same margin on every page.
fn state_frame<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(12)
        .inner_margin(Margin::same(24))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui)
        })
        .inner
}

/// Something is on its way: a spinner, what is happening and why it takes a moment.
pub(super) fn loading_state(ui: &mut egui::Ui, title: &str, explanation: &str) {
    state_frame(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(title).strong().size(theme::TEXT_HEADING));
        });
        ui.label(
            RichText::new(explanation)
                .color(theme::MUTED)
                .size(theme::TEXT_BODY),
        );
    });
}

/// Nothing to show yet: why, and the one action that changes it. Returns whether the action
/// was clicked.
pub(super) fn empty_state(
    ui: &mut egui::Ui,
    title: &str,
    explanation: &str,
    action: Option<&str>,
) -> bool {
    state_frame(ui, |ui| {
        ui.label(RichText::new(title).strong().size(theme::TEXT_HEADING));
        ui.label(RichText::new(explanation).size(theme::TEXT_BODY));
        action.is_some_and(|label| {
            ui.add_space(10.0);
            primary_button(ui, label).clicked()
        })
    })
}

/// Something failed: what, the known cause in plain words, and the action that may solve it.
/// Returns whether the action was clicked.
pub(super) fn error_state(ui: &mut egui::Ui, title: &str, cause: &str, action: &str) -> bool {
    state_frame(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(egui_phosphor::regular::WARNING_CIRCLE)
                    .color(theme::WARNING)
                    .size(theme::TEXT_HEADING),
            );
            ui.label(RichText::new(title).strong().size(theme::TEXT_HEADING));
        });
        ui.add(egui::Label::new(RichText::new(cause).size(theme::TEXT_BODY)).wrap());
        ui.add_space(10.0);
        primary_button(ui, action).clicked()
    })
}

/// A row of Configurações (T037.05): the title and a short explanation on the left, the
/// controls on the right. Returns what the controls return.
pub(super) fn settings_row<R>(
    ui: &mut egui::Ui,
    title: &str,
    explanation: &str,
    controls: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        let text_width = (ui.available_width() * 0.55).max(220.0);
        ui.allocate_ui_with_layout(
            egui::vec2(text_width, 0.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_max_width(text_width);
                ui.label(RichText::new(title).strong().size(theme::TEXT_BODY));
                ui.add(
                    egui::Label::new(
                        RichText::new(explanation)
                            .color(theme::MUTED)
                            .size(theme::TEXT_DETAIL),
                    )
                    .wrap(),
                );
            },
        );
        ui.with_layout(Layout::right_to_left(Align::Center), controls)
            .inner
    })
    .inner
}

/// The line of a section with nothing in it ("Nenhum projeto rodando agora.").
pub(super) fn empty_hint(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .color(theme::MUTED)
            .size(theme::TEXT_BODY),
    );
}

fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    announced(
        ui.add(
            egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
                .fill(theme::PRIMARY_FILL)
                .min_size(egui::vec2(180.0, 34.0)),
        ),
        egui::Role::Button,
        label,
    )
}

/// The confirmation of an action that changes data or processes: the title, what it reaches
/// (drawn by the caller), its consequence, "Cancelar" and a label that names the action. The
/// rest of the app takes no input while it is open; the focus starts on "Cancelar", Tab stays
/// inside, and Esc cancels.
pub(super) struct Confirmation<'a> {
    pub id: &'a str,
    pub title: &'a str,
    /// What happens after confirming, shown last, in the warning color.
    pub consequence: &'a str,
    pub confirm: &'a str,
    /// The action removes or replaces data: the confirm button is red.
    pub destructive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Confirm,
    Cancel,
}

impl Confirmation<'_> {
    /// Shows the confirmation; `scope` draws what the action reaches (folders, volumes).
    /// Returns the person's choice in the frame it is made.
    pub(super) fn show(
        &self,
        ctx: &egui::Context,
        scope: impl FnOnce(&mut egui::Ui),
    ) -> Option<Choice> {
        let mut choice = None;
        let focus_key = egui::Id::unique(("confirmation-focus", self.id));
        let modal = egui::Modal::new(egui::Id::unique(("confirmation", self.id))).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 520.0));
            ui.label(RichText::new(self.title).strong().size(theme::TEXT_TITLE));
            ui.add_space(6.0);
            scope(ui);
            ui.add(
                egui::Label::new(
                    RichText::new(self.consequence)
                        .color(theme::WARNING)
                        .size(theme::TEXT_BODY),
                )
                .wrap(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let cancel = ui.button("Cancelar");
                if !ui.data(|data| data.get_temp::<bool>(focus_key).unwrap_or(false)) {
                    cancel.request_focus();
                    ui.data_mut(|data| data.insert_temp(focus_key, true));
                }
                if cancel.clicked() {
                    choice = Some(Choice::Cancel);
                }
                let fill = if self.destructive {
                    theme::DANGER.gamma_multiply(0.22)
                } else {
                    theme::PRIMARY_FILL
                };
                if ui.add(egui::Button::new(self.confirm).fill(fill)).clicked() {
                    choice = Some(Choice::Confirm);
                }
            });
        });
        announced(modal.response.clone(), egui::Role::Dialog, self.title);
        if choice.is_none() && modal.should_close() {
            choice = Some(Choice::Cancel);
        }
        if choice.is_some() {
            ctx.data_mut(|data| data.remove::<bool>(focus_key));
        }
        choice
    }
}
