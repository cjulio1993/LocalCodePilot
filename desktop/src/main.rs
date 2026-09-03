mod app;
mod pages;
mod theme;

use app::LocalCodePilot;
use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LocalCodePilot")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "LocalCodePilot",
        options,
        Box::new(|cc| Ok(Box::new(LocalCodePilot::new(cc)))),
    )
}
