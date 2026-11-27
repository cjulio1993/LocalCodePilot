#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
#[cfg(test)]
#[path = "../build_support.rs"]
mod build_support;
mod config;
mod diagnostic_export;
mod pages;
mod secrets;
mod storage;
mod theme;
mod update_progress_ui;
mod update_window;
mod updates;

use app::LocalCodePilot;
use eframe::egui;

fn app_icon() -> Option<egui::IconData> {
    eframe::icon_data::from_png_bytes(include_bytes!("../../assets/branding/icon-512.png")).ok()
}

fn main() -> eframe::Result {
    if let Some(exit_code) = updates::try_run_update_helper(update_window::show) {
        std::process::exit(exit_code);
    }
    #[cfg(feature = "update-test-feed")]
    updates::simulate_test_startup();
    updates::spawn_stale_stage_cleanup();
    let icon = app_icon().expect("the embedded LocalCodePilot icon must be a valid PNG");
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
            let startup_update = updates::complete_updated_startup();
            Ok(Box::new(LocalCodePilot::new(cc, startup_update)))
        }),
    )
}
