//! The page of one project (T038.01): everything about it in one place. The header says the
//! state, why, and the one action to take; then what runs (its processes and containers), its
//! database, its logs and the technical details. The same summary as the cards and rows, so the
//! project says the same thing everywhere.

use super::*;
use egui_phosphor::regular::CARET_LEFT;

impl LocalCodePilot {
    /// Opens the page of a project, from its name, its row or a "Ver" link elsewhere.
    pub(in crate::app) fn open_project_page(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.touch_project(&path);
        let uses_containers = self.projects.iter().any(|project| {
            project.path == path && project.technologies.contains(&TechnologyKind::Containers)
        });
        self.focused_project = Some(path.clone());
        self.clear_container_filter();
        self.open_project = Some(path);
        self.page = Page::Project;
        // The keyboard lands at the top of the page (T038, CA-11).
        ctx.data_mut(|data| data.insert_temp(focus_page_key(), true));
        if uses_containers && !self.container_refresh_pending() {
            self.refresh_containers(ctx.clone());
        }
    }

    /// The projects with a process the app started that still runs, in list order, with
    /// their names (T039.03).
    pub(in crate::app) fn running_projects(&self) -> Vec<(PathBuf, String)> {
        self.projects
            .iter()
            .filter(|project| self.managed.project_is_running(&project.path))
            .map(|project| (project.path.clone(), project.name.clone()))
            .collect()
    }

    /// Shows a project's page where no context is at hand (a notice, a guided fix): the page
    /// refreshes its containers on its own schedule (T038.04).
    pub(in crate::app) fn show_project_page(&mut self, path: PathBuf) {
        self.focused_project = Some(path.clone());
        self.open_project = Some(path);
        self.page = Page::Project;
    }

    /// "Ver logs" when the project has a single service; "Ver logs de {serviço}" when it has
    /// more, so the button says which logs it opens (T039.01, CA-7).
    pub(in crate::app) fn logs_label(&self, path: &Path, id: &str) -> String {
        let processes = self
            .managed
            .processes
            .iter()
            .filter(|process| process.project_path == path)
            .collect::<Vec<_>>();
        match processes.iter().find(|process| process.id == id) {
            Some(process) if processes.len() > 1 => {
                format!("{TERMINAL}  Ver logs de {}", process.name)
            }
            _ => format!("{TERMINAL}  Ver logs"),
        }
    }

    /// The process whose logs "Ver logs" opens on the project page: one that runs, else the
    /// last one with saved logs.
    pub(in crate::app) fn project_log_target(&self, path: &Path) -> Option<String> {
        let processes = self
            .managed
            .processes
            .iter()
            .filter(|process| process.project_path == path)
            .collect::<Vec<_>>();
        processes
            .iter()
            .find(|process| self.managed.running.contains_key(&process.id))
            .or_else(|| {
                processes.iter().find(|process| {
                    self.workspace_state
                        .service(&process.id)
                        .is_some_and(|history| !history.logs.is_empty())
                })
            })
            .map(|process| process.id.clone())
    }

