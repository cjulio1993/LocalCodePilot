use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

pub const BACKGROUND: Color32 = Color32::from_rgb(15, 17, 23);
pub const SIDEBAR: Color32 = Color32::from_rgb(18, 21, 28);
pub const SURFACE: Color32 = Color32::from_rgb(26, 29, 38);
pub const BORDER: Color32 = Color32::from_rgb(44, 49, 60);
pub const PRIMARY: Color32 = Color32::from_rgb(79, 140, 255);
/// The fill of buttons with white text: 5.2:1 with white, where PRIMARY gives 3.2:1
/// (T039.02, CA-10). PRIMARY stays for links and accents on the dark background.
pub const PRIMARY_FILL: Color32 = Color32::from_rgb(37, 99, 235);
pub const SUCCESS: Color32 = Color32::from_rgb(46, 204, 113);
pub const TEXT: Color32 = Color32::from_rgb(245, 245, 245);
pub const MUTED: Color32 = Color32::from_rgb(160, 167, 180);
pub const WARNING: Color32 = Color32::from_rgb(255, 205, 75);
pub const DANGER: Color32 = Color32::from_rgb(235, 87, 87);

// Text sizes by role (T036), from the sizes the screens already used most.
/// Window and modal titles.
pub const TEXT_TITLE: f32 = 16.0;
/// The title of a page state (loading, empty, error).
pub const TEXT_HEADING: f32 = 15.0;
/// Explanations and the main text of a card.
pub const TEXT_BODY: f32 = 13.0;
/// Secondary lines: details, hints, badges.
pub const TEXT_DETAIL: f32 = 12.0;

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

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
    });
}

/// The default fonts plus the Phosphor icons used in labels.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let icon_font = "phosphor";
    fonts.font_data.insert(
        icon_font.into(),
        egui::FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()).into(),
    );
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(family.len().min(1), icon_font.into());
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance and contrast ratio.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |color: Color32| {
            let channel = |value: u8| {
                let value = f32::from(value) / 255.0;
                if value <= 0.039_28 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
        };
        let (light, dark) = {
            let (x, y) = (luminance(a), luminance(b));
            (x.max(y), x.min(y))
        };
        (light + 0.05) / (dark + 0.05)
    }

    #[test]
    fn text_and_buttons_have_enough_contrast() {
        // Text and state colors on both backgrounds: 4.5:1 (T039.02, CA-10).
        for text in [TEXT, MUTED, PRIMARY, SUCCESS, WARNING, DANGER] {
            for background in [BACKGROUND, SURFACE] {
                assert!(
                    contrast(text, background) >= 4.5,
                    "{text:?} on {background:?}"
                );
            }
        }
        // White text on filled buttons: 4.5:1; the button against the page: 3:1.
        assert!(contrast(Color32::WHITE, PRIMARY_FILL) >= 4.5);
        assert!(contrast(PRIMARY_FILL, SURFACE) >= 3.0);
        // The old fill did not pass, which is why PRIMARY_FILL exists.
        assert!(contrast(Color32::WHITE, PRIMARY) < 4.5);
    }
}
