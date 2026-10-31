use std::{
    ffi::OsStr,
    fmt,
    path::{Component, Path, PathBuf, Prefix},
};

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
    /// Compose project name label (`com.docker.compose.project`).
    pub compose_project: Option<String>,
    pub project_directory: Option<PathBuf>,
    pub running_for: Option<String>,
}

impl RuntimeContainer {
    pub fn browser_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        // Engines list the same binding once per address family.
        for url in self.ports.split(',').filter_map(runtime_browser_url) {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        urls
    }

    /// Whether the Compose working directory recorded by the engine is the project or one of
    /// its subfolders.
    pub fn belongs_to(&self, project: &Path) -> bool {
        self.project_directory
            .as_deref()
            .is_some_and(|directory| path_is_within(directory, project))
    }

    /// Whether `host`, as written in an application configuration, names this container
    /// inside the Compose network.
    pub fn answers_to_host(&self, host: &str) -> bool {
        self.service.as_deref() == Some(host) || self.name == host
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
    let scheme = web_scheme(target)?;
    let host = match host {
        "0.0.0.0" | "" => "localhost",
        "localhost" | "127.0.0.1" => host,
        _ => return None,
    };
    Some(format!("{scheme}://{host}:{published}"))
}

/// Reads one `host:published->target/tcp` entry as printed by `docker ps`.
fn runtime_browser_url(mapping: &str) -> Option<String> {
    let (published, target) = mapping.trim().split_once("->")?;
    let target = target.strip_suffix("/tcp")?.parse::<u16>().ok()?;
    let (host, port) = published.rsplit_once(':')?;
    let host = match host {
        "0.0.0.0" | "" | "::" | "[::]" => "localhost",
        "127.0.0.1" | "localhost" => host,
        _ => return None,
    };
    let port = port.parse::<u16>().ok().filter(|port| *port > 0)?;
    Some(format!("{}://{host}:{port}", web_scheme(target)?))
}

/// Container ports conventionally served over HTTP(S) by web servers and development
/// tools. Database, FastCGI (9000) and other protocol ports are deliberately absent.
fn web_scheme(target: u16) -> Option<&'static str> {
    match target {
        443 | 8443 => Some("https"),
        80 | 3000 | 3001 | 4000 | 4173 | 4200 | 5000 | 5173 | 5174 | 8000 | 8001 | 8008 | 8025
        | 8080 | 8081 | 8888 => Some("http"),
        _ => None,
    }
}

/// Compares paths lexically, without touching the filesystem, so it is cheap enough for
/// every frame. On Windows it also tolerates how engines record Compose directories:
/// drive-letter and name case, `\\?\` prefixes, either separator and WSL `/mnt/<drive>`.
pub fn path_is_within(path: &Path, root: &Path) -> bool {
    let root = path_key(root);
    !root.is_empty() && path_key(path).starts_with(&root)
}

fn path_key(path: &Path) -> Vec<String> {
    let mut key = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Prefix(prefix) => key.push(match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                    format!("{}:", char::from(drive).to_ascii_lowercase())
                }
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                    format!(r"\\{}\{}", fold_case(server), fold_case(share))
                }
                _ => fold_case(component.as_os_str()),
            }),
            Component::RootDir => key.push("/".into()),
            other => key.push(fold_case(other.as_os_str())),
        }
    }
    if cfg!(windows)
        && let [root, mnt, drive, ..] = key.as_slice()
        && root == "/"
        && mnt == "mnt"
        && drive.len() == 1
        && drive.chars().all(|letter| letter.is_ascii_alphabetic())
    {
        let drive = format!("{drive}:");
        key.splice(0..3, [drive, "/".to_owned()]);
    }
    key
}

