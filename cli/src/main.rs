use localcodepilot_core::{discovery::DiscoveryService, projects::Project};
use localcodepilot_platform::{NativePlatform, Platform, filesystem::FilesystemProjectSource};
use localcodepilot_runtime::ManifestRuntimeDetector;
use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("status") => status(),
        Some("scan") => scan(),
        Some("inspect") => inspect(env::args_os().nth(2).map(PathBuf::from)),
        Some("containers") => containers(env::args_os().nth(2).map(PathBuf::from)),
        Some("databases") => databases(env::args_os().nth(2).map(PathBuf::from)),
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
    let technologies = localcodepilot_runtime::detect_technologies(&path);
    let project = Project::new(path, runtimes).with_technologies(technologies);
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
        "LocalCodePilot CLI\n\n  localcodepilot status\n  localcodepilot scan\n  localcodepilot inspect <pasta>\n  localcodepilot containers <pasta>\n  localcodepilot databases <pasta>"
    );
}

fn databases(path: Option<PathBuf>) -> ExitCode {
    let Some(path) = path.filter(|path| path.is_dir()) else {
        eprintln!("Uso: localcodepilot databases <pasta existente>");
        return ExitCode::FAILURE;
    };
    let requirements = localcodepilot_runtime::databases::detect_database_requirements(&path);
    if requirements.is_empty() {
        println!("Nenhuma configuração DB_CONNECTION ativa encontrada em .env ou .env.local.");
        return ExitCode::SUCCESS;
    }
    let mut unavailable = false;
    for database in requirements {
        println!("{}: {}", database.source.display(), database.engine.label());
        if let (Some(host), Some(port)) = (&database.host, database.port) {
            println!("  Destino: {host}:{port}");
        }
        use localcodepilot_runtime::databases::DatabasePreparation;
        match localcodepilot_runtime::databases::database_preparation(&database) {
            DatabasePreparation::LocalServerReachable(_) => {
                println!("  TCP local: disponível; autenticação e schema não verificados")
            }
            DatabasePreparation::PrepareLocalServer(engine) => {
                println!(
                    "  Próxima etapa: preparar servidor local de {}",
                    engine.label()
                );
                unavailable = true;
            }
            DatabasePreparation::RemoteServer(_) => {
                println!("  Servidor remoto: nenhuma instalação local planejada");
            }
            DatabasePreparation::FileDatabase => {
                println!("  SQLite: arquivo e schema não verificados");
            }
            DatabasePreparation::NeedsConfiguration(_) => {
                println!("  Configuração incompleta: host ou porta local não definido");
                unavailable = true;
            }
        }
    }
    if unavailable {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn containers(path: Option<PathBuf>) -> ExitCode {
    let Some(path) = path.filter(|path| path.is_dir()) else {
        eprintln!("Uso: localcodepilot containers <pasta existente>");
        return ExitCode::FAILURE;
    };
    let projects = localcodepilot_runtime::containers::detect_container_projects(&path);
    println!(
        "Diagnóstico de containers — nenhuma instalação ou execução de containers será iniciada."
    );
    for engine in localcodepilot_platform::containers::inspect_container_engines(None) {
        println!(
            "{}: {:?} · Compose: {}",
            engine.engine,
            engine.state,
            if engine.compose_available {
                "disponível"
            } else {
                "não confirmado"
            }
        );
    }
    let mut invalid = false;
    for project in &projects {
        println!("\n{}", project.directory.display());
        if let Some(file) = &project.dockerfile {
            println!("Dockerfile: {}", file.display());
        }
        if let Some(file) = &project.compose_file {
            println!("Compose: {}", file.display());
        }
        if let Some(error) = &project.error {
            eprintln!("Erro: {error}");
            invalid = true;
        }
        for service in &project.services {
            println!(
                "  {} · imagem: {} · build: {} · portas: {} · perfis: {}",
                service.name,
                service.image.as_deref().unwrap_or("não declarada"),
                service.builds_image,
                service.ports.join(", "),
                service.profiles.join(", ")
            );
        }
        for warning in &project.warnings {
            println!("Aviso: {warning}");
        }
        for engine in [
            localcodepilot_core::containers::ContainerEngine::Docker,
            localcodepilot_core::containers::ContainerEngine::Podman,
        ] {
            for command in project.proposed_commands(engine) {
                println!("Prévia — {}: {}", command.label, command.display());
            }
        }
    }
    if projects.is_empty() {
        println!("Nenhum manifesto de containers encontrado.");
    }
    if invalid {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
