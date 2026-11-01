//! Overview and Projects: one summary per project (state, reason, HTTP availability, URL and
//! a single main action) rendered with the same vocabulary everywhere.

use super::*;
use egui_phosphor::regular::{LIST, SQUARES_FOUR, STOP, WARNING_CIRCLE};

/// Execution state shown to the user. HTTP availability is reported separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjectState {
    Running,
    Starting,
    Stopped,
    Problem,
}

impl ProjectState {
    fn label(self) -> &'static str {
        match self {
            Self::Running => "Rodando",
            Self::Starting => "Iniciando",
            Self::Stopped => "Parado",
            Self::Problem => "Com problema",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Running => theme::SUCCESS,
            Self::Starting => theme::PRIMARY,
            Self::Stopped => theme::MUTED,
            Self::Problem => theme::WARNING,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Result of the last HTTP check. Without a check there is no badge, only the address.
pub(super) enum HttpAvailability {
    Responding(u16),
    /// 4xx: the server is up; the page is missing or not accessible.
    ServerAnswered(u16),
    /// 5xx.
    Error(u16),
    NoResponse,
}

impl HttpAvailability {
    fn label(self) -> &'static str {
        match self {
            Self::Responding(_) => "Site respondendo",
            Self::ServerAnswered(404) => "Servidor respondeu; página não encontrada",
            Self::ServerAnswered(_) => "Acesso não disponível",
            Self::Error(_) => "Site com erro",
            Self::NoResponse => "Site sem resposta",
        }
    }

    /// Technical detail shown on hover.
    fn detail(self) -> String {
        match self {
            Self::Responding(code) | Self::ServerAnswered(code) | Self::Error(code) => {
                format!("Código HTTP {code}")
            }
            Self::NoResponse => "O endereço não respondeu à verificação HTTP".into(),
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Responding(_) => theme::SUCCESS,
            Self::ServerAnswered(_) => theme::MUTED,
            Self::Error(_) | Self::NoResponse => theme::WARNING,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PrimaryAction {
    OpenSite(String),
    Start,
    StartWithContainers,
    ViewLogs,
    ViewInstallLogs,
    PrepareEnvironment,
    InstallDependencies,
    Details,
    /// Something is in progress; no button.
    Wait,
}

impl PrimaryAction {
    fn label(&self) -> Option<String> {
        Some(match self {
            Self::OpenSite(_) => format!("{ARROW_SQUARE_OUT}  Abrir site"),
            Self::Start | Self::StartWithContainers => format!("{PLAY}  Iniciar"),
            Self::ViewLogs | Self::ViewInstallLogs => format!("{TERMINAL}  Ver logs"),
            Self::PrepareEnvironment => "Preparar ambiente".into(),
            Self::InstallDependencies => "Instalar dependências".into(),
            Self::Details => "Detalhes".into(),
            Self::Wait => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectSummary {
    pub state: ProjectState,
    /// One sentence a junior developer can act on.
    pub reason: Option<String>,
    pub http: Option<HttpAvailability>,
    pub url: Option<String>,
    pub primary: PrimaryAction,
    /// Runs through Containers, where logs and stop live.
    pub uses_containers: bool,
}

/// Everything the summary depends on, gathered from the app state.
pub(super) struct ProjectFacts<'a> {
    pub environment: &'a ProjectEnvironment,
    pub environment_pending: bool,
    pub resolving_plan: bool,
    pub dependency_install: Option<&'a DependencyInstallStatus>,
    pub starting: bool,
    pub running: bool,
    pub failed_process: Option<&'a str>,
    pub application_url: Option<&'a str>,
    pub http: Option<&'a HttpHealthStatus>,
    pub has_containers: bool,
    pub container_running: bool,
    pub container_url: Option<&'a str>,
}

/// Name of a state filter for assistive technologies, with its count.
fn filter_name(label: &str, count: usize) -> String {
    match count {
        1 => format!("{label}, 1 projeto"),
        _ => format!("{label}, {count} projetos"),
    }
}

pub(super) fn summarize_project(facts: &ProjectFacts<'_>) -> ProjectSummary {
    let environment = facts.environment;
    let summary = |state, reason: Option<String>, primary| ProjectSummary {
        state,
        reason,
        http: None,
        url: None,
        primary,
        uses_containers: false,
    };
    if facts.running || facts.container_running {
        let url = facts
            .application_url
            .filter(|_| facts.running)
            .or(facts.container_url)
            .map(str::to_owned);
        let http = match (facts.running, facts.http) {
            (true, Some(HttpHealthStatus::Healthy { status_code, .. })) => {
                Some(HttpAvailability::Responding(*status_code))
            }
            (true, Some(HttpHealthStatus::Attention { status_code, .. }))
                if (400..500).contains(status_code) =>
            {
                Some(HttpAvailability::ServerAnswered(*status_code))
            }
            (true, Some(HttpHealthStatus::Attention { status_code, .. })) => {
                Some(HttpAvailability::Error(*status_code))
            }
            (true, Some(HttpHealthStatus::Unreachable { .. })) => {
                Some(HttpAvailability::NoResponse)
            }
            _ => None,
        };
        return ProjectSummary {
            state: ProjectState::Running,
            reason: None,
            http: url.as_ref().and(http),
            primary: url
                .clone()
                .map_or(PrimaryAction::ViewLogs, PrimaryAction::OpenSite),
            url,
            uses_containers: !facts.running,
        };
    }
    match facts.dependency_install {
        Some(DependencyInstallStatus::Running | DependencyInstallStatus::Cancelling) => {
            return summary(
                ProjectState::Starting,
                Some("Instalando dependências".into()),
                PrimaryAction::ViewInstallLogs,
            );
        }
        Some(DependencyInstallStatus::Failed { .. }) => {
            return summary(
                ProjectState::Problem,
                Some("A instalação das dependências falhou".into()),
                PrimaryAction::ViewInstallLogs,
            );
        }
        _ => {}
    }
    if facts.starting {
        return summary(
            ProjectState::Starting,
            Some("Iniciando os serviços".into()),
            PrimaryAction::ViewLogs,
        );
    }
    if facts.resolving_plan {
        return summary(
            ProjectState::Starting,
            Some("Consultando as versões disponíveis".into()),
            PrimaryAction::Wait,
        );
    }
    if facts.environment_pending {
        return summary(
            ProjectState::Stopped,
            Some("Verificando o projeto".into()),
            PrimaryAction::Wait,
        );
    }
    if let Some(name) = facts.failed_process {
        return summary(
            ProjectState::Problem,
            Some(format!("{name} parou com erro")),
            PrimaryAction::ViewLogs,
        );
    }
    if let Some(error) = environment.manifest_errors.first() {
        return summary(
            ProjectState::Problem,
            Some(format!(
                "{error}. Corrija o arquivo para iniciar este projeto"
            )),
            PrimaryAction::Details,
        );
    }
    if facts.has_containers && environment.command_count == 0 {
        let mut stopped = summary(
            ProjectState::Stopped,
            Some("Roda com containers".into()),
            PrimaryAction::StartWithContainers,
        );
        stopped.uses_containers = true;
        return stopped;
    }
    if environment.command_count == 0 {
        return summary(
            ProjectState::Stopped,
            Some("Nenhum comando para iniciar foi encontrado".into()),
            PrimaryAction::Details,
        );
    }
    if environment.is_ready()
        && !environment
            .databases
            .iter()
            .any(|(_, available)| *available == Some(false))
    {
        return summary(ProjectState::Stopped, None, PrimaryAction::Start);
    }
    let reason = environment
        .problem_details()
        .unwrap_or_else(|| "Requisitos pendentes".into());
    let primary = if environment.can_prepare_environment() {
        PrimaryAction::PrepareEnvironment
    } else if environment.missing_programs.is_empty()
        && !environment.missing_dependencies.is_empty()
        && environment.can_install_dependencies()
    {
        PrimaryAction::InstallDependencies
    } else {
        PrimaryAction::Details
    };
    summary(ProjectState::Problem, Some(reason), primary)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjectFilter {
    All,
    Running,
    Problem,
    Stopped,
}

impl ProjectFilter {
    fn from_key(key: &str) -> Self {
        match key {
            "running" => Self::Running,
            "problem" => Self::Problem,
            "stopped" => Self::Stopped,
            _ => Self::All,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::All => "",
            Self::Running => "running",
            Self::Problem => "problem",
            Self::Stopped => "stopped",
        }
    }

    /// "Iniciando" is grouped with running work so it is not hidden while a start is in
    /// progress.
    fn matches(self, state: ProjectState) -> bool {
        match self {
            Self::All => true,
            Self::Running => matches!(state, ProjectState::Running | ProjectState::Starting),
            Self::Problem => state == ProjectState::Problem,
            Self::Stopped => state == ProjectState::Stopped,
        }
    }
}

/// Transient view state: which project cards have their details open, and whether the
/// scanned folders are listed.
#[derive(Default)]
pub(super) struct ProjectUiState {
    expanded: HashSet<PathBuf>,
    /// The list of scanned folders is open on Projects.
    pub(super) show_roots: bool,
}

/// Projects with their summaries, in display order.
type Summaries = Vec<(Project, ProjectSummary)>;

impl LocalCodePilot {
    pub(super) fn project_summary(&self, project: &Project) -> ProjectSummary {
        let fallback = ProjectEnvironment::default();
        let environment = self
            .project_environments
            .get(&project.path)
            .unwrap_or(&fallback);
        let processes = self
            .processes
            .iter()
            .filter(|process| process.project_path == project.path)
            .collect::<Vec<_>>();
        let application_url = processes
            .iter()
            .filter(|process| process.state == ProcessState::Running)
            .filter(|process| process_exposes_application_url(project, process))
            .find_map(|process| {
                let running = self.running_processes.get(&process.id)?;
                if running.finished {
                    return None;
                }
                running
                    .urls
                    .last()
                    .map(|url| url.address().to_owned())
                    .or_else(|| {
                        self.assigned_ports
                            .get(&process.id)
                            .map(|port| format!("http://localhost:{}", port.value()))
                    })
            });
        let http = processes
            .iter()
            .filter_map(|process| self.running_processes.get(&process.id))
            .find_map(|running| running.http_health.as_ref().map(|health| &health.status));
        let (container_running, container_url) = self.project_container_status(&project.path);
        let dependency_install = self
            .dependency_installs
            .get(&project.path)
            .map(|session| &session.status);
        summarize_project(&ProjectFacts {
            environment,
            environment_pending: self.discovery.is_some()
                && !self.project_environments.contains_key(&project.path),
            resolving_plan: self
                .environment_plan_request
                .as_ref()
                .is_some_and(|(path, _)| path == &project.path),
            dependency_install,
            starting: processes
                .iter()
                .any(|process| process.state == ProcessState::Starting),
            running: processes
                .iter()
                .any(|process| process.state == ProcessState::Running),
            failed_process: processes
                .iter()
                .find(|process| process.state == ProcessState::Failed)
                .map(|process| process.name.as_str()),
            application_url: application_url.as_deref(),
            http,
            has_containers: project.technologies.contains(&TechnologyKind::Containers),
            container_running,
            container_url: container_url.as_deref(),
        })
    }

    fn summaries(&self) -> Summaries {
        let query = self.search.trim().to_lowercase();
        let mut summaries = self
            .projects
            .iter()
            .filter(|project| {
                query.is_empty()
                    || project.name.to_lowercase().contains(&query)
                    || project
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|project| (project.clone(), self.project_summary(project)))
            .collect::<Vec<_>>();
        summaries.sort_by_key(|(project, _)| {
            let last_activity = self
                .workspace_state
                .project(&project.path)
                .map(|history| history.last_activity)
                .unwrap_or_else(|| system_time_timestamp(project.modified_at));
            std::cmp::Reverse((self.is_favorite(&project.path), last_activity))
        });
        summaries
    }

    /// Turns a card or row action into navigation or a process operation.
    fn apply_project_action(&mut self, ctx: &egui::Context, action: ProjectCardAction) {
        match action {
            ProjectCardAction::ViewDetails(path) => {
                self.focused_project = Some(path);
                self.search.clear();
                self.page = Page::Processes;
            }
            ProjectCardAction::ViewContainers(path) => {
                self.touch_project(&path);
                self.focused_project = Some(path);
                self.page = Page::Containers;
            }
            ProjectCardAction::Open(path) => match open_in_vscode(&path) {
                Ok(()) => {
                    self.touch_project(&path);
                    self.notify("Abrindo no VS Code...");
                }
                Err(error) => self.notify(error),
            },
            ProjectCardAction::OpenUrl(url) => {
                ctx.open_url(egui::OpenUrl::new_tab(&url));
                self.notify(format!("Abrindo {url}"));
            }
            ProjectCardAction::Start(path) => self.start_project(&path),
            ProjectCardAction::Stop(path) => self.stop_project(&path),
            ProjectCardAction::PrepareEnvironment(path) => {
                self.request_environment_plan(&path, ctx.clone());
            }
            ProjectCardAction::InstallDependencies(path) => {
                self.install_dependencies(&path, ctx.clone());
            }
            ProjectCardAction::ViewDependencyLogs(path) => {
                self.dependency_terminal = Some(path);
            }
            ProjectCardAction::PrepareLaravelMigration(path) => {
                self.prepare_laravel_migration(&path);
            }
            ProjectCardAction::ToggleFavorite(path) => self.toggle_favorite(&path),
            ProjectCardAction::ToggleRecentVisibility(path) => {
                self.toggle_recent_visibility(&path);
            }
            ProjectCardAction::ToggleDetails(path) => {
                if !self.project_ui.expanded.remove(&path) {
                    self.project_ui.expanded.insert(path);
                }
            }
            ProjectCardAction::ShowInProjects(path, name) => {
                self.project_ui.expanded.insert(path);
                self.search = name;
                self.workspace_state.project_filter.clear();
                self.page = Page::Projects;
            }
        }
    }

    pub(super) fn overview_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Visão geral").strong().size(24.0));
        ui.label(
            RichText::new("O que está rodando, o que precisa de atenção e o que está parado.")
                .color(theme::MUTED)
                .size(13.0),
        );
        ui.add_space(16.0);
        let summaries = self.summaries();
        let count = |filter: ProjectFilter| {
            summaries
                .iter()
                .filter(|(_, summary)| filter.matches(summary.state))
                .count()
        };
        let mut action = None;
        let mut open_filter = None;
        ui.horizontal_wrapped(|ui| {
            for (filter, label, color) in [
                (ProjectFilter::Running, "Rodando", theme::SUCCESS),
                (ProjectFilter::Problem, "Com problema", theme::WARNING),
                (ProjectFilter::Stopped, "Parados", theme::MUTED),
            ] {
                if counter(ui, label, count(filter), color)
                    .on_hover_text("Ver estes projetos na aba Projetos")
                    .clicked()
                {
                    open_filter = Some(filter);
                }
            }
        });
        ui.add_space(18.0);

        if self.projects.is_empty() {
            self.overview_without_projects(ui);
            return;
        }

        let running = summaries
            .iter()
            .filter(|(_, summary)| ProjectFilter::Running.matches(summary.state))
            .collect::<Vec<_>>();
        section_title(ui, "Rodando", running.len());
        if running.is_empty() {
            empty_hint(ui, "Nenhum projeto rodando agora.");
            if let Some((project, summary)) = summaries.iter().find(|(project, summary)| {
                summary.state == ProjectState::Stopped
                    && matches!(
                        summary.primary,
                        PrimaryAction::Start | PrimaryAction::StartWithContainers
                    )
                    && !is_localcodepilot_qa_project(&project.path)
            }) {
                ui.label(RichText::new(format!("Você pode iniciar {}.", project.name)).size(13.0));
                ui.add_space(4.0);
                if primary_button(ui, &summary.primary, 180.0) {
                    action = Some(primary_to_card_action(project, summary));
                }
            }
        }
        for (project, summary) in &running {
            if let Some(selected) = project_row(ui, project, summary) {
                action = Some(selected);
            }
        }
        ui.add_space(18.0);

        let problems = summaries
            .iter()
            .filter(|(_, summary)| summary.state == ProjectState::Problem)
            .collect::<Vec<_>>();
        section_title(ui, "Com problema", problems.len());
        if problems.is_empty() {
            empty_hint(ui, "Nenhum problema encontrado.");
        }
        for (project, summary) in &problems {
            if let Some(selected) = project_row(ui, project, summary) {
                action = Some(selected);
            }
        }
        ui.add_space(18.0);

        let recent_stopped = summaries
            .iter()
            .filter(|(project, summary)| {
                summary.state == ProjectState::Stopped
                    && !is_localcodepilot_qa_project(&project.path)
            })
            .take(5)
            .collect::<Vec<_>>();
        section_title(ui, "Parados recentes", recent_stopped.len());
        for (project, summary) in &recent_stopped {
            if let Some(selected) = project_row(ui, project, summary) {
                action = Some(selected);
            }
        }
        if ui.button("Ver todos os parados").clicked() {
            open_filter = Some(ProjectFilter::Stopped);
        }

        if let Some(filter) = open_filter {
            self.workspace_state.project_filter = filter.key().to_owned();
            self.workspace_dirty = true;
            self.page = Page::Projects;
        }
        if let Some(action) = action {
            self.apply_project_action(ui.ctx(), action);
        }
    }

    fn overview_without_projects(&mut self, ui: &mut egui::Ui) {
        Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(12)
            .inner_margin(Margin::same(24))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.discovery.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Procurando projetos...").strong().size(15.0));
                    });
                    ui.label(
                        RichText::new("Estamos examinando as pastas mais comuns da sua máquina.")
                            .color(theme::MUTED)
                            .size(13.0),
                    );
                    return;
                }
                ui.label(RichText::new("Nenhum projeto encontrado").strong().size(15.0));
                ui.label(
                    RichText::new(
                        "Escolha a pasta onde ficam seus projetos. O LocalCodePilot encontra e organiza todos eles.",
                    )
                    .size(13.0),
                );
                ui.add_space(10.0);
                let add = format!("{PLUS}  Adicionar pasta");
                if announced(
                    ui.add(
                        egui::Button::new(RichText::new(&add).color(Color32::WHITE).strong())
                            .fill(theme::PRIMARY)
                            .min_size(egui::vec2(180.0, 34.0)),
                    ),
                    egui::Role::Button,
                    &add,
                )
                .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title("Escolha uma pasta com projetos")
                        .pick_folder()
                {
                    let ctx = ui.ctx().clone();
                    self.add_scan_root(&ctx, path);
                }
            });
    }

    /// Filter bar, cards or list, on the Projects page.
    pub(super) fn project_catalog(&mut self, ui: &mut egui::Ui) {
        let summaries = self.summaries();
        let mut filter = ProjectFilter::from_key(&self.workspace_state.project_filter);
        let mut list = self.workspace_state.project_view == "list";
        let (old_filter, old_list) = (filter, list);
        ui.horizontal_wrapped(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Buscar por nome...")
                    .desired_width(ui.available_width().min(200.0)),
            );
            name_search_field(&search, "Buscar projetos por nome", &self.search);
            for (option, label) in [
                (ProjectFilter::All, "Todos"),
                (ProjectFilter::Running, "Rodando"),
                (ProjectFilter::Problem, "Com problema"),
                (ProjectFilter::Stopped, "Parados"),
            ] {
                let count = summaries
                    .iter()
                    .filter(|(_, summary)| option.matches(summary.state))
                    .count();
                let response = ui.selectable_value(&mut filter, option, format!("{label} {count}"));
                let name = filter_name(label, count);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Button, true, filter == option, &name)
                });
            }
            // The two view buttons wrap together.
            if ui.available_size_before_wrap().x < 90.0 {
                ui.end_row();
            }
            for (cards, icon, name) in [
                (true, SQUARES_FOUR, "Ver como cartões"),
                (false, LIST, "Ver como lista"),
            ] {
                let response = ui
                    .selectable_value(&mut list, !cards, RichText::new(icon).size(15.0))
                    .on_hover_text(name);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Button, true, list != cards, name)
                });
            }
        });
        if (filter, list) != (old_filter, old_list) {
            self.workspace_state.project_filter = filter.key().to_owned();
            self.workspace_state.project_view = if list { "list".into() } else { String::new() };
            self.workspace_dirty = true;
        }
        ui.add_space(10.0);

        let visible = summaries
            .into_iter()
            .filter(|(_, summary)| filter.matches(summary.state))
            .collect::<Vec<_>>();
        if visible.is_empty() {
            empty_hint(
                ui,
                if self.discovery.is_some() {
                    "Procurando projetos..."
                } else if !self.search.trim().is_empty() {
                    "Nenhum projeto com esse nome."
                } else if self.projects.is_empty() {
                    "Nenhum projeto encontrado. Use Adicionar pasta para escolher onde ficam seus projetos."
                } else {
                    "Nenhum projeto neste filtro."
                },
            );
            return;
        }

        let mut action = None;
        if list {
            for (project, summary) in &visible {
                if let Some(selected) = project_row(ui, project, summary) {
                    action = Some(selected);
                }
            }
        } else {
            let available_width = ui.available_width();
            let columns: usize = if available_width >= 900.0 {
                3
            } else if available_width >= 560.0 {
                2
            } else {
                1
            };
            let gap = 14.0;
            let card_width =
                ((available_width - gap * (columns - 1) as f32) / columns as f32).floor();
            for row in visible.chunks(columns) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (project, summary) in row {
                        let card = ProjectCard {
                            project,
                            summary,
                            environment: self.project_environments.get(&project.path),
                            favorite: self.is_favorite(&project.path),
                            expanded: self.project_ui.expanded.contains(&project.path),
                            hidden_from_recent: self
                                .workspace_state
                                .hidden_recent_projects
                                .contains(&project.path),
                            has_install_logs: self.dependency_installs.contains_key(&project.path),
                        };
                        ui.allocate_ui_with_layout(
                            egui::vec2(card_width, 0.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                if let Some(selected) = card.show(ui, card_width) {
                                    action = Some(selected);
                                }
                            },
                        );
                    }
                });
                ui.add_space(gap);
            }
        }
        if let Some(action) = action {
            self.apply_project_action(ui.ctx(), action);
        }
    }
}

