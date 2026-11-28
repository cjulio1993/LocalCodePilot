//! Running a Laravel project's pending migrations from "Iniciar", after the person confirms
//! (T027). Offered only when the destination is proven: the SQLite file the app prepared inside
//! the project, or a database on this computer whose login the app tested. Never for another
//! computer or a production `.env`, and never a command that rebuilds or wipes data.

use super::*;
use localcodepilot_runtime::database_connection::loopback_address;
use localcodepilot_runtime::databases::DatabaseRequirement;

const CONFIRM_FOCUS: &str = "migration-confirmation";
const STATUS_TIMEOUT: Duration = Duration::from_secs(90);
const MIGRATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_LOG_LINES: usize = 2000;

pub(super) const TITLE: &str = "Rodar as migrations deste projeto?";
pub(super) const EXPLANATION: &str = "O banco está pronto, mas há migrations pendentes. O LocalCodePilot vai aplicá-las antes de iniciar o projeto. Elas podem alterar a estrutura e os dados do banco.";
pub(super) const RUNNING: &str = "Rodando as migrations do projeto...";
pub(super) const SUCCEEDED: &str = "Migrations concluídas. Iniciando o projeto...";
pub(super) const FAILED: &str =
    "Não foi possível concluir as migrations. O projeto não foi iniciado.";

/// The only commands this module runs. Nothing that rebuilds, rolls back, wipes or seeds.
pub(super) const STATUS_ARGS: [&str; 4] =
    ["artisan", "migrate:status", "--no-ansi", "--no-interaction"];
pub(super) const MIGRATE_ARGS: [&str; 5] = [
    "artisan",
    "migrate",
    "--force",
    "--no-interaction",
    "--no-ansi",
];

/// Where the migrations would run, proven before anything is offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MigrationTarget {
    /// For "Detalhes técnicos": the engine and where the data lives.
    pub destination: String,
}

/// Whether `project` gets the offer, and where the migrations would run.
pub(super) fn laravel_target(
    project: &Path,
    requirements: &[DatabaseRequirement],
) -> Option<MigrationTarget> {
    if !project.join("artisan").is_file()
        || !has_migration_files(&project.join("database").join("migrations"))
    {
        return None;
    }
    let env = fs::read_to_string(project.join(".env")).ok()?;
    if env_value(&env, "APP_ENV").is_some_and(|value| value.eq_ignore_ascii_case("production")) {
        return None;
    }
    let requirement = requirements.iter().find(|requirement| {
        requirement
            .source
            .file_name()
            .is_some_and(|name| name == ".env")
    })?;
    target_for(requirement, &env)
}

fn target_for(requirement: &DatabaseRequirement, env: &str) -> Option<MigrationTarget> {
    if requirement.engine == DatabaseEngine::Sqlite {
        // start_project prepared it and refuses paths outside the project.
        let file =
            env_value(env, "DB_DATABASE").unwrap_or_else(|| "database/database.sqlite".into());
        return Some(MigrationTarget {
            destination: format!("SQLite ({file})"),
        });
    }
    if requirement.compose_service.is_some() {
        return None;
    }
    let host = requirement.host.as_deref()?;
    loopback_address(host)?;
    let database = env_value(env, "DB_DATABASE").unwrap_or_else(|| "?".into());
    Some(MigrationTarget {
        destination: format!(
            "{} em {host}:{}, banco {database}",
            requirement.engine.label(),
            requirement
                .port
                .map_or_else(|| "?".into(), |port| port.to_string())
        ),
    })
}

fn has_migration_files(folder: &Path) -> bool {
    fs::read_dir(folder).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "php")
        })
    })
}

fn env_value(env: &str, key: &str) -> Option<String> {
    env.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        (name.trim() == key).then(|| value.trim().trim_matches(['"', '\'']).to_owned())
    })
}

/// The pending migrations in the output of `php artisan migrate:status`, in both the list
/// (Laravel 10 and later) and the table (Laravel 8 and 9) formats.
pub(super) fn parse_pending(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter_map(|line| {
            if let Some(cells) = line.strip_prefix('|') {
                // | No   | 2019_08_19_000000_create_failed_jobs_table |       |
                let cells = cells.split('|').map(str::trim).collect::<Vec<_>>();
                return (cells.first() == Some(&"No"))
                    .then(|| cells.get(1).map(|name| (*name).to_owned()))?;
            }
            // 2024_01_01_000000_create_posts_table ............ Pending
            let name = line.split_whitespace().next()?;
            (line.ends_with("Pending") && !name.starts_with('.')).then(|| name.to_owned())
        })
        .collect()
}

