//! Starting an installed container engine that is stopped, after the person confirms (T026).
//! The plan comes from `localcodepilot_core::engine_start`; this module gathers the read-only
//! facts for it, runs the start and waits until the engine and Compose answer. Every command
//! goes through [`EngineRunner`], so tests never touch a real engine.

use crate::installations::{configure_background_command, find_executable};
use localcodepilot_core::containers::ContainerEngine;
use localcodepilot_core::engine_start::{
    DefaultMachine, EngineStart, PodmanConnection, StartFacts,
};
use std::{
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// How long the engine may take to answer after the start: Docker Desktop and a Podman
/// machine boot a virtual machine.
pub const START_TIMEOUT: Duration = Duration::from_secs(3 * 60);
/// A new Podman machine downloads its image first.
pub const CREATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Each read-only query, including the readiness checks, ends within this time.
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

/// Why a start did not leave the engine ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartFailure {
    /// A start command failed; the text is its error output.
    Command(String),
    /// The engine did not answer in time.
    TimedOut,
    /// The engine answers, but Compose does not.
    ComposeMissing,
}

/// What the engine answers while the app waits for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    NotYet,
    EngineOnly,
    Ready,
}

/// Runs the commands of a start. The real one is [`SystemRunner`].
pub trait EngineRunner {
    /// Runs a command to completion; `Err` carries its error output.
    fn run(&mut self, program: &str, args: &[&str], timeout: Duration) -> Result<(), String>;
    /// Opens an application without waiting for it.
    fn open(&mut self, executable: &Path) -> Result<(), String>;
    fn readiness(&mut self, engine: ContainerEngine) -> Readiness;
    fn sleep(&mut self, duration: Duration);
    fn elapsed(&self) -> Duration;
}

/// Runs the start and waits until the engine and Compose answer.
pub fn start_engine(
    engine: ContainerEngine,
    start: &EngineStart,
    runner: &mut dyn EngineRunner,
) -> Result<(), StartFailure> {
    let command_timeout = if start.downloads() {
        CREATE_TIMEOUT
    } else {
        START_TIMEOUT
    };
    match start {
        EngineStart::DockerDesktopCli => {
            runner.run("docker", &["desktop", "start", "--detach"], command_timeout)
        }
        EngineStart::DockerDesktopApp(executable) => runner.open(executable),
        EngineStart::SystemdUser(unit) => {
            runner.run("systemctl", &["--user", "start", unit], command_timeout)
        }
        EngineStart::PodmanMachine { name, create } => {
            if *create {
                runner
                    .run("podman", &["machine", "init", name], CREATE_TIMEOUT)
                    .map_err(StartFailure::Command)?;
            }
            runner.run("podman", &["machine", "start", name], command_timeout)
        }
    }
    .map_err(StartFailure::Command)?;
    wait_until_ready(engine, START_TIMEOUT, runner)
}

/// Asks the engine every [`POLL_INTERVAL`] until it and Compose answer, or `timeout` passes.
pub fn wait_until_ready(
    engine: ContainerEngine,
    timeout: Duration,
    runner: &mut dyn EngineRunner,
) -> Result<(), StartFailure> {
    let started = runner.elapsed();
    loop {
        let last = match runner.readiness(engine) {
            Readiness::Ready => return Ok(()),
            other => other,
        };
        if runner.elapsed().saturating_sub(started) >= timeout {
            return Err(if last == Readiness::EngineOnly {
                StartFailure::ComposeMissing
            } else {
                StartFailure::TimedOut
            });
        }
        runner.sleep(POLL_INTERVAL);
    }
}

