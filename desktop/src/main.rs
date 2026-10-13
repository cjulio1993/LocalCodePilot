#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod config;
mod pages;
mod theme;
mod updates;

use app::LocalCodePilot;
use eframe::egui;

fn main() -> eframe::Result {
    if let Some(exit_code) = updates::try_run_update_helper() {
        std::process::exit(exit_code);
    }
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../../assets/branding/icon-512.png"))
            .expect("the embedded LocalCodePilot icon must be a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LocalCodePilot")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "LocalCodePilot",
        options,
        Box::new(|cc| {
            let startup_notification = updates::complete_updated_startup();
            Ok(Box::new(LocalCodePilot::new(cc, startup_notification)))
        }),
    )
}
