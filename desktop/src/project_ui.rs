//! Overview and Projects: one summary per project (state, reason, HTTP availability, URL and
//! a single main action) rendered with the same vocabulary everywhere.

use super::*;
use egui_phosphor::regular::{STOP, WARNING_CIRCLE};

#[path = "project_page_ui.rs"]
mod project_page;

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
    /// The output of a failed run of the project's migrations.
    ViewMigrationLogs,
    PrepareEnvironment,
    InstallDependencies,
    /// No command was found: the person tells how the project starts.
    DefineCommand,
    /// A guided fix of the project's database (another port, the user's password, a new data
    /// folder), always confirmed first.
    DatabaseFix(database_ui::DatabaseFix),
    Details,
    /// Something is in progress; no button.
    Wait,
}

impl PrimaryAction {
    fn label(&self) -> Option<String> {
        Some(match self {
            Self::OpenSite(_) => format!("{ARROW_SQUARE_OUT}  Abrir site"),
            Self::Start | Self::StartWithContainers => format!("{PLAY}  Iniciar"),
            Self::ViewLogs | Self::ViewInstallLogs | Self::ViewMigrationLogs => {
                format!("{TERMINAL}  Ver logs")
            }
            Self::PrepareEnvironment => "Preparar ambiente".into(),
            Self::InstallDependencies => "Instalar dependências".into(),
            Self::DefineCommand => "Definir comando de início".into(),
            Self::DatabaseFix(fix) => fix.label().into(),
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
    /// Runs through Containers, where logs live.
    pub uses_containers: bool,
    /// The card can confirm the stop of its containers itself; otherwise "Parar" leads to
    /// Containers to choose.
    pub stops_containers_here: bool,
    /// Shown under "Detalhes técnicos": process, command, exit code, port owner.
    pub technical: Vec<String>,
    /// "Tentar de novo" next to the main action: a start that stopped at the database for a
    /// reason that may pass (the server did not start).
    pub retry: bool,
    /// "Parar" while the project is not running yet: it is compiling (T028.02).
    pub stoppable: bool,
    /// The rule that produced this summary, shown in "Detalhes técnicos" (T034).
    pub rule: DiagnosticRule,
    /// What the app observed for that rule, one fact per line; empty: nothing verified.
    pub evidence: Vec<String>,
}

/// The rules of the card's diagnosis, in the order they are evaluated: the first whose
/// condition holds decides the state, the reason and the action (T034). The order is the
/// precedence between rules that hold at the same time, documented in
/// docs/DETALHES-TECNICOS.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiagnosticRule {
    Running,
    PreparationRunning,
    PreparationFailed,
    DatabasePreparing,
    Building,
    Starting,
    DatabaseFailed,
    MigrationFailed,
    ResolvingPlan,
    Verifying,
    ProcessFailed,
    ManifestInvalid,
    ContainersOnly,
    NoCommand,
    Ready,
    DatabaseStopped,
    NeedsPreparation,
    RequirementProblem,
}

impl DiagnosticRule {
    /// Every rule, in the order of evaluation.
    #[cfg(test)]
    pub(super) const ALL: [Self; 18] = [
        Self::Running,
        Self::PreparationRunning,
        Self::PreparationFailed,
        Self::DatabasePreparing,
        Self::Building,
        Self::Starting,
        Self::DatabaseFailed,
        Self::MigrationFailed,
        Self::ResolvingPlan,
        Self::Verifying,
        Self::ProcessFailed,
        Self::ManifestInvalid,
        Self::ContainersOnly,
        Self::NoCommand,
        Self::Ready,
        Self::DatabaseStopped,
        Self::NeedsPreparation,
        Self::RequirementProblem,
    ];

    /// The name shown after "Regra:".
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Running => "Projeto rodando",
            Self::PreparationRunning => "Preparação em andamento",
            Self::PreparationFailed => "Preparação falhou",
            Self::DatabasePreparing => "Banco de dados em preparo",
            Self::Building => "Compilação em andamento",
            Self::Starting => "Início em andamento",
            Self::DatabaseFailed => "Banco de dados impediu o início",
            Self::MigrationFailed => "Migrations falharam",
            Self::ResolvingPlan => "Consulta de versões em andamento",
            Self::Verifying => "Verificação do projeto em andamento",
            Self::ProcessFailed => "Processo parou com erro",
            Self::ManifestInvalid => "Manifesto ilegível",
            Self::ContainersOnly => "Projeto só com containers",
            Self::NoCommand => "Sem comando de início",
            Self::Ready => "Pronto para iniciar",
            Self::DatabaseStopped => "Banco de dados parado",
            Self::NeedsPreparation => "Falta preparar o ambiente",
            Self::RequirementProblem => "Requisito incompatível ou pendente",
        }
    }
}

/// A process of the project that failed, with what the app knows about why.
pub(super) struct FailedProcess<'a> {
    pub name: &'a str,
    pub command: String,
    pub exit_code: Option<i32>,
    pub failure: Option<&'a ProcessFailure>,
}

/// Everything the summary depends on, gathered from the app state.
pub(super) struct ProjectFacts<'a> {
    pub environment: &'a ProjectEnvironment,
    pub environment_pending: bool,
    pub resolving_plan: bool,
    pub dependency_install: Option<&'a DependencyInstallStatus>,
    /// The preparation running or last run installs the C/C++ compiler (T033.03).
    pub preparing_compiler: bool,
    pub starting: bool,
    /// Steps before the program are running (compile before run, T028.02).
    pub building: bool,
    pub running: bool,
    pub failed_process: Option<FailedProcess<'a>>,
    pub application_url: Option<&'a str>,
    pub http: Option<&'a HttpHealthStatus>,
    pub has_containers: bool,
    pub container_running: bool,
    /// One environment up whose stop reaches a service and passes the Compose checks.
    pub container_stop_here: bool,
    pub container_url: Option<&'a str>,
    /// Languages found in the project files.
    pub languages: &'a [RuntimeKind],
    /// The project's database is being prepared or started: the step to show.
    pub database_step: Option<String>,
    /// The last start stopped at the project's database, why, and the fix it allows.
    pub database_failure: Option<&'a database_ui::DatabaseFailure>,
    /// Shown with "Rodando" after "Criar banco novo".
    pub database_note: Option<&'a str>,
    /// The last run of the project's migrations failed (T027).
    pub migration_failed: bool,
}

/// Why there is nothing to start: a language the app does not start yet is named, so the
/// card does not suggest a command is missing from the project.
fn no_command_reason(languages: &[RuntimeKind], ambiguous: Option<&str>) -> String {
    // C/C++ builds programs (CMake, Meson, Visual Studio): the card speaks of programs (T033).
    if let Some(entries) = ambiguous {
        return if entries.starts_with("C/C++") {
            "Encontramos mais de um programa para iniciar neste projeto. Defina qual comando usar."
        } else {
            "Encontramos mais de uma forma de iniciar este projeto. Defina qual comando usar."
        }
        .into();
    }
    match languages {
        [] => "Nenhum comando para iniciar foi encontrado".into(),
        [RuntimeKind::Cpp] => {
            "Reconhecemos C/C++, mas não encontramos um programa único para iniciar. Defina o comando de início."
                .into()
        }
        languages => {
            let names = languages
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Reconhecemos {names}, mas não encontramos um arquivo ou alvo de entrada. Defina o comando de início."
            )
        }
    }
}

/// The card's diagnosis: the first rule that holds, with what the app observed for it.
pub(super) fn summarize_project(facts: &ProjectFacts<'_>) -> ProjectSummary {
    let mut summary = decide(facts);
    summary.evidence = evidence(summary.rule, facts);
    summary
}

