//! "Definir comando de início": the person tells how a project starts when the app finds no
//! command. The command runs without a shell, in a folder of the project, and becomes a
//! process like the detected ones.

use super::*;
use localcodepilot_core::containers::path_is_within;
use localcodepilot_core::custom_commands::{split_arguments, terminal_feature};

/// What the dialog holds while the person types.
pub(super) struct CommandDraft {
    project: PathBuf,
    project_name: String,
    /// Name of the command being edited; `None` for a new one.
    original: Option<String>,
    name: String,
    program: String,
    args: String,
    folder: String,
    port: String,
    error: Option<String>,
}

/// Checks a draft and turns it into a command, or says what to fix in one sentence.
fn validate_draft(
    draft: &CommandDraft,
    existing: &[CustomCommand],
    environment_path: Option<&OsStr>,
) -> Result<CustomCommand, String> {
    let name = draft.name.trim();
    if name.is_empty() {
        return Err("Dê um nome ao comando, como Servidor ou Worker.".into());
    }
    if existing.iter().any(|command| {
        command.belongs_to(&draft.project)
            && command.name.eq_ignore_ascii_case(name)
            && draft.original.as_deref() != Some(command.name.as_str())
    }) {
        return Err(format!("Este projeto já tem um comando chamado {name}."));
    }
    let program = draft.program.trim();
    if program.is_empty() {
        return Err("Informe o programa que inicia o projeto.".into());
    }
    for text in [program, draft.args.as_str()] {
        if let Some(feature) = terminal_feature(text) {
            return Err(format!(
                "Este comando usa um recurso do terminal ({feature}). Coloque-o num script do projeto e informe o script como programa."
            ));
        }
    }
    let folder = draft.folder.trim().trim_matches(['/', '\\']).to_owned();
    let working_directory = if folder.is_empty() {
        draft.project.clone()
    } else {
        draft.project.join(&folder)
    };
    if !working_directory.is_dir() || !path_is_within(&working_directory, &draft.project) {
        return Err("A pasta de execução não existe dentro do projeto.".into());
    }
    // A program inside the project is saved with its full path: it runs from the folder of
    // the project, not from the folder of the app.
    let inside = working_directory.join(program);
    let program = if Path::new(program).is_absolute() {
        // A full path to an installed program outside `PATH`, such as Git's own perl.exe.
        if !Path::new(program).is_file() {
            return Err(format!("O programa {program} não existe."));
        }
        program.to_owned()
    } else if Path::new(program).components().count() > 1 || program.starts_with('.') {
        if !inside.is_file() || !path_is_within(&inside, &draft.project) {
            return Err(format!(
                "O programa {program} não existe na pasta do projeto."
            ));
        }
        inside.to_string_lossy().into_owned()
    } else if find_executable(program, environment_path).is_some() {
        program.to_owned()
    } else if inside.is_file() {
        inside.to_string_lossy().into_owned()
    } else {
        return Err(format!(
            "O programa {program} não foi encontrado no PATH nem na pasta do projeto."
        ));
    };
    let args = split_arguments(&draft.args)?;
    let port = match draft.port.trim() {
        "" => None,
        text => Some(
            text.parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or("A porta precisa ser um número entre 1 e 65535.")?,
        ),
    };
    Ok(CustomCommand {
        project: draft.project.clone(),
        name: name.to_owned(),
        program,
        args,
        folder,
        port,
    })
}

impl LocalCodePilot {
    pub(super) fn custom_commands_of(&self, project: &Path) -> Vec<CustomCommand> {
        self.workspace_state
            .custom_commands
            .iter()
            .filter(|command| command.belongs_to(project))
            .cloned()
            .collect()
    }

