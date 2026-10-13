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
pub struct ContainerService {
    pub name: String,
    pub image: Option<String>,
    pub builds_image: bool,
    pub ports: Vec<String>,
    pub profiles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerProject {
    pub directory: PathBuf,
    pub dockerfile: Option<PathBuf>,
    pub compose_file: Option<PathBuf>,
    pub services: Vec<ContainerService>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerCommand {
    pub label: &'static str,
    pub program: &'static str,
    pub args: Vec<String>,
    pub working_directory: PathBuf,
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
            return [
                ("Validar configuração", vec!["config", "--quiet"]),
                ("Iniciar ambiente", vec!["up", "--detach"]),
                ("Parar serviços", vec!["stop"]),
                ("Consultar estado", vec!["ps"]),
                ("Acompanhar logs", vec!["logs", "--follow"]),
            ]
            .into_iter()
            .map(|(label, action)| ContainerCommand {
                label,
                program: engine.program(),
                args: [
                    "compose".into(),
                    "--file".into(),
                    file.file_name()
                        .unwrap_or(file.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                ]
                .into_iter()
                .chain(action.into_iter().map(str::to_owned))
                .collect(),
                working_directory: self.directory.clone(),
            })
            .collect();
        }
        self.dockerfile
            .as_ref()
            .map(|file| ContainerCommand {
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
    fn preserves_paths_and_previews_both_engines_without_deleting_volumes() {
        let project = ContainerProject {
            directory: PathBuf::from("project with spaces"),
            dockerfile: None,
            compose_file: Some(PathBuf::from("project with spaces/compose.yaml")),
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
            assert_eq!(commands[2].args.last().unwrap(), "stop");
            assert!(
                commands
                    .iter()
                    .all(|cmd| !cmd.args.iter().any(|arg| arg == "--volumes"))
            );
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
