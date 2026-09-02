use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

pub const BACKGROUND: Color32 = Color32::from_rgb(15, 17, 23);
pub const SIDEBAR: Color32 = Color32::from_rgb(18, 21, 28);
pub const SURFACE: Color32 = Color32::from_rgb(26, 29, 38);
pub const BORDER: Color32 = Color32::from_rgb(44, 49, 60);
pub const PRIMARY: Color32 = Color32::from_rgb(79, 140, 255);
pub const SUCCESS: Color32 = Color32::from_rgb(46, 204, 113);
pub const TEXT: Color32 = Color32::from_rgb(245, 245, 245);
pub const MUTED: Color32 = Color32::from_rgb(160, 167, 180);

pub fn configure(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = SIDEBAR;
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    visuals.selection.bg_fill = PRIMARY.gamma_multiply(0.45);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    ctx.set_style(style);
}