    pub(in crate::app) fn project_page(&mut self, ui: &mut egui::Ui) {
        let Some(project) = self
            .open_project
            .as_ref()
            .and_then(|path| self.projects.iter().find(|project| project.path == *path))
            .cloned()
        else {
            if self.discovery.is_some() {
                components::loading_state(
                    ui,
                    "Procurando projetos...",
                    "Estamos examinando as pastas mais comuns da sua máquina.",
                );
            } else {
                // The project left the list (folder removed or moved): back to Projetos.
                self.page = Page::Projects;
            }
            return;
        };
        // Processes and containers below show only this project.
        self.focused_project = Some(project.path.clone());
        let summary = self.project_summary(&project);
        let folder = self.distinguishing_folders().remove(&project.path);
        let mut action = None;

        let back = format!("{CARET_LEFT}  Projetos");
        let back = announced(ui.link(&back), egui::Role::Link, "Voltar para Projetos");
        if ui
            .ctx()
            .data_mut(|data| data.remove_temp::<bool>(focus_page_key()))
            == Some(true)
        {
            back.request_focus();
        }
        if back.clicked() {
            // Back on the list, the keyboard returns to this project's name.
            let path = project.path.clone();
            ui.ctx()
                .data_mut(|data| data.insert_temp(return_focus_key(), path));
            self.page = Page::Projects;
        }
        ui.add_space(4.0);
        page_section(ui, "Resumo");
        let log_target = self.project_log_target(&project.path);
        let logs_label = log_target
            .as_deref()
            .map(|id| self.logs_label(&project.path, id));
        let header = self.project_header(
            ui,
            &project,
            &summary,
            folder.as_deref(),
            log_target.as_deref().zip(logs_label.as_deref()),
            &mut action,
        );
        if let Some(id) = header.logs {
            self.managed.terminal = Some(id);
        }
        if header.retest {
            self.retest_database(&project.path);
        }

        let uses_containers = project.technologies.contains(&TechnologyKind::Containers);
        let has_processes = self
            .managed
            .processes
            .iter()
            .any(|process| process.project_path == project.path);
        page_section(ui, "O que roda");
        if has_processes {
            self.processes_page(ui);
        }
        if uses_containers {
            let (checking, executing) = self.container_activity();
            self.container_execution_panel(ui);
            self.engine_overview(ui, checking || executing);
            ui.add_space(6.0);
            self.container_groups(ui, checking, executing);
            self.volume_confirmation_dialog(ui.ctx());
        }
        if !has_processes && !uses_containers {
            empty_hint(
                ui,
                "O LocalCodePilot não encontrou como iniciar este projeto. Defina o comando de início em Detalhes técnicos.",
            );
        }

        self.project_database_section(ui, &project, &summary, &mut action);

        page_section(ui, "Logs");
        let has_install_logs = self.dependency_installs.contains_key(&project.path);
        if log_target.is_some() || has_install_logs {
            ui.horizontal_wrapped(|ui| {
                if let Some((id, label)) = log_target.as_ref().zip(logs_label.as_ref())
                    && announced(ui.button(label), egui::Role::Button, label).clicked()
                {
                    self.managed.terminal = Some(id.clone());
                }
                if has_install_logs
                    && announced(
                        ui.button("Ver logs da instalação"),
                        egui::Role::Button,
                        "Ver logs da instalação",
                    )
                    .clicked()
                {
                    action = Some(ProjectCardAction::ViewDependencyLogs(project.path.clone()));
                }
            });
        } else {
            ui.label(
                RichText::new(if uses_containers {
                    "Os logs dos containers ficam em cada serviço, em O que roda."
                } else {
                    "Os logs aparecem aqui depois que o projeto for iniciado."
                })
                .color(theme::MUTED)
                .size(theme::TEXT_BODY),
            );
        }

        page_section(ui, "Detalhes técnicos");
        let card = ProjectCard {
            project: &project,
            summary: &summary,
            languages: self.language_lines(&project),
            custom_commands: self.custom_commands_of(&project.path),
            environment: self.project_environments.get(&project.path),
            hidden_from_recent: self
                .workspace_state
                .hidden_recent_projects
                .contains(&project.path),
            has_install_logs,
            database: self.database_card_extras(&project.path),
        };
        let open = notice::heading_disclosure(
            ui,
            technical_key(&project.path),
            "Comandos, caminhos e diagnóstico",
            false,
        );
        if open && let Some(selected) = card.details(ui) {
            action = Some(selected);
        }
        ui.add_space(16.0);

        if let Some(action) = action {
            self.apply_project_action(ui.ctx(), action);
        }
    }

