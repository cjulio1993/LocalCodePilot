//! Turning on an installed container engine that is stopped, from "Iniciar" or Containers,
//! after the person confirms (T026.02). The plan comes from the diagnosis (T026.01): without
//! a plan, nothing here is offered. After the engine answers, the original action goes
//! through the usual container confirmation and Compose checks, with nothing skipped.

use super::*;
use localcodepilot_core::engine_start::EngineStart;
use localcodepilot_platform::engine_start::{EngineRunner, StartFailure, start_engine};

const START_FOCUS: &str = "engine-start-confirmation";
const DOCKER_SETUP_WAITING: &str = "O Docker Desktop está aguardando você concluir a configuração na janela dele. Depois, clique em Iniciar de novo.";

/// The confirmation waiting for an answer.
pub(super) struct EngineStartRequest {
    /// Engines the person can choose from; one when the choice is clear.
    choices: Vec<ContainerEngine>,
    engine: ContainerEngine,
    /// The project to start once the engine answers, with its command per engine.
    project: Option<String>,
    resume: Vec<(ContainerEngine, ContainerCommand)>,
    /// The control that opened the confirmation, focused again when it closes.
    origin: Option<egui::Id>,
}

/// A start running in the background.
pub(super) struct EngineStartJob {
    engine: ContainerEngine,
    plan: EngineStart,
    receiver: Receiver<Result<(), StartFailure>>,
    resume: Option<ContainerCommand>,
}

/// Why the last start failed, shown in the engine's notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EngineStartFailure {
    pub engine: ContainerEngine,
    pub sentence: String,
    pub technical: Vec<String>,
}

#[derive(Default)]
pub(super) struct EngineStartState {
    pub request: Option<EngineStartRequest>,
    pub job: Option<EngineStartJob>,
    pub failure: Option<EngineStartFailure>,
    /// The project action to resume once the containers are checked again.
    pub resume: Option<(ContainerEngine, ContainerCommand)>,
}

impl EngineStartState {
    pub fn starting(&self, engine: ContainerEngine) -> bool {
        self.job.as_ref().is_some_and(|job| job.engine == engine)
    }

    pub fn busy(&self) -> bool {
        self.job.is_some() || self.request.is_some()
    }
}

pub(super) fn stopped_sentence(engine: ContainerEngine) -> String {
    format!("O {engine} está instalado, mas parado. Para iniciar este projeto, ligue o motor.")
}

pub(super) fn progress_sentence(engine: ContainerEngine) -> String {
    format!("Iniciando o {engine}…")
}

/// "Iniciando o {motor}…", announced when it appears.
pub(super) fn progress_line(ui: &mut egui::Ui, engine: ContainerEngine) {
    ui.horizontal(|ui| {
        ui.spinner();
        let sentence = progress_sentence(engine);
        let label = announced(
            ui.label(RichText::new(&sentence).strong()),
            egui::Role::Label,
            &sentence,
        );
        notice::mark_live(ui, label.id);
    });
}

/// Lines for "Detalhes técnicos": commands and error output, with secrets masked.
pub(super) fn technical_lines(lines: &[String]) -> Vec<String> {
    crate::secrets::mask_lines(lines)
}

/// The reason after "Não foi possível ligar o {motor}.".
pub(super) fn failure_reason(
    engine: ContainerEngine,
    plan: &EngineStart,
    failure: &StartFailure,
) -> String {
    match failure {
        StartFailure::TimedOut
            if matches!(
                plan,
                EngineStart::DockerDesktopCli | EngineStart::DockerDesktopApp(_)
            ) =>
        {
            DOCKER_SETUP_WAITING.to_owned()
        }
        StartFailure::TimedOut => format!("O {engine} não respondeu a tempo."),
        StartFailure::ComposeMissing => format!("O {engine} ligou, mas o Compose não respondeu."),
        StartFailure::Command(_) => format!("O comando para ligar o {engine} falhou."),
    }
}