fn primary_to_card_action(project: &Project, summary: &ProjectSummary) -> ProjectCardAction {
    let path = project.path.clone();
    match &summary.primary {
        PrimaryAction::OpenSite(url) => ProjectCardAction::OpenUrl(url.clone()),
        PrimaryAction::Start => ProjectCardAction::Start(path),
        PrimaryAction::StartWithContainers => ProjectCardAction::ViewContainers(path),
        PrimaryAction::ViewLogs => logs_action(project, summary),
        PrimaryAction::ViewInstallLogs => ProjectCardAction::ViewDependencyLogs(path),
        PrimaryAction::PrepareEnvironment => ProjectCardAction::PrepareEnvironment(path),
        PrimaryAction::InstallDependencies => ProjectCardAction::InstallDependencies(path),
        PrimaryAction::Details | PrimaryAction::Wait => {
            ProjectCardAction::ShowInProjects(path, project.name.clone())
        }
    }
}

fn logs_action(project: &Project, summary: &ProjectSummary) -> ProjectCardAction {
    if summary.uses_containers {
        ProjectCardAction::ViewContainers(project.path.clone())
    } else {
        ProjectCardAction::ViewDetails(project.path.clone())
    }
}

fn stop_action(project: &Project, summary: &ProjectSummary) -> ProjectCardAction {
    if summary.uses_containers {
        // Compose stop needs its confirmation and conflict checks in Containers.
        ProjectCardAction::ViewContainers(project.path.clone())
    } else {
        ProjectCardAction::Stop(project.path.clone())
    }
}

