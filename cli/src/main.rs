use localcodepilot_core::{discovery::DiscoveryService, paths::display_path, projects::Project};
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

/// Name resolution and engine state for the Compose protection, per ready engine.
struct ComposeState {
    engine: localcodepilot_core::containers::ContainerEngine,
    catalog: Vec<localcodepilot_core::compose_safety::ComposeEnvironment>,
    containers: Option<Vec<localcodepilot_core::containers::RuntimeContainer>>,
    volumes: Option<Vec<localcodepilot_core::compose_safety::ComposeVolume>>,
}

fn compose_states(
    projects: &[localcodepilot_core::containers::ContainerProject],
    engines: &[localcodepilot_core::containers::EngineDiagnosis],
) -> Vec<ComposeState> {
    use localcodepilot_core::compose_safety::{ComposeEnvironment, ComposeName};
    use localcodepilot_platform::containers as platform;
    engines
        .iter()
        .filter(|diagnosis| diagnosis.state == localcodepilot_core::containers::EngineState::Ready)
        .map(|diagnosis| ComposeState {
            engine: diagnosis.engine,
            catalog: projects
                .iter()
                .filter(|project| project.compose_file.is_some())
                .map(|project| ComposeEnvironment {
                    directory: project.directory.clone(),
                    name: platform::resolve_compose_name(diagnosis.engine, project, None)
                        .map_or_else(
                            |_| {
                                ComposeName::Estimated(
                                    localcodepilot_runtime::containers::estimate_compose_name(
                                        project,
                                    ),
                                )
                            },
                            ComposeName::Resolved,
                        ),
                })
                .collect(),
            containers: platform::query_engine_containers(diagnosis.engine, None).ok(),
            volumes: platform::list_compose_volumes(diagnosis.engine, None).ok(),
        })
        .collect()
}

fn print_compose_safety(
    project: &localcodepilot_core::containers::ContainerProject,
    states: &[ComposeState],
) {
    use localcodepilot_core::compose_safety::{ComposeSafety, classify_environment, explain};
    for state in states {
        let Some(environment) = state
            .catalog
            .iter()
            .find(|environment| environment.directory == project.directory)
        else {
            continue;
        };
        let safety = classify_environment(
            environment,
            &state.catalog,
            state.engine,
            state.containers.as_deref(),
            state.volumes.as_deref(),
            &[],
        );
        println!(
            "Ambiente Compose no {}: \"{}\"",
            state.engine,
            environment.name.value()
        );
        if safety == ComposeSafety::Safe {
            println!("  Proteção: nenhum conflito com outros ambientes");
            continue;
        }
        let blocked = explain(&safety, &project.directory);
        println!("  Bloqueado para iniciar e parar: {}", blocked.title);
        println!("  {}", blocked.explanation);
        for (index, step) in blocked.steps.iter().enumerate() {
            println!("  {}. {step}", index + 1);
        }
        if let Some(snippet) = &blocked.snippet {
            println!("       {snippet}");
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
        eprintln!("A pasta não existe: {}", display_path(&path));
        return ExitCode::FAILURE;
    }
    let runtimes = localcodepilot_runtime::detect(&path);
    let technologies = localcodepilot_runtime::detect_technologies(&path);
    let project = Project::new(path, runtimes).with_technologies(technologies);
    println!("{} — {}", project.name, project.display_stack());
    println!("{}", display_path(&project.path));
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
                    display_path(&project.path)
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
        println!("Nenhuma configuração de banco encontrada em .env, .env.local ou wp-config.php.");
        return ExitCode::SUCCESS;
    }
    let mut unavailable = false;
    let mut runtime_containers = None;
    for database in requirements {
        println!(
            "{}: {}",
            display_path(&database.source),
            database.engine.label()
        );
        if let (Some(host), Some(port)) = (&database.host, database.port) {
            println!("  Destino: {host}:{port}");
        }
        use localcodepilot_runtime::databases::DatabaseRoute;
        match localcodepilot_runtime::databases::route_database(&database) {
            DatabaseRoute::LocalServerReachable(_) => {
                println!("  TCP local: disponível; autenticação e schema não verificados")
            }
            DatabaseRoute::LocalServerUnavailable(engine) => {
                println!(
                    "  {} local indisponível; verifique instalação, serviço e porta",
                    engine.label()
                );
                unavailable = true;
            }
            DatabaseRoute::ComposeService(_) => {
                let service = database.compose_service.as_deref().unwrap_or_default();
                println!(
                    "  Serviço Compose `{service}`: o banco roda na rede dos containers e não é testado a partir do host"
                );
                let containers = runtime_containers.get_or_insert_with(|| {
                    localcodepilot_platform::containers::list_runtime_containers(None)
                });
                match containers.iter().find(|container| {
                    container.belongs_to(&path) && container.answers_to_host(service)
                }) {
                    Some(container) if container.state.eq_ignore_ascii_case("running") => {
                        println!("  Container {}: em execução", container.name)
                    }
                    Some(container) => {
                        println!(
                            "  Container {}: {}; inicie os serviços em Containers",
                            container.name, container.state
                        );
                        unavailable = true;
                    }
                    None => {
                        println!(
                            "  Nenhum container criado; inicie os serviços em Containers ou com `compose up`"
                        );
                        unavailable = true;
                    }
                }
            }
            DatabaseRoute::RemoteServer(_) => {
                println!(
                    "  Servidor remoto: conexão não testada; nenhuma alteração local será feita"
                );
            }
            DatabaseRoute::FileDatabase => {
                println!("  SQLite: arquivo e schema não verificados");
            }
            DatabaseRoute::NeedsConfiguration(_) => {
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
    let host = localcodepilot_platform::containers::inspect_container_host(None);
    let engines = localcodepilot_platform::containers::inspect_container_engines(None);
    for engine in &engines {
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
        if let Some(guidance) = localcodepilot_core::container_setup::engine_guidance(engine, &host)
        {
            println!("  {}", guidance.summary);
            for (index, step) in guidance.steps.iter().enumerate() {
                println!("  {}. {}", index + 1, step.text);
                if let Some(command) = step.command {
                    println!("       {command}");
                }
                if let Some(link) = step.link {
                    println!("       {link}");
                }
            }
            for note in &guidance.notes {
                println!("  Nota: {note}");
            }
        }
    }
    let compose_states = compose_states(&projects, &engines);
    let mut invalid = false;
    for project in &projects {
        println!("\n{}", display_path(&project.directory));
        if let Some(file) = &project.dockerfile {
            println!("Dockerfile: {}", display_path(file));
        }
        if let Some(file) = &project.compose_file {
            println!("Compose: {}", display_path(file));
        }
        print_compose_safety(project, &compose_states);
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