/// What the app observed for a rule. Only facts the app has; a fact it did not check is
/// said to be unverified, never inferred.
fn evidence(rule: DiagnosticRule, facts: &ProjectFacts<'_>) -> Vec<String> {
    let environment = facts.environment;
    match rule {
        DiagnosticRule::Running => [
            facts
                .running
                .then(|| "Processos iniciados pelo app estão rodando.".to_owned()),
            facts
                .container_running
                .then(|| "Containers do projeto estão rodando.".to_owned()),
        ]
        .into_iter()
        .flatten()
        .collect(),
        DiagnosticRule::PreparationRunning => {
            vec!["Uma preparação deste projeto está em andamento.".into()]
        }
        DiagnosticRule::PreparationFailed => match facts.dependency_install {
            Some(DependencyInstallStatus::Failed { error, .. }) => {
                vec![format!("A última preparação terminou com erro: {error}")]
            }
            _ => vec![],
        },
        DiagnosticRule::DatabasePreparing => facts.database_step.iter().cloned().collect(),
        DiagnosticRule::Building => {
            vec!["As etapas de compilação estão em andamento.".into()]
        }
        DiagnosticRule::Starting => vec!["Um processo do projeto está iniciando.".into()],
        DiagnosticRule::DatabaseFailed => facts
            .database_failure
            .map(|failure| failure.reason.clone())
            .into_iter()
            .collect(),
        DiagnosticRule::MigrationFailed => {
            vec!["A última execução das migrations terminou com erro.".into()]
        }
        DiagnosticRule::ResolvingPlan => {
            vec!["O app está consultando as versões para preparar o ambiente.".into()]
        }
        DiagnosticRule::Verifying => {
            vec!["A verificação do projeto ainda não terminou.".into()]
        }
        DiagnosticRule::ProcessFailed => {
            let Some(failed) = &facts.failed_process else {
                return vec![];
            };
            let mut lines = vec![match failed.exit_code {
                Some(code) => format!("{} terminou com o código de saída {code}.", failed.name),
                None => format!("{} terminou; código de saída não verificado.", failed.name),
            }];
            lines.push(match failed.failure {
                Some(ProcessFailure::Exited) | None => {
                    "nenhuma causa específica foi verificada.".into()
                }
                Some(failure) => format!("Causa verificada: {}", failure.sentence()),
            });
            lines
        }
        DiagnosticRule::ManifestInvalid => environment.manifest_errors.clone(),
        DiagnosticRule::ContainersOnly => {
            vec!["Arquivo do Compose encontrado; nenhum comando de processo foi detectado.".into()]
        }
        DiagnosticRule::NoCommand => {
            let mut lines = vec![if facts.languages.is_empty() {
                "Nenhuma linguagem com início automático foi reconhecida.".to_owned()
            } else {
                format!(
                    "Linguagens reconhecidas: {}.",
                    facts
                        .languages
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }];
            if let Some(entries) = &environment.ambiguous_entry {
                lines.push(format!("Entradas possíveis: {entries}."));
            }
            lines
        }
        DiagnosticRule::Ready => vec![format!(
            "{} encontrado(s); requisitos verificados.",
            match environment.command_count {
                1 => "1 comando".to_owned(),
                count => format!("{count} comandos"),
            }
        )],
        DiagnosticRule::DatabaseStopped => environment
            .databases
            .iter()
            .filter(|database| database.not_running())
            .map(|database| format!("{} não está rodando.", database.requirement.engine.label()))
            .collect(),
        DiagnosticRule::NeedsPreparation | DiagnosticRule::RequirementProblem => {
            requirement_evidence(environment)
        }
    }
}

/// What is missing or incompatible in the environment, one fact per line.
fn requirement_evidence(environment: &ProjectEnvironment) -> Vec<String> {
    let mut lines = Vec::new();
    if environment.missing_compiler {
        lines.push("Compilador C/C++: não encontrado.".to_owned());
    }
    for requirement in &environment.runtime_requirements {
        match &requirement.status {
            RequirementStatus::Missing => lines.push(format!(
                "{} {}: não encontrado.",
                requirement.runtime, requirement.version.value
            )),
            RequirementStatus::Incompatible {
                installed_version, ..
            } => lines.push(format!(
                "{} {}: instalado {installed_version}, incompatível.",
                requirement.runtime, requirement.version.value
            )),
            _ => {}
        }
    }
    if !environment.missing_programs.is_empty() {
        lines.push(format!(
            "Programas não encontrados: {}.",
            environment.missing_programs.join(", ")
        ));
    }
    if !environment.missing_dependencies.is_empty() {
        lines.push(format!(
            "Dependências não instaladas: {}.",
            environment.missing_dependencies.join(", ")
        ));
    }
    if !environment.missing_php_extensions.is_empty() {
        lines.push(format!(
            "Extensões PHP ausentes: {}.",
            environment.missing_php_extensions.join(", ")
        ));
    }
    lines.extend(
        environment
            .databases
            .iter()
            .filter_map(DatabaseStatus::rejection),
    );
    lines
}

/// The rules in order: the first that holds decides.
fn decide(facts: &ProjectFacts<'_>) -> ProjectSummary {
    let environment = facts.environment;
    let summary = |rule, state, reason: Option<String>, primary| ProjectSummary {
        state,
        reason,
        http: None,
        url: None,
        primary,
        uses_containers: false,
        stops_containers_here: false,
        technical: vec![],
        retry: false,
        stoppable: false,
        rule,
        evidence: vec![],
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
            reason: facts.database_note.map(str::to_owned),
            http: url.as_ref().and(http),
            primary: url
                .clone()
                .map_or(PrimaryAction::ViewLogs, PrimaryAction::OpenSite),
            url,
            uses_containers: !facts.running,
            stops_containers_here: !facts.running && facts.container_stop_here,
            technical: vec![],
            retry: false,
            stoppable: false,
            rule: DiagnosticRule::Running,
            evidence: vec![],
        };
    }
    match facts.dependency_install {
        Some(DependencyInstallStatus::Running | DependencyInstallStatus::Cancelling) => {
            return summary(
                DiagnosticRule::PreparationRunning,
                ProjectState::Starting,
                Some(if facts.preparing_compiler {
                    "Preparando o compilador C/C++".into()
                } else {
                    "Instalando dependências".into()
                }),
                PrimaryAction::ViewInstallLogs,
            );
        }
        Some(DependencyInstallStatus::Failed { .. }) => {
            return summary(
                DiagnosticRule::PreparationFailed,
                ProjectState::Problem,
                Some(if facts.preparing_compiler {
                    super::COMPILER_FAILED.into()
                } else {
                    "A instalação das dependências falhou".into()
                }),
                PrimaryAction::ViewInstallLogs,
            );
        }
        _ => {}
    }
    if let Some(step) = &facts.database_step {
        return summary(
            DiagnosticRule::DatabasePreparing,
            ProjectState::Starting,
            Some(step.clone()),
            PrimaryAction::Wait,
        );
    }
    if facts.building {
        return ProjectSummary {
            stoppable: true,
            ..summary(
                DiagnosticRule::Building,
                ProjectState::Starting,
                Some(build_steps::BUILDING.into()),
                PrimaryAction::ViewLogs,
            )
        };
    }
    if facts.starting {
        return summary(
            DiagnosticRule::Starting,
            ProjectState::Starting,
            Some("Iniciando os serviços".into()),
            PrimaryAction::ViewLogs,
        );
    }
    if let Some(failure) = facts.database_failure {
        let mut problem = summary(
            DiagnosticRule::DatabaseFailed,
            ProjectState::Problem,
            Some(failure.reason.clone()),
            failure
                .fix
                .map_or(PrimaryAction::Details, PrimaryAction::DatabaseFix),
        );
        problem.retry = failure.retry;
        return problem;
    }
    if facts.migration_failed {
        let mut problem = summary(
            DiagnosticRule::MigrationFailed,
            ProjectState::Problem,
            Some(migration_ui::FAILED.into()),
            PrimaryAction::ViewMigrationLogs,
        );
        problem.retry = true;
        return problem;
    }
    if facts.resolving_plan {
        return summary(
            DiagnosticRule::ResolvingPlan,
            ProjectState::Starting,
            Some("Consultando as versões disponíveis".into()),
            PrimaryAction::Wait,
        );
    }
    if facts.environment_pending {
        return summary(
            DiagnosticRule::Verifying,
            ProjectState::Stopped,
            Some("Verificando o projeto".into()),
            PrimaryAction::Wait,
        );
    }
    if let Some(failed) = &facts.failed_process {
        // Without a known cause, the card says so and points to the logs (T034).
        let reason = match failed.failure {
            Some(ProcessFailure::Exited) | None => format!(
                "{} parou com erro. Veja os logs para encontrar a causa.",
                failed.name
            ),
            Some(failure) => failure.sentence(),
        };
        let mut problem = summary(
            DiagnosticRule::ProcessFailed,
            ProjectState::Problem,
            Some(reason),
            PrimaryAction::ViewLogs,
        );
        problem.technical = vec![
            format!("Processo: {}", failed.name),
            format!("Comando: {}", failed.command),
        ];
        if let Some(code) = failed.exit_code {
            problem.technical.push(format!("Código de saída: {code}"));
        }
        problem.technical.extend(
            failed
                .failure
                .map(ProcessFailure::details)
                .unwrap_or_default(),
        );
        return problem;
    }
    if let Some(error) = environment.manifest_errors.first() {
        return summary(
            DiagnosticRule::ManifestInvalid,
            ProjectState::Problem,
            Some(format!(
                "{error}. Corrija o arquivo indicado e atualize o projeto."
            )),
            PrimaryAction::Details,
        );
    }
    if facts.has_containers && environment.command_count == 0 {
        let mut stopped = summary(
            DiagnosticRule::ContainersOnly,
            ProjectState::Stopped,
            Some("Roda com containers".into()),
            PrimaryAction::StartWithContainers,
        );
        stopped.uses_containers = true;
        return stopped;
    }
    if environment.command_count == 0 {
        return summary(
            DiagnosticRule::NoCommand,
            ProjectState::Stopped,
            Some(no_command_reason(
                facts.languages,
                environment.ambiguous_entry.as_deref(),
            )),
            PrimaryAction::DefineCommand,
        );
    }
    // A SQLite file that does not exist yet is created when the project starts; only a
    // database server that does not answer keeps the project from starting.
    if environment.is_ready()
        && !environment
            .databases
            .iter()
            .any(|database| database.not_running() || database.rejection().is_some())
    {
        return summary(
            DiagnosticRule::Ready,
            ProjectState::Stopped,
            None,
            PrimaryAction::Start,
        );
    }
    // A database server that is not running, before any start, is not a failure: the project
    // is stopped and says why. A refused login or a missing database is a problem.
    if environment.is_ready()
        && environment
            .databases
            .iter()
            .all(|database| database.rejection().is_none())
        && environment
            .databases
            .iter()
            .any(DatabaseStatus::not_running)
    {
        // "Iniciar" prepares or starts the database when the app can (with a confirmation the
        // first time); otherwise the person reads why in "Detalhes".
        let can_prepare = environment
            .databases
            .iter()
            .filter(|database| database.not_running())
            .all(DatabaseStatus::app_can_prepare);
        // Named engine when the app cannot prepare it on this system (T029, CA-6).
        let reason = environment
            .databases
            .iter()
            .find_map(DatabaseStatus::not_preparable_here)
            .filter(|_| !can_prepare)
            .or_else(|| {
                environment
                    .databases
                    .iter()
                    .find_map(DatabaseStatus::container_start_note)
                    .filter(|_| can_prepare)
            })
            .unwrap_or_else(|| "O banco de dados deste projeto não está rodando.".into());
        return summary(
            DiagnosticRule::DatabaseStopped,
            ProjectState::Stopped,
            Some(reason),
            if can_prepare {
                PrimaryAction::Start
            } else {
                PrimaryAction::Details
            },
        );
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
    // A fresh clone without its dependencies, or a machine without the language, is not
    // a failure (#101): the project is stopped and the card shows what to prepare.
    if only_needs_preparation(environment) {
        let reason = if only_dependencies_missing(environment) {
            format!(
                "Faltam dependências de {} para preparar este projeto.",
                environment.missing_dependencies.join(", ")
            )
        } else {
            reason
        };
        return summary(
            DiagnosticRule::NeedsPreparation,
            ProjectState::Stopped,
            Some(reason),
            primary,
        );
    }
    summary(
        DiagnosticRule::RequirementProblem,
        ProjectState::Problem,
        Some(reason),
        primary,
    )
}

fn language_missing(environment: &ProjectEnvironment) -> bool {
    environment.missing_compiler
        || !environment.missing_programs.is_empty()
        || environment
            .runtime_requirements
            .iter()
            .any(|requirement| matches!(requirement.status, RequirementStatus::Missing))
}

fn only_dependencies_missing(environment: &ProjectEnvironment) -> bool {
    !environment.missing_dependencies.is_empty() && !language_missing(environment)
}

/// Only things a first preparation solves are missing: the project's dependencies or a
/// language that is not installed. An incompatible version, missing PHP extensions or a
/// database that refuses the project still need attention, so they stay "Com problema".
fn only_needs_preparation(environment: &ProjectEnvironment) -> bool {
    let incompatible = environment
        .runtime_requirements
        .iter()
        .any(|requirement| matches!(requirement.status, RequirementStatus::Incompatible { .. }));
    // A server that is not running is something to start, not a failure; a refused login or
    // a missing database is.
    let database_down = environment
        .databases
        .iter()
        .any(|database| database.rejection().is_some());
    !incompatible
        && !database_down
        && environment.missing_php_extensions.is_empty()
        && (language_missing(environment) || !environment.missing_dependencies.is_empty())
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

/// Transient view state: whether the scanned folders are listed.
#[derive(Default)]
pub(super) struct ProjectUiState {
    /// The list of scanned folders is open on Projects.
    pub(super) show_roots: bool,
}

/// Projects with their summaries, in display order.
type Summaries = Vec<(Project, ProjectSummary)>;

impl LocalCodePilot {
    /// For each project whose name another one also has, the part of its folder that tells
    /// them apart (#100). Projects with a name of their own are not in the map.
    pub(super) fn distinguishing_folders(&self) -> HashMap<PathBuf, String> {
        distinguishing_folders(
            self.projects
                .iter()
                .map(|project| (project.name.as_str(), project.path.as_path())),
        )
    }

    /// "Exportar diagnóstico": the person picks where to save; Cancel writes nothing. The
    /// file holds only the fields of `diagnostic_export::Report`, sanitized (T034).
    fn export_diagnostic(&mut self, path: &Path) {
        use crate::diagnostic_export::{self, Command, Database, Report};
        let Some(project) = self
            .projects
            .iter()
            .find(|project| project.path == path)
            .cloned()
        else {
            return;
        };
        let summary = self.project_summary(&project);
        let fallback = ProjectEnvironment::default();
        let environment = self
            .project_environments
            .get(&project.path)
            .unwrap_or(&fallback);
        let commands = self
            .managed
            .processes
            .iter()
            .filter(|process| process.project_path == project.path)
            .map(|process| Command {
                name: process.name.clone(),
                program: process.program.clone(),
                args: process.args.clone(),
                folder: process.working_directory.clone(),
            })
            .collect();
        let mut requirements = environment
            .runtime_requirements
            .iter()
            .map(|requirement| {
                let status = match &requirement.status {
                    RequirementStatus::Available { version, .. } => format!("instalado: {version}"),
                    RequirementStatus::Missing => "não encontrado".into(),
                    RequirementStatus::Incompatible {
                        installed_version, ..
                    } => format!("incompatível: {installed_version}"),
                    RequirementStatus::Pending => diagnostic_export::NOT_VERIFIED.into(),
                };
                format!(
                    "{} {}: {status}",
                    requirement.runtime, requirement.version.value
                )
            })
            .collect::<Vec<_>>();
        if environment.missing_compiler {
            requirements.push("Compilador C/C++: não encontrado".into());
        }
        requirements.extend(
            environment
                .missing_programs
                .iter()
                .map(|program| format!("{program}: não encontrado")),
        );
        let databases = environment
            .databases
            .iter()
            .map(|database| Database {
                engine: database.requirement.engine.label().into(),
                port: database.requirement.port,
                local: Some(database.requirement.host.as_deref().is_none_or(|host| {
                    matches!(
                        host.to_ascii_lowercase().as_str(),
                        "localhost" | "127.0.0.1" | "::1"
                    )
                })),
                running: if database.not_running() {
                    Some(false)
                } else {
                    database.available
                },
            })
            .collect();
        let technologies = project
            .technologies
            .iter()
            .map(ToString::to_string)
            .collect();
        let report = Report {
            project_name: &project.name,
            project_path: &project.path,
            technologies,
            state: summary.state.label(),
            reason: summary.reason.as_deref(),
            rule: summary.rule.name(),
            evidence: &summary.evidence,
            technical: &summary.technical,
            commands,
            requirements,
            databases,
        };
        let home =
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
        let text = diagnostic_export::render(&report, home.as_deref());
        let Some(destination) = rfd::FileDialog::new()
            .set_title("Exportar diagnóstico")
            .set_file_name(diagnostic_export::file_name(&project.name))
            .add_filter("JSON", &["json"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&destination, text.as_bytes()) {
            Ok(()) => self
                .notify_important("Diagnóstico salvo. Confira o arquivo antes de compartilhá-lo."),
            Err(_) => self.notify_important(
                "Não foi possível salvar o diagnóstico. Escolha outro local e tente de novo.",
            ),
        }
    }

    pub(super) fn project_summary(&self, project: &Project) -> ProjectSummary {
        let fallback = ProjectEnvironment::default();
        let environment = self
            .project_environments
            .get(&project.path)
            .unwrap_or(&fallback);
        let processes = self
            .managed
            .processes
            .iter()
            .filter(|process| process.project_path == project.path)
            .collect::<Vec<_>>();
        let application_url = processes
            .iter()
            .filter(|process| process.state == ProcessState::Running)
            .filter(|process| process_exposes_application_url(project, process))
            .find_map(|process| {
                let running = self.managed.running.get(&process.id)?;
                if running.finished {
                    return None;
                }
                running
                    .urls
                    .last()
                    .map(|url| url.address().to_owned())
                    .or_else(|| {
                        self.managed
                            .assigned_ports
                            .get(&process.id)
                            .map(|port| format!("http://localhost:{}", port.value()))
                    })
            });
        let http = processes
            .iter()
            .filter_map(|process| self.managed.running.get(&process.id))
            .find_map(|running| running.http_health.as_ref().map(|health| &health.status));
        let (container_running, container_url) = self.project_container_status(&project.path);
        let dependency_install = self
            .dependency_installs
            .get(&project.path)
            .map(|session| &session.status);
        let mut summary = summarize_project(&ProjectFacts {
            environment,
            environment_pending: self.discovery.is_some()
                && !self.project_environments.contains_key(&project.path),
            resolving_plan: self
                .environment_plan_request
                .as_ref()
                .is_some_and(|(path, _)| path == &project.path),
            dependency_install,
            preparing_compiler: self
                .dependency_installs
                .get(&project.path)
                .is_some_and(|session| super::prepares_compiler(&session.plan)),
            starting: processes
                .iter()
                .any(|process| process.state == ProcessState::Starting),
            building: processes
                .iter()
                .any(|process| self.managed.builds.contains_key(&process.id)),
            running: processes
                .iter()
                .any(|process| process.state == ProcessState::Running),
            failed_process: processes
                .iter()
                .find(|process| process.state == ProcessState::Failed)
                .map(|process| FailedProcess {
                    name: &process.name,
                    command: process.command_line(),
                    exit_code: process.exit_code,
                    failure: self.managed.failures.get(&process.id),
                }),
            application_url: application_url.as_deref(),
            http,
            has_containers: project.technologies.contains(&TechnologyKind::Containers),
            container_running,
            container_stop_here: container_running
                && self.card_container_stop(&project.path).is_some(),
            container_url: container_url.as_deref(),
            languages: &project.runtimes,
            database_step: self
                .database
                .jobs
                .get(&project.path)
                .filter(|job| job.visible)
                .map(|job| job.step.text(job.engine))
                .or_else(|| self.migrations.step(&project.path)),
            database_failure: self.database.failures.get(&project.path),
            database_note: self.database.notes.get(&project.path).map(String::as_str),
            migration_failed: self.migrations.failed.contains(&project.path),
        });
        // A Compose protection that blocks the start is the reason; no Iniciar is offered
        // until it is solved (T038, CA-5 and CA-6).
        if summary.primary == PrimaryAction::StartWithContainers
            && let container_ui::CardStart::Blocked(reason) =
                self.card_container_start(&project.path)
        {
            summary.reason = Some(reason.to_owned());
            summary.primary = PrimaryAction::Details;
        }
        summary
    }

    /// What a project runs, as the search sees it (T038.03): each process with its port and
    /// each container the last check saw, with a label that says what was found.
    fn search_candidates(&self, project: &Project) -> Vec<(String, Vec<String>)> {
        let mut candidates = Vec::new();
        for process in self
            .managed
            .processes
            .iter()
            .filter(|process| process.project_path == project.path)
        {
            let port = self
                .managed
                .assigned_ports
                .get(&process.id)
                .copied()
                .or(process.expected_port)
                .map(|port| port.value().to_string());
            let label = match &port {
                Some(port) => format!("serviço {} · porta {port}", process.name),
                None => format!("serviço {}", process.name),
            };
            candidates.push((
                label,
                [Some(process.name.clone()), port]
                    .into_iter()
                    .flatten()
                    .collect(),
            ));
        }
        for container in self.project_containers(&project.path) {
            let label = match &container.service {
                Some(service) => format!("container {} · serviço {service}", container.name),
                None => format!("container {}", container.name),
            };
            let words = [
                Some(container.name.clone()),
                container.service.clone(),
                Some(container.image.clone()),
                Some(container.ports.clone()),
            ];
            candidates.push((label, words.into_iter().flatten().collect()));
        }
        candidates
    }

    /// Why a project shows up in a search that is not its name or folder, if it does.
    fn search_hit(&self, project: &Project, query: &str) -> Option<String> {
        if query.is_empty() {
            return None;
        }
        service_hit(query, &self.search_candidates(project))
    }

    fn summaries(&self) -> Summaries {
        let query = self.search.trim().to_lowercase();
        let mut summaries = self
            .projects
            .iter()
            .filter(|project| {
                query.is_empty()
                    || name_matches(project, &query)
                    || self.search_hit(project, &query).is_some()
            })
            .map(|project| (project.clone(), self.project_summary(project)))
            .collect::<Vec<_>>();
        // A stable order (T038, CA-4): favorites first, then by name, so a project does not
        // move when it starts, stops or is opened.
        summaries.sort_by_cached_key(|(project, _)| {
            (
                !self.is_favorite(&project.path),
                project.name.to_lowercase(),
                project.path.clone(),
            )
        });
        summaries
    }

    /// Turns a card or row action into navigation or a process operation.
    pub(super) fn apply_project_action(&mut self, ctx: &egui::Context, action: ProjectCardAction) {
        match action {
            // The logs and the containers are on the project's page (T038.02).
            ProjectCardAction::ViewDetails(path) => {
                if self.page != Page::Project {
                    self.open_project_page(ctx, path.clone());
                }
                if let Some(id) = self.project_log_target(&path) {
                    self.managed.terminal = Some(id);
                }
            }
            // Iniciar of a Compose project: the same checks as the environment's Iniciar.
            ProjectCardAction::StartContainers(path) => match self.card_container_start(&path) {
                container_ui::CardStart::Run(command) => self.confirm_container_command(command),
                container_ui::CardStart::StartEngine {
                    choices,
                    context,
                    resume,
                } => self.request_engine_start(choices, Some(context), resume, None),
                container_ui::CardStart::Blocked(_) | container_ui::CardStart::Elsewhere => {
                    if self.page != Page::Project {
                        self.open_project_page(ctx, path.clone());
                    }
                    self.reveal_containers_of(ctx, &path);
                }
            },
            // The environment's own Iniciar is under O que roda, with the Compose checks.
            ProjectCardAction::ViewContainers(path) => {
                if self.page != Page::Project {
                    self.open_project_page(ctx, path.clone());
                }
                self.reveal_containers_of(ctx, &path);
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
            ProjectCardAction::Start(path) => {
                // A note from the last start (a new data folder) is about that start only.
                self.database.notes.remove(&path);
                self.start_project(&path);
            }
            ProjectCardAction::DatabaseFix(path, fix) => self.ask_database_fix(&path, fix),
            ProjectCardAction::StopDatabase(path) => self.stop_project_database(&path),
            ProjectCardAction::TestRemoteDatabase(path) => self.test_remote_database(&path),
            ProjectCardAction::Stop(path) => {
                // The database server may serve other projects: it keeps running (T038, CA-9).
                let database_running =
                    self.project_environments
                        .get(&path)
                        .is_some_and(|environment| {
                            environment.databases.iter().any(|status| {
                                status
                                    .managed
                                    .as_ref()
                                    .is_some_and(|managed| managed.running)
                            })
                        });
                self.stop_project(&path);
                if database_running {
                    self.notify("Projeto parado. O banco de dados continua rodando.");
                }
            }
            ProjectCardAction::StopContainers(path) => {
                self.touch_project(&path);
                match self.card_container_stop(&path) {
                    // Same confirmation as in Containers, with what the stop reaches.
                    Some(command) => self.confirm_container_command(command),
                    None => self.choose_containers_to_stop(ctx, &path),
                }
            }
            ProjectCardAction::PrepareEnvironment(path) => {
                self.request_environment_plan(&path, ctx.clone());
            }
            ProjectCardAction::InstallDependencies(path) => {
                self.install_dependencies(&path, ctx.clone());
            }
            ProjectCardAction::ViewDependencyLogs(path) => {
                self.dependency_terminal = Some(path);
            }
            ProjectCardAction::ViewMigrationLogs(path) => {
                self.migrations.log_window = Some(path);
            }
            ProjectCardAction::DefineCommand(path) => self.open_command_dialog(&path, None),
            ProjectCardAction::EditCommand(path, name) => {
                self.open_command_dialog(&path, Some(&name));
            }
            ProjectCardAction::RemoveCommand(path, name) => {
                self.remove_custom_command(&path, &name);
            }
            ProjectCardAction::PrepareLaravelMigration(path) => {
                self.prepare_laravel_migration(&path);
            }
            ProjectCardAction::ToggleFavorite(path) => self.toggle_favorite(&path),
            ProjectCardAction::ToggleRecentVisibility(path) => {
                self.toggle_recent_visibility(&path);
            }
            ProjectCardAction::ExportDiagnostic(path) => self.export_diagnostic(&path),
            // Everything about one project is on its page (T038.01).
            ProjectCardAction::OpenProject(path) => {
                self.open_project_page(ctx, path);
            }
        }
    }

    /// Projetos with no project yet: still looking, the search failed, or nothing found.
    fn projects_empty_state(&mut self, ui: &mut egui::Ui) {
        if self.discovery.is_some() {
            components::loading_state(
                ui,
                "Procurando projetos...",
                "Estamos examinando as pastas mais comuns da sua máquina.",
            );
            return;
        }
        // The search failed: say why instead of "Nenhum projeto encontrado" (T036).
        if let Some(error) = self.discovery_error.clone() {
            if components::error_state(
                ui,
                "Não foi possível concluir a varredura",
                &error,
                "Tentar de novo",
            ) {
                let ctx = ui.ctx().clone();
                self.refresh_projects(&ctx);
            }
            return;
        }
        let add = format!("{PLUS}  Adicionar pasta");
        if components::empty_state(
            ui,
            "Nenhum projeto encontrado",
            "Escolha a pasta onde ficam seus projetos. O LocalCodePilot encontra e organiza todos eles.",
            Some(&add),
        ) && let Some(path) = rfd::FileDialog::new()
            .set_title("Escolha uma pasta com projetos")
            .pick_folder()
        {
            let ctx = ui.ctx().clone();
            self.add_scan_root(&ctx, path);
        }
    }

    /// Projetos, the start page (T038.02): the search, three filters and one list in rows,
    /// with a header like a table. The order stays put while states change: favorites
    /// first, then by name.
    pub(super) fn project_catalog(&mut self, ui: &mut egui::Ui) {
        if self.projects.is_empty() {
            self.projects_empty_state(ui);
            return;
        }
        let summaries = self.summaries();
        let folders = self.distinguishing_folders();
        let mut filter = ProjectFilter::from_key(&self.workspace_state.project_filter);
        let old_filter = filter;
        ui.horizontal_wrapped(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Buscar projeto, serviço, porta ou container")
                    .desired_width(ui.available_width().min(320.0)),
            );
            name_search_field(
                &search,
                "Buscar projeto, serviço, porta ou container",
                &self.search,
            );
            ui.add_space(8.0);
            // A filter narrows the list; clicking it again shows every project.
            for (option, label) in [
                (ProjectFilter::Problem, "Com problema"),
                (ProjectFilter::Running, "Rodando"),
                (ProjectFilter::Stopped, "Parado"),
            ] {
                let selected = filter == option;
                let response = ui.add(egui::Button::selectable(selected, label).corner_radius(14));
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::Button,
                        true,
                        selected,
                        format!("Filtrar: {label}"),
                    )
                });
                if response.clicked() {
                    filter = if selected { ProjectFilter::All } else { option };
                }
            }
        });
        if filter != old_filter {
            self.workspace_state.project_filter = filter.key().to_owned();
            self.workspace_dirty = true;
        }
        ui.add_space(10.0);

        let visible = summaries
            .iter()
            .filter(|(_, summary)| filter.matches(summary.state))
            .collect::<Vec<_>>();
        if visible.is_empty() {
            let text = if self.search.trim().is_empty() {
                "Nenhum projeto neste filtro.".to_owned()
            } else if let Some(engine) = self.engine_not_running() {
                // A stopped engine shows no containers: say so instead of "nothing exists".
                format!(
                    "Nada encontrado. O {engine} não está rodando, então os containers dele não entram na busca."
                )
            } else {
                "Nenhum projeto, serviço, porta ou container encontrado.".to_owned()
            };
            empty_hint(ui, &text);
            return;
        }
        let query = self.search.trim().to_lowercase();

        let mut action = None;
        Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(Margin::symmetric(14, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let actions = visible
                    .iter()
                    .map(|(_, summary)| row_actions_width(ui, summary))
                    .fold(0.0, f32::max);
                let any_address = visible.iter().any(|(_, summary)| summary.url.is_some());
                let columns = RowColumns::for_width(
                    ui.available_width(),
                    actions,
                    any_address,
                    ui.spacing().item_spacing.x,
                );
                if columns.wide {
                    table_header(ui, columns);
                }
                for (index, (project, summary)) in visible.iter().enumerate() {
                    if index > 0 || columns.wide {
                        ui.separator();
                    }
                    let folder = folders.get(&project.path).map(String::as_str);
                    // What matched, when it was not the name or the folder.
                    let found = (!name_matches(project, &query))
                        .then(|| self.search_hit(project, &query))
                        .flatten();
                    if let Some(selected) =
                        project_table_row(ui, project, summary, folder, found.as_deref(), columns)
                    {
                        action = Some(selected);
                    }
                }
            });
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
        PrimaryAction::StartWithContainers => ProjectCardAction::StartContainers(path),
        PrimaryAction::ViewLogs => logs_action(project, summary),
        PrimaryAction::ViewInstallLogs => ProjectCardAction::ViewDependencyLogs(path),
        PrimaryAction::ViewMigrationLogs => ProjectCardAction::ViewMigrationLogs(path),
        PrimaryAction::PrepareEnvironment => ProjectCardAction::PrepareEnvironment(path),
        PrimaryAction::InstallDependencies => ProjectCardAction::InstallDependencies(path),
        PrimaryAction::DefineCommand => ProjectCardAction::DefineCommand(path),
        PrimaryAction::DatabaseFix(fix) => ProjectCardAction::DatabaseFix(path, *fix),
        PrimaryAction::Details | PrimaryAction::Wait => ProjectCardAction::OpenProject(path),
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
        ProjectCardAction::StopContainers(project.path.clone())
    } else {
        ProjectCardAction::Stop(project.path.clone())
    }
}

