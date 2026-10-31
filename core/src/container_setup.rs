//! Guided preparation of Docker and Podman. The app only explains: every command here is
//! shown for the user to run, never executed by LocalCodePilot.

use crate::containers::{ContainerEngine, EngineDiagnosis, EngineState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Windows,
    MacOs,
    Linux,
}

impl HostOs {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// State of the Podman virtual machine on Windows and macOS, from `podman machine list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PodmanMachine {
    NotCreated,
    Stopped,
    Running,
}

/// Read-only facts about the host gathered alongside the engine diagnosis. `None` means the
/// fact does not apply to this system or could not be queried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerHostFacts {
    pub os: HostOs,
    pub wsl_ready: Option<bool>,
    pub podman_machine: Option<PodmanMachine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupStep {
    pub text: &'static str,
    /// Shown for the user to copy and run in a terminal.
    pub command: Option<&'static str>,
    pub link: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineGuidance {
    pub summary: &'static str,
    pub steps: Vec<SetupStep>,
    pub notes: Vec<&'static str>,
}

const WSL_DOCS: &str = "https://learn.microsoft.com/windows/wsl/install";
const DOCKER_WINDOWS_DOCS: &str = "https://docs.docker.com/desktop/setup/install/windows-install/";
const DOCKER_MAC_DOCS: &str = "https://docs.docker.com/desktop/setup/install/mac-install/";
const DOCKER_ENGINE_DOCS: &str = "https://docs.docker.com/engine/install/";
const DOCKER_LICENSE_DOCS: &str = "https://docs.docker.com/subscription/desktop-license/";
const DOCKER_COMPOSE_DOCS: &str = "https://docs.docker.com/compose/install/";
const PODMAN_DOCS: &str = "https://podman.io/docs/installation";
const PODMAN_COMPOSE_DOCS: &str = "https://docs.podman.io/en/latest/markdown/podman-compose.1.html";

const DOCKER_LICENSE_NOTE: &str = "Docker Desktop é gratuito para uso pessoal, educação, projetos open source não comerciais e pequenas empresas; empresas maiores precisam de assinatura. Confira os termos atuais antes de instalar.";
const PODMAN_LICENSE_NOTE: &str = "Podman é open source (Apache 2.0) e não exige assinatura.";
const MANUAL_NOTE: &str = "O LocalCodePilot não executa estes comandos. Rode-os no terminal e depois verifique os motores novamente.";

fn step(text: &'static str) -> SetupStep {
    SetupStep {
        text,
        command: None,
        link: None,
    }
}

fn command(text: &'static str, command: &'static str) -> SetupStep {
    SetupStep {
        text,
        command: Some(command),
        link: None,
    }
}

fn link(text: &'static str, link: &'static str) -> SetupStep {
    SetupStep {
        text,
        command: None,
        link: Some(link),
    }
}

/// Steps that take the engine from its diagnosed state to ready with Compose. Returns `None`
/// when nothing is missing. Compose is only diagnosed once the engine answers: with Podman,
/// `podman compose version` also fails while the machine is stopped.
pub fn engine_guidance(
    diagnosis: &EngineDiagnosis,
    facts: &ContainerHostFacts,
) -> Option<EngineGuidance> {
    let engine = diagnosis.engine;
    let (summary, steps) = match diagnosis.state {
        EngineState::Ready if diagnosis.compose_available => return None,
        EngineState::Ready => (
            "Motor pronto, mas sem Compose",
            compose_steps(engine, facts),
        ),
        EngineState::Missing => ("Motor não instalado", install_steps(engine, facts)),
        EngineState::Unavailable => ("Motor instalado, mas parado", start_steps(engine, facts)),
        EngineState::TimedOut | EngineState::QueryFailed => (
            "O motor não respondeu à consulta",
            vec![command(
                "Rode a consulta no terminal para ver a mensagem de erro completa.",
                match engine {
                    ContainerEngine::Docker => "docker info",
                    ContainerEngine::Podman => "podman info",
                },
            )],
        ),
    };
    let mut notes = Vec::new();
    if diagnosis.state == EngineState::Missing {
        notes.push(match engine {
            ContainerEngine::Docker => DOCKER_LICENSE_NOTE,
            ContainerEngine::Podman => PODMAN_LICENSE_NOTE,
        });
        if engine == ContainerEngine::Docker && facts.os == HostOs::Linux {
            notes.push("No Linux, entrar no grupo docker dá privilégios equivalentes aos de administrador. Decida conscientemente antes de fazer isso.");
        }
    }
    notes.push(MANUAL_NOTE);
    Some(EngineGuidance {
        summary,
        steps,
        notes,
    })
}

fn install_steps(engine: ContainerEngine, facts: &ContainerHostFacts) -> Vec<SetupStep> {
    let mut steps = Vec::new();
    if facts.os == HostOs::Windows && facts.wsl_ready == Some(false) {
        steps.push(command(
            "Instale o WSL 2 num terminal como administrador e reinicie o computador quando pedido. A virtualização precisa estar ativa no firmware.",
            "wsl --install",
        ));
        steps.push(link("Requisitos do WSL", WSL_DOCS));
    }
    if engine == ContainerEngine::Docker && facts.os != HostOs::Linux {
        steps.push(link(
            "Confira se a licença do Docker Desktop se aplica ao seu uso.",
            DOCKER_LICENSE_DOCS,
        ));
    }
    match (engine, facts.os) {
        (ContainerEngine::Docker, HostOs::Windows) => {
            steps.push(command(
                "Instale o Docker Desktop pelo winget ou pelo instalador oficial.",
                "winget install -e --id Docker.DockerDesktop",
            ));
            steps.push(link(
                "Guia oficial do Docker Desktop para Windows",
                DOCKER_WINDOWS_DOCS,
            ));
            steps.push(step(
                "Abra o Docker Desktop, aceite os termos e aguarde o motor iniciar.",
            ));
        }
        (ContainerEngine::Docker, HostOs::MacOs) => {
            steps.push(link(
                "Baixe o Docker Desktop para o seu processador (Apple Silicon ou Intel).",
                DOCKER_MAC_DOCS,
            ));
            steps.push(step(
                "Abra o Docker Desktop, aceite os termos e aguarde o motor iniciar.",
            ));
        }
        (ContainerEngine::Docker, HostOs::Linux) => {
            steps.push(link(
                "Instale o Docker Engine pelo repositório oficial da sua distribuição.",
                DOCKER_ENGINE_DOCS,
            ));
        }
        (ContainerEngine::Podman, HostOs::Windows) => {
            steps.push(command(
                "Instale o Podman pelo winget ou pelo instalador oficial.",
                "winget install -e --id RedHat.Podman",
            ));
            steps.push(link("Guia oficial do Podman", PODMAN_DOCS));
            steps.extend(podman_machine_steps(None));
        }
        (ContainerEngine::Podman, HostOs::MacOs) => {
            steps.push(command(
                "Instale o Podman pelo Homebrew ou pelo instalador oficial.",
                "brew install podman",
            ));
            steps.push(link("Guia oficial do Podman", PODMAN_DOCS));
            steps.extend(podman_machine_steps(None));
        }
        (ContainerEngine::Podman, HostOs::Linux) => {
            steps.push(link(
                "Instale o Podman pelo gerenciador de pacotes da sua distribuição.",
                PODMAN_DOCS,
            ));
        }
    }
    if facts.os != HostOs::Linux {
        steps.push(step(
            "Abra um novo terminal para usar o comando instalado. O LocalCodePilot relê o PATH ao verificar os motores, sem precisar reiniciar.",
        ));
    }
    steps
}

fn start_steps(engine: ContainerEngine, facts: &ContainerHostFacts) -> Vec<SetupStep> {
    match (engine, facts.os) {
        (ContainerEngine::Docker, HostOs::Linux) => vec![command(
            "Inicie o serviço do Docker (exige administrador).",
            "sudo systemctl start docker",
        )],
        (ContainerEngine::Docker, _) => vec![command(
            "Abra o Docker Desktop pelo menu do sistema ou pelo comando abaixo e aguarde o motor iniciar.",
            "docker desktop start",
        )],
        (ContainerEngine::Podman, HostOs::Linux) => vec![command(
            "O Podman no Linux não depende de serviço; rode a consulta para ver o erro.",
            "podman info",
        )],
        (ContainerEngine::Podman, _) => podman_machine_steps(facts.podman_machine),
    }
}

fn podman_machine_steps(machine: Option<PodmanMachine>) -> Vec<SetupStep> {
    let init = command(
        "Crie a máquina virtual do Podman. A primeira vez baixa a imagem do sistema.",
        "podman machine init",
    );
    let start = command(
        "Inicie a máquina virtual do Podman.",
        "podman machine start",
    );
    match machine {
        Some(PodmanMachine::Running) => vec![command(
            "A máquina está ativa, mas a conexão padrão não respondeu. Confira qual conexão está marcada como padrão.",
            "podman system connection list",
        )],
        Some(PodmanMachine::Stopped) => vec![start],
        Some(PodmanMachine::NotCreated) | None => vec![init, start],
    }
}

fn compose_steps(engine: ContainerEngine, facts: &ContainerHostFacts) -> Vec<SetupStep> {
    match (engine, facts.os) {
        (ContainerEngine::Docker, HostOs::Linux) => vec![link(
            "Instale o plugin Docker Compose pelo repositório oficial.",
            DOCKER_COMPOSE_DOCS,
        )],
        (ContainerEngine::Docker, _) => vec![link(
            "O Compose acompanha o Docker Desktop. Atualize ou reinstale o Docker Desktop.",
            DOCKER_COMPOSE_DOCS,
        )],
        (ContainerEngine::Podman, HostOs::Windows) => vec![
            command(
                "O podman compose usa um provedor externo. Instale o Docker Compose (sem o Docker Desktop):",
                "winget install -e --id Docker.DockerCompose",
            ),
            link("Provedores do podman compose", PODMAN_COMPOSE_DOCS),
        ],
        (ContainerEngine::Podman, _) => vec![link(
            "O podman compose usa um provedor externo, como docker-compose ou podman-compose.",
            PODMAN_COMPOSE_DOCS,
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(os: HostOs) -> ContainerHostFacts {
        ContainerHostFacts {
            os,
            wsl_ready: (os == HostOs::Windows).then_some(true),
            podman_machine: None,
        }
    }

    fn diagnosis(engine: ContainerEngine, state: EngineState, compose: bool) -> EngineDiagnosis {
        EngineDiagnosis {
            engine,
            state,
            compose_available: compose,
        }
    }

    fn commands(guidance: &EngineGuidance) -> Vec<&'static str> {
        guidance
            .steps
            .iter()
            .filter_map(|step| step.command)
            .collect()
    }

    #[test]
    fn ready_engine_with_compose_needs_no_guidance() {
        for engine in [ContainerEngine::Docker, ContainerEngine::Podman] {
            for os in [HostOs::Windows, HostOs::MacOs, HostOs::Linux] {
                assert_eq!(
                    engine_guidance(&diagnosis(engine, EngineState::Ready, true), &facts(os)),
                    None
                );
            }
        }
    }

    #[test]
    fn every_guidance_is_manual_and_actionable() {
        for engine in [ContainerEngine::Docker, ContainerEngine::Podman] {
            for os in [HostOs::Windows, HostOs::MacOs, HostOs::Linux] {
                for state in [
                    EngineState::Missing,
                    EngineState::Unavailable,
                    EngineState::TimedOut,
                    EngineState::QueryFailed,
                ] {
                    for compose in [false, true] {
                        let guidance =
                            engine_guidance(&diagnosis(engine, state, compose), &facts(os))
                                .unwrap();
                        assert!(!guidance.steps.is_empty(), "{engine} {os:?} {state:?}");
                        assert!(guidance.notes.contains(&MANUAL_NOTE));
                        assert!(guidance.steps.iter().all(|step| {
                            step.link.is_none_or(|link| link.starts_with("https://"))
                        }));
                    }
                }
                let compose_only =
                    engine_guidance(&diagnosis(engine, EngineState::Ready, false), &facts(os))
                        .unwrap();
                assert_eq!(compose_only.summary, "Motor pronto, mas sem Compose");
            }
        }
    }

    #[test]
    fn windows_installation_checks_wsl_first_and_shows_license_terms() {
        let mut host = facts(HostOs::Windows);
        host.wsl_ready = Some(false);
        let docker = engine_guidance(
            &diagnosis(ContainerEngine::Docker, EngineState::Missing, false),
            &host,
        )
        .unwrap();
        assert_eq!(
            commands(&docker),
            [
                "wsl --install",
                "winget install -e --id Docker.DockerDesktop"
            ]
        );
        assert!(docker.notes.contains(&DOCKER_LICENSE_NOTE));

        host.wsl_ready = Some(true);
        let podman = engine_guidance(
            &diagnosis(ContainerEngine::Podman, EngineState::Missing, false),
            &host,
        )
        .unwrap();
        assert_eq!(
            commands(&podman),
            [
                "winget install -e --id RedHat.Podman",
                "podman machine init",
                "podman machine start"
            ]
        );
        assert!(podman.notes.contains(&PODMAN_LICENSE_NOTE));
    }

    #[test]
    fn stopped_engines_get_start_steps_matching_the_podman_machine() {
        let mut host = facts(HostOs::Windows);
        let docker = engine_guidance(
            &diagnosis(ContainerEngine::Docker, EngineState::Unavailable, true),
            &host,
        )
        .unwrap();
        assert_eq!(commands(&docker), ["docker desktop start"]);

        for (machine, expected) in [
            (
                PodmanMachine::NotCreated,
                vec!["podman machine init", "podman machine start"],
            ),
            // Compose cannot be confirmed while the machine is stopped; it must not be
            // reported as missing.
            (PodmanMachine::Stopped, vec!["podman machine start"]),
            (
                PodmanMachine::Running,
                vec!["podman system connection list"],
            ),
        ] {
            host.podman_machine = Some(machine);
            let podman = engine_guidance(
                &diagnosis(ContainerEngine::Podman, EngineState::Unavailable, false),
                &host,
            )
            .unwrap();
            assert_eq!(commands(&podman), expected, "{machine:?}");
        }
    }

    #[test]
    fn missing_compose_points_to_a_provider_without_installing_docker_desktop() {
        let podman = engine_guidance(
            &diagnosis(ContainerEngine::Podman, EngineState::Ready, false),
            &facts(HostOs::Windows),
        )
        .unwrap();
        assert_eq!(
            commands(&podman),
            ["winget install -e --id Docker.DockerCompose"]
        );
        let linux_docker = engine_guidance(
            &diagnosis(ContainerEngine::Docker, EngineState::Missing, false),
            &facts(HostOs::Linux),
        )
        .unwrap();
        assert!(
            linux_docker
                .notes
                .iter()
                .any(|note| note.contains("grupo docker"))
        );
    }
}