fn counter(ui: &mut egui::Ui, label: &str, count: usize, color: Color32) -> egui::Response {
    let inner = Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(count.to_string())
                        .strong()
                        .size(22.0)
                        .color(color),
                );
                ui.label(RichText::new(label).size(14.0));
            });
        });
    inner.response.interact(Sense::click())
}

fn section_title(ui: &mut egui::Ui, title: &str, count: usize) {
    ui.label(
        RichText::new(format!("{title} ({count})"))
            .strong()
            .size(16.0),
    );
    ui.add_space(6.0);
}

fn empty_hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::MUTED).size(13.0));
}

fn state_badge(ui: &mut egui::Ui, summary: &ProjectSummary) {
    let state = summary.state;
    let icon = if state == ProjectState::Problem {
        WARNING_CIRCLE
    } else {
        CIRCLE
    };
    ui.label(RichText::new(icon).color(state.color()).size(12.0));
    ui.label(
        RichText::new(state.label())
            .color(state.color())
            .strong()
            .size(13.0),
    );
}

fn http_badge(ui: &mut egui::Ui, http: HttpAvailability) {
    ui.label(RichText::new(http.label()).color(http.color()).size(12.0))
        .on_hover_text(http.detail());
}

/// Container addresses come from published ports and may not be a site.
fn address_label(ui: &mut egui::Ui, summary: &ProjectSummary, url: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if summary.uses_containers {
            ui.label(
                RichText::new("Endereço provável:")
                    .color(theme::MUTED)
                    .size(12.0),
            );
        }
        ui.label(RichText::new(short_url(url)).monospace().size(12.0));
    });
}