use super::components::empty_hint;

fn state_badge(ui: &mut egui::Ui, summary: &ProjectSummary) {
    let state = summary.state;
    let icon = if state == ProjectState::Problem {
        WARNING_CIRCLE
    } else {
        CIRCLE
    };
    ui.label(
        RichText::new(icon)
            .color(state.color())
            .size(theme::TEXT_DETAIL),
    );
    ui.label(
        RichText::new(state.label())
            .color(state.color())
            .strong()
            .size(theme::TEXT_BODY),
    );
}

fn http_badge(ui: &mut egui::Ui, http: HttpAvailability) {
    ui.label(
        RichText::new(http.label())
            .color(http.color())
            .size(theme::TEXT_DETAIL),
    )
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
                    .size(theme::TEXT_DETAIL),
            );
        }
        ui.label(
            RichText::new(short_url(url))
                .monospace()
                .size(theme::TEXT_DETAIL),
        );
    });
}

/// When the card cannot state what a stop of its containers reaches, the button says it
/// leads to Containers.
fn stop_label(summary: &ProjectSummary) -> String {
    if summary.uses_containers && !summary.stops_containers_here {
        format!("{STOP}  Escolher o que parar")
    } else {
        format!("{STOP}  Parar")
    }
}

fn short_url(url: &str) -> &str {
    url.trim_start_matches("http://")
        .trim_start_matches("https://")
}

