use crate::installations::{configure_background_command, find_executable};
use localcodepilot_core::container_setup::{ContainerHostFacts, HostOs, PodmanMachine};
use localcodepilot_core::containers::{
    ContainerCommand, ContainerEngine, EngineDiagnosis, EngineState, RuntimeContainer,
};
use localcodepilot_core::ports::strip_terminal_sequences;
use std::{
    ffi::OsStr,
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

const LIST_TIMEOUT: Duration = Duration::from_secs(6);

/// Queries real containers without starting or changing them. Each engine is isolated so an
/// unavailable daemon cannot hide containers from the other engine.
pub fn list_runtime_containers(path: Option<&OsStr>) -> Vec<RuntimeContainer> {
    [ContainerEngine::Docker, ContainerEngine::Podman]
        .into_iter()
        .flat_map(|engine| list_engine_containers(engine, path).unwrap_or_default())
        .collect()
}

fn list_engine_containers(
    engine: ContainerEngine,
    path: Option<&OsStr>,
) -> Result<Vec<RuntimeContainer>, String> {
    let output = read_command_output(
        engine.program(),
        &["ps", "--all", "--format", "{{json .}}"],
        path,
    )?;
    Ok(parse_container_rows(engine, &output))
}

/// Read-only facts used to guide the user through preparing an engine. Nothing is installed,
/// started or created here.
pub fn inspect_container_host(path: Option<&OsStr>) -> ContainerHostFacts {
    let os = HostOs::current();
    let wsl_ready = (os == HostOs::Windows).then(|| probe("wsl", &["--status"], path));
    let podman_machine = (os != HostOs::Linux)
        .then(|| read_command_output("podman", &["machine", "list", "--format", "json"], path).ok())
        .flatten()
        .and_then(|output| parse_podman_machines(&output));
    ContainerHostFacts {
        os,
        wsl_ready: match wsl_ready {
            Some(ProbeResult::Success) => Some(true),
            Some(ProbeResult::Missing | ProbeResult::Failed) => Some(false),
            Some(ProbeResult::TimedOut) | None => None,
        },
        podman_machine,
    }
}

fn parse_podman_machines(output: &str) -> Option<PodmanMachine> {
    let machines = serde_json::from_str::<Vec<serde_json::Value>>(output).ok()?;
    Some(if machines.is_empty() {
        PodmanMachine::NotCreated
    } else if machines.iter().any(|machine| {
        machine
            .get("Running")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }) {
        PodmanMachine::Running
    } else {
        PodmanMachine::Stopped
    })
}

/// Runs a read-only query and returns its standard output, capped at 1 MiB and stopped after
/// `LIST_TIMEOUT`.
fn read_command_output(
    program: &str,
    args: &[&str],
    path: Option<&OsStr>,
) -> Result<String, String> {
    let executable = find_executable(program, path).ok_or("CLI indisponível")?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    configure_background_command(&mut command);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child.stdout.take().ok_or("Saída indisponível")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(1_048_576)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < LIST_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                terminate_container_process(&mut child);
                let _ = reader.join();
                return Err("Consulta excedeu o tempo limite".into());
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "Leitura interrompida")?
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("Motor indisponível".into());
    }
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn parse_container_rows(engine: ContainerEngine, output: &str) -> Vec<RuntimeContainer> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|row| {
            let field = |name| row.get(name).and_then(serde_json::Value::as_str);
            let id = field("ID").or_else(|| field("Id"))?.to_owned();
            // Docker prints `Names` as text; Podman returns an array.
            let name = field("Names")
                .or_else(|| field("Name"))
                .or_else(|| {
                    row.get("Names")?
                        .as_array()?
                        .iter()
                        .find_map(serde_json::Value::as_str)
                })
                .unwrap_or(&id)
                .to_owned();
            let label = |name: &str| {
                row.get("Labels")
                    .and_then(|labels| labels.get(name))
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        field("Labels")?
                            .split(',')
                            .find_map(|label| label.trim().strip_prefix(name)?.strip_prefix('='))
                    })
                    .map(str::to_owned)
            };
            Some(RuntimeContainer {
                engine,
                id,
                name,
                image: field("Image").unwrap_or_default().to_owned(),
                state: field("State").unwrap_or_default().to_owned(),
                status: field("Status").unwrap_or_default().to_owned(),
                ports: field("Ports").map_or_else(
                    || {
                        row.get("Ports")
                            .and_then(serde_json::Value::as_array)
                            .map(|ports| podman_ports(ports))
                            .unwrap_or_default()
                    },
                    str::to_owned,
                ),
                service: label("com.docker.compose.service")
                    .or_else(|| label("io.podman.compose.service")),
                project_directory: label("com.docker.compose.project.working_dir")
                    .or_else(|| label("io.podman.compose.project.working_dir"))
                    .map(Into::into),
                running_for: field("RunningFor").map(str::to_owned),
            })
        })
        .collect()
}

