//! Serviços > Bancos de dados (T037.06): the databases of every project, one row per server.
//! A server several projects use shows once, with those projects. The actions follow the
//! rules of the project card: "Parar" only for a server the app created, when no project that
//! uses it is running; "Testar conexão" for a server on another computer; nothing that changes
//! a server the app did not create (already running on the port, or a Compose service).

use super::*;
use localcodepilot_runtime::databases::DatabaseEngine;

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

impl LocalCodePilot {
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
        let entries = self.database_entries();
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
        let mut test = None;
        let mut open = None;
        for entry in &entries {
            let (state, color) = entry_state(&entry.status);
            // "Parar" follows the card: the app's own server, no project using it running.
            let mut can_stop = None;
            let mut blocked_by = Vec::new();
            let mut remote = None;
            for (path, _) in &entry.projects {
                let extras = self.database_card_extras(path);
                if extras.can_stop && can_stop.is_none() {
                    can_stop = Some(path.clone());
                }
                for name in extras.stop_blocked_by {
                    if !blocked_by.contains(&name) {
                        blocked_by.push(name);
                    }
                }
                if extras.remote && remote.is_none() {
                    remote = Some(path.clone());
                }
            }
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
                            if let Some(path) = &can_stop
                                && ui.button("Parar").clicked()
                            {
                                stop = Some(path.clone());
                            }
                            if let Some(path) = &remote
                                && ui.button("Testar conexão").clicked()
                            {
                                test = Some(path.clone());
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
                    if can_stop.is_none() && !blocked_by.is_empty() {
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
        if let Some(path) = stop {
            self.stop_project_database(&path);
        }
        if let Some(path) = test {
            self.test_remote_database(&path);
        }
        if let Some((path, name)) = open {
            self.apply_project_action(ui.ctx(), ProjectCardAction::ShowInProjects(path, name));
        }
    }
}
