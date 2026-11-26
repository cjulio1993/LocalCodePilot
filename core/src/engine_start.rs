//! Deciding whether an installed container engine is certainly stopped, and how the app can
//! start it after the person confirms (T026). Pure logic: the facts come from `platform`, and
//! nothing here runs a command. Any doubt (a permission error, another connection, an unknown
//! message) leaves the engine without a start plan: the app then explains instead of acting.

use crate::container_setup::HostOs;
use crate::containers::ContainerEngine;
use std::path::PathBuf;

/// What the failed `info` query says about the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoFailure {
    /// The engine's daemon or virtual machine is not running.
    Stopped,
    /// The engine runs, but this user may not use it.
    PermissionDenied,
    /// The CLI points to another host or connection than this computer's default.
    OtherConnection,
    /// Anything else: not enough to act on.
    Unknown,
}

/// How the app starts an installed engine that is stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineStart {
    /// `docker desktop start --detach` (Docker Desktop 4.37 and later).
    DockerDesktopCli,
    /// Opens Docker Desktop from its installed executable (Windows) or app bundle (macOS).
    DockerDesktopApp(PathBuf),
    /// `systemctl --user start <unit>`: Docker Desktop for Linux, rootless Docker or the
    /// Podman socket of this user. Never a system service, which asks for a password.
    SystemdUser(String),
    /// The default Podman machine; `create` runs `podman machine init` first.
    PodmanMachine { name: String, create: bool },
}

impl EngineStart {
    /// Whether starting downloads files and takes disk space (a new Podman machine).
    pub fn downloads(&self) -> bool {
        matches!(self, Self::PodmanMachine { create: true, .. })
    }

    /// The commands the app runs, for "Detalhes técnicos".
    pub fn commands(&self) -> Vec<String> {
        match self {
            Self::DockerDesktopCli => vec!["docker desktop start --detach".into()],
            Self::DockerDesktopApp(path) => vec![format!("abrir {}", path.display())],
            Self::SystemdUser(unit) => vec![format!("systemctl --user start {unit}")],
            Self::PodmanMachine { name, create } => {
                let mut commands = Vec::new();
                if *create {
                    commands.push(format!("podman machine init {name}"));
                }
                commands.push(format!("podman machine start {name}"));
                commands
            }
        }
    }
}

/// The default Podman machine, from `podman machine list --format json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultMachine {
    NotCreated,
    Stopped(String),
    Running(String),
    /// Machines exist, but none is marked as the default: the app does not choose one.
    Ambiguous,
}

/// The connection the Podman CLI uses by default, from `podman system connection list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodmanConnection {
    /// No connection is configured (no machine yet, or Podman on Linux talking locally).
    NoneConfigured,
    /// The default connection belongs to this machine (`<machine>` or `<machine>-root`).
    Machine(String),
    /// Anything else: a remote host, another machine, or a list that could not be read.
    Other,
}

/// Read-only facts gathered to plan a start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartFacts {
    /// `docker desktop version` answered.
    pub docker_desktop_cli: bool,
    /// The installed Docker Desktop executable or app bundle.
    pub docker_desktop_app: Option<PathBuf>,
    /// User units that exist: `docker-desktop.service`, `docker.service`, `podman.socket`.
    pub systemd_user_units: Vec<String>,
    pub podman_default: Option<DefaultMachine>,
    /// `DOCKER_HOST` or `CONTAINER_HOST` is set: the CLI may point elsewhere.
    pub host_override: bool,
    /// The endpoint of the active Docker context, such as `npipe:////./pipe/dockerDesktopLinuxEngine`.
    pub docker_endpoint: Option<String>,
    pub podman_connection: Option<PodmanConnection>,
}

/// Whether a Docker endpoint is this computer's own daemon (a named pipe or a Unix socket).
pub fn is_local_docker_endpoint(endpoint: &str) -> bool {
    endpoint.starts_with("npipe://") || endpoint.starts_with("unix://")
}

pub const PODMAN_DEFAULT_MACHINE: &str = "podman-machine-default";