/// Main action as a filled button. Returns true when clicked.
/// In a list, `of` names the project, so a screen reader says "Iniciar loja-web".
fn primary_button(ui: &mut egui::Ui, action: &PrimaryAction, width: f32, of: Option<&str>) -> bool {
    let Some(label) = action.label() else {
        ui.spinner();
        return false;
    };
    announced(
        ui.add(
            egui::Button::new(RichText::new(&label).color(Color32::WHITE).strong())
                .fill(theme::PRIMARY_FILL)
                .min_size(egui::vec2(width, 32.0)),
        ),
        egui::Role::Button,
        &spoken(&label, of),
    )
    .clicked()
}

/// A button's words, followed by the project's name when the button is in a list.
fn spoken(label: &str, of: Option<&str>) -> String {
    match of {
        Some(project) => format!("{} {project}", words(label)),
        None => label.to_owned(),
    }
}

/// After a failure (a port in use, a server that closed), the project can start again once
/// the cause is solved, without leaving the card.
fn can_retry(summary: &ProjectSummary) -> bool {
    summary.state == ProjectState::Problem
        && (matches!(summary.primary, PrimaryAction::ViewLogs) || summary.retry)
        && !summary.uses_containers
}

fn retry_button(ui: &mut egui::Ui) -> bool {
    let label = format!("{PLAY}  Tentar de novo");
    announced(ui.button(&label), egui::Role::Button, &label).clicked()
}