    /// Name, type, folder, state, the reason and the actions: what the card says, larger.
    fn project_header(
        &self,
        ui: &mut egui::Ui,
        project: &Project,
        summary: &ProjectSummary,
        folder: Option<&str>,
        logs: Option<(&str, &str)>,
        action: &mut Option<ProjectCardAction>,
    ) -> HeaderRequests {
        let mut requests = HeaderRequests::default();
        let can_retest = self.rejected_database(&project.path).is_some();
        let favorite = self.is_favorite(&project.path);
        Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(
                1.0,
                if summary.state == ProjectState::Problem {
                    theme::WARNING.gamma_multiply(0.5)
                } else {
                    theme::BORDER
                },
            ))
            .corner_radius(12)
            .inner_margin(Margin::same(18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&project.name)
                            .strong()
                            .size(theme::TEXT_TITLE + 4.0),
                    )
                    .on_hover_text(display_path(&project.path));
                    for (label, color) in card_badges(project) {
                        ui.label(RichText::new(label).color(color).size(theme::TEXT_DETAIL));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let name = if favorite {
                            "Remover dos favoritos"
                        } else {
                            "Adicionar aos favoritos"
                        };
                        let star = ui
                            .button(RichText::new(STAR).color(if favorite {
                                theme::PRIMARY
                            } else {
                                theme::MUTED
                            }))
                            .on_hover_text(name);
                        star.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::Button,
                                true,
                                favorite,
                                format!("{name}: {}", project.name),
                            )
                        });
                        if star.clicked() {
                            *action = Some(ProjectCardAction::ToggleFavorite(project.path.clone()));
                        }
                    });
                });
                folder_line(ui, &project.path, folder);
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    state_badge(ui, summary);
                    if let Some(http) = summary.http {
                        http_badge(ui, http);
                    }
                    if let Some(url) = &summary.url {
                        address_label(ui, summary, url);
                    }
                });
                if let Some(reason) = &summary.reason {
                    ui.add(egui::Label::new(RichText::new(reason).size(theme::TEXT_BODY)).wrap());
                }
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    // "Detalhes" as the main action: open the technical details below and, when
                    // the server refused the project, test again (T039.01, CA-1). The page never
                    // leaves only "Abrir no VS Code".
                    if matches!(summary.primary, PrimaryAction::Details) {
                        let label = "Ver detalhes técnicos";
                        let button =
                            egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
                                .fill(theme::PRIMARY_FILL)
                                .min_size(egui::vec2(140.0, 32.0));
                        if announced(ui.add(button), egui::Role::Button, label).clicked() {
                            notice::reveal(ui.ctx(), technical_key(&project.path));
                        }
                        if can_retest
                            && announced(
                                ui.button("Testar conexão de novo"),
                                egui::Role::Button,
                                "Testar conexão de novo",
                            )
                            .clicked()
                        {
                            requests.retest = true;
                        }
                    } else if primary_button(ui, &summary.primary, 140.0, None) {
                        *action = Some(primary_to_card_action(project, summary));
                    }
                    if can_retry(summary) && retry_button(ui) {
                        *action = Some(ProjectCardAction::Start(project.path.clone()));
                    }
                    // The logs of the process that runs, or of the last run (T038, CA-2).
                    if let Some((id, label)) = logs
                        && !matches!(summary.primary, PrimaryAction::ViewLogs)
                        && announced(ui.button(label), egui::Role::Button, label).clicked()
                    {
                        requests.logs = Some(id.to_owned());
                    }
                    if summary.state == ProjectState::Running || summary.stoppable {
                        if let Some(url) = &summary.url
                            && !matches!(summary.primary, PrimaryAction::OpenSite(_))
                        {
                            let label = format!("{ARROW_SQUARE_OUT}  Abrir site");
                            if announced(ui.button(&label), egui::Role::Button, "Abrir site")
                                .clicked()
                            {
                                *action = Some(ProjectCardAction::OpenUrl(url.clone()));
                            }
                        }
                        // The whole project (its processes, or its Compose environment with the
                        // confirmation that lists what stops), said on the button (CA-7).
                        let label = if summary.uses_containers && !summary.stops_containers_here {
                            format!("{STOP}  Escolher o que parar")
                        } else {
                            format!("{STOP}  Parar o projeto")
                        };
                        if announced(
                            ui.button(RichText::new(&label).color(theme::DANGER)),
                            egui::Role::Button,
                            &label,
                        )
                        .clicked()
                        {
                            *action = Some(stop_action(project, summary));
                        }
                    }
                    let vs_code = format!("{CODE_SIMPLE}  Abrir no VS Code");
                    if announced(ui.button(&vs_code), egui::Role::Button, "Abrir no VS Code")
                        .clicked()
                    {
                        *action = Some(ProjectCardAction::Open(project.path.clone()));
                    }
                });
            });
        requests
    }

    /// The project's databases, one row each: engine, state and where it is, with the
    /// actions the card offers (a guided fix, Parar, Testar conexão).
    fn project_database_section(
        &self,
        ui: &mut egui::Ui,
        project: &Project,
        summary: &ProjectSummary,
        action: &mut Option<ProjectCardAction>,
    ) {
        page_section(ui, "Banco de dados");
        let Some(environment) = self
            .project_environments
            .get(&project.path)
            .filter(|environment| !environment.databases.is_empty())
        else {
            ui.label(
                RichText::new("Este projeto não usa banco de dados.")
                    .color(theme::MUTED)
                    .size(theme::TEXT_BODY),
            );
            return;
        };
        let extras = self.database_card_extras(&project.path);
        for database in &environment.databases {
            let matching_container = database
                .requirement
                .host
                .as_deref()
                .and_then(|host| self.database_container_running(&project.path, host));
            let state = database.card_state(matching_container);
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(database.requirement.engine.label())
                                .strong()
                                .size(theme::TEXT_BODY),
                        );
                        if let Some(port) = database.requirement.port {
                            ui.label(
                                RichText::new(format!("porta {port}"))
                                    .color(theme::MUTED)
                                    .size(theme::TEXT_DETAIL),
                            );
                        }
                        ui.label(RichText::new(state).size(theme::TEXT_BODY));
                    })
                    .response
                    .on_hover_text(database.technical().join("\n"));
                });
            ui.add_space(4.0);
        }
        ui.horizontal_wrapped(|ui| {
            if let Some(fix) = extras
                .failure
                .as_ref()
                .and_then(|failure| failure.detail_fix)
                && ui.button(fix.label()).clicked()
            {
                *action = Some(ProjectCardAction::DatabaseFix(project.path.clone(), fix));
            }
            if extras.can_stop && ui.button("Parar o banco de dados").clicked() {
                *action = Some(ProjectCardAction::StopDatabase(project.path.clone()));
            }
            if extras.remote && ui.button("Testar conexão").clicked() {
                *action = Some(ProjectCardAction::TestRemoteDatabase(project.path.clone()));
            }
        });
        // Stopping the project leaves a shared server on (T038, CA-9): say so where it is.
        if database_keeps_running(summary.state, &environment.databases) {
            ui.label(
                RichText::new("Projeto parado. O banco de dados continua rodando.")
                    .size(theme::TEXT_BODY),
            );
        }
        if !extras.stop_blocked_by.is_empty() {
            ui.label(
                RichText::new(format!(
                    "Para parar o banco de dados, pare primeiro: {}",
                    extras.stop_blocked_by.join(", ")
                ))
                .color(theme::MUTED)
                .size(theme::TEXT_DETAIL),
            );
        }
    }
}

