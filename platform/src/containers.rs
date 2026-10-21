use crate::installations::{configure_background_command, find_executable};
use localcodepilot_core::containers::{
    ContainerCommand, ContainerEngine, EngineDiagnosis, EngineState,
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