/// Stopping a Compose environment needs the confirmation and checks in Containers, so the
/// button says where it leads.
fn stop_label(summary: &ProjectSummary) -> String {
    if summary.uses_containers {
        format!("{STOP}  Parar em Containers")
    } else {
        format!("{STOP}  Parar")
    }
}

fn short_url(url: &str) -> &str {
    url.trim_start_matches("http://")
        .trim_start_matches("https://")
}

/// Main action as a filled button. Returns true when clicked.
fn primary_button(ui: &mut egui::Ui, action: &PrimaryAction, width: f32) -> bool {
    let Some(label) = action.label() else {
        ui.spinner();
        return false;
    };
    announced(
        ui.add(
            egui::Button::new(RichText::new(&label).color(Color32::WHITE).strong())
                .fill(theme::PRIMARY)
                .min_size(egui::vec2(width, 32.0)),
        ),
        egui::Role::Button,
        &label,
    )
    .clicked()
}

fn logs_button(ui: &mut egui::Ui) -> bool {
    let label = format!("{TERMINAL}  Ver logs");
    announced(ui.button(&label), egui::Role::Button, &label).clicked()
}

fn stop_button(ui: &mut egui::Ui, summary: &ProjectSummary) -> bool {
    let label = stop_label(summary);
    announced(
        ui.button(RichText::new(&label).color(theme::DANGER)),
        egui::Role::Button,
        &label,
    )
    .clicked()
}

