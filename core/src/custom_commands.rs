//! Start commands the person defines for a project when the app finds none (C/C++, Lua, R,
//! or any language outside the table). They become processes like the detected ones, so
//! starting, stopping, logs, the port and "Abrir site" work the same way. They run without a
//! shell: features of a terminal are refused instead of being passed to one.

use crate::{
    ports::Port,
    processes::{ProcessState, ProjectProcess},
    projects::Project,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Process ids of defined commands start with this, which tells them apart from detected
/// ones.
pub const ID_PREFIX: &str = "custom::";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomCommand {
    /// The project folder as the catalog stores it.
    pub project: PathBuf,
    /// Shown as the process name, such as "Servidor" or "Worker".
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    /// Folder relative to the project where the command runs; empty for the project folder.
    pub folder: String,
    pub port: Option<u16>,
}

impl CustomCommand {
    pub fn belongs_to(&self, project: &Path) -> bool {
        self.project == project
    }

    pub fn working_directory(&self) -> PathBuf {
        if self.folder.trim().is_empty() {
            self.project.clone()
        } else {
            self.project.join(self.folder.trim())
        }
    }

    pub fn process_id(&self) -> String {
        format!("{ID_PREFIX}{}::{}", self.project.display(), self.name)
    }

    pub fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(|part| {
                if part.contains(char::is_whitespace) {
                    format!("\"{part}\"")
                } else {
                    part.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn to_process(&self, project: &Project) -> ProjectProcess {
        ProjectProcess {
            id: self.process_id(),
            project_name: project.name.clone(),
            project_path: project.path.clone(),
            working_directory: self.working_directory(),
            name: self.name.clone(),
            program: self.program.clone(),
            args: self.args.clone(),
            state: ProcessState::Stopped,
            process_id: None,
            exit_code: None,
            expected_port: self.port.and_then(Port::new),
            port_override: None,
        }
    }
}

pub fn is_custom(process: &ProjectProcess) -> bool {
    process.id.starts_with(ID_PREFIX)
}

/// The processes of `project` defined by the person.
pub fn processes_for(project: &Project, commands: &[CustomCommand]) -> Vec<ProjectProcess> {
    commands
        .iter()
        .filter(|command| command.belongs_to(&project.path))
        .map(|command| command.to_process(project))
        .collect()
}

/// What in `text` only a terminal understands: pipes, chaining, redirection, substitution,
/// variables and `NAME=value` before the program.
pub fn terminal_feature(text: &str) -> Option<&'static str> {
    const FEATURES: [(&str, &str); 9] = [
        ("&&", "&&"),
        ("||", "||"),
        ("|", "|"),
        (";", ";"),
        (">", ">"),
        ("<", "<"),
        ("`", "`"),
        ("$(", "$("),
        ("${", "${"),
    ];
    if let Some((_, feature)) = FEATURES.iter().find(|(pattern, _)| text.contains(pattern)) {
        return Some(feature);
    }
    if text.contains('$')
        && text
            .split('$')
            .skip(1)
            .any(|rest| rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
    {
        return Some("$VARIÁVEL");
    }
    // `%PATH%` style variables of cmd.exe.
    let percent = text.matches('%').count();
    if percent >= 2 {
        return Some("%VARIÁVEL%");
    }
    let first = text.split_whitespace().next().unwrap_or_default();
    if let Some((name, _)) = first.split_once('=')
        && !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Some("VARIÁVEL=valor");
    }
    None
}

/// Arguments separated by spaces, with double quotes keeping spaces together:
/// `main.lua --name "Ana Maria"` → `["main.lua", "--name", "Ana Maria"]`.
pub fn split_arguments(text: &str) -> Result<Vec<String>, String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for character in text.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err("Há uma aspa sem fechamento nos argumentos.".into());
    }
    if started {
        arguments.push(current);
    }
    Ok(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_keep_quoted_spaces_together() {
        assert_eq!(
            split_arguments(r#"main.lua --name "Ana Maria" ""  -v"#).unwrap(),
            ["main.lua", "--name", "Ana Maria", "", "-v"]
        );
        assert!(split_arguments(r#"--name "Ana"#).is_err());
        assert!(split_arguments("   ").unwrap().is_empty());
    }

    #[test]
    fn terminal_features_are_refused() {
        for (text, feature) in [
            ("make && ./app", "&&"),
            ("cat log | grep x", "|"),
            ("app > out.txt", ">"),
            ("echo $(date)", "$("),
            ("serve --dir $HOME", "$VARIÁVEL"),
            ("serve --dir %USERPROFILE%", "%VARIÁVEL%"),
            ("PORT=3000 node app.js", "VARIÁVEL=valor"),
            ("a; b", ";"),
        ] {
            assert_eq!(terminal_feature(text), Some(feature), "{text}");
        }
        for text in [
            "lua main.lua",
            "Rscript -e shiny::runApp()",
            "julia --project=. src/app.jl",
            "perl app.pl --port=3000",
            "price $5",
        ] {
            assert_eq!(terminal_feature(text), None, "{text}");
        }
    }

    #[test]
    fn a_defined_command_becomes_a_process_of_its_project() {
        let project = Project {
            name: "jogo".into(),
            path: PathBuf::from("C:/dev/jogo"),
            runtimes: vec![],
            technologies: vec![],
            modified_at: None,
        };
        let command = CustomCommand {
            project: project.path.clone(),
            name: "Servidor".into(),
            program: "lua".into(),
            args: vec!["server.lua".into(), "--title".into(), "Meu jogo".into()],
            folder: "src".into(),
            port: Some(8080),
        };
        let other = CustomCommand {
            project: PathBuf::from("C:/dev/outro"),
            ..command.clone()
        };
        let processes = processes_for(&project, &[command.clone(), other]);
        assert_eq!(processes.len(), 1);
        let process = &processes[0];
        assert!(is_custom(process));
        assert_eq!(
            process.working_directory,
            PathBuf::from("C:/dev/jogo").join("src")
        );
        assert_eq!(process.expected_port, Port::new(8080));
        assert_eq!(process.command_line(), "lua server.lua --title Meu jogo");
        assert_eq!(
            command.command_line(),
            "lua server.lua --title \"Meu jogo\""
        );
    }
}