pub(super) fn failure_sentence(
    engine: ContainerEngine,
    with_project: bool,
    reason: &str,
) -> String {
    if with_project {
        format!("Não foi possível ligar o {engine}. O projeto não foi iniciado. {reason}")
    } else {
        format!("Não foi possível ligar o {engine}. {reason}")
    }
}

fn description(engine: ContainerEngine, project: Option<&str>) -> String {
    match project {
        Some(project) => format!(
            "O LocalCodePilot vai ligar o {engine} neste computador e, quando ele estiver pronto, iniciar o projeto {project}. Motor: {engine}. Projeto: {project}."
        ),
        None => format!("O LocalCodePilot vai ligar o {engine} neste computador."),
    }
}

/// The engines a start may be offered for: stopped, with a safe plan. When the environment
/// already ran on one engine, that is the choice; otherwise, with both startable, the person
/// chooses.
pub(super) fn start_choices(
    engines: &[EngineDiagnosis],
    in_use: &[ContainerEngine],
    preferred: ContainerEngine,
) -> Vec<ContainerEngine> {
    let startable = |engine: ContainerEngine| {
        engines
            .iter()
            .any(|diagnosis| diagnosis.engine == engine && diagnosis.start.is_some())
    };
    if let [engine] = in_use {
        return if startable(*engine) {
            vec![*engine]
        } else {
            vec![]
        };
    }
    let mut choices = [ContainerEngine::Docker, ContainerEngine::Podman]
        .into_iter()
        .filter(|engine| startable(*engine))
        .collect::<Vec<_>>();
    // The preferred engine comes first, as the default choice.
    choices.sort_by_key(|engine| *engine != preferred);
    choices
}

fn escape_pressed(ui: &egui::Ui) -> bool {
    ui.input(|input| input.key_pressed(egui::Key::Escape))
}

fn focus_safe_choice_once(ui: &egui::Ui, button: &egui::Response) {
    let id = egui::Id::unique(("first-focus", START_FOCUS));
    if !ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id))
        .unwrap_or(false)
    {
        button.request_focus();
        ui.ctx().data_mut(|data| data.insert_temp(id, true));
    }
}

fn forget_first_focus(ui: &egui::Ui) {
    ui.ctx()
        .data_mut(|data| data.remove::<bool>(egui::Id::unique(("first-focus", START_FOCUS))));
}

impl LocalCodePilot {
    fn engine_plan(&self, engine: ContainerEngine) -> Option<EngineStart> {
        self.container_state
            .engines
            .iter()
            .find(|diagnosis| diagnosis.engine == engine)
            .and_then(|diagnosis| diagnosis.start.clone())
    }

    /// Opens "Ligar o {motor}?" for `choices`, resuming `resume` once the engine answers.
    pub(super) fn request_engine_start(
        &mut self,
        choices: Vec<ContainerEngine>,
        project: Option<String>,
        resume: Vec<(ContainerEngine, ContainerCommand)>,
        origin: Option<egui::Id>,
    ) {
        let Some(engine) = choices.first().copied() else {
            return;
        };
        if self.container_state.engine_start.busy() {
            return;
        }
        self.container_state.engine_start.failure = None;
        self.container_state.engine_start.request = Some(EngineStartRequest {
            choices,
            engine,
            project,
            resume,
            origin,
        });
    }

