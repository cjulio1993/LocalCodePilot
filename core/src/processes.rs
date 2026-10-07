use crate::ports::Port;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Stopped,
    Starting,
    Running,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortOverride {
    LongOption { separator: bool },
    Positional,
    PhpServerAddress { argument_index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProcess {
    pub id: String,
    pub command: String,
    pub state: ProcessState,
    pub process_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectProcess {
    pub id: String,
    pub project_name: String,
    pub project_path: PathBuf,
    pub working_directory: PathBuf,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub state: ProcessState,
    pub process_id: Option<u32>,
    pub exit_code: Option<i32>,
    pub expected_port: Option<Port>,
    pub port_override: Option<PortOverride>,
}

impl ProjectProcess {
    pub fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn args_with_port(&self, port: Port) -> Option<Vec<String>> {
        let strategy = self.port_override?;
        let mut args = self.args.clone();
        match strategy {
            PortOverride::LongOption { separator } => {
                if separator {
                    args.push("--".into());
                }
                args.push("--port".into());
                args.push(port.value().to_string());
            }
            PortOverride::Positional => args.push(port.value().to_string()),
            PortOverride::PhpServerAddress { argument_index } => {
                let address = args.get_mut(argument_index)?;
                let host = address
                    .rsplit_once(':')
                    .map_or(address.as_str(), |(host, _)| host)
                    .to_owned();
                *address = format!("{host}:{}", port.value());
            }
        }
        Some(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(args: &[&str], port_override: PortOverride) -> ProjectProcess {
        ProjectProcess {
            id: "test".into(),
            project_name: "Test".into(),
            project_path: PathBuf::from("test"),
            working_directory: PathBuf::from("test"),
            name: "Development server".into(),
            program: "npm".into(),
            args: args.iter().map(|argument| (*argument).into()).collect(),
            state: ProcessState::Stopped,
            process_id: None,
            exit_code: None,
            expected_port: Port::new(5173),
            port_override: Some(port_override),
        }
    }

    #[test]
    fn builds_safe_port_overrides_for_supported_commands() {
        let npm = process(
            &["run", "dev"],
            PortOverride::LongOption { separator: true },
        );
        assert_eq!(
            npm.args_with_port(Port::new(5174).unwrap()).unwrap(),
            ["run", "dev", "--", "--port", "5174"]
        );

        let django = process(&["manage.py", "runserver"], PortOverride::Positional);
        assert_eq!(
            django.args_with_port(Port::new(8001).unwrap()).unwrap(),
            ["manage.py", "runserver", "8001"]
        );

        let php = process(
            &["-S", "localhost:8000"],
            PortOverride::PhpServerAddress { argument_index: 1 },
        );
        assert_eq!(
            php.args_with_port(Port::new(8001).unwrap()).unwrap(),
            ["-S", "localhost:8001"]
        );
    }
}