fn stop_button(ui: &mut egui::Ui, summary: &ProjectSummary, of: Option<&str>) -> bool {
    let label = stop_label(summary);
    announced(
        ui.button(RichText::new(&label).color(theme::DANGER)),
        egui::Role::Button,
        &spoken(&label, of),
    )
    .clicked()
}

/// Column widths of the project list (T038.02). Below 760 px each row stacks instead.
#[derive(Clone, Copy, Debug)]
struct RowColumns {
    wide: bool,
    name: f32,
    state: f32,
    address: f32,
    actions: f32,
}

impl RowColumns {
    /// The actions get the width their longest buttons need first, so "Instalar
    /// dependências" and "Detalhes" are never cut (T039.02, CA-3). The address column is
    /// narrow when no visible row has an address (CA-6). Name and state share the rest; when
    /// they would be too narrow, each row stacks its parts instead.
    fn for_width(width: f32, actions: f32, any_address: bool, spacing: f32) -> Self {
        let address = if any_address {
            (width * 0.15).clamp(110.0, 180.0)
        } else {
            24.0
        };
        let rest = width - actions - address - spacing * 3.0;
        let name = (rest * 0.42).max(0.0);
        Self {
            wide: width >= 760.0 && rest >= 320.0,
            name,
            state: (rest - name).max(0.0),
            address,
            actions,
        }
    }
}

/// The width the actions of a row need: its main action (or Parar) and "Detalhes".
fn row_actions_width(ui: &egui::Ui, summary: &ProjectSummary) -> f32 {
    let main = match summary.state {
        ProjectState::Running => Some(stop_label(summary)),
        ProjectState::Starting if summary.stoppable => Some(stop_label(summary)),
        ProjectState::Starting => None,
        ProjectState::Stopped | ProjectState::Problem => summary
            .primary
            .label()
            .filter(|_| !matches!(summary.primary, PrimaryAction::Details)),
    };
    let spacing = ui.spacing().item_spacing.x;
    // Bold text on the main button is a little wider than the measure: keep a margin.
    let main = main.map_or(24.0, |label| super::button_width(ui, &label) + 12.0);
    main + spacing + super::button_width(ui, "Detalhes") + 4.0
}

/// A fixed-width column of a row, top-aligned.
fn row_column<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(egui::vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_min_width(width);
        ui.set_max_width(width);
        add(ui)
    })
    .inner
}

/// The column titles of the list, like the head of a table.
fn table_header(ui: &mut egui::Ui, columns: RowColumns) {
    let title = |ui: &mut egui::Ui, text: &str| {
        ui.label(
            RichText::new(text)
                .color(theme::MUTED)
                .strong()
                .size(theme::TEXT_DETAIL),
        );
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        row_column(ui, columns.name, |ui| title(ui, "Projeto"));
        row_column(ui, columns.state, |ui| title(ui, "Estado"));
        // A narrow address column has no room for its title, and nothing to show.
        row_column(ui, columns.address, |ui| {
            if columns.address > 60.0 {
                title(ui, "Endereço");
            }
        });
        row_column(ui, columns.actions, |ui| title(ui, "Ação"));
    });
}

/// What the project is, in one neutral label: the stack, else the language.
fn project_type(project: &Project) -> Option<String> {
    project
        .shown_technologies()
        .find(|technology| *technology != TechnologyKind::Containers)
        .map(|technology| technology.to_string())
        .or_else(|| project.runtimes.first().map(ToString::to_string))
        .or_else(|| {
            project
                .technologies
                .contains(&TechnologyKind::Containers)
                .then(|| "Containers".to_owned())
        })
}

/// The name (a link to the project's page), the type and, for same-name projects, the
/// folder that tells them apart.
fn name_cell(
    ui: &mut egui::Ui,
    project: &Project,
    folder: Option<&str>,
    found: Option<&str>,
    action: &mut Option<ProjectCardAction>,
) {
    ui.horizontal(|ui| {
        if project_name_link(ui, &project.name, theme::TEXT_BODY + 1.0, &project.path) {
            *action = Some(ProjectCardAction::OpenProject(project.path.clone()));
        }
        if let Some(kind) = project_type(project) {
            ui.label(
                RichText::new(kind)
                    .color(theme::MUTED)
                    .size(theme::TEXT_DETAIL),
            );
        }
    });
    folder_line(ui, &project.path, folder);
    if let Some(found) = found {
        ui.label(
            RichText::new(format!("Encontrado: {found}"))
                .color(theme::PRIMARY)
                .size(theme::TEXT_DETAIL),
        );
    }
}

/// The state in words (with its icon), the site check and the reason in up to 2 lines.
fn state_cell(ui: &mut egui::Ui, summary: &ProjectSummary) {
    ui.horizontal_wrapped(|ui| {
        state_badge(ui, summary);
        if let Some(http) = summary.http {
            http_badge(ui, http);
        }
    });
    // A problem's cause is read first: normal text, not the grey of a hint (CA-6).
    if let Some(reason) = &summary.reason {
        reason_label(ui, reason, summary.state == ProjectState::Problem);
    }
}

/// The address: a link that opens the site while the project runs.
fn address_cell(ui: &mut egui::Ui, summary: &ProjectSummary) -> Option<ProjectCardAction> {
    let Some(url) = &summary.url else {
        ui.label(RichText::new("—").color(theme::MUTED));
        return None;
    };
    let text = short_url(url);
    if summary.state != ProjectState::Running {
        ui.label(
            RichText::new(text)
                .monospace()
                .color(theme::MUTED)
                .size(theme::TEXT_DETAIL),
        );
        return None;
    }
    let mut response = announced(
        ui.link(RichText::new(text).monospace().size(theme::TEXT_DETAIL)),
        egui::Role::Link,
        &format!("Abrir site {text}"),
    );
    if summary.uses_containers {
        response = response.on_hover_text("Endereço provável, pelas portas publicadas");
    }
    response
        .clicked()
        .then(|| ProjectCardAction::OpenUrl(url.clone()))
}

/// The one action for the state (T038, CA-6): Parar while it runs; while it starts, only
/// Parar for a build that can be interrupted; otherwise the main action of the summary.
/// Then "Detalhes", which opens the project's page.
fn actions_cell(
    ui: &mut egui::Ui,
    project: &Project,
    summary: &ProjectSummary,
    action: &mut Option<ProjectCardAction>,
) {
    ui.horizontal(|ui| {
        match summary.state {
            ProjectState::Running => {
                if stop_button(ui, summary, Some(&project.name)) {
                    *action = Some(stop_action(project, summary));
                }
            }
            ProjectState::Starting => {
                if summary.stoppable {
                    if stop_button(ui, summary, Some(&project.name)) {
                        *action = Some(stop_action(project, summary));
                    }
                } else {
                    ui.spinner();
                }
            }
            ProjectState::Stopped | ProjectState::Problem => {
                if !matches!(summary.primary, PrimaryAction::Details)
                    && primary_button(ui, &summary.primary, 0.0, Some(&project.name))
                {
                    *action = Some(primary_to_card_action(project, summary));
                }
            }
        }
        let name = format!("Detalhes de {}", project.name);
        if announced(ui.button("Detalhes"), egui::Role::Button, &name).clicked() {
            *action = Some(ProjectCardAction::OpenProject(project.path.clone()));
        }
    });
}

/// One project in the list. Wide windows show the columns; narrow ones stack the same parts.
/// A click anywhere else on the row opens the project's page: the row senses clicks below
/// its buttons, so the buttons keep theirs.
fn project_table_row(
    ui: &mut egui::Ui,
    project: &Project,
    summary: &ProjectSummary,
    folder: Option<&str>,
    found: Option<&str>,
    columns: RowColumns,
) -> Option<ProjectCardAction> {
    let mut action = None;
    let row = ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("project-row", &project.path))
            // Clicks only: the keyboard reaches the project by its name link, so the row
            // itself is not an extra unnamed stop (T039.02, CA-10).
            .sense(egui::Sense::CLICK),
        |ui| {
            // Compact rows, like a table.
            ui.spacing_mut().item_spacing.y = 2.0;
            if columns.wide {
                ui.horizontal(|ui| {
                    row_column(ui, columns.name, |ui| {
                        name_cell(ui, project, folder, found, &mut action);
                    });
                    row_column(ui, columns.state, |ui| state_cell(ui, summary));
                    row_column(ui, columns.address, |ui| {
                        if let Some(selected) = address_cell(ui, summary) {
                            action = Some(selected);
                        }
                    });
                    row_column(ui, columns.actions, |ui| {
                        actions_cell(ui, project, summary, &mut action);
                    });
                });
            } else {
                name_cell(ui, project, folder, found, &mut action);
                state_cell(ui, summary);
                if summary.url.is_some()
                    && let Some(selected) = address_cell(ui, summary)
                {
                    action = Some(selected);
                }
                actions_cell(ui, project, summary, &mut action);
            }
        },
    );
    if action.is_none()
        && row
            .response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    {
        action = Some(ProjectCardAction::OpenProject(project.path.clone()));
    }
    action
}

/// The search matches the project's name or folder (T038.03).
fn name_matches(project: &Project, query: &str) -> bool {
    query.is_empty()
        || project.name.to_lowercase().contains(query)
        || project
            .path
            .to_string_lossy()
            .to_lowercase()
            .contains(query)
}

