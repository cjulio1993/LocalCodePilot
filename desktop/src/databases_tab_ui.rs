//! The databases of Ambiente (T037.06, T038.03): the databases of every project, one row per server.
//! A server several projects use shows once, with those projects. The actions follow the
//! rules of the project card: "Parar" only for a server the app created, when no project that
//! uses it is running; "Testar conexão" for a server on another computer; nothing that changes
//! a server the app did not create (already running on the port, or a Compose service).

use super::*;
use localcodepilot_runtime::databases::DatabaseEngine;

/// "Parar" asked in the databases of Ambiente, waiting for the confirmation: the project
/// used to run the job, the server, its engine and the projects that use it.
#[derive(Debug, Clone)]
pub(super) struct DatabaseStopRequest {
    project: PathBuf,
    server: database_ui::ManagedInfo,
    engine: &'static str,
    projects: Vec<String>,
}

/// One server, or one SQLite file, and the projects that use it.
struct DatabaseEntry {
    engine: DatabaseEngine,
    /// "Este computador", "Outro computador" or "Compose: db".
    location: String,
    port: Option<u16>,
    status: DatabaseStatus,
    projects: Vec<(PathBuf, String)>,
}

/// The state of a row, in the vocabulary of the cards.
fn entry_state(status: &DatabaseStatus) -> (&'static str, Color32) {
    if status.rejection().is_some() {
        ("Com problema", theme::DANGER)
    } else if status.not_running() {
        ("Parado", theme::MUTED)
    } else if matches!(
        status.connection.as_ref().map(|test| &test.outcome),
        Some(ConnectionOutcome::Connected)
    ) || status.available == Some(true)
    {
        ("Rodando", theme::SUCCESS)
    } else {
        ("Não verificado", theme::WARNING)
    }
}

/// The group of a row in Ambiente (T039.03, CA-8): a refused or failed connection first,
/// then the ones not verified yet (not a failure), then the rest.
fn entry_group(state: &str) -> usize {
    match state {
        "Com problema" => 0,
        "Não verificado" => 1,
        _ => 2,
    }
}

const GROUPS: [(&str, &str); 3] = [
    (
        "Conexões com problema",
        "O banco respondeu, mas recusou o acesso ou o banco de dados não existe.",
    ),
    (
        "Conexões não verificadas",
        "Ainda sem teste de conexão: isso não quer dizer que há uma falha.",
    ),
    (
        "Outros bancos",
        "Rodando ou parados, sem problema encontrado.",
    ),
];

impl LocalCodePilot {
    /// How many database rows have a problem and how many are not verified, for the summary
    /// at the top of Ambiente.
    pub(super) fn database_counts(&self) -> (usize, usize, usize) {
        let entries = self.database_entries();
        let in_group = |group: usize| {
            entries
                .iter()
                .filter(|entry| entry_group(entry_state(&entry.status).0) == group)
                .count()
        };
        (in_group(0), in_group(1), entries.len())
    }

    /// The databases of all projects, grouped by server.
    fn database_entries(&self) -> Vec<DatabaseEntry> {
        let mut entries: Vec<DatabaseEntry> = Vec::new();
        for project in &self.projects {
            let Some(environment) = self.project_environments.get(&project.path) else {
                continue;
            };
            for status in &environment.databases {
                let requirement = &status.requirement;
                let location = if let Some(service) = &requirement.compose_service {
                    format!("Compose: {service}")
                } else if requirement.engine == DatabaseEngine::Sqlite {
                    "Arquivo do projeto".to_owned()
                } else if requirement.host.as_deref().is_none_or(|host| {
                    localcodepilot_runtime::database_connection::loopback_address(host).is_some()
                }) {
                    "Este computador".to_owned()
                } else {
                    "Outro computador".to_owned()
                };
                // The same server (engine, place, port) shows once; a SQLite file or a Compose
                // service belongs to its project.
                let shared = requirement.engine != DatabaseEngine::Sqlite
                    && requirement.compose_service.is_none();
                let existing = entries.iter_mut().find(|entry| {
                    shared
                        && entry.engine == requirement.engine
                        && entry.location == location
                        && entry.port == requirement.port
                        && entry.status.requirement.host == requirement.host
                });
                match existing {
                    Some(entry) => {
                        if !entry.projects.iter().any(|(path, _)| *path == project.path) {
                            entry
                                .projects
                                .push((project.path.clone(), project.name.clone()));
                        }
                    }
                    None => entries.push(DatabaseEntry {
                        engine: requirement.engine,
                        location,
                        port: requirement.port,
                        status: status.clone(),
                        projects: vec![(project.path.clone(), project.name.clone())],
                    }),
                }
            }
        }
        entries
    }

