use std::{fmt, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerEngine {
    Docker,
    Podman,
}

impl ContainerEngine {
    pub fn program(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }
}

impl fmt::Display for ContainerEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Docker => "Docker",
            Self::Podman => "Podman",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    Missing,
    Ready,
    Unavailable,
    TimedOut,
    QueryFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineDiagnosis {
    pub engine: ContainerEngine,
    pub state: EngineState,
    pub compose_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContainer {
    pub engine: ContainerEngine,
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
    pub service: Option<String>,
    pub project_directory: Option<PathBuf>,
    pub running_for: Option<String>,
}

impl RuntimeContainer {
    pub fn browser_urls(&self) -> Vec<String> {
        self.ports
            .split(',')
            .filter_map(|mapping| {
                let mapping = mapping.trim();
                let (published, target) = mapping.split_once("->")?;
                let target = target.strip_suffix("/tcp")?.parse::<u16>().ok()?;
                let (host, port) = published.rsplit_once(':')?;
                let host = match host {
                    "0.0.0.0" | "" => "localhost",
                    "127.0.0.1" | "localhost" => host,
                    _ => return None,
                };
                let port = port.parse::<u16>().ok().filter(|port| *port > 0)?;
                let scheme = match target {
                    443 | 8443 => "https",
                    80 | 8000 | 8080 | 3000 | 5173 | 8888 => "http",
                    _ => return None,
                };
                Some(format!("{scheme}://{host}:{port}"))
            })
            .collect()
    }

    pub fn command(&self, action: ContainerAction) -> Option<ContainerCommand> {
        if self.id.len() < 6
            || self.id.len() > 64
            || !self
                .id
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return None;
        }
        let (label, args): (&'static str, Vec<String>) = match action {
            ContainerAction::StartExisting if !self.state.eq_ignore_ascii_case("running") => {
                ("Iniciar container", vec!["start".into(), self.id.clone()])
            }
            ContainerAction::StopExisting if self.state.eq_ignore_ascii_case("running") => {
                ("Parar container", vec!["stop".into(), self.id.clone()])
            }
            ContainerAction::RestartExisting if self.state.eq_ignore_ascii_case("running") => (
                "Reiniciar container",
                vec!["restart".into(), self.id.clone()],
            ),
            ContainerAction::LogsExisting => (
                "Visualizar logs",
                vec![
                    "logs".into(),
                    "--tail".into(),
                    "200".into(),
                    self.id.clone(),
                ],
            ),
            _ => return None,
        };
        Some(ContainerCommand {
            action,
            label,
            program: self.engine.program(),
            args,
            working_directory: self
                .project_directory
                .as_ref()
                .filter(|directory| directory.is_dir())
                .cloned()
                .unwrap_or_else(|| PathBuf::from(".")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerService {
    pub name: String,
    pub image: Option<String>,
    pub builds_image: bool,
    pub ports: Vec<String>,
    pub profiles: Vec<String>,
}

impl ContainerService {
    /// Returns browser candidates only for published TCP ports that look like web services.
    /// Compose variables and dynamic ports require a runtime query.
    pub fn browser_urls(&self) -> Vec<String> {
        self.ports
            .iter()
            .filter_map(|mapping| browser_url(&self.name, mapping))
            .collect()
    }
}

fn browser_url(_service: &str, mapping: &str) -> Option<String> {
    let mapping = mapping.strip_suffix("/tcp").unwrap_or(mapping);
    if mapping.ends_with("/udp") || mapping.contains(['$', '{', '}', '→']) {
        return None;
    }
    let parts = mapping.split(':').collect::<Vec<_>>();
    let (host, published, target) = match parts.as_slice() {
        [published, target] => ("localhost", *published, *target),
        [host, published, target] => (*host, *published, *target),
        _ => return None,
    };
    let published = published.parse::<u16>().ok().filter(|port| *port > 0)?;
    let target = target.parse::<u16>().ok().filter(|port| *port > 0)?;
    let scheme = match target {
        443 | 8443 => "https",
        80 | 8000 | 8080 | 3000 | 5173 | 8888 => "http",
        _ => return None,
    };
    let host = match host {
        "0.0.0.0" | "" => "localhost",
        "localhost" | "127.0.0.1" => host,
        _ => return None,
    };
    Some(format!("{scheme}://{host}:{published}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerProject {
    pub directory: PathBuf,
    pub dockerfile: Option<PathBuf>,
    pub compose_file: Option<PathBuf>,
    pub compose_overrides: Vec<PathBuf>,
    pub services: Vec<ContainerService>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerCommand {
    pub action: ContainerAction,
    pub label: &'static str,
    pub program: &'static str,
    pub args: Vec<String>,
    pub working_directory: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerAction {
    Validate,
    Start,
    Stop,
    Status,
    Logs,
    Build,
    StartExisting,
    StopExisting,
    RestartExisting,
    LogsExisting,
}

impl ContainerAction {
    pub fn description(self) -> &'static str {
        match self {
            Self::Validate => "Valida e resolve a configuração Compose sem iniciar containers.",
            Self::Start => {
                "Pode baixar ou construir imagens e cria ou inicia os serviços em segundo plano."
            }
            Self::Stop => "Interrompe os serviços sem remover volumes.",
            Self::Status => "Consulta o estado e as portas publicadas pelos serviços.",
            Self::Logs => "Consulta as 200 linhas mais recentes dos serviços.",
            Self::Build => "Constrói uma imagem local e pode baixar imagens-base.",
            Self::StartExisting => "Inicia um container existente que está parado.",
            Self::StopExisting => "Interrompe um container em execução sem removê-lo.",
            Self::RestartExisting => "Reinicia um container existente sem removê-lo.",
            Self::LogsExisting => "Consulta as 200 linhas mais recentes do container.",
        }
    }

    pub fn changes_environment(self) -> bool {
        matches!(
            self,
            Self::Start
                | Self::Stop
                | Self::Build
                | Self::StartExisting
                | Self::StopExisting
                | Self::RestartExisting
        )
    }
}

impl ContainerCommand {
    pub fn display(&self) -> String {
        std::iter::once(self.program.to_owned())
            .chain(self.args.iter().map(|arg| {
                // Presentation only; execution must use the argument vector directly.
                if arg.chars().any(char::is_whitespace) {
                    format!("{arg:?}")
                } else {
                    arg.clone()
                }
            }))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl ContainerProject {
    pub fn proposed_commands(&self, engine: ContainerEngine) -> Vec<ContainerCommand> {
        if self.error.is_some() {
            return Vec::new();
        }
        if let Some(file) = &self.compose_file {
            let compose_arguments = std::iter::once(file)
                .chain(&self.compose_overrides)
                .flat_map(|file| {
                    [
                        "--file".to_owned(),
                        file.file_name()
                            .unwrap_or(file.as_os_str())
                            .to_string_lossy()
                            .into_owned(),
                    ]
                })
                .collect::<Vec<_>>();
            let validate_args = match engine {
                ContainerEngine::Docker => vec!["config", "--quiet"],
                ContainerEngine::Podman => vec!["config"],
            };
            let status_args = match engine {
                ContainerEngine::Docker => vec!["ps", "--all"],
                ContainerEngine::Podman => vec!["ps"],
            };
            return [
                (
                    ContainerAction::Validate,
                    "Validar configuração",
                    validate_args,
                ),
                (
                    ContainerAction::Start,
                    "Iniciar ambiente",
                    vec!["up", "--detach"],
                ),
                (ContainerAction::Stop, "Parar serviços", vec!["stop"]),
                (ContainerAction::Status, "Consultar estado", status_args),
                (
                    ContainerAction::Logs,
                    "Consultar logs",
                    vec!["logs", "--tail", "200"],
                ),
            ]
            .into_iter()
            .map(|(action, label, args)| ContainerCommand {
                action,
                label,
                program: engine.program(),
                args: std::iter::once("compose".into())
                    .chain(compose_arguments.clone())
                    .chain(args.into_iter().map(str::to_owned))
                    .collect(),
                working_directory: self.directory.clone(),
            })
            .collect();
        }
        self.dockerfile
            .as_ref()
            .map(|file| ContainerCommand {
                action: ContainerAction::Build,
                label: "Construir imagem",
                program: engine.program(),
                args: vec![
                    "build".into(),
                    "--file".into(),
                    file.file_name()
                        .unwrap_or(file.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                    ".".into(),
                ],
                working_directory: self.directory.clone(),
            })
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_urls_require_published_tcp_web_ports() {
        let service = ContainerService {
            name: "wordpress".into(),
            image: None,
            builds_image: false,
            ports: vec![
                "8080:80/tcp".into(),
                "127.0.0.1:8443:443/tcp".into(),
                "3306:3306/tcp".into(),
                "53:53/udp".into(),
                "${WEB_PORT}:80/tcp".into(),
            ],
            profiles: vec![],
        };
        assert_eq!(
            service.browser_urls(),
            ["http://localhost:8080", "https://127.0.0.1:8443"]
        );
    }

    #[test]
    fn runtime_container_actions_and_ports_use_real_container_ids() {
        let container = RuntimeContainer {
            engine: ContainerEngine::Docker,
            id: "abcdef123456".into(),
            name: "wordpress-web-1".into(),
            image: "wordpress:latest".into(),
            state: "running".into(),
            status: "Up".into(),
            ports: "0.0.0.0:8080->80/tcp, 0.0.0.0:3306->3306/tcp".into(),
            service: Some("web".into()),
            project_directory: None,
            running_for: None,
        };
        assert_eq!(container.browser_urls(), ["http://localhost:8080"]);
        assert_eq!(
            container
                .command(ContainerAction::StopExisting)
                .unwrap()
                .args,
            ["stop", "abcdef123456"]
        );
        assert!(container.command(ContainerAction::StartExisting).is_none());
        let mut invalid = container;
        invalid.id = "--rm".into();
        assert!(invalid.command(ContainerAction::StopExisting).is_none());
    }

    #[test]
    fn preserves_paths_and_previews_both_engines_without_deleting_volumes() {
        let project = ContainerProject {
            directory: PathBuf::from("project with spaces"),
            dockerfile: None,
            compose_file: Some(PathBuf::from("project with spaces/compose.yaml")),
            compose_overrides: vec![],
            services: vec![],
            warnings: vec![],
            error: None,
        };
        for engine in [ContainerEngine::Docker, ContainerEngine::Podman] {
            let commands = project.proposed_commands(engine);
            assert_eq!(commands.len(), 5);
            assert_eq!(commands[0].program, engine.program());
            assert_eq!(commands[0].args[2], "compose.yaml");
            assert_eq!(commands[0].working_directory, project.directory);
            assert_eq!(commands[1].action, ContainerAction::Start);
            assert_eq!(commands[2].args.last().unwrap(), "stop");
            assert_eq!(commands[4].args.last().unwrap(), "200");
            assert!(
                commands
                    .iter()
                    .all(|cmd| !cmd.args.iter().any(|arg| arg == "--volumes"))
            );
            if engine == ContainerEngine::Docker {
                assert_eq!(commands[0].args.last().unwrap(), "--quiet");
                assert_eq!(commands[3].args.last().unwrap(), "--all");
            } else {
                assert_eq!(commands[0].args.last().unwrap(), "config");
                assert_eq!(commands[3].args.last().unwrap(), "ps");
            }
        }
        let mut invalid = project;
        invalid.error = Some("invalid".into());
        assert!(
            invalid
                .proposed_commands(ContainerEngine::Docker)
                .is_empty()
        );
    }
}