    /// Opens the dialog for a new command, or to edit the one called `name`.
    pub(super) fn open_command_dialog(&mut self, project: &Path, name: Option<&str>) {
        let project_name = self
            .projects
            .iter()
            .find(|candidate| candidate.path == project)
            .map(|candidate| candidate.name.clone())
            .unwrap_or_default();
        let existing = name.and_then(|name| {
            self.workspace_state
                .custom_commands
                .iter()
                .find(|command| command.belongs_to(project) && command.name == name)
                .cloned()
        });
        self.command_draft = Some(match existing {
            Some(command) => CommandDraft {
                project: project.to_path_buf(),
                project_name,
                original: Some(command.name.clone()),
                name: command.name.clone(),
                program: command.program.clone(),
                args: command
                    .args
                    .iter()
                    .map(|argument| {
                        if argument.is_empty() || argument.contains(char::is_whitespace) {
                            format!("\"{argument}\"")
                        } else {
                            argument.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                folder: command.folder.clone(),
                port: command
                    .port
                    .map(|port| port.to_string())
                    .unwrap_or_default(),
                error: None,
            },
            None => CommandDraft {
                project: project.to_path_buf(),
                project_name,
                original: None,
                name: if self.custom_commands_of(project).is_empty() {
                    "Servidor".into()
                } else {
                    String::new()
                },
                program: String::new(),
                args: String::new(),
                folder: String::new(),
                port: String::new(),
                error: None,
            },
        });
    }

    fn command_running(&self, id: &str) -> bool {
        self.processes.iter().any(|process| {
            process.id == id
                && matches!(
                    process.state,
                    ProcessState::Running | ProcessState::Starting
                )
        })
    }

    /// Puts the defined commands of `project` back among its processes and recomputes what
    /// the card shows.
    fn refresh_custom_processes(&mut self, project_path: &Path) {
        let Some(project) = self
            .projects
            .iter()
            .find(|project| project.path == project_path)
            .cloned()
        else {
            return;
        };
        self.processes.retain(|process| {
            !(process.project_path == project_path && custom_commands::is_custom(process))
        });
        self.processes.extend(custom_commands::processes_for(
            &project,
            &self.workspace_state.custom_commands,
        ));
        let environment = inspect_project_environment(
            &project,
            &self.processes,
            self.environment_path.as_deref(),
        );
        self.project_environments
            .insert(project_path.to_path_buf(), environment);
    }

    fn save_command_draft(&mut self) -> Result<(), String> {
        let Some(draft) = &self.command_draft else {
            return Ok(());
        };
        let command = validate_draft(
            draft,
            &self.workspace_state.custom_commands,
            self.environment_path.as_deref(),
        )?;
        if let Some(original) = &draft.original {
            let old = CustomCommand {
                project: draft.project.clone(),
                name: original.clone(),
                ..CustomCommand::default()
            };
            if self.command_running(&old.process_id()) {
                return Err("Pare este comando antes de alterá-lo.".into());
            }
        }
        let project = draft.project.clone();
        let original = draft.original.clone();
        self.workspace_state.custom_commands.retain(|existing| {
            !(existing.belongs_to(&project)
                && original
                    .as_deref()
                    .is_some_and(|name| existing.name == name))
        });
        self.workspace_state.custom_commands.push(command);
        self.workspace_dirty = true;
        self.persist_workspace_state(true);
        self.refresh_custom_processes(&project);
        self.command_draft = None;
        self.notify("Comando de início salvo. Use Iniciar para rodar o projeto.");
        Ok(())
    }

    pub(super) fn remove_custom_command(&mut self, project: &Path, name: &str) {
        let target = CustomCommand {
            project: project.to_path_buf(),
            name: name.to_owned(),
            ..CustomCommand::default()
        };
        if self.command_running(&target.process_id()) {
            self.notify("Pare este comando antes de removê-lo.");
            return;
        }
        self.workspace_state
            .custom_commands
            .retain(|command| !(command.belongs_to(project) && command.name == name));
        self.workspace_dirty = true;
        self.persist_workspace_state(true);
        self.refresh_custom_processes(project);
        self.notify(format!("Comando {name} removido."));
    }

    pub(super) fn command_dialog(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.command_draft else {
            return;
        };
        let check = validate_draft(
            draft,
            &self.workspace_state.custom_commands,
            self.environment_path.as_deref(),
        );
        let mut save = false;
        let mut cancel = false;
        let title = if draft.original.is_some() {
            "Editar comando de início"
        } else {
            "Definir comando de início"
        };
        egui::Window::new(title)
            .id(egui::Id::unique("custom-command-dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 48.0).clamp(260.0, 560.0));
                ui.label(RichText::new(&draft.project_name).strong().size(16.0));
                ui.label("Este comando será executado na pasta do projeto, sem terminal.");
                ui.add_space(8.0);
                let field = |ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str| {
                    ui.label(RichText::new(label).strong().size(12.0));
                    let response = ui.add(
                        egui::TextEdit::singleline(value)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY),
                    );
                    announced(response, egui::Role::TextInput, label);
                };
                field(ui, "Nome", &mut draft.name, "Servidor");
                field(
                    ui,
                    "Programa",
                    &mut draft.program,
                    "lua, Rscript, julia ou scripts/iniciar.bat",
                );
                field(ui, "Argumentos", &mut draft.args, "main.lua --porta 8080");
                field(
                    ui,
                    "Pasta de execução",
                    &mut draft.folder,
                    "pasta do projeto",
                );
                field(ui, "Porta (opcional)", &mut draft.port, "8080");
                ui.add_space(6.0);
                match &check {
                    Ok(command) => {
                        ui.label(RichText::new("Será executado:").size(12.0));
                        ui.label(RichText::new(command.command_line()).monospace());
                        notice::detail_line(
                            ui,
                            &format!("Pasta: {}", display_path(&command.working_directory())),
                        );
                    }
                    Err(problem) => {
                        ui.label(RichText::new(problem).color(theme::WARNING));
                    }
                }
                if let Some(error) = &draft.error {
                    ui.label(RichText::new(error).color(theme::DANGER));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add_enabled(
                            check.is_ok(),
                            egui::Button::new(
                                RichText::new("Salvar comando")
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(theme::PRIMARY),
                        )
                        .clicked()
                    {
                        save = true;
                    }
                });
            });
        if cancel {
            self.command_draft = None;
        } else if save
            && let Err(error) = self.save_command_draft()
            && let Some(draft) = &mut self.command_draft
        {
            draft.error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn draft(project: &Path) -> CommandDraft {
        CommandDraft {
            project: project.to_path_buf(),
            project_name: "jogo".into(),
            original: None,
            name: "Servidor".into(),
            program: String::new(),
            args: String::new(),
            folder: String::new(),
            port: String::new(),
            error: None,
        }
    }

    #[test]
    fn drafts_are_checked_before_saving() {
        let project = std::env::temp_dir().join(format!(
            "lcp-custom-command-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(project.join("scripts")).unwrap();
        fs::write(project.join("scripts/iniciar.bat"), "@echo off\n").unwrap();
        let path = std::env::var_os("PATH");
        let check = |draft: &CommandDraft| validate_draft(draft, &[], path.as_deref());

        let mut empty = draft(&project);
        assert!(check(&empty).unwrap_err().contains("programa"));
        empty.program = "programa-que-nao-existe-lcp".into();
        assert!(check(&empty).unwrap_err().contains("não foi encontrado"));

        let mut shell = draft(&project);
        shell.program = "scripts/iniciar.bat".into();
        shell.args = "--a && --b".into();
        assert!(
            check(&shell)
                .unwrap_err()
                .contains("recurso do terminal (&&)")
        );

        let mut script = draft(&project);
        script.program = "scripts/iniciar.bat".into();
        script.args = r#"--titulo "Meu jogo""#.into();
        script.port = "8080".into();
        let command = check(&script).unwrap();
        assert!(Path::new(&command.program).is_absolute());
        assert_eq!(command.args, ["--titulo", "Meu jogo"]);
        assert_eq!(command.port, Some(8080));

        let mut folder = script.clone_for_test();
        folder.folder = "../fora".into();
        assert!(check(&folder).unwrap_err().contains("pasta"));
        let mut port = script.clone_for_test();
        port.port = "99999".into();
        assert!(check(&port).unwrap_err().contains("porta"));

        // A second command with the same name in the same project is refused.
        assert!(
            validate_draft(&script, &[command], path.as_deref())
                .unwrap_err()
                .contains("já tem")
        );
        let _ = fs::remove_dir_all(project);
    }

    impl CommandDraft {
        fn clone_for_test(&self) -> Self {
            Self {
                project: self.project.clone(),
                project_name: self.project_name.clone(),
                original: self.original.clone(),
                name: self.name.clone(),
                program: self.program.clone(),
                args: self.args.clone(),
                folder: self.folder.clone(),
                port: self.port.clone(),
                error: None,
            }
        }
    }
}
