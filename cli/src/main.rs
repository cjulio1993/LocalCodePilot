use localcodepilot_core::{discovery::DiscoveryService, projects::Project};
use localcodepilot_platform::{NativePlatform, Platform, filesystem::FilesystemProjectSource};
use localcodepilot_runtime::ManifestRuntimeDetector;
use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("status") => status(),
        Some("scan") => scan(),
        Some("inspect") => inspect(env::args_os().nth(2).map(PathBuf::from)),
        Some("--help" | "-h" | "help") => {
            help();
            ExitCode::SUCCESS
        }
        Some(command) => {
            eprintln!("Comando desconhecido: {command}\n");
            help();
            ExitCode::FAILURE
        }
    }
}

fn status() -> ExitCode {
    let mut platform = NativePlatform::default();
    let snapshot = platform.snapshot();
    println!("LocalCodePilot");
    println!("Sistema: {:?}", platform.operating_system());
    println!(
        "Memória: {:.1} / {:.1} GB",
        snapshot.used_memory_bytes as f64 / 1_073_741_824.0,
        snapshot.total_memory_bytes as f64 / 1_073_741_824.0
    );
    ExitCode::SUCCESS
}

fn inspect(path: Option<PathBuf>) -> ExitCode {
    let Some(path) = path else {
        eprintln!("Uso: localcodepilot inspect <pasta>");
        return ExitCode::FAILURE;
    };
    if !path.is_dir() {
        eprintln!("A pasta não existe: {}", path.display());
        return ExitCode::FAILURE;
    }
    let runtimes = localcodepilot_runtime::detect(&path);
    let project = Project::new(path, runtimes);
    println!("{} — {}", project.name, project.display_stack());
    println!("{}", project.path.display());
    ExitCode::SUCCESS
}

fn scan() -> ExitCode {
    let service = DiscoveryService::new(
        FilesystemProjectSource::common_locations(),
        ManifestRuntimeDetector,
    );
    match service.discover() {
        Ok(catalog) => {
            for project in catalog.projects() {
                println!(
                    "{} — {}\n  {}",
                    project.name,
                    project.display_stack(),
                    project.path.display()
                );
            }
            println!("\n{} projeto(s) encontrado(s)", catalog.projects().len());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("A varredura falhou: {error}");
            ExitCode::FAILURE
        }
    }
}

fn help() {
    println!(
        "LocalCodePilot CLI\n\n  localcodepilot status\n  localcodepilot scan\n  localcodepilot inspect <pasta>"
    );
}