fn fold_case(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value.into_owned()
    }
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
    /// `--file` arguments for the main manifest and its overrides, relative to `directory`.
    /// Every Compose call, including the project-name query, uses the same list.
    pub fn compose_file_arguments(&self) -> Vec<String> {
        self.compose_file
            .iter()
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
            .collect()
    }

    pub fn proposed_commands(&self, engine: ContainerEngine) -> Vec<ContainerCommand> {
        if self.error.is_some() {
            return Vec::new();
        }
        if self.compose_file.is_some() {
            let compose_arguments = self.compose_file_arguments();
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
            compose_project: None,
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

    fn runtime_container(ports: &str, project_directory: Option<&str>) -> RuntimeContainer {
        RuntimeContainer {
            engine: ContainerEngine::Docker,
            id: "abcdef123456".into(),
            name: "shop-db-1".into(),
            image: "mariadb".into(),
            state: "running".into(),
            status: "Up".into(),
            ports: ports.into(),
            service: Some("db".into()),
            project_directory: project_directory.map(PathBuf::from),
            running_for: None,
            compose_project: None,
        }
    }

    #[test]
    fn runtime_urls_cover_common_dev_servers_once_per_binding() {
        let container = runtime_container(
            "0.0.0.0:4200->4200/tcp, :::4200->4200/tcp, [::]:4200->4200/tcp, \
             127.0.0.1:5001->5000/tcp, 0.0.0.0:8025->8025/tcp, 0.0.0.0:9000->9000/tcp, \
             0.0.0.0:3307->3306/tcp, 0.0.0.0:8080-8081->80-81/tcp, 10.0.0.5:8082->80/tcp",
            None,
        );
        assert_eq!(
            container.browser_urls(),
            [
                "http://localhost:4200",
                "http://127.0.0.1:5001",
                "http://localhost:8025"
            ]
        );
    }

    #[test]
    fn compose_and_runtime_urls_share_the_web_port_list() {
        for target in [3001, 4000, 4173, 4200, 5000, 5174, 8001, 8008, 8025, 8081] {
            let service = ContainerService {
                name: "web".into(),
                image: None,
                builds_image: false,
                ports: vec![format!("18000:{target}")],
                profiles: vec![],
            };
            assert_eq!(service.browser_urls(), ["http://localhost:18000"]);
            let container = runtime_container(&format!("0.0.0.0:18000->{target}/tcp"), None);
            assert_eq!(container.browser_urls(), ["http://localhost:18000"]);
        }
    }

    #[test]
    fn containers_belong_to_their_project_and_its_subfolders_only() {
        let root = std::env::temp_dir().join("lcp-shop");
        let compose_in_subfolder = root.join("docker");
        let container = runtime_container("", compose_in_subfolder.to_str());
        assert!(container.belongs_to(&root));
        assert!(container.belongs_to(&compose_in_subfolder));
        assert!(!container.belongs_to(&root.join("docker/other")));
        assert!(!container.belongs_to(&std::env::temp_dir().join("lcp-sho")));
        assert!(!runtime_container("", None).belongs_to(&root));
        assert!(!path_is_within(&root, Path::new("")));
        assert!(container.answers_to_host("db"));
        assert!(container.answers_to_host("shop-db-1"));
        assert!(!container.answers_to_host("localhost"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_match_regardless_of_how_the_engine_recorded_them() {
        let project = Path::new(r"C:\Users\Dev\Área de Trabalho\Shop");
        for recorded in [
            r"c:\Users\Dev\Área de Trabalho\Shop",
            r"C:\users\dev\área de trabalho\shop\docker",
            "C:/Users/Dev/Área de Trabalho/Shop/",
            r"\\?\C:\Users\Dev\Área de Trabalho\Shop",
            "/mnt/c/Users/Dev/Área de Trabalho/Shop",
        ] {
            assert!(path_is_within(Path::new(recorded), project), "{recorded}");
        }
        assert!(!path_is_within(
            Path::new(r"D:\Users\Dev\Área de Trabalho\Shop"),
            project
        ));
        assert!(!path_is_within(
            Path::new("/mnt/d/Users/Dev/Área de Trabalho/Shop"),
            project
        ));
        assert!(path_is_within(
            Path::new(r"\\Server\Share\shop"),
            Path::new(r"\\server\share")
        ));
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
