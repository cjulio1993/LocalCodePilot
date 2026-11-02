//! Two-layer messages: one sentence and one action anyone can follow, then the steps to resolve
//! and the technical details for whoever asks for them. Independent of the screen, so the same
//! notice can move when a page is reorganized.

use super::*;
use egui_phosphor::regular::{CARET_DOWN, CARET_RIGHT, WARNING_CIRCLE};

pub(super) struct Notice<'a> {
    id: egui::Id,
    sentence: &'a str,
    action: Option<(&'a str, bool)>,
    resolve_label: &'a str,
}

/// Content of the expandable parts. `None` hides the toggle.
pub(super) struct NoticeParts<'r, 'd> {
    pub resolve: Option<&'r mut dyn FnMut(&mut egui::Ui)>,
    pub details: Option<&'d mut dyn FnMut(&mut egui::Ui)>,
}

impl<'a> Notice<'a> {
    pub(super) fn new(id: egui::Id, sentence: &'a str) -> Self {
        Self {
            id,
            sentence,
            action: None,
            resolve_label: "Como resolver",
        }
    }

    /// The one action of the first layer.
    pub(super) fn action(mut self, label: &'a str, enabled: bool) -> Self {
        self.action = Some((label, enabled));
        self
    }

    pub(super) fn resolve_label(mut self, label: &'a str) -> Self {
        self.resolve_label = label;
        self
    }

    /// Returns whether the action was clicked.
    pub(super) fn show(self, ui: &mut egui::Ui, parts: NoticeParts<'_, '_>) -> bool {
        let mut clicked = false;
        let frame = Frame::new()
            .fill(theme::WARNING.gamma_multiply(0.08))
            .stroke(Stroke::new(1.0, theme::WARNING.gamma_multiply(0.6)))
            .corner_radius(8)
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let sentence = format!("{WARNING_CIRCLE}  {}", self.sentence);
                announced(
                    ui.label(RichText::new(&sentence).color(theme::WARNING).strong()),
                    egui::Role::Label,
                    &sentence,
                );
                let resolve_key = self.id.with("resolve");
                let details_key = self.id.with("details");
                let mut resolve_open = parts.resolve.is_some() && is_open(ui, resolve_key);
                let mut details_open = parts.details.is_some() && is_open(ui, details_key);
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if let Some((label, enabled)) = self.action {
                        clicked = ui.add_enabled(enabled, egui::Button::new(label)).clicked();
                    }
                    if parts.resolve.is_some() {
                        toggle(ui, resolve_key, self.resolve_label, &mut resolve_open);
                    }
                    if parts.details.is_some() {
                        toggle(ui, details_key, "Detalhes técnicos", &mut details_open);
                    }
                });
                if let Some(resolve) = parts.resolve
                    && resolve_open
                {
                    ui.add_space(6.0);
                    resolve(ui);
                }
                if let Some(details) = parts.details
                    && details_open
                {
                    ui.add_space(6.0);
                    details(ui);
                }
            });
        let scroll_key = self.id.with("scroll");
        if ui
            .data_mut(|data| data.remove_temp::<bool>(scroll_key))
            .is_some()
        {
            frame.response.scroll_to_me(Some(Align::TOP));
        }
        clicked
    }
}

/// Opens the "Como resolver" part of a notice and scrolls to it, for links that lead to it
/// from another screen.
pub(super) fn open_resolve(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|data| {
        data.insert_temp(id.with("resolve"), true);
        data.insert_temp(id.with("scroll"), true);
    });
}

/// A line of technical detail.
pub(super) fn detail_line(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::MUTED).size(12.0));
}

/// A collapsible part outside a notice, closed or open by `default` until the user toggles
/// it. Returns whether it is open.
pub(super) fn disclosure(ui: &mut egui::Ui, key: egui::Id, label: &str, default: bool) -> bool {
    let mut open = ui
        .data(|data| data.get_temp::<bool>(key))
        .unwrap_or(default);
    toggle(ui, key, label, &mut open);
    open
}

/// The title of a group that opens and closes, such as a project in Containers.
pub(super) fn heading_disclosure(
    ui: &mut egui::Ui,
    key: egui::Id,
    label: &str,
    default: bool,
) -> bool {
    let mut open = ui
        .data(|data| data.get_temp::<bool>(key))
        .unwrap_or(default);
    let caret = if open { CARET_DOWN } else { CARET_RIGHT };
    let response = ui.add(
        egui::Button::new(
            RichText::new(format!("{caret}  {label}"))
                .strong()
                .size(16.0),
        )
        .frame(false),
    );
    remember(ui, key, response, label, &mut open);
    open
}

/// Opens a group made with `disclosure` or `heading_disclosure`, for links that lead to it.
pub(super) fn open(ctx: &egui::Context, key: egui::Id) {
    ctx.data_mut(|data| data.insert_temp(key, true));
}

fn is_open(ui: &egui::Ui, key: egui::Id) -> bool {
    ui.data(|data| data.get_temp::<bool>(key)).unwrap_or(false)
}

/// Expands or collapses one part; announced as a selected button while open.
fn toggle(ui: &mut egui::Ui, key: egui::Id, label: &str, open: &mut bool) {
    let caret = if *open { CARET_DOWN } else { CARET_RIGHT };
    let response = ui.add(egui::Button::selectable(*open, format!("{caret}  {label}")));
    remember(ui, key, response, label, open);
}

fn remember(
    ui: &mut egui::Ui,
    key: egui::Id,
    response: egui::Response,
    label: &str,
    open: &mut bool,
) {
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, *open, label));
    if response.clicked() {
        *open = !*open;
        ui.data_mut(|data| data.insert_temp(key, *open));
    }
}