/// One line per project, used by the Overview and the list view. Wraps on narrow windows.
fn project_row(
    ui: &mut egui::Ui,
    project: &Project,
    summary: &ProjectSummary,
) -> Option<ProjectCardAction> {
    let mut action = None;
    Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&project.name).strong().size(15.0))
                    .on_hover_text(display_path(&project.path));
                // One label only: the stack, not how it runs (the actions already say Containers).
                if let Some(technology) = project
                    .shown_technologies()
                    .find(|technology| *technology != TechnologyKind::Containers)
                {
                    ui.label(
                        RichText::new(technology.to_string())
                            .color(technology_color(technology))
                            .size(12.0),
                    );
                } else if let Some(runtime) = project.runtimes.first() {
                    ui.label(
                        RichText::new(runtime.to_string())
                            .color(runtime_color(*runtime))
                            .size(12.0),
                    );
                }
                ui.add_space(6.0);
                state_badge(ui, summary);
                if let Some(http) = summary.http {
                    http_badge(ui, http);
                }
                if let Some(url) = &summary.url {
                    address_label(ui, summary, url);
                }
                if let Some(reason) = &summary.reason {
                    ui.label(RichText::new(reason).color(theme::MUTED).size(13.0));
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if primary_button(ui, &summary.primary, 120.0) {
                    action = Some(primary_to_card_action(project, summary));
                }
                if summary.state == ProjectState::Running {
                    if !matches!(summary.primary, PrimaryAction::ViewLogs) && logs_button(ui) {
                        action = Some(logs_action(project, summary));
                    }
                    if stop_button(ui, summary) {
                        action = Some(stop_action(project, summary));
                    }
                }
            });
        });
    ui.add_space(8.0);
    action
}