/// The first service, port or container whose words contain the query, by its label.
fn service_hit(query: &str, candidates: &[(String, Vec<String>)]) -> Option<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return None;
    }
    candidates
        .iter()
        .find(|(_, words)| {
            words
                .iter()
                .any(|word| word.to_lowercase().contains(&query))
        })
        .map(|(label, _)| label.clone())
}

/// The project's name, which opens its page (T038.01). Looks like a title, works as a link.
fn project_name_link(ui: &mut egui::Ui, name: &str, size: f32, path: &Path) -> bool {
    let response = ui
        .add(
            egui::Button::new(RichText::new(name).strong().size(size))
                .frame(false)
                .min_size(egui::vec2(0.0, 24.0)),
        )
        .on_hover_text(display_path(path))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    // Back from the project's page, the keyboard continues from here (T038, CA-11).
    let key = project_page::return_focus_key();
    if ui
        .ctx()
        .data(|data| data.get_temp::<PathBuf>(key))
        .as_deref()
        == Some(path)
    {
        ui.ctx().data_mut(|data| data.remove::<PathBuf>(key));
        response.request_focus();
    }
    announced(response, egui::Role::Link, &format!("Abrir {name}")).clicked()
}

/// "Pasta: acme\web" under the name, only when another project has the same name (#100).
pub(super) fn folder_line(ui: &mut egui::Ui, path: &Path, folder: Option<&str>) {
    if let Some(folder) = folder {
        ui.add(
            egui::Label::new(
                RichText::new(format!("Pasta: {folder}"))
                    .color(theme::MUTED)
                    .size(theme::TEXT_DETAIL),
            )
            .wrap(),
        )
        .on_hover_text(display_path(path));
    }
}

/// The technology and language badges of a card, in display order.
fn card_badges(project: &Project) -> Vec<(String, Color32)> {
    let mut badges = project
        .shown_technologies()
        .map(|technology| (technology.to_string(), technology_color(technology)))
        .chain(
            project
                .runtimes
                .iter()
                .copied()
                .filter(|runtime| project.shows_runtime(*runtime))
                .map(|runtime| (runtime.to_string(), runtime_color(runtime))),
        )
        .collect::<Vec<_>>();
    if badges.is_empty() {
        badges.push(("Projeto local".to_owned(), theme::MUTED));
    }
    badges
}

/// The reason of a row in up to 2 lines, ending in "…" when longer. The whole sentence goes
/// to the hover, to screen readers and to the project's page.
fn reason_label(ui: &mut egui::Ui, reason: &str, emphasized: bool) {
    let (size, color) = if emphasized {
        (theme::TEXT_BODY, theme::TEXT)
    } else {
        (theme::TEXT_DETAIL, theme::MUTED)
    };
    let mut job = egui::text::LayoutJob::simple(
        reason.to_owned(),
        egui::FontId::proportional(size),
        color,
        ui.available_width(),
    );
    job.wrap.max_rows = 2;
    job.wrap.overflow_character = Some('…');
    let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
    let elided = galley.elided;
    let mut response = ui.add(egui::Label::new(galley));
    if elided {
        response = response.on_hover_text(reason);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, reason));
}

/// What "Detalhes técnicos" of the project's page shows.
struct ProjectCard<'a> {
    project: &'a Project,
    summary: &'a ProjectSummary,
    /// Start commands the person defined for this project.
    custom_commands: Vec<CustomCommand>,
    /// Each language, where it was found and whether this computer has it.
    languages: Vec<String>,
    environment: Option<&'a ProjectEnvironment>,
    hidden_from_recent: bool,
    has_install_logs: bool,
    /// The project's database: what the app did, the failure and the actions it allows.
    database: database_ui::DatabaseCardExtras,
}

