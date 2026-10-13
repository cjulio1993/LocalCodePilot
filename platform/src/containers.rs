use crate::installations::{configure_background_command, find_executable};
use localcodepilot_core::containers::{ContainerEngine, EngineDiagnosis, EngineState};
use std::{
    ffi::OsStr,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

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
}