    pub(crate) fn engine_start_confirmation(&mut self, root_ui: &mut egui::Ui) {
        let Some(mut request) = self.container_state.engine_start.request.take() else {
            return;
        };
        let Some(plan) = self.engine_plan(request.engine) else {
            // The diagnosis changed meanwhile: nothing safe to offer any more.
            forget_first_focus(root_ui);
            return;
        };
        let mut open = true;
        let mut cancel = false;
        let mut confirm = false;
        let shown = egui::Window::new(format!("Ligar o {}?", request.engine))
            .id(egui::Id::unique(START_FOCUS))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(500.0)
            .show(root_ui, |ui| {
                if request.choices.len() > 1 {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Iniciar com:");
                        for choice in request.choices.clone() {
                            ui.selectable_value(&mut request.engine, choice, choice.to_string());
                        }
                    });
                    ui.add_space(6.0);
                }
                ui.label(description(request.engine, request.project.as_deref()));
                if plan.downloads() {
                    ui.add_space(6.0);
                    ui.colored_label(
                        theme::WARNING,
                        "A máquina padrão do Podman ainda não existe. Criá-la pode baixar arquivos e ocupar espaço neste computador.",
                    );
                }
                egui::CollapsingHeader::new("Detalhes técnicos")
                    .id_salt("engine-start-details")
                    .show(ui, |ui| {
                        for command in plan.commands() {
                            ui.label(RichText::new(command).monospace().size(12.0));
                        }
                        if let Some(diagnosis) = self
                            .container_state
                            .engines
                            .iter()
                            .find(|diagnosis| diagnosis.engine == request.engine)
                            && let Some(failure) = &diagnosis.failure
                        {
                            for line in technical_lines(std::slice::from_ref(failure)) {
                                ui.label(RichText::new(line).color(theme::MUTED).size(12.0));
                            }
                        }
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let not_now = ui.button("Agora não");
                    focus_safe_choice_once(ui, &not_now);
                    if not_now.clicked() {
                        cancel = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Ligar e iniciar").color(Color32::WHITE).strong(),
                            )
                            .fill(theme::PRIMARY),
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
            forget_first_focus(root_ui);
        }
        // Closed without starting: the focus goes back to the control that opened it.
        if (cancel || !open)
            && let Some(origin) = request.origin
        {
            root_ui
                .ctx()
                .memory_mut(|memory| memory.request_focus(origin));
        }
        if confirm {
            // The plan of the engine chosen in the dialog.
            if let Some(plan) = self.engine_plan(request.engine) {
                let resume = request
                    .resume
                    .into_iter()
                    .find(|(engine, _)| *engine == request.engine)
                    .map(|(_, command)| command);
                self.spawn_engine_start(request.engine, plan, resume, root_ui.ctx().clone());
            }
        } else if !cancel && open {
            self.container_state.engine_start.request = Some(request);
        }
    }

    fn spawn_engine_start(
        &mut self,
        engine: ContainerEngine,
        plan: EngineStart,
        resume: Option<ContainerCommand>,
        ctx: egui::Context,
    ) {
        let (sender, receiver) = mpsc::channel();
        let path = self.environment_path.clone();
        let job_plan = plan.clone();
        std::thread::spawn(move || {
            let result = run_engine_start(engine, &job_plan, path.as_deref());
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.container_state.engine_start.job = Some(EngineStartJob {
            engine,
            plan,
            receiver,
            resume,
        });
    }

    /// Follows a running start; on success the containers are checked again and the project
    /// action resumes from there.
    pub(crate) fn poll_engine_start(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.container_state.engine_start.job else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(StartFailure::Command(
                "a partida do motor foi interrompida".into(),
            )),
        };
        let Some(job) = self.container_state.engine_start.job.take() else {
            return;
        };
        match result {
            Ok(()) => {
                self.container_state.engine_start.failure = None;
                self.container_state.engine_start.resume =
                    job.resume.map(|command| (job.engine, command));
                self.refresh_containers(ctx.clone());
            }
            Err(failure) => {
                let reason = failure_reason(job.engine, &job.plan, &failure);
                let mut technical = job.plan.commands();
                if let StartFailure::Command(error) = &failure
                    && !error.is_empty()
                {
                    technical.push(error.clone());
                }
                self.container_state.engine_start.failure = Some(EngineStartFailure {
                    engine: job.engine,
                    sentence: failure_sentence(job.engine, job.resume.is_some(), &reason),
                    technical: technical_lines(&technical),
                });
                self.refresh_containers(ctx.clone());
            }
        }
    }

    /// After the containers were checked again: the project action goes through the usual
    /// confirmation, which checks the Compose environment again before anything runs.
    pub(super) fn resume_after_engine_start(&mut self) {
        let Some((engine, command)) = self.container_state.engine_start.resume.take() else {
            return;
        };
        let (ready, compose) = self.engine_readiness(engine);
        if !ready || !compose {
            self.container_state.engine_start.failure = Some(EngineStartFailure {
                engine,
                sentence: failure_sentence(
                    engine,
                    true,
                    &format!("O {engine} ligou, mas o Compose não respondeu."),
                ),
                technical: vec![],
            });
            return;
        }
        // The containers were just checked again: a block found now stops here, and the
        // environment shows why. The execution checks the environment once more right before
        // it runs.
        if command.action == ContainerAction::Start
            && let Some(project) = self
                .container_state
                .projects
                .iter()
                .find(|project| same_folder(&project.directory, &command.working_directory))
            && project.compose_file.is_some()
        {
            let safety = self.compose_environment_safety(&project.directory, engine);
            if guard_environment_action(command.action, &safety, &project.directory).is_err() {
                return;
            }
        }
        if command.action.changes_environment() {
            self.container_state.confirmation = Some(command);
        } else {
            self.start_container_execution(command);
        }
    }
}

/// The real start, or a simulated one in test builds (never in a release build).
fn run_engine_start(
    engine: ContainerEngine,
    plan: &EngineStart,
    path: Option<&OsStr>,
) -> Result<(), StartFailure> {
    #[cfg(feature = "engine-start-test")]
    if let Ok(outcome) = std::env::var("LOCALCODEPILOT_TEST_ENGINE_START") {
        // Each simulated start is written down, so a test can prove none ran before the
        // confirmation or after "Agora não".
        if let Ok(log) = std::env::var("LOCALCODEPILOT_TEST_ENGINE_START_LOG") {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
            {
                let _ = writeln!(file, "{engine} {}", plan.commands().join(" && "));
            }
        }
        std::thread::sleep(Duration::from_secs(8));
        return match outcome.as_str() {
            "ok" => Ok(()),
            "compose" => Err(StartFailure::ComposeMissing),
            "falha" => Err(StartFailure::Command("Error: simulado".into())),
            _ => Err(StartFailure::TimedOut),
        };
    }
    let mut runner = localcodepilot_platform::engine_start::SystemRunner::new(path);
    start_engine(engine, plan, &mut runner as &mut dyn EngineRunner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use localcodepilot_core::containers::EngineState;
    use std::path::PathBuf;

    fn diagnosis(engine: ContainerEngine, start: Option<EngineStart>) -> EngineDiagnosis {
        EngineDiagnosis {
            engine,
            state: EngineState::Unavailable,
            compose_available: false,
            start,
            failure: None,
        }
    }

    #[test]
    fn an_environment_that_ran_on_one_engine_offers_only_that_engine() {
        let both = [
            diagnosis(ContainerEngine::Docker, Some(EngineStart::DockerDesktopCli)),
            diagnosis(
                ContainerEngine::Podman,
                Some(EngineStart::PodmanMachine {
                    name: "podman-machine-default".into(),
                    create: false,
                }),
            ),
        ];
        assert_eq!(
            start_choices(&both, &[ContainerEngine::Podman], ContainerEngine::Docker),
            vec![ContainerEngine::Podman]
        );
        // Never used: the person chooses, the preferred engine first.
        assert_eq!(
            start_choices(&both, &[], ContainerEngine::Podman),
            vec![ContainerEngine::Podman, ContainerEngine::Docker]
        );
        // Without a safe plan, nothing is offered.
        let unsafe_docker = [diagnosis(ContainerEngine::Docker, None)];
        assert!(
            start_choices(
                &unsafe_docker,
                &[ContainerEngine::Docker],
                ContainerEngine::Docker
            )
            .is_empty()
        );
        assert!(start_choices(&unsafe_docker, &[], ContainerEngine::Docker).is_empty());
    }

    #[test]
    fn technical_details_never_show_secrets_from_the_error_output() {
        let lines = technical_lines(&[
            "podman machine start podman-machine-default".to_owned(),
            "Error: DOCKER_PASSWORD=senha-ficticia-1 rejected".to_owned(),
            "pull https://usuario:senha-ficticia-2@registro.exemplo/img failed".to_owned(),
            "Authorization: Bearer token-ficticio-3".to_owned(),
        ])
        .join("\n");
        for secret in ["senha-ficticia-1", "senha-ficticia-2", "token-ficticio-3"] {
            assert!(!lines.contains(secret), "{secret} appeared: {lines}");
        }
        assert!(lines.contains("podman machine start podman-machine-default"));
    }

    #[test]
    fn progress_and_failure_are_live_regions_for_screen_readers() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut live = vec![];
        for _ in 0..2 {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                progress_line(ui, ContainerEngine::Docker);
                notice::Notice::new(
                    egui::Id::unique("falha-teste"),
                    "Não foi possível ligar o Docker.",
                )
                .live()
                .show(
                    ui,
                    notice::NoticeParts {
                        resolve: None,
                        details: None,
                    },
                );
            });
            output.textures_delta.clear();
            if let Some(update) = output.platform_output.accesskit_update {
                let nodes = update
                    .nodes
                    .iter()
                    .cloned()
                    .collect::<std::collections::HashMap<_, _>>();
                // egui keeps the text of a label in its text runs.
                let text = |node: &egui::accesskit::Node| {
                    node.label()
                        .or(node.value())
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            node.children()
                                .iter()
                                .filter_map(|child| nodes.get(child))
                                .filter_map(|child| child.value().or(child.label()))
                                .collect::<String>()
                        })
                };
                live = update
                    .nodes
                    .iter()
                    .filter(|(_, node)| node.live() == Some(egui::accesskit::Live::Polite))
                    .map(|(_, node)| text(node))
                    .collect();
            }
        }
        assert!(
            live.iter()
                .any(|label| label.contains("Iniciando o Docker")),
            "{live:?}"
        );
        assert!(
            live.iter()
                .any(|label| label.contains("Não foi possível ligar o Docker")),
            "{live:?}"
        );
    }

    #[test]
    fn texts_follow_the_approved_wording() {
        assert_eq!(
            stopped_sentence(ContainerEngine::Docker),
            "O Docker está instalado, mas parado. Para iniciar este projeto, ligue o motor."
        );
        assert_eq!(
            progress_sentence(ContainerEngine::Podman),
            "Iniciando o Podman…"
        );
        assert_eq!(
            description(ContainerEngine::Docker, Some("loja")),
            "O LocalCodePilot vai ligar o Docker neste computador e, quando ele estiver pronto, iniciar o projeto loja. Motor: Docker. Projeto: loja."
        );
        let reason = failure_reason(
            ContainerEngine::Docker,
            &EngineStart::DockerDesktopApp(PathBuf::from("Docker Desktop.exe")),
            &StartFailure::TimedOut,
        );
        assert_eq!(reason, DOCKER_SETUP_WAITING);
        assert_eq!(
            failure_sentence(ContainerEngine::Docker, true, &reason),
            format!(
                "Não foi possível ligar o Docker. O projeto não foi iniciado. {DOCKER_SETUP_WAITING}"
            )
        );
        let podman = EngineStart::PodmanMachine {
            name: "podman-machine-default".into(),
            create: false,
        };
        assert_eq!(
            failure_reason(ContainerEngine::Podman, &podman, &StartFailure::TimedOut),
            "O Podman não respondeu a tempo."
        );
        assert_eq!(
            failure_reason(
                ContainerEngine::Podman,
                &podman,
                &StartFailure::Command("x".into())
            ),
            "O comando para ligar o Podman falhou."
        );
    }
}