/// A database without the migrations table has every migration pending.
pub(super) fn migration_table_missing(output: &str) -> bool {
    output.to_lowercase().contains("migration table not found")
}

fn migration_names(project: &Path) -> Vec<String> {
    let mut names = fs::read_dir(project.join("database").join("migrations"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let path = entry.path();
                    if path.extension()? == "php" {
                        Some(path.file_stem()?.to_string_lossy().into_owned())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    names.sort();
    names
}

enum MigrationEvent {
    Checked(Result<Vec<String>, String>),
    Log(String),
    Finished(Result<(), String>),
}

pub(super) struct MigrationJob {
    running: bool,
    receiver: Receiver<MigrationEvent>,
}

pub(super) struct MigrationRequest {
    project: PathBuf,
    name: String,
    target: MigrationTarget,
    pending: Vec<String>,
}

#[derive(Default)]
pub(super) struct MigrationState {
    jobs: HashMap<PathBuf, MigrationJob>,
    pub confirmation: Option<MigrationRequest>,
    /// Checked or migrated for the start in progress: the next pass starts the project.
    cleared: HashSet<PathBuf>,
    /// The last run failed: the card shows it until the next "Iniciar".
    pub failed: HashSet<PathBuf>,
    pub logs: HashMap<PathBuf, VecDeque<String>>,
    pub log_window: Option<PathBuf>,
}

impl MigrationState {
    pub fn step(&self, project: &Path) -> Option<String> {
        self.jobs.get(project).map(|job| {
            if job.running {
                RUNNING.to_owned()
            } else {
                "Iniciando os serviços".to_owned()
            }
        })
    }
}

/// What `start_project` does next.
pub(super) enum MigrationGate {
    Proceed,
    Waiting,
}

fn run_artisan(
    project: &Path,
    args: &[&str],
    path: Option<&OsStr>,
    timeout: Duration,
    on_line: &mut dyn FnMut(String),
) -> Result<String, String> {
    let php = find_executable("php", path).ok_or("o programa php não foi encontrado")?;
    let mut command = Command::new(php);
    command
        .args(args)
        .current_dir(plain_path(project))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    localcodepilot_platform::installations::configure_background_command(&mut command);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let (sender, lines) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        spawn_output_reader(stdout, sender.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_output_reader(stderr, sender);
    }
    let started = Instant::now();
    let mut output = String::new();
    loop {
        while let Ok(line) = lines.try_recv() {
            output.push_str(&line);
            output.push('\n');
            on_line(line);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // Late lines after the exit.
                std::thread::sleep(Duration::from_millis(100));
                while let Ok(line) = lines.try_recv() {
                    output.push_str(&line);
                    output.push('\n');
                    on_line(line);
                }
                return if status.success() {
                    Ok(output)
                } else {
                    Err(output)
                };
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Ok(None) => {
                let _ = terminate_process_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("o comando não terminou em {} s", timeout.as_secs()));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn escape_pressed(ui: &egui::Ui) -> bool {
    ui.input(|input| input.key_pressed(egui::Key::Escape))
}

fn focus_once(ui: &egui::Ui, button: &egui::Response) {
    let id = egui::Id::unique(("first-focus", CONFIRM_FOCUS));
    if !ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id))
        .unwrap_or(false)
    {
        button.request_focus();
        ui.ctx().data_mut(|data| data.insert_temp(id, true));
    }
}

fn forget_focus(ui: &egui::Ui) {
    ui.ctx()
        .data_mut(|data| data.remove::<bool>(egui::Id::unique(("first-focus", CONFIRM_FOCUS))));
}

impl LocalCodePilot {
    /// Before the processes start: a Laravel project with a proven destination is checked for
    /// pending migrations, and the person confirms before they run.
    pub(super) fn migration_gate(&mut self, project: &Path) -> MigrationGate {
        if self.migrations.cleared.remove(project) {
            return MigrationGate::Proceed;
        }
        if self.migrations.jobs.contains_key(project) {
            return MigrationGate::Waiting;
        }
        let laravel = self.projects.iter().any(|candidate| {
            candidate.path == project && candidate.technologies.contains(&TechnologyKind::Laravel)
        });
        if !laravel {
            return MigrationGate::Proceed;
        }
        let requirements = detect_database_requirements(project);
        let Some(target) = laravel_target(project, &requirements) else {
            return MigrationGate::Proceed;
        };
        self.migrations.failed.remove(project);
        let (sender, receiver) = mpsc::channel();
        let path = self.environment_path.clone();
        let directory = project.to_path_buf();
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            let result = match run_artisan(
                &directory,
                &STATUS_ARGS,
                path.as_deref(),
                STATUS_TIMEOUT,
                &mut |_| {},
            ) {
                Ok(output) => Ok(parse_pending(&output)),
                Err(output) if migration_table_missing(&output) => Ok(migration_names(&directory)),
                Err(output) => Err(output),
            };
            let _ = sender.send(MigrationEvent::Checked(result));
            repaint.request_repaint();
        });
        self.migrations.jobs.insert(
            project.to_path_buf(),
            MigrationJob {
                running: false,
                receiver,
            },
        );
        self.migration_targets.insert(project.to_path_buf(), target);
        MigrationGate::Waiting
    }

    pub(super) fn poll_migrations(&mut self) {
        let projects = self.migrations.jobs.keys().cloned().collect::<Vec<_>>();
        for project in projects {
            while let Some(job) = self.migrations.jobs.get(&project) {
                let event = match job.receiver.try_recv() {
                    Ok(event) => event,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => MigrationEvent::Finished(Err(
                        "a execução das migrations foi interrompida".into(),
                    )),
                };
                match event {
                    MigrationEvent::Log(line) => {
                        let logs = self.migrations.logs.entry(project.clone()).or_default();
                        logs.push_back(crate::secrets::mask_secrets(&line));
                        while logs.len() > MAX_LOG_LINES {
                            logs.pop_front();
                        }
                    }
                    MigrationEvent::Checked(result) => {
                        self.migrations.jobs.remove(&project);
                        match result {
                            Ok(pending) if !pending.is_empty() => {
                                let name = self
                                    .projects
                                    .iter()
                                    .find(|candidate| candidate.path == project)
                                    .map(|candidate| candidate.name.clone())
                                    .unwrap_or_default();
                                if let Some(target) = self.migration_targets.get(&project).cloned()
                                {
                                    self.migrations.confirmation = Some(MigrationRequest {
                                        project: project.clone(),
                                        name,
                                        target,
                                        pending,
                                    });
                                }
                            }
                            // Nothing pending, or the check itself failed: the start goes on as
                            // before, and a failed check is kept for "Ver logs".
                            other => {
                                if let Err(output) = other {
                                    let logs =
                                        self.migrations.logs.entry(project.clone()).or_default();
                                    logs.clear();
                                    logs.extend(output.lines().map(crate::secrets::mask_secrets));
                                }
                                self.migrations.cleared.insert(project.clone());
                                self.start_project(&project);
                            }
                        }
                        break;
                    }
                    MigrationEvent::Finished(result) => {
                        self.migrations.jobs.remove(&project);
                        match result {
                            Ok(()) => {
                                self.notify(SUCCEEDED);
                                self.migrations.cleared.insert(project.clone());
                                self.start_project(&project);
                            }
                            Err(error) => {
                                let logs = self.migrations.logs.entry(project.clone()).or_default();
                                logs.push_back(crate::secrets::mask_secrets(&error));
                                self.migrations.failed.insert(project.clone());
                                self.notify(FAILED);
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    fn run_migrations(&mut self, project: &Path, ctx: &egui::Context) {
        self.notify(RUNNING);
        let (sender, receiver) = mpsc::channel();
        let path = self.environment_path.clone();
        let directory = project.to_path_buf();
        let repaint = ctx.clone();
        self.migrations.logs.insert(
            project.to_path_buf(),
            VecDeque::from([format!("$ php {}", MIGRATE_ARGS.join(" "))]),
        );
        std::thread::spawn(move || {
            let log = sender.clone();
            let log_repaint = repaint.clone();
            let result = run_artisan(
                &directory,
                &MIGRATE_ARGS,
                path.as_deref(),
                MIGRATE_TIMEOUT,
                &mut |line| {
                    let _ = log.send(MigrationEvent::Log(line));
                    log_repaint.request_repaint();
                },
            )
            .map(|_| ())
            .map_err(|output| {
                output
                    .lines()
                    .last()
                    .unwrap_or("as migrations falharam")
                    .to_owned()
            });
            let _ = sender.send(MigrationEvent::Finished(result));
            repaint.request_repaint();
        });
        self.migrations.jobs.insert(
            project.to_path_buf(),
            MigrationJob {
                running: true,
                receiver,
            },
        );
    }

    pub(super) fn migration_confirmation(&mut self, root_ui: &mut egui::Ui) {
        let Some(request) = self.migrations.confirmation.take() else {
            return;
        };
        let mut open = true;
        let mut cancel = false;
        let mut confirm = false;
        let shown = egui::Window::new(TITLE)
            .id(egui::Id::unique(CONFIRM_FOCUS))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(520.0)
            .show(root_ui, |ui| {
                ui.label(RichText::new(&request.name).strong());
                ui.add_space(4.0);
                ui.label(EXPLANATION);
                ui.add_space(4.0);
                // The destination is part of the decision, not a technical detail (CA-2).
                ui.label(RichText::new(format!("Banco: {}", request.target.destination)).strong());
                egui::CollapsingHeader::new("Detalhes técnicos")
                    .id_salt("migration-details")
                    .show(ui, |ui| {
                        for line in [
                            format!("Comando: php {}", MIGRATE_ARGS.join(" ")),
                            format!("Pasta: {}", display_path(&request.project)),
                            format!("Destino: {}", request.target.destination),
                            format!("Migrations pendentes ({}):", request.pending.len()),
                        ] {
                            ui.label(RichText::new(line).color(theme::MUTED).size(12.0));
                        }
                        for name in request.pending.iter().take(30) {
                            ui.label(RichText::new(format!("  {name}")).monospace().size(11.0));
                        }
                        if request.pending.len() > 30 {
                            ui.label(
                                RichText::new(format!("  … e mais {}", request.pending.len() - 30))
                                    .size(11.0),
                            );
                        }
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let not_now = ui.button("Agora não");
                    focus_once(ui, &not_now);
                    if not_now.clicked() {
                        cancel = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Confirmar e iniciar")
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(theme::PRIMARY_FILL),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if escape_pressed(root_ui) {
            cancel = true;
        }
        // Modal only while it stays open: on the closing frame the focus may go back behind it.
        if !(confirm || cancel || !open) {
            super::notice::keep_focus_inside(
                root_ui.ctx(),
                shown.as_ref().map(|shown| &shown.response),
            );
        }
        if confirm || cancel || !open {
            forget_focus(root_ui);
        }
        if confirm {
            self.run_migrations(&request.project, root_ui.ctx());
        } else if !cancel && open {
            self.migrations.confirmation = Some(request);
        }
    }

    /// "Ver logs" of a failed run: the command's output, with secrets masked.
    pub(super) fn migration_log_window(&mut self, ctx: &egui::Context) {
        let Some(project) = self.migrations.log_window.clone() else {
            return;
        };
        let name = self
            .projects
            .iter()
            .find(|candidate| candidate.path == project)
            .map_or_else(String::new, |candidate| candidate.name.clone());
        let mut open = true;
        egui::Window::new(format!("Logs das migrations · {name}"))
            .id(egui::Id::unique("migration-logs"))
            .open(&mut open)
            .default_width(640.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in self.migrations.logs.get(&project).into_iter().flatten() {
                            ui.label(RichText::new(line).monospace().size(12.0));
                        }
                    });
            });
        if !open {
            self.migrations.log_window = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LARAVEL_11: &str = "
  Migration name .................................................................... Batch / Status
  0001_01_01_000000_create_users_table ..................................................... [1] Ran
  0001_01_01_000001_create_cache_table ..................................................... [1] Ran
  2026_11_20_120000_create_posts_table ..................................................... Pending
  2026_11_21_090000_add_slug_to_posts .................................................... Pending
";

    const LARAVEL_9: &str = "
+------+------------------------------------------------+-------+
| Ran? | Migration                                      | Batch |
+------+------------------------------------------------+-------+
| Yes  | 2014_10_12_000000_create_users_table           | 1     |
| No   | 2019_08_19_000000_create_failed_jobs_table     |       |
+------+------------------------------------------------+-------+
";

    #[test]
    fn reads_pending_migrations_in_both_laravel_formats() {
        assert_eq!(
            parse_pending(LARAVEL_11),
            vec![
                "2026_11_20_120000_create_posts_table",
                "2026_11_21_090000_add_slug_to_posts"
            ]
        );
        assert_eq!(
            parse_pending(LARAVEL_9),
            vec!["2019_08_19_000000_create_failed_jobs_table"]
        );
        assert!(parse_pending("  Nothing to migrate.").is_empty());
        assert!(migration_table_missing(
            "\n   ERROR  Migration table not found.\n"
        ));
    }

    #[test]
    fn never_runs_a_command_that_rebuilds_wipes_or_seeds() {
        for args in [STATUS_ARGS.as_slice(), MIGRATE_ARGS.as_slice()] {
            for forbidden in [
                "fresh", "reset", "rollback", "refresh", "wipe", "seed", "db:",
            ] {
                assert!(
                    !args.iter().any(|arg| arg.contains(forbidden)),
                    "{args:?} contains {forbidden}"
                );
            }
        }
        assert_eq!(MIGRATE_ARGS[1], "migrate");
    }

    fn laravel_project(name: &str, env: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("lcp-t027-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("database").join("migrations")).unwrap();
        fs::write(root.join("artisan"), "<?php").unwrap();
        fs::write(
            root.join("database/migrations/2026_11_20_120000_create_posts_table.php"),
            "<?php",
        )
        .unwrap();
        fs::write(root.join(".env"), env).unwrap();
        root
    }

    fn requirement(
        engine: DatabaseEngine,
        host: Option<&str>,
        compose: Option<&str>,
    ) -> DatabaseRequirement {
        DatabaseRequirement {
            source: PathBuf::from(".env"),
            engine,
            host: host.map(str::to_owned),
            port: Some(3306),
            compose_service: compose.map(str::to_owned),
        }
    }

    #[test]
    fn the_offer_needs_a_proven_destination() {
        let local = laravel_project(
            "local",
            "APP_ENV=local\nDB_CONNECTION=mysql\nDB_DATABASE=loja\nDB_PASSWORD=senha-ficticia-1\n",
        );
        let mysql = |host: &str| [requirement(DatabaseEngine::MySql, Some(host), None)];
        let target = laravel_target(&local, &mysql("127.0.0.1")).unwrap();
        assert!(target.destination.contains("banco loja"));
        assert!(
            !target.destination.contains("senha-ficticia-1"),
            "the password never shows"
        );
        assert!(laravel_target(&local, &mysql("localhost")).is_some());
        assert_eq!(
            laravel_target(&local, &mysql("db.empresa.exemplo")),
            None,
            "another computer"
        );
        assert_eq!(
            laravel_target(
                &local,
                &[requirement(DatabaseEngine::MySql, Some("db"), Some("db"))]
            ),
            None,
            "a Compose service"
        );
        assert_eq!(laravel_target(&local, &[]), None, "no known database");
        let sqlite =
            laravel_target(&local, &[requirement(DatabaseEngine::Sqlite, None, None)]).unwrap();
        assert!(sqlite.destination.starts_with("SQLite"));

        let production = laravel_project("producao", "APP_ENV=production\nDB_CONNECTION=mysql\n");
        assert_eq!(
            laravel_target(&production, &mysql("127.0.0.1")),
            None,
            "production"
        );

        let no_migrations = laravel_project("sem-migrations", "APP_ENV=local\n");
        fs::remove_file(
            no_migrations.join("database/migrations/2026_11_20_120000_create_posts_table.php"),
        )
        .unwrap();
        assert_eq!(laravel_target(&no_migrations, &mysql("127.0.0.1")), None);

        assert_eq!(
            migration_names(&local),
            vec!["2026_11_20_120000_create_posts_table"]
        );
        for root in [local, production, no_migrations] {
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn texts_follow_the_approved_wording() {
        assert_eq!(TITLE, "Rodar as migrations deste projeto?");
        assert_eq!(RUNNING, "Rodando as migrations do projeto...");
        assert_eq!(SUCCEEDED, "Migrations concluídas. Iniciando o projeto...");
        assert_eq!(
            FAILED,
            "Não foi possível concluir as migrations. O projeto não foi iniciado."
        );
    }
}