    pub(super) fn databases_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Bancos de dados");
        ui.label(
            RichText::new(
                "Os bancos usados pelos seus projetos. Um servidor usado por vários projetos aparece uma vez.",
            )
            .color(theme::MUTED),
        );
        ui.add_space(12.0);
        let mut entries = self.database_entries();
        // Problems first, then not verified, then the rest; the order inside a group stays.
        entries.sort_by_key(|entry| entry_group(entry_state(&entry.status).0));
        if entries.is_empty() {
            if self.discovery.is_some() {
                components::loading_state(
                    ui,
                    "Procurando projetos...",
                    "Estamos examinando as pastas mais comuns da sua máquina.",
                );
            } else {
                components::empty_hint(ui, "Nenhum projeto deste computador usa banco de dados.");
            }
            return;
        }
        let mut stop = None;
        // Same-name projects show their folder in the confirmation (T039.01, CA-4).
        let folders = self.distinguishing_folders();
        let mut test = None;
        let mut open = None;
        let mut shown_group = None;
        for entry in &entries {
            let (state, color) = entry_state(&entry.status);
            let group = entry_group(state);
            if shown_group != Some(group) {
                shown_group = Some(group);
                let count = entries
                    .iter()
                    .filter(|other| entry_group(entry_state(&other.status).0) == group)
                    .count();
                let (title, explanation) = GROUPS[group];
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("{title} ({count})"))
                        .strong()
                        .size(theme::TEXT_BODY),
                );
                ui.label(
                    RichText::new(explanation)
                        .color(theme::MUTED)
                        .size(theme::TEXT_DETAIL),
                );
                ui.add_space(4.0);
            }
            // The actions come from this row's own database, never from another database of
            // the same project. "Parar": a server the app created, running, and no project
            // that uses it running. "Testar conexão": a database on another computer.
            let managed = entry
                .status
                .managed
                .clone()
                .filter(|managed| managed.running);
            let blocked_by = entry
                .projects
                .iter()
                .filter(|(path, _)| self.managed.project_is_running(path))
                .map(|(_, name)| name.clone())
                .collect::<Vec<_>>();
            let can_stop = managed
                .filter(|_| blocked_by.is_empty())
                .and_then(|managed| Some((entry.projects.first()?.0.clone(), managed)));
            let remote = database_ui::is_remote_database(&entry.status.requirement)
                .then(|| entry.projects.first().map(|(path, _)| path.clone()))
                .flatten();
            Frame::new()
                .fill(theme::BACKGROUND)
                .stroke(Stroke::new(1.0, theme::BORDER.gamma_multiply(0.6)))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let column = |width: f32| {
                            (
                                egui::vec2(width, 24.0),
                                Layout::left_to_right(Align::Center),
                            )
                        };
                        let (size, layout) = column(160.0);
                        ui.allocate_ui_with_layout(size, layout, |ui| {
                            ui.set_min_width(size.x);
                            ui.label(
                                RichText::new(entry.engine.label())
                                    .strong()
                                    .size(theme::TEXT_BODY),
                            );
                        });
                        let (size, layout) = column(130.0);
                        ui.allocate_ui_with_layout(size, layout, |ui| {
                            ui.set_min_width(size.x);
                            runtime_badge(ui, state, color);
                        });
                        let (size, layout) = column(56.0);
                        ui.allocate_ui_with_layout(size, layout, |ui| {
                            ui.set_min_width(size.x);
                            ui.label(
                                RichText::new(
                                    entry
                                        .port
                                        .map_or_else(|| "—".to_owned(), |port| port.to_string()),
                                )
                                .monospace()
                                .color(theme::MUTED)
                                .size(theme::TEXT_DETAIL),
                            )
                            .on_hover_text("Porta");
                        });
                        ui.label(
                            RichText::new(&entry.location)
                                .color(theme::MUTED)
                                .size(theme::TEXT_DETAIL),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if let Some((path, server)) = &can_stop
                                && ui.button("Parar").clicked()
                            {
                                stop = Some((
                                    path.clone(),
                                    server.clone(),
                                    entry.engine.label(),
                                    entry
                                        .projects
                                        .iter()
                                        .map(|(path, name)| match folders.get(path) {
                                            Some(folder) => format!("{name} ({folder})"),
                                            None => name.clone(),
                                        })
                                        .collect(),
                                ));
                            }
                            if let Some(path) = &remote
                                && ui.button("Testar conexão").clicked()
                            {
                                test = Some((path.clone(), entry.status.requirement.clone()));
                            }
                        });
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new(entry.status.card_state(None))
                                .color(theme::MUTED)
                                .size(theme::TEXT_DETAIL),
                        )
                        .wrap(),
                    );
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(if entry.projects.len() == 1 {
                                "Projeto:"
                            } else {
                                "Projetos:"
                            })
                            .color(theme::MUTED)
                            .size(theme::TEXT_DETAIL),
                        );
                        for (path, name) in &entry.projects {
                            if announced(
                                ui.link(RichText::new(name).size(theme::TEXT_DETAIL)),
                                egui::Role::Link,
                                &format!("Ver {name} em Projetos"),
                            )
                            .clicked()
                            {
                                open = Some((path.clone(), name.clone()));
                            }
                        }
                    });
                    if entry
                        .status
                        .managed
                        .as_ref()
                        .is_some_and(|managed| managed.running)
                        && !blocked_by.is_empty()
                    {
                        ui.label(
                            RichText::new(format!(
                                "Para parar o banco de dados, pare primeiro: {}",
                                blocked_by.join(", ")
                            ))
                            .color(theme::MUTED)
                            .size(theme::TEXT_DETAIL),
                        );
                    }
                });
            ui.add_space(4.0);
        }
        if let Some((project, server, engine, projects)) = stop {
            self.database_stop_request = Some(DatabaseStopRequest {
                project,
                server,
                engine,
                projects,
            });
        }
        // Nothing stops before the person confirms (T037.06, CA-9).
        if let Some(request) = self.database_stop_request.clone() {
            let consequence = format!(
                "O servidor {} deste computador será parado. Projetos que o usam: {}. Os dados continuam guardados, e Iniciar no projeto liga o servidor de novo.",
                request.engine,
                request.projects.join(", ")
            );
            match (components::Confirmation {
                id: "database-stop",
                title: "Parar o banco de dados?",
                consequence: &consequence,
                confirm: "Parar o banco de dados",
                destructive: false,
            })
            .show(ui.ctx(), |_| {})
            {
                Some(components::Choice::Confirm) => {
                    self.database_stop_request = None;
                    self.stop_managed_server(&request.project, request.server);
                }
                Some(components::Choice::Cancel) => self.database_stop_request = None,
                None => {}
            }
        }
        if let Some((path, requirement)) = test {
            self.test_remote_requirement(&path, requirement);
        }
        if let Some((path, _)) = open {
            self.apply_project_action(ui.ctx(), ProjectCardAction::OpenProject(path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use localcodepilot_runtime::databases::DatabaseRequirement;

    fn requirement(host: Option<&str>, compose: Option<&str>) -> DatabaseRequirement {
        DatabaseRequirement {
            source: PathBuf::from(".env"),
            engine: DatabaseEngine::MySql,
            host: host.map(str::to_owned),
            port: Some(3306),
            compose_service: compose.map(str::to_owned),
        }
    }

    #[test]
    fn only_a_database_on_another_computer_offers_a_connection_test() {
        // Each row decides from its own database, so a project with a local and a remote
        // database gets "Testar conexão" only on the remote row.
        assert!(database_ui::is_remote_database(&requirement(
            Some("db.exemplo.invalid"),
            None
        )));
        assert!(!database_ui::is_remote_database(&requirement(
            Some("127.0.0.1"),
            None
        )));
        assert!(!database_ui::is_remote_database(&requirement(
            Some("localhost"),
            None
        )));
        assert!(!database_ui::is_remote_database(&requirement(
            Some("db"),
            Some("db")
        )));
        assert!(!database_ui::is_remote_database(&requirement(None, None)));
    }
}