impl ProjectCard<'_> {
    /// Technical details and secondary actions, shown only on request.
    fn details(&self, ui: &mut egui::Ui) -> Option<ProjectCardAction> {
        let project = self.project;
        let path = || project.path.clone();
        let mut action = None;
        // The reason is already in the summary; repeating it here adds nothing.
        let reason = self.summary.reason.as_deref().unwrap_or_default();
        let detail = |ui: &mut egui::Ui, text: &str| {
            if !reason.starts_with(text.trim_end_matches('.')) {
                ui.label(
                    RichText::new(text)
                        .color(theme::MUTED)
                        .size(theme::TEXT_DETAIL),
                );
            }
        };
        // Why the card says what it says: the rule and what the app found (T034).
        ui.label(
            RichText::new("Por que este diagnóstico?")
                .strong()
                .size(theme::TEXT_DETAIL),
        );
        ui.add(
            egui::Label::new(
                RichText::new(format!("Regra: {}", self.summary.rule.name()))
                    .size(theme::TEXT_DETAIL),
            )
            .selectable(true),
        );
        if self.summary.evidence.is_empty() {
            ui.label(RichText::new("Encontrado: não verificado").size(theme::TEXT_DETAIL));
        }
        for line in &self.summary.evidence {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Encontrado: {line}")).size(theme::TEXT_DETAIL),
                )
                .selectable(true)
                .wrap(),
            );
        }
        // A file for support, without secrets; the person reviews it before sharing.
        if ui.button("Exportar diagnóstico").clicked() {
            action = Some(ProjectCardAction::ExportDiagnostic(path()));
        }
        for line in self.summary.technical.iter().chain(&self.languages) {
            detail(ui, line);
        }
        // Each database: where it is configured, the address tested, the result and when.
        for database in self
            .environment
            .into_iter()
            .flat_map(|environment| &environment.databases)
        {
            for line in database.technical() {
                ui.label(
                    RichText::new(line)
                        .color(theme::MUTED)
                        .size(theme::TEXT_DETAIL),
                );
            }
        }
        let extras = &self.database;
        for line in extras
            .lines
            .iter()
            .chain(extras.failure.iter().flat_map(|failure| &failure.details))
        {
            ui.label(
                RichText::new(line)
                    .color(theme::MUTED)
                    .size(theme::TEXT_DETAIL),
            );
        }
        if let Some(test) = &extras.remote_test {
            let status = DatabaseStatus {
                requirement: localcodepilot_runtime::databases::DatabaseRequirement {
                    source: PathBuf::new(),
                    engine: DatabaseEngine::MySql,
                    host: None,
                    port: None,
                    compose_service: None,
                },
                available: None,
                connection: Some(test.clone()),
                managed: None,
            };
            if let Some(line) = status.technical().get(1) {
                ui.label(
                    RichText::new(line)
                        .color(theme::MUTED)
                        .size(theme::TEXT_DETAIL),
                );
            }
        }
        ui.horizontal_wrapped(|ui| {
            if let Some(fix) = extras
                .failure
                .as_ref()
                .and_then(|failure| failure.detail_fix)
                && ui.button(fix.label()).clicked()
            {
                action = Some(ProjectCardAction::DatabaseFix(path(), fix));
            }
            if extras.can_stop && ui.button("Parar o banco de dados").clicked() {
                action = Some(ProjectCardAction::StopDatabase(path()));
            }
            if extras.remote && ui.button("Testar conexão").clicked() {
                action = Some(ProjectCardAction::TestRemoteDatabase(path()));
            }
        });
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
                .size(theme::TEXT_DETAIL),
            )
            .on_hover_text(display_path(&project.path));
            if ui.small_button("Copiar caminho").clicked() {
                ui.ctx().copy_text(display_path(&project.path));
            }
        });
        if let Some(environment) = self.environment {
            // Commands the person defined are listed below on their own.
            let detected = environment
                .command_count
                .saturating_sub(self.custom_commands.len());
            if detected > 0 {
                detail(ui, &format!("Comandos detectados: {detected}"));
                // What runs and where: the format (in the name), the entry and the folder.
                for (name, command, folder) in &environment.detected_commands {
                    if self
                        .custom_commands
                        .iter()
                        .any(|custom| custom.name == *name)
                    {
                        continue;
                    }
                    detail(ui, &format!("{name}: {command} (em {folder})"));
                }
            }
            if let Some(candidates) = &environment.ambiguous_entry {
                detail(ui, &format!("Entradas possíveis: {candidates}"));
            }
            if let Some(problem) = environment.problem_details() {
                detail(ui, &problem);
            }
            for error in &environment.manifest_errors {
                if !reason.starts_with(error.as_str()) {
                    detail(
                        ui,
                        &format!("{error}. Corrija o arquivo e clique em Atualizar."),
                    );
                }
            }
        }
        for command in &self.custom_commands {
            ui.label(
                RichText::new(format!(
                    "Comando definido por você · {}: {}",
                    command.name,
                    command.command_line()
                ))
                .color(theme::MUTED)
                .size(theme::TEXT_DETAIL),
            );
            ui.horizontal_wrapped(|ui| {
                let edit = format!("Editar {}", command.name);
                if announced(ui.link("Editar"), egui::Role::Link, &edit).clicked() {
                    action = Some(ProjectCardAction::EditCommand(path(), command.name.clone()));
                }
                let remove = format!("Remover {}", command.name);
                if announced(ui.link("Remover"), egui::Role::Link, &remove).clicked() {
                    action = Some(ProjectCardAction::RemoveCommand(
                        path(),
                        command.name.clone(),
                    ));
                }
            });
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
        let define = if self.custom_commands.is_empty() {
            "Definir comando de início"
        } else {
            "Adicionar outro comando de início"
        };
        if ui.link(define).clicked() {
            action = Some(ProjectCardAction::DefineCommand(path()));
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

    #[test]
    fn a_project_that_is_compiling_says_so_and_can_be_stopped() {
        let environment = ready();
        let mut building = facts(&environment);
        building.building = true;
        building.starting = true;
        let summary = summarize_project(&building);
        assert_eq!(summary.state, ProjectState::Starting);
        assert_eq!(summary.reason.as_deref(), Some("Compilando o projeto…"));
        assert_eq!(summary.primary, PrimaryAction::ViewLogs);
        assert!(summary.stoppable);
        assert!(!summarize_project(&facts(&environment)).stoppable);
    }

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
            preparing_compiler: false,
            starting: false,
            building: false,
            running: false,
            failed_process: None,
            application_url: None,
            http: None,
            has_containers: false,
            container_running: false,
            container_stop_here: false,
            container_url: None,
            languages: &[],
            database_step: None,
            database_failure: None,
            database_note: None,
            migration_failed: false,
        }
    }

    fn database(available: Option<bool>, outcome: Option<ConnectionOutcome>) -> DatabaseStatus {
        DatabaseStatus {
            requirement: localcodepilot_runtime::databases::DatabaseRequirement {
                source: std::path::PathBuf::from(".env"),
                engine: DatabaseEngine::MySql,
                host: Some("127.0.0.1".into()),
                port: Some(3306),
                compose_service: None,
            },
            available,
            connection: outcome.map(|outcome| ConnectionTest {
                outcome,
                target: "127.0.0.1:3306".into(),
                user: Some("app".into()),
                database: Some("loja".into()),
                insufficient: vec![],
                at: std::time::SystemTime::now(),
            }),
            managed: None,
        }
    }

    fn with_database(status: DatabaseStatus) -> ProjectEnvironment {
        ProjectEnvironment {
            databases: vec![status],
            ..ready()
        }
    }

    #[test]
    fn only_an_authenticated_login_says_connected() {
        let connected = database(Some(true), Some(ConnectionOutcome::Connected));
        assert_eq!(connected.card_state(None), "Banco de dados conectado.");
        // A port that answers, without a login test, is not a connection.
        assert_eq!(
            database(Some(true), None).card_state(None),
            "conexão não verificada"
        );
        // A running container is not a connection either.
        assert_eq!(
            connected.card_state(Some(true)),
            "Container ativo; conexão não verificada."
        );
        assert_eq!(
            database(Some(true), Some(ConnectionOutcome::InsufficientData)).card_state(None),
            "Conexão não verificada: faltam dados de acesso que o aplicativo consiga ler."
        );
        let summary = summarize_project(&facts(&with_database(connected)));
        assert_eq!(summary.state, ProjectState::Stopped);
        assert_eq!(summary.primary, PrimaryAction::Start);
    }

    #[test]
    fn a_database_that_is_not_running_keeps_the_project_stopped_with_the_reason() {
        for status in [
            database(Some(false), None),
            database(Some(true), Some(ConnectionOutcome::NotResponding)),
        ] {
            assert!(status.not_running());
            let summary = summarize_project(&facts(&with_database(status)));
            assert_eq!(summary.state, ProjectState::Stopped);
            // Where the app has a MySQL package (Windows, macOS 15+, Linux x86_64, T029)
            // Iniciar prepares it; elsewhere the card names the engine and the person reads
            // why in "Detalhes".
            let preparable = localcodepilot_runtime::database_packages::package_for_this_system(
                DatabaseEngine::MySql,
            )
            .is_some();
            let (reason, primary) = if preparable {
                (
                    "O banco de dados deste projeto não está rodando.",
                    PrimaryAction::Start,
                )
            } else {
                (
                    "O MySQL deste projeto não está rodando. O LocalCodePilot ainda não consegue prepará-lo neste sistema.",
                    PrimaryAction::Details,
                )
            };
            assert_eq!(summary.reason.as_deref(), Some(reason));
            assert_eq!(summary.primary, primary);
        }
    }

    #[test]
    fn a_refused_login_and_a_missing_database_are_problems_with_their_own_reason() {
        let refused = summarize_project(&facts(&with_database(database(
            Some(true),
            Some(ConnectionOutcome::AccessDenied),
        ))));
        assert_eq!(refused.state, ProjectState::Problem);
        assert_eq!(
            refused.reason.as_deref(),
            Some(
                "O banco recusou o acesso deste projeto, que não consegue usar o banco com estes dados. Confira os dados de conexão em Detalhes."
            )
        );
        let missing = summarize_project(&facts(&with_database(database(
            Some(true),
            Some(ConnectionOutcome::DatabaseMissing),
        ))));
        assert_eq!(missing.state, ProjectState::Problem);
        assert_eq!(
            missing.reason.as_deref(),
            Some(
                "O banco respondeu, mas o banco de dados loja não foi encontrado. Confira a configuração de conexão."
            )
        );
    }

    #[test]
    fn technical_details_show_the_tested_address_but_never_a_password() {
        let lines = database(Some(true), Some(ConnectionOutcome::Connected)).technical();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("Conexão do banco MySQL · Origem da configuração: .env"));
        assert!(lines[0].ends_with("· Host: 127.0.0.1 · Porta: 3306"));
        // Date and time of the test, with how long ago (T039.01, CA-2).
        let moment = lines[1]
            .strip_prefix("Último teste: ")
            .and_then(|rest| rest.split(" · ").next())
            .unwrap();
        let bytes = moment.as_bytes();
        assert!(bytes[2] == b'/' && bytes[5] == b'/' && moment[10..].starts_with(" às "));
        assert!(moment.ends_with("(agora)"));
        assert!(lines[1].contains(
            "· Endereço: 127.0.0.1:3306 · Usuário: app · Banco: loja · Resultado: conectado"
        ));
        // Never tested: said, not guessed (T039.01).
        let untested = database(Some(true), None).technical();
        assert_eq!(untested[1], "Último teste: não testado");
        // The secrets of the project's configuration never show (CA-2).
        let folder = std::env::temp_dir().join(format!("lcp-t03901-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let env = folder.join(".env");
        std::fs::write(
            &env,
            "DB_USERNAME=app\nDB_PASSWORD=SEGREDO-FIXTURE-91\nAPI_TOKEN=TOKEN-FIXTURE-27\n",
        )
        .unwrap();
        let mut from_file = database(Some(true), Some(ConnectionOutcome::AccessDenied));
        from_file.requirement.source = env;
        for line in from_file.technical().iter().chain(&lines).chain(&untested) {
            assert!(!line.contains("SEGREDO-FIXTURE-91") && !line.contains("TOKEN-FIXTURE-27"));
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_actions_column_always_fits_its_buttons() {
        // 760 px wide (the smallest window), the longest action and an address.
        let columns = RowColumns::for_width(700.0, 260.0, true, 8.0);
        let used = columns.name + columns.state + columns.address + columns.actions + 3.0 * 8.0;
        assert!(used <= 700.0 + 0.5, "{columns:?}");
        assert_eq!(columns.actions, 260.0);
        // Without addresses the column shrinks and gives the space back.
        let none = RowColumns::for_width(700.0, 260.0, false, 8.0);
        assert!(none.address < 30.0 && none.name > columns.name);
        // Too narrow for name and state next to the actions: the rows stack.
        assert!(!RowColumns::for_width(600.0, 260.0, true, 8.0).wide);
    }

    #[test]
    fn the_search_finds_a_service_a_port_or_a_container_and_says_which() {
        let candidates = vec![
            (
                "serviço start · porta 3000".to_owned(),
                vec!["start".to_owned(), "3000".to_owned()],
            ),
            (
                "container loja-web-1 · serviço web".to_owned(),
                vec![
                    "loja-web-1".to_owned(),
                    "web".to_owned(),
                    "nginx:alpine".to_owned(),
                    "0.0.0.0:8089->80/tcp".to_owned(),
                ],
            ),
        ];
        assert_eq!(
            service_hit("3000", &candidates).as_deref(),
            Some("serviço start · porta 3000")
        );
        assert_eq!(
            service_hit("NGINX", &candidates).as_deref(),
            Some("container loja-web-1 · serviço web")
        );
        assert_eq!(
            service_hit("8089", &candidates).as_deref(),
            Some("container loja-web-1 · serviço web")
        );
        assert_eq!(service_hit("redis", &candidates), None);
        assert_eq!(service_hit("  ", &candidates), None);
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
        // Without a confirmed reach, "Parar" leads to Containers and says so.
        assert!(!running.stops_containers_here);
        assert_eq!(
            stop_label(&running),
            format!("{STOP}  Escolher o que parar")
        );
        containers.container_stop_here = true;
        let direct = summarize_project(&containers);
        assert!(direct.stops_containers_here);
        assert_eq!(stop_label(&direct), format!("{STOP}  Parar"));
    }

    #[test]
    fn a_failed_start_keeps_its_reason_on_the_card_and_the_owner_in_details() {
        let environment = ready();
        let failure = ProcessFailure::PortTaken {
            port: Port::new(5173).unwrap(),
            owner: Some(PortOwner {
                process_id: 1234,
                process_name: Some("node.exe".into()),
            }),
        };
        let mut failed = facts(&environment);
        failed.failed_process = Some(FailedProcess {
            name: "Servidor",
            command: "npm run dev".into(),
            exit_code: None,
            failure: Some(&failure),
        });
        let summary = summarize_project(&failed);
        assert_eq!(summary.state, ProjectState::Problem);
        assert_eq!(
            summary.reason.as_deref(),
            Some("Outro programa está usando a porta 5173. Este projeto não iniciou.")
        );
        // Ver logs, never an action that ends the other program, and a new attempt once the
        // port is free.
        assert_eq!(summary.primary, PrimaryAction::ViewLogs);
        assert!(can_retry(&summary));
        assert_eq!(
            summary.technical,
            [
                "Processo: Servidor",
                "Comando: npm run dev",
                "A porta 5173 está em uso por node.exe (PID 1234), que não pertence a este projeto.",
            ]
        );
        // A new attempt hides the failure of the previous one.
        failed.starting = true;
        let retrying = summarize_project(&failed);
        assert_eq!(retrying.state, ProjectState::Starting);
        assert!(retrying.technical.is_empty());
        failed.starting = false;
        failed.running = true;
        assert_eq!(summarize_project(&failed).state, ProjectState::Running);

        for failure in [
            ProcessFailure::NoResponse(Port::new(5173).unwrap()),
            ProcessFailure::ClosedBeforeReady,
            ProcessFailure::PortLost(Port::new(5173).unwrap()),
            ProcessFailure::Exited,
            ProcessFailure::ProgramMissing("npm".into()),
            ProcessFailure::FolderMissing(PathBuf::from("C:/projetos/loja")),
            ProcessFailure::SpawnFailed("Acesso negado. (os error 5)".into()),
        ] {
            let sentence = failure.sentence();
            assert!(
                !sentence.contains("PID") && !sentence.contains("os error"),
                "{sentence}"
            );
            assert!(!sentence.contains("projetos/loja"), "{sentence}");
        }
    }

    #[test]
    fn each_diagnosis_names_its_rule_and_what_was_found() {
        // A process that stopped without a known cause: the card says so, and Detalhes
        // says the cause was not verified instead of guessing one.
        let environment = ready();
        let mut stopped = facts(&environment);
        stopped.failed_process = Some(FailedProcess {
            name: "web",
            command: "npm run dev".into(),
            exit_code: None,
            failure: None,
        });
        let summary = summarize_project(&stopped);
        assert_eq!(summary.rule, DiagnosticRule::ProcessFailed);
        assert_eq!(
            summary.reason.as_deref(),
            Some("web parou com erro. Veja os logs para encontrar a causa.")
        );
        assert_eq!(
            summary.evidence,
            vec![
                "web terminou; código de saída não verificado.".to_owned(),
                "nenhuma causa específica foi verificada.".to_owned()
            ]
        );
        // A known cause keeps its own sentence and is the verified cause.
        let port = ProcessFailure::PortTaken {
            port: Port::new(3000).unwrap(),
            owner: None,
        };
        stopped.failed_process = Some(FailedProcess {
            name: "web",
            command: "npm run dev".into(),
            exit_code: Some(1),
            failure: Some(&port),
        });
        let summary = summarize_project(&stopped);
        assert_eq!(summary.reason.as_deref(), Some(port.sentence().as_str()));
        assert_eq!(
            summary.evidence,
            vec![
                "web terminou com o código de saída 1.".to_owned(),
                format!("Causa verificada: {}", port.sentence())
            ]
        );

        // A missing compiler and a missing program: both found, one per line.
        let missing = ProjectEnvironment {
            command_count: 1,
            missing_compiler: true,
            missing_programs: vec!["cmake".into()],
            ..ProjectEnvironment::default()
        };
        let summary = summarize_project(&facts(&missing));
        assert_eq!(summary.rule, DiagnosticRule::NeedsPreparation);
        assert_eq!(
            summary.evidence,
            vec![
                "Compilador C/C++: não encontrado.".to_owned(),
                "Programas não encontrados: cmake.".to_owned()
            ]
        );

        // Ready to start.
        let summary = summarize_project(&facts(&environment));
        assert_eq!(summary.rule, DiagnosticRule::Ready);
        assert_eq!(
            summary.evidence,
            vec!["1 comando encontrado(s); requisitos verificados.".to_owned()]
        );
    }

    #[test]
    fn the_first_rule_that_holds_decides_and_the_same_facts_give_the_same_result() {
        // Two conditions at once: an invalid manifest and a failed process. The process rule
        // comes first in the documented order.
        let invalid = ProjectEnvironment {
            command_count: 1,
            manifest_errors: vec!["package.json inválido (linha 1)".into()],
            ..ProjectEnvironment::default()
        };
        let mut both = facts(&invalid);
        both.failed_process = Some(FailedProcess {
            name: "web",
            command: "npm run dev".into(),
            exit_code: Some(1),
            failure: None,
        });
        let first = summarize_project(&both);
        assert_eq!(first.rule, DiagnosticRule::ProcessFailed);
        for _ in 0..5 {
            assert_eq!(summarize_project(&both), first);
        }
        let position = |rule| {
            DiagnosticRule::ALL
                .iter()
                .position(|candidate| *candidate == rule)
                .unwrap()
        };
        assert!(
            position(DiagnosticRule::ProcessFailed) < position(DiagnosticRule::ManifestInvalid)
        );
        // Building comes before starting: a project compiling and starting says compiling.
        let environment = ready();
        let mut busy = facts(&environment);
        busy.building = true;
        busy.starting = true;
        assert_eq!(summarize_project(&busy).rule, DiagnosticRule::Building);
    }

    #[test]
    fn the_documented_precedence_is_the_order_of_the_code() {
        let doc = include_str!("../../docs/DETALHES-TECNICOS.md");
        let mut cursor = 0;
        for (index, rule) in DiagnosticRule::ALL.iter().enumerate() {
            let line = format!("| {} | {} |", index + 1, rule.name());
            let found = doc[cursor..]
                .find(&line)
                .unwrap_or_else(|| panic!("falta na documentação, nesta ordem: {line}"));
            cursor += found + line.len();
        }
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
            Some(
                "package.json inválido (linha 1). Corrija o arquivo indicado e atualize o projeto."
            )
        );
        assert_eq!(summary.rule, DiagnosticRule::ManifestInvalid);
        assert_eq!(
            summary.evidence,
            vec!["package.json inválido (linha 1)".to_owned()]
        );
        assert_eq!(summary.primary, PrimaryAction::Details);

        let environment = ready();
        let mut crashed = facts(&environment);
        crashed.failed_process = Some(FailedProcess {
            name: "npm run start",
            command: "npm run start".into(),
            exit_code: Some(1),
            failure: None,
        });
        let summary = summarize_project(&crashed);
        assert_eq!(summary.state, ProjectState::Problem);
        assert_eq!(
            summary.reason.as_deref(),
            Some("npm run start parou com erro. Veja os logs para encontrar a causa.")
        );
        assert_eq!(summary.primary, PrimaryAction::ViewLogs);
        assert!(summary.technical.contains(&"Código de saída: 1".to_owned()));

        let failed = DependencyInstallStatus::Failed {
            error: "exit 1".into(),
            suggestion: String::new(),
        };
        let mut install = facts(&environment);
        install.dependency_install = Some(&failed);
        let summary = summarize_project(&install);
        assert_eq!(summary.state, ProjectState::Problem);
        assert_eq!(summary.primary, PrimaryAction::ViewInstallLogs);

        // An incompatible version, missing PHP extensions and a database server that does
        // not answer still need attention.
        let mut incompatible = ready();
        incompatible.runtime_requirements = vec![requirement(RequirementStatus::Incompatible {
            installed_version: "16.20.0".into(),
            required_version: ">=20".into(),
            path: PathBuf::from("node"),
        })];
        incompatible.missing_dependencies = vec!["Node.js".into()];
        let summary = summarize_project(&facts(&incompatible));
        assert_eq!(summary.state, ProjectState::Problem);
        assert_eq!(
            summary.reason.as_deref(),
            Some("Node 16.20.0 não atende ao projeto (>=20)")
        );
        let mut extensions = ready();
        extensions.missing_php_extensions = vec!["zip".into()];
        assert_eq!(
            summarize_project(&facts(&extensions)).state,
            ProjectState::Problem
        );
    }

    fn requirement(status: RequirementStatus) -> RuntimeRequirement {
        RuntimeRequirement {
            runtime: RuntimeKind::Node,
            command: "node".into(),
            version: localcodepilot_core::environments::VersionConstraint::declared("20"),
            source: localcodepilot_core::environments::RequirementSource::Recommended,
            status,
        }
    }

    #[test]
    fn a_fresh_clone_is_stopped_with_what_to_prepare_not_a_problem() {
        // Only the project's dependencies are missing (#101): stopped, with the reason and
        // the install action; never "Iniciar" before the install.
        let mut fresh = ready();
        fresh.missing_dependencies = vec!["Node.js".into()];
        fresh.install_steps = vec![DependencyInstall {
            label: "dependências Node.js".into(),
            program: "npm".into(),
            args: vec!["install".into()],
            working_directory: PathBuf::from("C:/projetos/loja"),
        }];
        let summary = summarize_project(&facts(&fresh));
        assert_eq!(summary.state, ProjectState::Stopped);
        assert_eq!(
            summary.reason.as_deref(),
            Some("Faltam dependências de Node.js para preparar este projeto.")
        );
        assert_eq!(summary.primary, PrimaryAction::InstallDependencies);
        assert!(!ProjectFilter::Problem.matches(summary.state));

        // A language that is not installed is also something to prepare, not a failure.
        let mut no_language = ready();
        no_language.missing_programs = vec!["cargo".into()];
        let summary = summarize_project(&facts(&no_language));
        assert_eq!(summary.state, ProjectState::Stopped);
        assert_eq!(summary.reason.as_deref(), Some("Instale: cargo"));
        let mut no_node = ready();
        no_node.runtime_requirements = vec![requirement(RequirementStatus::Missing)];
        no_node.missing_dependencies = vec!["Node.js".into()];
        let summary = summarize_project(&facts(&no_node));
        assert_eq!(summary.state, ProjectState::Stopped);
        assert_eq!(summary.reason.as_deref(), Some("Instale Node 20"));

        // Once the install fails, the same project is "Com problema" with its logs.
        let failed = DependencyInstallStatus::Failed {
            error: "exit 1".into(),
            suggestion: String::new(),
        };
        let mut install = facts(&fresh);
        install.dependency_install = Some(&failed);
        let summary = summarize_project(&install);
        assert_eq!(summary.state, ProjectState::Problem);
        assert_eq!(
            summary.reason.as_deref(),
            Some("A instalação das dependências falhou")
        );
        assert_eq!(summary.primary, PrimaryAction::ViewInstallLogs);
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
        // Any project without a command can be given one.
        assert_eq!(summary.primary, PrimaryAction::DefineCommand);
        let mut ruby = facts(&empty);
        ruby.languages = &[RuntimeKind::Ruby, RuntimeKind::Lua];
        let summary = summarize_project(&ruby);
        assert_eq!(
            summary.reason.as_deref(),
            Some(
                "Reconhecemos Ruby, Lua, mas não encontramos um arquivo ou alvo de entrada. Defina o comando de início."
            )
        );
        assert_eq!(summary.primary, PrimaryAction::DefineCommand);
        // C/C++: the card speaks of programs (T033).
        let mut cpp = facts(&empty);
        cpp.languages = &[RuntimeKind::Cpp];
        assert_eq!(
            summarize_project(&cpp).reason.as_deref(),
            Some(
                "Reconhecemos C/C++, mas não encontramos um programa único para iniciar. Defina o comando de início."
            )
        );
        let cpp_ambiguous = ProjectEnvironment {
            ambiguous_entry: Some("C/C++: servidor, cliente".into()),
            ..ProjectEnvironment::default()
        };
        let mut two_programs = facts(&cpp_ambiguous);
        two_programs.languages = &[RuntimeKind::Cpp];
        assert_eq!(
            summarize_project(&two_programs).reason.as_deref(),
            Some(
                "Encontramos mais de um programa para iniciar neste projeto. Defina qual comando usar."
            )
        );
        // Several plausible entries: the card says so instead of picking one (T028.01).
        let ambiguous = ProjectEnvironment {
            ambiguous_entry: Some("Perl: a.pl, b.pl".into()),
            ..ProjectEnvironment::default()
        };
        let summary = summarize_project(&facts(&ambiguous));
        assert_eq!(
            summary.reason.as_deref(),
            Some(
                "Encontramos mais de uma forma de iniciar este projeto. Defina qual comando usar."
            )
        );
        assert_eq!(summary.primary, PrimaryAction::DefineCommand);
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