/// Read-only facts for planning a start. Nothing is started here.
pub fn gather_start_facts(path: Option<&OsStr>) -> StartFacts {
    let docker_desktop_cli = run_quietly("docker", &["desktop", "version"], path).is_ok();
    let podman_default = (!cfg!(target_os = "linux"))
        .then(|| run_quietly("podman", &["machine", "list", "--format", "json"], path).ok())
        .flatten()
        .and_then(|output| default_podman_machine(&output));
    let systemd_user_units = if cfg!(target_os = "linux") {
        run_quietly(
            "systemctl",
            &[
                "--user",
                "list-unit-files",
                "--no-legend",
                "docker-desktop.service",
                "docker.service",
                "podman.socket",
            ],
            path,
        )
        .map(|output| parse_unit_files(&output))
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let docker_endpoint = run_quietly(
        "docker",
        &[
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ],
        path,
    )
    .ok()
    .map(|output| output.trim().to_owned())
    .filter(|endpoint| !endpoint.is_empty());
    let podman_connection = run_quietly(
        "podman",
        &["system", "connection", "list", "--format", "json"],
        path,
    )
    .ok()
    .map(|output| default_podman_connection(&output));
    StartFacts {
        docker_desktop_cli,
        docker_endpoint,
        podman_connection,
        docker_desktop_app: docker_desktop_app(),
        systemd_user_units,
        podman_default,
        host_override: env::var_os("DOCKER_HOST").is_some()
            || env::var_os("CONTAINER_HOST").is_some(),
    }
}

/// The default machine in `podman machine list --format json`: the one marked `Default`, or
/// the only machine there is.
pub fn default_podman_machine(output: &str) -> Option<DefaultMachine> {
    let machines = serde_json::from_str::<Vec<serde_json::Value>>(output).ok()?;
    if machines.is_empty() {
        return Some(DefaultMachine::NotCreated);
    }
    let flagged = machines
        .iter()
        .filter(|machine| machine.get("Default").and_then(serde_json::Value::as_bool) == Some(true))
        .collect::<Vec<_>>();
    let machine = match (flagged.as_slice(), machines.as_slice()) {
        ([one], _) => *one,
        ([], [one]) => one,
        _ => return Some(DefaultMachine::Ambiguous),
    };
    let name = machine.get("Name")?.as_str()?.to_owned();
    let running = machine
        .get("Running")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Some(if running {
        DefaultMachine::Running(name)
    } else {
        DefaultMachine::Stopped(name)
    })
}

/// The default connection in `podman system connection list --format json`.
pub fn default_podman_connection(output: &str) -> PodmanConnection {
    let Ok(connections) = serde_json::from_str::<Vec<serde_json::Value>>(output) else {
        return PodmanConnection::Other;
    };
    if connections.is_empty() {
        return PodmanConnection::NoneConfigured;
    }
    let Some(default) = connections.iter().find(|connection| {
        connection
            .get("Default")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    }) else {
        return PodmanConnection::Other;
    };
    let is_machine = default
        .get("IsMachine")
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    match default.get("Name").and_then(serde_json::Value::as_str) {
        Some(name) if is_machine => PodmanConnection::Machine(name.to_owned()),
        _ => PodmanConnection::Other,
    }
}

/// Unit names in the first column of `systemctl list-unit-files --no-legend`.
fn parse_unit_files(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

fn docker_desktop_app() -> Option<PathBuf> {
    let candidate = if cfg!(windows) {
        env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .map(|folder| {
                folder
                    .join("Docker")
                    .join("Docker")
                    .join("Docker Desktop.exe")
            })?
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Applications/Docker.app")
    } else {
        return None;
    };
    candidate.exists().then_some(candidate)
}

/// Runs a short read-only query and returns its output.
fn run_quietly(program: &str, args: &[&str], path: Option<&OsStr>) -> Result<String, String> {
    let executable = find_executable(program, path).ok_or("CLI indisponível")?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    configure_background_command(&mut command);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut stdout, &mut text);
            text
        })
    });
    let stderr = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr, &mut text);
            text
        })
    });
    let started = Instant::now();
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() < QUERY_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = crate::ports::terminate_process_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{program} não respondeu em {} s",
                    QUERY_TIMEOUT.as_secs()
                ));
            }
        }
    };
    let stdout = stdout
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    let stderr = stderr
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    if success {
        Ok(stdout)
    } else {
        Err(stderr.trim().to_owned())
    }
}

/// Runs the real commands. Used only after the person confirms.
pub struct SystemRunner<'a> {
    path: Option<&'a OsStr>,
    started: Instant,
}

impl<'a> SystemRunner<'a> {
    pub fn new(path: Option<&'a OsStr>) -> Self {
        Self {
            path,
            started: Instant::now(),
        }
    }
}