/// Reads the error output of a failed `info`. Messages are in English, and on Windows the
/// pipe error may come in the system language.
pub fn classify_info_failure(engine: ContainerEngine, os: HostOs, stderr: &str) -> InfoFailure {
    let text = stderr.to_lowercase();
    if text.contains("permission denied")
        || text.contains("access is denied")
        || text.contains("acesso negado")
    {
        return InfoFailure::PermissionDenied;
    }
    if text.contains("tcp://") || text.contains("ssh://") {
        return InfoFailure::OtherConnection;
    }
    match engine {
        ContainerEngine::Docker => {
            let missing_pipe = os == HostOs::Windows
                && text.contains("//./pipe/docker")
                && (text.contains("cannot find the file")
                    || text.contains("não pode encontrar o arquivo")
                    || text.contains("nao pode encontrar o arquivo"));
            let no_daemon = text.contains("cannot connect to the docker daemon")
                && text.contains("is the docker daemon running");
            if missing_pipe || no_daemon {
                InfoFailure::Stopped
            } else {
                InfoFailure::Unknown
            }
        }
        ContainerEngine::Podman => {
            if text.contains("cannot connect to podman") {
                InfoFailure::Stopped
            } else {
                InfoFailure::Unknown
            }
        }
    }
}

/// The start the app may offer, or `None` when it cannot act safely.
pub fn plan_start(
    engine: ContainerEngine,
    os: HostOs,
    failure: InfoFailure,
    facts: &StartFacts,
) -> Option<EngineStart> {
    if failure != InfoFailure::Stopped || facts.host_override {
        return None;
    }
    let has_unit = |unit: &str| facts.systemd_user_units.iter().any(|known| known == unit);
    // The CLI must point to this computer's default target, or the start would act on
    // something else than what the person is shown.
    let target_is_default = match engine {
        ContainerEngine::Docker => facts
            .docker_endpoint
            .as_deref()
            .is_some_and(is_local_docker_endpoint),
        ContainerEngine::Podman => match (&facts.podman_connection, &facts.podman_default, os) {
            (Some(PodmanConnection::NoneConfigured), _, HostOs::Linux) => true,
            (Some(PodmanConnection::NoneConfigured), Some(DefaultMachine::NotCreated), _) => true,
            (
                Some(PodmanConnection::Machine(connection)),
                Some(DefaultMachine::Stopped(machine)),
                _,
            ) => connection == machine || *connection == format!("{machine}-root"),
            _ => false,
        },
    };
    if !target_is_default {
        return None;
    }
    match (engine, os) {
        (ContainerEngine::Docker, HostOs::Windows | HostOs::MacOs) => {
            if facts.docker_desktop_cli {
                Some(EngineStart::DockerDesktopCli)
            } else {
                facts
                    .docker_desktop_app
                    .clone()
                    .map(EngineStart::DockerDesktopApp)
            }
        }
        (ContainerEngine::Docker, HostOs::Linux) => {
            if has_unit("docker-desktop.service") {
                Some(EngineStart::SystemdUser("docker-desktop".into()))
            } else if has_unit("docker.service") {
                Some(EngineStart::SystemdUser("docker".into()))
            } else {
                // The system service needs a password: the app does not ask for one.
                None
            }
        }
        (ContainerEngine::Podman, HostOs::Windows | HostOs::MacOs) => {
            match facts.podman_default.as_ref()? {
                DefaultMachine::NotCreated => Some(EngineStart::PodmanMachine {
                    name: PODMAN_DEFAULT_MACHINE.into(),
                    create: true,
                }),
                DefaultMachine::Stopped(name) => Some(EngineStart::PodmanMachine {
                    name: name.clone(),
                    create: false,
                }),
                // Running but unreachable, or no clear default: something else is wrong.
                DefaultMachine::Running(_) | DefaultMachine::Ambiguous => None,
            }
        }
        (ContainerEngine::Podman, HostOs::Linux) => {
            has_unit("podman.socket").then(|| EngineStart::SystemdUser("podman.socket".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCKER_WINDOWS_STOPPED: &str = "error during connect: Get \"http://%2F%2F.%2Fpipe%2FdockerDesktopLinuxEngine/v1.47/info\": open //./pipe/dockerDesktopLinuxEngine: The system cannot find the file specified.";
    /// Docker CLI 28 on Windows 11 with Docker Desktop stopped (captured 2026-11-26).
    const DOCKER_WINDOWS_STOPPED_NEW: &str = "failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine; check if the path is correct and if the daemon is running: open //./pipe/dockerDesktopLinuxEngine: The system cannot find the file specified.";
    const DOCKER_WINDOWS_STOPPED_PT: &str = "error during connect: open //./pipe/dockerDesktopLinuxEngine: O sistema não pode encontrar o arquivo especificado.";
    const DOCKER_UNIX_STOPPED: &str = "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?";
    const DOCKER_PERMISSION: &str = "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock: Get \"http://%2Fvar%2Frun%2Fdocker.sock/v1.47/info\": dial unix /var/run/docker.sock: connect: permission denied";
    const DOCKER_REMOTE: &str =
        "Cannot connect to the Docker daemon at tcp://10.0.0.5:2376. Is the docker daemon running?";
    const PODMAN_STOPPED: &str = "Cannot connect to Podman. Please verify your connection to the Linux system using `podman system connection list`, or try `podman machine init` and `podman machine start` to manage a new Linux VM";

    fn facts() -> StartFacts {
        StartFacts {
            docker_desktop_cli: true,
            docker_desktop_app: Some(PathBuf::from("Docker Desktop.exe")),
            systemd_user_units: vec!["docker-desktop.service".into()],
            podman_default: Some(DefaultMachine::Stopped(PODMAN_DEFAULT_MACHINE.into())),
            host_override: false,
            docker_endpoint: Some("npipe:////./pipe/dockerDesktopLinuxEngine".into()),
            podman_connection: Some(PodmanConnection::Machine(PODMAN_DEFAULT_MACHINE.into())),
        }
    }

    #[test]
    fn a_cli_pointing_elsewhere_plans_no_start() {
        let docker = |endpoint: Option<&str>| {
            let facts = StartFacts {
                docker_endpoint: endpoint.map(str::to_owned),
                ..facts()
            };
            plan_start(
                ContainerEngine::Docker,
                HostOs::Windows,
                InfoFailure::Stopped,
                &facts,
            )
        };
        assert!(docker(Some("npipe:////./pipe/dockerDesktopLinuxEngine")).is_some());
        assert!(docker(Some("unix:///var/run/docker.sock")).is_some());
        assert_eq!(docker(Some("tcp://10.0.0.5:2376")), None);
        assert_eq!(docker(Some("ssh://deploy@servidor")), None);
        assert_eq!(docker(None), None, "an unreadable context is not proof");

        let podman = |connection: Option<PodmanConnection>, machine: DefaultMachine, os: HostOs| {
            let facts = StartFacts {
                podman_connection: connection,
                podman_default: Some(machine),
                ..facts()
            };
            plan_start(ContainerEngine::Podman, os, InfoFailure::Stopped, &facts)
        };
        let stopped = || DefaultMachine::Stopped(PODMAN_DEFAULT_MACHINE.into());
        let machine = |name: &str| Some(PodmanConnection::Machine(name.into()));
        assert!(podman(machine(PODMAN_DEFAULT_MACHINE), stopped(), HostOs::Windows).is_some());
        assert!(
            podman(
                machine("podman-machine-default-root"),
                stopped(),
                HostOs::Windows
            )
            .is_some()
        );
        assert_eq!(
            podman(machine("outra-maquina"), stopped(), HostOs::Windows),
            None
        );
        assert_eq!(
            podman(Some(PodmanConnection::Other), stopped(), HostOs::MacOs),
            None
        );
        assert_eq!(
            podman(None, stopped(), HostOs::Windows),
            None,
            "an unreadable list is not proof"
        );
        // No connection yet is expected only when the machine does not exist.
        assert!(
            podman(
                Some(PodmanConnection::NoneConfigured),
                DefaultMachine::NotCreated,
                HostOs::Windows
            )
            .is_some()
        );
        assert_eq!(
            podman(
                Some(PodmanConnection::NoneConfigured),
                stopped(),
                HostOs::Windows
            ),
            None
        );
    }

    #[test]
    fn only_a_certainly_stopped_engine_is_called_stopped() {
        use ContainerEngine::*;
        use HostOs::*;
        assert_eq!(
            classify_info_failure(Docker, Windows, DOCKER_WINDOWS_STOPPED),
            InfoFailure::Stopped
        );
        assert_eq!(
            classify_info_failure(Docker, Windows, DOCKER_WINDOWS_STOPPED_PT),
            InfoFailure::Stopped
        );
        assert_eq!(
            classify_info_failure(Docker, Windows, DOCKER_WINDOWS_STOPPED_NEW),
            InfoFailure::Stopped
        );
        assert_eq!(
            classify_info_failure(Docker, Linux, DOCKER_UNIX_STOPPED),
            InfoFailure::Stopped
        );
        assert_eq!(
            classify_info_failure(Docker, Linux, DOCKER_PERMISSION),
            InfoFailure::PermissionDenied
        );
        assert_eq!(
            classify_info_failure(Docker, Linux, DOCKER_REMOTE),
            InfoFailure::OtherConnection
        );
        assert_eq!(
            classify_info_failure(Podman, Windows, PODMAN_STOPPED),
            InfoFailure::Stopped
        );
        // A pipe message outside Windows, an empty error or anything new: not enough to act.
        assert_eq!(
            classify_info_failure(Docker, Linux, DOCKER_WINDOWS_STOPPED),
            InfoFailure::Unknown
        );
        assert_eq!(
            classify_info_failure(Docker, Windows, ""),
            InfoFailure::Unknown
        );
        assert_eq!(
            classify_info_failure(Podman, Linux, "Error: something new"),
            InfoFailure::Unknown
        );
    }

    #[test]
    fn anything_but_stopped_plans_no_start() {
        for failure in [
            InfoFailure::PermissionDenied,
            InfoFailure::OtherConnection,
            InfoFailure::Unknown,
        ] {
            for engine in [ContainerEngine::Docker, ContainerEngine::Podman] {
                for os in [HostOs::Windows, HostOs::MacOs, HostOs::Linux] {
                    assert_eq!(
                        plan_start(engine, os, failure, &facts()),
                        None,
                        "{engine:?} {os:?} {failure:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_host_override_plans_no_start() {
        let facts = StartFacts {
            host_override: true,
            ..facts()
        };
        assert_eq!(
            plan_start(
                ContainerEngine::Docker,
                HostOs::Windows,
                InfoFailure::Stopped,
                &facts
            ),
            None
        );
    }

    #[test]
    fn docker_desktop_starts_by_its_cli_or_its_app() {
        assert_eq!(
            plan_start(
                ContainerEngine::Docker,
                HostOs::Windows,
                InfoFailure::Stopped,
                &facts()
            ),
            Some(EngineStart::DockerDesktopCli)
        );
        let without_cli = StartFacts {
            docker_desktop_cli: false,
            ..facts()
        };
        assert_eq!(
            plan_start(
                ContainerEngine::Docker,
                HostOs::MacOs,
                InfoFailure::Stopped,
                &without_cli
            ),
            Some(EngineStart::DockerDesktopApp(PathBuf::from(
                "Docker Desktop.exe"
            )))
        );
        let nothing = StartFacts {
            docker_desktop_cli: false,
            docker_desktop_app: None,
            ..facts()
        };
        assert_eq!(
            plan_start(
                ContainerEngine::Docker,
                HostOs::Windows,
                InfoFailure::Stopped,
                &nothing
            ),
            None
        );
    }

    #[test]
    fn on_linux_only_user_units_are_started_never_a_system_service() {
        let docker = |units: &[&str]| {
            let facts = StartFacts {
                systemd_user_units: units.iter().map(|unit| (*unit).to_owned()).collect(),
                ..facts()
            };
            plan_start(
                ContainerEngine::Docker,
                HostOs::Linux,
                InfoFailure::Stopped,
                &facts,
            )
        };
        assert_eq!(
            docker(&["docker-desktop.service"]),
            Some(EngineStart::SystemdUser("docker-desktop".into()))
        );
        assert_eq!(
            docker(&["docker.service"]),
            Some(EngineStart::SystemdUser("docker".into()))
        );
        assert_eq!(
            docker(&[]),
            None,
            "the system service would ask for a password"
        );
        let podman = StartFacts {
            systemd_user_units: vec!["podman.socket".into()],
            podman_connection: Some(PodmanConnection::NoneConfigured),
            ..facts()
        };
        assert_eq!(
            plan_start(
                ContainerEngine::Podman,
                HostOs::Linux,
                InfoFailure::Stopped,
                &podman
            ),
            Some(EngineStart::SystemdUser("podman.socket".into()))
        );
    }

    #[test]
    fn podman_uses_only_the_default_machine_and_says_when_it_downloads() {
        let podman = |machine: DefaultMachine| {
            let connection = match &machine {
                DefaultMachine::NotCreated => PodmanConnection::NoneConfigured,
                DefaultMachine::Stopped(name) | DefaultMachine::Running(name) => {
                    PodmanConnection::Machine(name.clone())
                }
                DefaultMachine::Ambiguous => PodmanConnection::Other,
            };
            let facts = StartFacts {
                podman_default: Some(machine),
                podman_connection: Some(connection),
                ..facts()
            };
            plan_start(
                ContainerEngine::Podman,
                HostOs::Windows,
                InfoFailure::Stopped,
                &facts,
            )
        };
        let create = podman(DefaultMachine::NotCreated).unwrap();
        assert!(create.downloads());
        assert_eq!(
            create.commands(),
            vec![
                "podman machine init podman-machine-default".to_owned(),
                "podman machine start podman-machine-default".to_owned()
            ]
        );
        let start = podman(DefaultMachine::Stopped("trabalho".into())).unwrap();
        assert!(!start.downloads());
        assert_eq!(
            start.commands(),
            vec!["podman machine start trabalho".to_owned()]
        );
        assert_eq!(podman(DefaultMachine::Running("trabalho".into())), None);
        assert_eq!(podman(DefaultMachine::Ambiguous), None);
    }
}