fn focus_page_key() -> egui::Id {
    egui::Id::unique("project-page-focus")
}

/// The project whose name takes the keyboard focus back on the list.
pub(super) fn return_focus_key() -> egui::Id {
    egui::Id::unique("project-return-focus")
}

/// The project is stopped and a server the app created for it still runs.
fn database_keeps_running(state: ProjectState, databases: &[DatabaseStatus]) -> bool {
    state != ProjectState::Running
        && state != ProjectState::Starting
        && databases.iter().any(|status| {
            status
                .managed
                .as_ref()
                .is_some_and(|managed| managed.running)
        })
}

/// What the header asked for, done by the page once it is drawn.
#[derive(Default)]
struct HeaderRequests {
    /// Open the logs of this process.
    logs: Option<String>,
    /// "Testar conexão de novo".
    retest: bool,
}

/// The "Detalhes técnicos" group of a project's page, which "Ver detalhes técnicos" opens.
fn technical_key(path: &Path) -> egui::Id {
    egui::Id::unique(("project-technical", path.to_path_buf()))
}

/// A section title of the project page.
fn page_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(18.0);
    ui.label(RichText::new(title).strong().size(theme::TEXT_TITLE));
    ui.add_space(6.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use localcodepilot_runtime::databases::{DatabaseEngine, DatabaseRequirement};

    fn server(running: Option<bool>) -> DatabaseStatus {
        DatabaseStatus {
            requirement: DatabaseRequirement {
                source: PathBuf::from(".env"),
                engine: DatabaseEngine::MySql,
                host: Some("127.0.0.1".into()),
                port: Some(3306),
                compose_service: None,
            },
            available: running,
            connection: None,
            managed: running.map(|running| database_ui::ManagedInfo {
                id: "mysql-3306".into(),
                running,
                summary: String::new(),
            }),
        }
    }

    #[test]
    fn a_stopped_project_says_its_shared_database_keeps_running() {
        assert!(database_keeps_running(
            ProjectState::Stopped,
            &[server(Some(true))]
        ));
        // Not while the project runs, not for a stopped server, not for one the app did
        // not create.
        assert!(!database_keeps_running(
            ProjectState::Running,
            &[server(Some(true))]
        ));
        assert!(!database_keeps_running(
            ProjectState::Stopped,
            &[server(Some(false))]
        ));
        assert!(!database_keeps_running(
            ProjectState::Stopped,
            &[server(None)]
        ));
    }
}