impl EngineRunner for SystemRunner<'_> {
    fn run(&mut self, program: &str, args: &[&str], timeout: Duration) -> Result<(), String> {
        let executable = find_executable(program, self.path).ok_or("CLI indisponível")?;
        let mut command = Command::new(executable);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(path) = self.path {
            command.env("PATH", path);
        }
        configure_background_command(&mut command);
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(_)) => {
                    let mut text = String::new();
                    if let Some(mut stderr) = child.stderr.take() {
                        use std::io::Read;
                        let _ = stderr.read_to_string(&mut text);
                    }
                    return Err(text.trim().to_owned());
                }
                Ok(None) if started.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(200))
                }
                Ok(None) => {
                    let _ = crate::ports::terminate_process_tree(child.id());
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{program} não terminou em {} s", timeout.as_secs()));
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    fn open(&mut self, executable: &Path) -> Result<(), String> {
        let mut command = if cfg!(target_os = "macos") {
            let mut command = Command::new("open");
            command.arg("-a").arg(executable);
            command
        } else {
            Command::new(executable)
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn readiness(&mut self, engine: ContainerEngine) -> Readiness {
        let program = engine.program();
        if run_quietly(program, &["info"], self.path).is_err() {
            Readiness::NotYet
        } else if run_quietly(program, &["compose", "version"], self.path).is_err() {
            Readiness::EngineOnly
        } else {
            Readiness::Ready
        }
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records every command and answers from a script; never runs anything.
    #[derive(Default)]
    struct FakeRunner {
        calls: Vec<String>,
        fail: Option<String>,
        ready_after: Option<Duration>,
        compose: bool,
        now: Duration,
        /// Each readiness check takes this long, as a query that hangs until its time limit.
        check_takes: Duration,
    }

    impl EngineRunner for FakeRunner {
        fn run(&mut self, program: &str, args: &[&str], _: Duration) -> Result<(), String> {
            self.calls.push(format!("{program} {}", args.join(" ")));
            match &self.fail {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }
        fn open(&mut self, executable: &Path) -> Result<(), String> {
            self.calls.push(format!("abrir {}", executable.display()));
            Ok(())
        }
        fn readiness(&mut self, _: ContainerEngine) -> Readiness {
            self.now += self.check_takes;
            match self.ready_after {
                Some(after) if self.now >= after && self.compose => Readiness::Ready,
                Some(after) if self.now >= after => Readiness::EngineOnly,
                _ => Readiness::NotYet,
            }
        }
        fn sleep(&mut self, duration: Duration) {
            self.now += duration;
        }
        fn elapsed(&self) -> Duration {
            self.now
        }
    }

    fn ready_in(seconds: u64) -> FakeRunner {
        FakeRunner {
            ready_after: Some(Duration::from_secs(seconds)),
            compose: true,
            ..FakeRunner::default()
        }
    }

    #[test]
    fn each_plan_runs_only_its_own_commands_and_waits_for_the_engine() {
        let cases = [
            (
                EngineStart::DockerDesktopCli,
                vec!["docker desktop start --detach"],
            ),
            (
                EngineStart::DockerDesktopApp(PathBuf::from("Docker Desktop.exe")),
                vec!["abrir Docker Desktop.exe"],
            ),
            (
                EngineStart::SystemdUser("docker-desktop".into()),
                vec!["systemctl --user start docker-desktop"],
            ),
            (
                EngineStart::PodmanMachine {
                    name: "podman-machine-default".into(),
                    create: false,
                },
                vec!["podman machine start podman-machine-default"],
            ),
            (
                EngineStart::PodmanMachine {
                    name: "podman-machine-default".into(),
                    create: true,
                },
                vec![
                    "podman machine init podman-machine-default",
                    "podman machine start podman-machine-default",
                ],
            ),
        ];
        for (start, expected) in cases {
            let mut runner = ready_in(10);
            assert_eq!(
                start_engine(ContainerEngine::Docker, &start, &mut runner),
                Ok(())
            );
            assert_eq!(runner.calls, expected, "{start:?}");
        }
    }

    #[test]
    fn a_failed_command_stops_before_waiting_or_starting_more() {
        let mut runner = FakeRunner {
            fail: Some("Error: wsl não instalado".into()),
            ..ready_in(0)
        };
        let start = EngineStart::PodmanMachine {
            name: "podman-machine-default".into(),
            create: true,
        };
        assert_eq!(
            start_engine(ContainerEngine::Podman, &start, &mut runner),
            Err(StartFailure::Command("Error: wsl não instalado".into()))
        );
        assert_eq!(
            runner.calls,
            vec!["podman machine init podman-machine-default"]
        );
    }

    #[test]
    fn an_engine_that_never_answers_times_out_and_one_without_compose_says_so() {
        let mut never = FakeRunner::default();
        assert_eq!(
            wait_until_ready(ContainerEngine::Docker, START_TIMEOUT, &mut never),
            Err(StartFailure::TimedOut)
        );
        assert!(never.now >= START_TIMEOUT);
        let mut without_compose = FakeRunner {
            compose: false,
            ..ready_in(5)
        };
        assert_eq!(
            wait_until_ready(ContainerEngine::Podman, START_TIMEOUT, &mut without_compose),
            Err(StartFailure::ComposeMissing)
        );
    }

    #[test]
    fn checks_that_hang_until_their_limit_still_end_the_wait_in_time() {
        let mut hanging = FakeRunner {
            check_takes: QUERY_TIMEOUT,
            ..FakeRunner::default()
        };
        assert_eq!(
            wait_until_ready(ContainerEngine::Docker, START_TIMEOUT, &mut hanging),
            Err(StartFailure::TimedOut)
        );
        // Bounded: the last check started before the limit and took at most QUERY_TIMEOUT.
        assert!(hanging.now <= START_TIMEOUT + QUERY_TIMEOUT + POLL_INTERVAL);
    }

    #[test]
    fn reads_the_default_podman_connection() {
        // Trimmed `podman system connection list --format json` from Podman 6.1.4 on Windows.
        let machine = r#"[{"Name":"podman-machine-default","URI":"ssh://user@127.0.0.1:62034/run/user/1000/podman/podman.sock","IsMachine":true,"Default":true},{"Name":"podman-machine-default-root","IsMachine":true,"Default":false}]"#;
        assert_eq!(
            default_podman_connection(machine),
            PodmanConnection::Machine("podman-machine-default".into())
        );
        let remote = r#"[{"Name":"servidor","URI":"ssh://deploy@10.0.0.5/run/podman/podman.sock","IsMachine":false,"Default":true}]"#;
        assert_eq!(default_podman_connection(remote), PodmanConnection::Other);
        let no_default = r#"[{"Name":"podman-machine-default","IsMachine":true,"Default":false}]"#;
        assert_eq!(
            default_podman_connection(no_default),
            PodmanConnection::Other
        );
        assert_eq!(
            default_podman_connection("[]"),
            PodmanConnection::NoneConfigured
        );
        assert_eq!(default_podman_connection("Error"), PodmanConnection::Other);
    }

    #[test]
    fn reads_only_the_default_podman_machine() {
        assert_eq!(
            default_podman_machine("[]"),
            Some(DefaultMachine::NotCreated)
        );
        let one = r#"[{"Name":"podman-machine-default","Default":false,"Running":false}]"#;
        assert_eq!(
            default_podman_machine(one),
            Some(DefaultMachine::Stopped("podman-machine-default".into()))
        );
        let flagged = r#"[{"Name":"outra","Default":false,"Running":true},{"Name":"trabalho","Default":true,"Running":false}]"#;
        assert_eq!(
            default_podman_machine(flagged),
            Some(DefaultMachine::Stopped("trabalho".into()))
        );
        let none_flagged = r#"[{"Name":"a","Running":false},{"Name":"b","Running":false}]"#;
        assert_eq!(
            default_podman_machine(none_flagged),
            Some(DefaultMachine::Ambiguous)
        );
        let running = r#"[{"Name":"trabalho","Default":true,"Running":true}]"#;
        assert_eq!(
            default_podman_machine(running),
            Some(DefaultMachine::Running("trabalho".into()))
        );
        assert_eq!(default_podman_machine("Error: cannot connect"), None);
    }

    #[test]
    fn reads_unit_names_from_systemctl() {
        let output = "docker-desktop.service disabled enabled\npodman.socket enabled enabled\n";
        assert_eq!(
            parse_unit_files(output),
            vec![
                "docker-desktop.service".to_owned(),
                "podman.socket".to_owned()
            ]
        );
    }

    #[test]
    fn gathering_facts_with_an_empty_path_runs_and_starts_nothing() {
        let facts = gather_start_facts(Some(OsStr::new("")));
        assert!(!facts.docker_desktop_cli);
        assert_eq!(facts.podman_default, None);
        assert!(facts.systemd_user_units.is_empty());
    }
}