struct ProjectCard<'a> {
    project: &'a Project,
    summary: &'a ProjectSummary,
    environment: Option<&'a ProjectEnvironment>,
    favorite: bool,
    expanded: bool,
    hidden_from_recent: bool,
    has_install_logs: bool,
}

impl ProjectCard<'_> {
    fn show(&self, ui: &mut egui::Ui, width: f32) -> Option<ProjectCardAction> {
        let ProjectCard {
            project, summary, ..
        } = *self;
        let path = || project.path.clone();
        let mut action = None;
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
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(width - 32.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&project.name).strong().size(16.0))
                        .on_hover_text(display_path(&project.path));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let name = if self.favorite {
                            "Remover dos favoritos"
                        } else {
                            "Adicionar aos favoritos"
                        };
                        let star = ui
                            .button(RichText::new(STAR).color(if self.favorite {
                                theme::PRIMARY
                            } else {
                                theme::MUTED
                            }))
                            .on_hover_text(name);
                        star.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::Button,
                                true,
                                self.favorite,
                                format!("{name}: {}", project.name),
                            )
                        });
                        if star.clicked() {
                            action = Some(ProjectCardAction::ToggleFavorite(path()));
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    for technology in project.shown_technologies() {
                        runtime_badge(ui, &technology.to_string(), technology_color(technology));
                    }
                    for runtime in project
                        .runtimes
                        .iter()
                        .copied()
                        .filter(|runtime| project.shows_runtime(*runtime))
                    {
                        runtime_badge(ui, &runtime.to_string(), runtime_color(runtime));
                    }
                    if project.runtimes.is_empty() && project.shown_technologies().next().is_none()
                    {
                        runtime_badge(ui, "Projeto local", theme::MUTED);
                    }
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    state_badge(ui, summary);
                    if let Some(http) = summary.http {
                        http_badge(ui, http);
                    }
                });
                if let Some(url) = &summary.url {
                    address_label(ui, summary, url);
                }
                if let Some(reason) = &summary.reason {
                    ui.label(RichText::new(reason).size(13.0));
                }
                ui.add_space(10.0);
                if primary_button(ui, &summary.primary, ui.available_width()) {
                    action = Some(match summary.primary {
                        PrimaryAction::Details => ProjectCardAction::ToggleDetails(path()),
                        _ => primary_to_card_action(project, summary),
                    });
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if summary.state == ProjectState::Running {
                        if !matches!(summary.primary, PrimaryAction::ViewLogs) && logs_button(ui) {
                            action = Some(logs_action(project, summary));
                        }
                        if stop_button(ui, summary) {
                            action = Some(stop_action(project, summary));
                        }
                    }
                    let details = if self.expanded {
                        "Ocultar detalhes"
                    } else {
                        "Detalhes"
                    };
                    if summary.primary != PrimaryAction::Details || self.expanded {
                        let response = ui.button(details);
                        if response.clicked() {
                            action = Some(ProjectCardAction::ToggleDetails(path()));
                        }
                    }
                });
                if self.expanded {
                    ui.separator();
                    if let Some(selected) = self.details(ui) {
                        action = Some(selected);
                    }
                }
            });
        action
    }

    /// Technical details and secondary actions, shown only on request.
    fn details(&self, ui: &mut egui::Ui) -> Option<ProjectCardAction> {
        let project = self.project;
        let path = || project.path.clone();
        let mut action = None;
        let detail = |ui: &mut egui::Ui, text: &str| {
            ui.label(RichText::new(text).color(theme::MUTED).size(12.0));
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "Pasta do projeto: {}",
                    project
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ))
                .color(theme::MUTED)
                .size(12.0),
            )
            .on_hover_text(display_path(&project.path));
            if ui.small_button("Copiar caminho").clicked() {
                ui.ctx().copy_text(display_path(&project.path));
            }
        });
        if let Some(environment) = self.environment {
            if environment.command_count > 0 {
                detail(
                    ui,
                    &format!("Comandos detectados: {}", environment.command_count),
                );
            }
            if let Some(problem) = environment.problem_details() {
                detail(ui, &problem);
            }
            for error in &environment.manifest_errors {
                detail(
                    ui,
                    &format!("{error}. Corrija o arquivo e clique em Atualizar."),
                );
            }
        }
        ui.add_space(4.0);
        let logs = format!("{TERMINAL}  Ver logs e processos");
        if announced(ui.link(&logs), egui::Role::Link, &logs).clicked() {
            action = Some(logs_action(project, self.summary));
        }
        if project.technologies.contains(&TechnologyKind::Containers)
            && ui.link("Ver containers").clicked()
        {
            action = Some(ProjectCardAction::ViewContainers(path()));
        }
        if self.has_install_logs && ui.link("Ver logs da instalação").clicked() {
            action = Some(ProjectCardAction::ViewDependencyLogs(path()));
        }
        if self
            .environment
            .is_some_and(|environment| environment.legacy_laravel_constraint().is_some())
            && ui.link("Planejar atualização do Laravel").clicked()
        {
            action = Some(ProjectCardAction::PrepareLaravelMigration(path()));
        }
        let vs_code = format!("{CODE_SIMPLE}  Abrir no VS Code");
        if announced(ui.link(&vs_code), egui::Role::Link, &vs_code).clicked() {
            action = Some(ProjectCardAction::Open(path()));
        }
        if !is_localcodepilot_qa_project(&project.path)
            && ui
                .link(if self.hidden_from_recent {
                    "Mostrar nos recentes"
                } else {
                    "Ocultar dos recentes"
                })
                .clicked()
        {
            action = Some(ProjectCardAction::ToggleRecentVisibility(path()));
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> ProjectEnvironment {
        ProjectEnvironment {
            command_count: 1,
            ..ProjectEnvironment::default()
        }
    }

    fn facts(environment: &ProjectEnvironment) -> ProjectFacts<'_> {
        ProjectFacts {
            environment,
            environment_pending: false,
            resolving_plan: false,
            dependency_install: None,
            starting: false,
            running: false,
            failed_process: None,
            application_url: None,
            http: None,
            has_containers: false,
            container_running: false,
            container_url: None,
        }
    }

    #[test]
    fn filters_are_named_with_their_count() {
        assert_eq!(filter_name("Rodando", 1), "Rodando, 1 projeto");
        assert_eq!(filter_name("Parados", 3), "Parados, 3 projetos");
        assert_eq!(filter_name("Com problema", 0), "Com problema, 0 projetos");
    }

    #[test]
    fn running_is_separate_from_http_availability() {
        let environment = ready();
        let healthy = HttpHealthStatus::Healthy {
            status_code: 200,
            response_time_ms: 12,
        };
        let failing = HttpHealthStatus::Attention {
            status_code: 500,
            response_time_ms: 8,
        };
        let mut running = facts(&environment);
        running.running = true;
        running.application_url = Some("http://localhost:3100");
        running.http = Some(&healthy);
        let summary = summarize_project(&running);
        assert_eq!(summary.state, ProjectState::Running);
        assert_eq!(summary.http, Some(HttpAvailability::Responding(200)));
        assert_eq!(
            summary.primary,
            PrimaryAction::OpenSite("http://localhost:3100".into())
        );

        // A server answering 500 is still running, with the site in trouble.
        running.http = Some(&failing);
        let summary = summarize_project(&running);
        assert_eq!(summary.state, ProjectState::Running);
        assert_eq!(summary.http, Some(HttpAvailability::Error(500)));
        assert_eq!(HttpAvailability::Error(500).label(), "Site com erro");

        // An API without a page at the root answers 404: the server is up, nothing failed.
        let missing_page = HttpHealthStatus::Attention {
            status_code: 404,
            response_time_ms: 3,
        };
        running.http = Some(&missing_page);
        let summary = summarize_project(&running);
        assert_eq!(summary.http, Some(HttpAvailability::ServerAnswered(404)));
        assert_eq!(
            HttpAvailability::ServerAnswered(404).label(),
            "Servidor respondeu; página não encontrada"
        );
        assert_eq!(
            HttpAvailability::ServerAnswered(403).label(),
            "Acesso não disponível"
        );

        // Without a check yet, there is no badge.
        running.http = None;
        assert_eq!(summarize_project(&running).http, None);
        running.http = Some(&failing);

        // Without a URL there is no site to report; logs are the main action.
        running.application_url = None;
        let summary = summarize_project(&running);
        assert_eq!(summary.http, None);
        assert_eq!(summary.primary, PrimaryAction::ViewLogs);
    }

    #[test]
    fn containers_report_running_without_claiming_the_site_answers() {
        let environment = ProjectEnvironment::default();
        let mut containers = facts(&environment);
        containers.has_containers = true;
        let stopped = summarize_project(&containers);
        assert_eq!(stopped.state, ProjectState::Stopped);
        assert_eq!(stopped.primary, PrimaryAction::StartWithContainers);
        assert!(stopped.uses_containers);
        containers.container_running = true;
        containers.container_url = Some("http://localhost:18080");
        let running = summarize_project(&containers);
        assert_eq!(running.state, ProjectState::Running);
        // Containers have no HTTP check yet: only the address, no badge.
        assert_eq!(running.http, None);
        assert!(running.uses_containers);
    }

    #[test]
    fn problems_explain_the_reason_and_offer_one_action() {
        let invalid = ProjectEnvironment {
            manifest_errors: vec!["package.json inválido (linha 1)".into()],
            ..ProjectEnvironment::default()
        };
        let summary = summarize_project(&facts(&invalid));
        assert_eq!(summary.state, ProjectState::Problem);
        assert_eq!(
            summary.reason.as_deref(),
            Some("package.json inválido (linha 1). Corrija o arquivo para iniciar este projeto")
        );
        assert_eq!(summary.primary, PrimaryAction::Details);

        let environment = ready();
        let mut crashed = facts(&environment);
        crashed.failed_process = Some("npm run start");
        let summary = summarize_project(&crashed);
        assert_eq!(summary.state, ProjectState::Problem);
        assert_eq!(
            summary.reason.as_deref(),
            Some("npm run start parou com erro")
        );
        assert_eq!(summary.primary, PrimaryAction::ViewLogs);

        let failed = DependencyInstallStatus::Failed {
            error: "exit 1".into(),
            suggestion: String::new(),
        };
        let mut install = facts(&environment);
        install.dependency_install = Some(&failed);
        let summary = summarize_project(&install);
        assert_eq!(summary.state, ProjectState::Problem);
        assert_eq!(summary.primary, PrimaryAction::ViewInstallLogs);

        let mut missing = ready();
        missing.missing_programs = vec!["cargo".into()];
        let summary = summarize_project(&facts(&missing));
        assert_eq!(summary.state, ProjectState::Problem);
        assert_eq!(summary.reason.as_deref(), Some("Instale: cargo"));
    }

    #[test]
    fn stopped_projects_offer_start_or_explain_why_not() {
        let environment = ready();
        let summary = summarize_project(&facts(&environment));
        assert_eq!(summary.state, ProjectState::Stopped);
        assert_eq!(summary.primary, PrimaryAction::Start);
        assert_eq!(summary.reason, None);

        let empty = ProjectEnvironment::default();
        let summary = summarize_project(&facts(&empty));
        assert_eq!(summary.state, ProjectState::Stopped);
        assert_eq!(
            summary.reason.as_deref(),
            Some("Nenhum comando para iniciar foi encontrado")
        );
    }

    #[test]
    fn work_in_progress_is_reported_as_starting() {
        let environment = ready();
        let mut starting = facts(&environment);
        starting.starting = true;
        assert_eq!(summarize_project(&starting).state, ProjectState::Starting);
        let installing = DependencyInstallStatus::Running;
        let mut install = facts(&environment);
        install.dependency_install = Some(&installing);
        let summary = summarize_project(&install);
        assert_eq!(summary.state, ProjectState::Starting);
        assert_eq!(summary.reason.as_deref(), Some("Instalando dependências"));
    }

    #[test]
    fn filters_group_starting_with_running() {
        assert!(ProjectFilter::Running.matches(ProjectState::Starting));
        assert!(ProjectFilter::Running.matches(ProjectState::Running));
        assert!(!ProjectFilter::Running.matches(ProjectState::Stopped));
        assert!(ProjectFilter::Problem.matches(ProjectState::Problem));
        assert!(ProjectFilter::All.matches(ProjectState::Problem));
        for filter in [
            ProjectFilter::All,
            ProjectFilter::Running,
            ProjectFilter::Problem,
            ProjectFilter::Stopped,
        ] {
            assert_eq!(ProjectFilter::from_key(filter.key()), filter);
        }
    }

    #[test]
    fn every_state_uses_the_approved_vocabulary() {
        let labels = [
            ProjectState::Running,
            ProjectState::Starting,
            ProjectState::Stopped,
            ProjectState::Problem,
        ]
        .map(ProjectState::label);
        assert_eq!(labels, ["Rodando", "Iniciando", "Parado", "Com problema"]);
        assert_eq!(
            PrimaryAction::Start.label().as_deref(),
            Some(format!("{PLAY}  Iniciar").as_str())
        );
        assert_eq!(
            PrimaryAction::ViewLogs.label().as_deref(),
            Some(format!("{TERMINAL}  Ver logs").as_str())
        );
        assert_eq!(PrimaryAction::Wait.label(), None);
    }
}