/// Rewrites Podman's port objects in the `docker ps` text form the rest of the app reads,
/// e.g. `0.0.0.0:8080->80/tcp`. Podman 4+ uses snake_case keys; Podman 3 used camelCase.
fn podman_ports(ports: &[serde_json::Value]) -> String {
    const MAX_RANGE: u64 = 100;
    let mut mappings = Vec::new();
    for port in ports {
        let number = |keys: [&str; 2]| keys.iter().find_map(|key| port.get(key)?.as_u64());
        let text = |keys: [&str; 2]| keys.iter().find_map(|key| port.get(key)?.as_str());
        let (Some(host_port), Some(container_port)) = (
            number(["host_port", "hostPort"]).filter(|port| *port > 0),
            number(["container_port", "containerPort"]),
        ) else {
            continue;
        };
        let host = match text(["host_ip", "hostIP"]).unwrap_or_default() {
            "" => "0.0.0.0".to_owned(),
            ip if ip.contains(':') => format!("[{ip}]"),
            ip => ip.to_owned(),
        };
        let range = port
            .get("range")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1)
            .clamp(1, MAX_RANGE);
        let protocols = port
            .get("protocol")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("tcp");
        for protocol in protocols.split(',').map(str::trim) {
            for offset in 0..range {
                let (Ok(published), Ok(target)) = (
                    u16::try_from(host_port + offset),
                    u16::try_from(container_port + offset),
                ) else {
                    break;
                };
                mappings.push(format!("{host}:{published}->{target}/{protocol}"));
            }
        }
    }
    mappings.join(", ")
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerExecutionOutcome {
    Succeeded,
    Failed(String),
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerExecutionEvent {
    Output(String),
    Finished(ContainerExecutionOutcome),
}

pub struct ContainerExecution {
    pub events: Receiver<ContainerExecutionEvent>,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ContainerExecution {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancellation_requested(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

impl Drop for ContainerExecution {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeResult {
    Missing,
    Success,
    Failed,
    TimedOut,
}

/// Read-only queries. No install, VM start, pull, build or container lifecycle action.
pub fn inspect_container_engines(path: Option<&OsStr>) -> Vec<EngineDiagnosis> {
    [ContainerEngine::Docker, ContainerEngine::Podman]
        .into_iter()
        .map(|engine| diagnose_engine(engine, |args| probe(engine.program(), args, path)))
        .collect()
}

pub fn execute_container_command(
    command: ContainerCommand,
    path: Option<&OsStr>,
) -> Result<ContainerExecution, String> {
    execute_container_command_with_timeout(command, path, EXECUTION_TIMEOUT)
}

fn execute_container_command_with_timeout(
    command: ContainerCommand,
    path: Option<&OsStr>,
    timeout: Duration,
) -> Result<ContainerExecution, String> {
    if !matches!(command.program, "docker" | "podman") {
        return Err("Comando de container não permitido".into());
    }
    if !command.working_directory.is_dir() {
        return Err(format!(
            "A pasta do projeto não existe: {}",
            command.working_directory.display()
        ));
    }
    let executable = find_executable(command.program, path)
        .ok_or_else(|| format!("{} não foi encontrado no PATH", command.program))?;
    let environment_path = path.map(ToOwned::to_owned);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let (sender, events) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut process = Command::new(executable);
        process
            .args(&command.args)
            .current_dir(&command.working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = environment_path {
            process.env("PATH", path);
        }
        configure_background_command(&mut process);
        let mut child = match process.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = sender.send(ContainerExecutionEvent::Finished(
                    ContainerExecutionOutcome::Failed(format!(
                        "Não foi possível iniciar {}: {error}",
                        command.display()
                    )),
                ));
                return;
            }
        };
        let stdout = child
            .stdout
            .take()
            .map(|stdout| spawn_execution_output_reader(stdout, sender.clone()));
        let stderr = child
            .stderr
            .take()
            .map(|stderr| spawn_execution_output_reader(stderr, sender.clone()));
        let started_at = Instant::now();
        let outcome = loop {
            if worker_cancel.load(Ordering::Relaxed) {
                terminate_container_process(&mut child);
                break ContainerExecutionOutcome::Cancelled;
            }
            if started_at.elapsed() >= timeout {
                terminate_container_process(&mut child);
                break ContainerExecutionOutcome::TimedOut;
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => {
                    break ContainerExecutionOutcome::Succeeded;
                }
                Ok(Some(status)) => {
                    break ContainerExecutionOutcome::Failed(format!(
                        "{} encerrou com código {}",
                        command.label,
                        status
                            .code()
                            .map_or_else(|| "desconhecido".into(), |code| code.to_string())
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    terminate_container_process(&mut child);
                    break ContainerExecutionOutcome::Failed(format!(
                        "Não foi possível acompanhar {}: {error}",
                        command.label
                    ));
                }
            }
        };
        if let Some(reader) = stdout {
            let _ = reader.join();
        }
        if let Some(reader) = stderr {
            let _ = reader.join();
        }
        let _ = sender.send(ContainerExecutionEvent::Finished(outcome));
    });
    Ok(ContainerExecution {
        events,
        cancel,
        worker: Some(worker),
    })
}

fn spawn_execution_output_reader(
    reader: impl Read + Send + 'static,
    sender: Sender<ContainerExecutionEvent>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            let _ = sender.send(ContainerExecutionEvent::Output(strip_terminal_sequences(
                &line,
            )));
        }
    })
}

fn terminate_container_process(child: &mut std::process::Child) {
    let _ = crate::ports::terminate_process_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

fn diagnose_engine(
    engine: ContainerEngine,
    mut run: impl FnMut(&[&str]) -> ProbeResult,
) -> EngineDiagnosis {
    let cli = run(&["--version"]);
    if cli != ProbeResult::Success {
        return EngineDiagnosis {
            engine,
            state: match cli {
                ProbeResult::Missing => EngineState::Missing,
                ProbeResult::TimedOut => EngineState::TimedOut,
                _ => EngineState::QueryFailed,
            },
            compose_available: false,
        };
    }
    let state = match run(&["info"]) {
        ProbeResult::Success => EngineState::Ready,
        ProbeResult::TimedOut => EngineState::TimedOut,
        _ => EngineState::Unavailable,
    };
    let compose_available = run(&["compose", "version"]) == ProbeResult::Success;
    EngineDiagnosis {
        engine,
        state,
        compose_available,
    }
}

fn probe(program: &str, args: &[&str], path: Option<&OsStr>) -> ProbeResult {
    let Some(executable) = find_executable(program, path) else {
        return ProbeResult::Missing;
    };
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    configure_background_command(&mut command);
    let Ok(mut child) = command.spawn() else {
        return ProbeResult::Failed;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    ProbeResult::Success
                } else {
                    ProbeResult::Failed
                };
            }
            Ok(None) if started.elapsed() < PROBE_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50))
            }
            result => {
                let _ = crate::ports::terminate_process_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return if result.is_err() {
                    ProbeResult::Failed
                } else {
                    ProbeResult::TimedOut
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_real_container_rows_and_compose_project_labels() {
        let output = r#"{"ID":"abc123","Names":"wordpress-web-1","Image":"wordpress:latest","State":"running","Status":"Up 2 minutes","Ports":"0.0.0.0:8080->80/tcp","Labels":"com.docker.compose.service=web,com.docker.compose.project.working_dir=/projects/wordpress"}"#;
        let rows = parse_container_rows(ContainerEngine::Docker, output);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].service.as_deref(), Some("web"));
        assert_eq!(
            rows[0].project_directory.as_deref(),
            Some(std::path::Path::new("/projects/wordpress"))
        );
        assert_eq!(rows[0].state, "running");
        let podman = r#"{"Id":"abcdef987654","Names":"db-1","Image":"mariadb","State":"exited","Labels":{"io.podman.compose.service":"db","io.podman.compose.project.working_dir":"/projects/wordpress"}}"#;
        let rows = parse_container_rows(ContainerEngine::Podman, podman);
        assert_eq!(rows[0].service.as_deref(), Some("db"));
        assert_eq!(rows[0].state, "exited");
    }

    #[test]
    fn reads_podman_rows_with_name_and_port_arrays() {
        // Shape of `podman ps --all --format "{{json .}}"` (Podman 4+, ListContainer).
        let output = concat!(
            r#"{"AutoRemove":false,"Command":["apache2-foreground"],"Created":1761900000,"Exited":false,"Id":"0f3c2b1a9d8e7f6a5b4c3d2e1f0a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2a","Image":"docker.io/library/wordpress:latest","Labels":{"com.docker.compose.project.working_dir":"C:\\Projetos\\Blog","com.docker.compose.service":"wordpress"},"Names":["blog-wordpress-1"],"Networks":["blog_default"],"Pod":"","Ports":[{"host_ip":"","container_port":80,"host_port":8080,"range":1,"protocol":"tcp"},{"host_ip":"127.0.0.1","container_port":8025,"host_port":8025,"range":2,"protocol":"tcp,udp"}],"State":"running","Status":"Up 3 minutes"}"#,
            "\n",
            r#"{"Id":"1a2b3c4d5e6f","Image":"docker.io/library/mariadb:11","Names":["blog-db-1"],"Ports":null,"State":"exited","Labels":{"com.docker.compose.service":"db"}}"#,
            "\n",
            r#"{"Id":"aabbccddeeff","Names":["legacy"],"Ports":[{"hostIP":"::","containerPort":3000,"hostPort":3000,"protocol":"tcp"},{"host_ip":"","container_port":80,"host_port":0,"protocol":"tcp"},{"host_ip":"","container_port":65535,"host_port":65535,"range":3,"protocol":"tcp"}],"State":"running"}"#,
        );
        let rows = parse_container_rows(ContainerEngine::Podman, output);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "blog-wordpress-1");
        assert_eq!(
            rows[0].ports,
            "0.0.0.0:8080->80/tcp, 127.0.0.1:8025->8025/tcp, 127.0.0.1:8026->8026/tcp, \
             127.0.0.1:8025->8025/udp, 127.0.0.1:8026->8026/udp"
        );
        assert_eq!(
            rows[0].browser_urls(),
            ["http://localhost:8080", "http://127.0.0.1:8025"]
        );
        assert_eq!(rows[0].service.as_deref(), Some("wordpress"));
        assert_eq!(rows[0].status, "Up 3 minutes");
        assert_eq!(rows[1].name, "blog-db-1");
        assert!(rows[1].ports.is_empty());
        assert_eq!(
            rows[2].ports,
            "[::]:3000->3000/tcp, 0.0.0.0:65535->65535/tcp"
        );
        assert_eq!(rows[2].browser_urls(), ["http://localhost:3000"]);
    }

    /// Manual check against real engines. Start a Compose project with a web service
    /// publishing port 80 and a `db` service on each engine, then run:
    /// `LCP_QA_PROJECT=<project folder> cargo test -p localcodepilot-platform -- --ignored real_engines`
    #[test]
    #[ignore = "requires Docker and Podman running a Compose fixture"]
    fn real_engines_list_fixture_containers_for_the_project() {
        let project = std::path::PathBuf::from(
            std::env::var_os("LCP_QA_PROJECT").expect("LCP_QA_PROJECT must name the fixture"),
        );
        let containers = list_runtime_containers(None);
        for engine in [ContainerEngine::Docker, ContainerEngine::Podman] {
            let rows = containers
                .iter()
                .filter(|container| container.engine == engine && container.belongs_to(&project))
                .collect::<Vec<_>>();
            for row in &rows {
                eprintln!(
                    "{engine} · {} · {} · {:?} · {:?}",
                    row.name,
                    row.state,
                    row.ports,
                    row.browser_urls()
                );
            }
            let web = rows
                .iter()
                .find(|container| !container.browser_urls().is_empty())
                .unwrap_or_else(|| panic!("{engine}: no web container with a URL"));
            assert_eq!(web.browser_urls().len(), 1, "{engine}: duplicated URLs");
            assert!(
                !web.name.starts_with(&web.id),
                "{engine}: name fell back to id"
            );
            assert!(
                rows.iter().any(
                    |container| container.answers_to_host("db") && container.state == "running"
                ),
                "{engine}: database container not associated"
            );
        }
    }

    #[test]
    fn reads_the_podman_machine_state_from_machine_list() {
        // Trimmed `podman machine list --format json` from Podman 6.1.4 on Windows.
        let stopped = r#"[{"Name":"podman-machine-default","Default":true,"Running":false,"Starting":false,"VMType":"wsl"}]"#;
        assert_eq!(parse_podman_machines(stopped), Some(PodmanMachine::Stopped));
        let running = r#"[{"Name":"other","Running":false},{"Name":"podman-machine-default","Running":true}]"#;
        assert_eq!(parse_podman_machines(running), Some(PodmanMachine::Running));
        assert_eq!(parse_podman_machines("[]"), Some(PodmanMachine::NotCreated));
        assert_eq!(parse_podman_machines("Error: cannot connect"), None);
    }

    #[test]
    fn host_inspection_with_empty_path_reports_missing_tools_without_changes() {
        let facts = inspect_container_host(Some(OsStr::new("")));
        assert_eq!(facts.os, HostOs::current());
        assert_eq!(facts.podman_machine, None);
        if facts.os == HostOs::Windows {
            assert_eq!(facts.wsl_ready, Some(false));
        } else {
            assert_eq!(facts.wsl_ready, None);
        }
    }

    #[test]
    fn missing_cli_never_queries_daemon_or_compose() {
        let mut calls = Vec::new();
        let result = diagnose_engine(ContainerEngine::Docker, |args| {
            calls.push(args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>());
            ProbeResult::Missing
        });
        assert_eq!(result.state, EngineState::Missing);
        assert_eq!(calls, vec![vec!["--version"]]);
        assert!(!result.compose_available);
    }

    #[test]
    fn distinguishes_engine_health_from_compose_provider() {
        for engine in [ContainerEngine::Docker, ContainerEngine::Podman] {
            let unavailable = diagnose_engine(engine, |args| {
                if args == ["info"] {
                    ProbeResult::Failed
                } else {
                    ProbeResult::Success
                }
            });
            assert_eq!(unavailable.state, EngineState::Unavailable);
            assert!(unavailable.compose_available);
            let ready = diagnose_engine(engine, |args| {
                if args == ["compose", "version"] {
                    ProbeResult::Failed
                } else {
                    ProbeResult::Success
                }
            });
            assert_eq!(ready.state, EngineState::Ready);
            assert!(!ready.compose_available);
            let timeout = diagnose_engine(engine, |args| {
                if args == ["info"] {
                    ProbeResult::TimedOut
                } else {
                    ProbeResult::Success
                }
            });
            assert_eq!(timeout.state, EngineState::TimedOut);
        }
    }

    #[test]
    fn inspection_with_empty_path_does_not_install_anything() {
        let diagnosis = inspect_container_engines(Some(OsStr::new("")));
        assert!(
            diagnosis
                .iter()
                .all(|engine| engine.state == EngineState::Missing)
        );
    }

    #[test]
    fn execution_rejects_unknown_programs_and_missing_directories() {
        let unknown = ContainerCommand {
            action: localcodepilot_core::containers::ContainerAction::Status,
            label: "Consultar estado",
            program: "shell",
            args: vec![],
            working_directory: std::env::temp_dir(),
        };
        assert!(execute_container_command(unknown, None).is_err());

        let missing = ContainerCommand {
            action: localcodepilot_core::containers::ContainerAction::Status,
            label: "Consultar estado",
            program: "docker",
            args: vec![],
            working_directory: std::env::temp_dir().join("localcodepilot-missing-container-dir"),
        };
        assert!(execute_container_command(missing, None).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn cancelled_and_timed_out_commands_stop_the_child_process() {
        use localcodepilot_core::containers::ContainerAction;
        use std::{fs, path::Path, sync::mpsc::RecvTimeoutError};

        let directory = std::env::temp_dir().join(format!(
            "localcodepilot-container-lifecycle-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        // A long-running Windows executable under the Docker name exercises the
        // exact runner without requiring a container engine in the test host.
        let fake_docker = directory.join("docker.exe");
        fs::copy(
            Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/ping.exe"),
            &fake_docker,
        )
        .unwrap();
        let command = ContainerCommand {
            action: ContainerAction::Status,
            label: "Consultar estado",
            program: "docker",
            args: vec!["-n".into(), "30".into(), "127.0.0.1".into()],
            working_directory: directory.clone(),
        };

        let cancelled = execute_container_command_with_timeout(
            command.clone(),
            Some(directory.as_os_str()),
            Duration::from_secs(10),
        )
        .unwrap();
        cancelled.cancel();
        assert_eq!(
            receive_outcome(&cancelled.events),
            ContainerExecutionOutcome::Cancelled
        );
        drop(cancelled);

        let timed_out = execute_container_command_with_timeout(
            command,
            Some(directory.as_os_str()),
            Duration::from_millis(250),
        )
        .unwrap();
        assert_eq!(
            receive_outcome(&timed_out.events),
            ContainerExecutionOutcome::TimedOut
        );
        drop(timed_out);

        let closing = execute_container_command_with_timeout(
            ContainerCommand {
                action: ContainerAction::Status,
                label: "Consultar estado",
                program: "docker",
                args: vec!["-n".into(), "30".into(), "127.0.0.1".into()],
                working_directory: directory.clone(),
            },
            Some(directory.as_os_str()),
            Duration::from_secs(10),
        )
        .unwrap();
        let close_started = Instant::now();
        drop(closing);
        assert!(close_started.elapsed() < Duration::from_secs(10));
        fs::remove_dir_all(directory).unwrap();

        fn receive_outcome(
            events: &Receiver<ContainerExecutionEvent>,
        ) -> ContainerExecutionOutcome {
            loop {
                match events.recv_timeout(Duration::from_secs(10)) {
                    Ok(ContainerExecutionEvent::Finished(outcome)) => return outcome,
                    Ok(ContainerExecutionEvent::Output(_)) => {}
                    Err(RecvTimeoutError::Timeout) => panic!("container command did not finish"),
                    Err(RecvTimeoutError::Disconnected) => panic!("container worker disconnected"),
                }
            }
        }
    }
}
