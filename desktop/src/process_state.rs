//! The managed processes and their ports (T025): what runs, the ports the app assigned or
//! found, port conflicts, failures, the pending request to end a port's owner and the process
//! shown in the terminal. Only processes the app started are kept here; closing the app or
//! "Parar" ends only these and their children.

use super::*;

#[derive(Default)]
pub(super) struct ManagedProcesses {
    pub processes: Vec<ProjectProcess>,
    pub running: HashMap<String, RunningProcess>,
    pub assigned_ports: HashMap<String, Port>,
    pub port_conflicts: HashMap<String, PortConflict>,
    pub failures: HashMap<String, ProcessFailure>,
    pub port_termination: Option<PortTerminationRequest>,
    /// The process whose output the terminal panel shows.
    pub terminal: Option<String>,
    /// Steps running before a process starts (compile before run, T028.02).
    pub builds: HashMap<String, super::build_steps::BuildJob>,
    /// Output of the last steps of a process whose program did not start.
    pub build_logs: HashMap<String, VecDeque<String>>,
    /// Steps finished for the start in progress: the next pass starts the program.
    pub built: HashSet<String>,
}

impl ManagedProcesses {
    /// Managed processes that would end if the app closed now.
    pub(super) fn active_runs(&self) -> Vec<ActiveRun> {
        self.processes
            .iter()
            .filter(|process| {
                matches!(
                    process.state,
                    ProcessState::Starting | ProcessState::Running
                )
            })
            .map(|process| ActiveRun {
                id: process.id.clone(),
                pid: process.process_id,
            })
            .collect()
    }

    pub(super) fn port_is_available(&self, port: Port) -> bool {
        !self
            .assigned_ports
            .values()
            .any(|assigned| *assigned == port)
            && matches!(inspect_tcp_port(port), PortAvailability::Available)
    }

    pub(super) fn has_active_processes(&self) -> bool {
        self.running.values().any(|running| !running.finished)
    }

    pub(super) fn command_running(&self, id: &str) -> bool {
        self.processes.iter().any(|process| {
            process.id == id
                && matches!(
                    process.state,
                    ProcessState::Running | ProcessState::Starting
                )
        })
    }

    pub(super) fn project_is_running(&self, project_path: &Path) -> bool {
        project_process_ids(&self.processes, project_path, false)
            .iter()
            .filter_map(|id| self.processes.iter().find(|process| process.id == *id))
            .any(|process| {
                matches!(
                    process.state,
                    ProcessState::Running | ProcessState::Starting
                )
            })
    }
}

pub(super) struct RunningProcess {
    pub(super) child: Child,
    pub(super) output: Receiver<String>,
    pub(super) logs: VecDeque<String>,
    pub(super) urls: Vec<LocalServerUrl>,
    pub(super) finished: bool,
    pub(super) started_at: Instant,
    pub(super) last_health_check: Instant,
    pub(super) port_unavailable_since: Option<Instant>,
    /// Listener confirmed to be this process or one of its children.
    pub(super) verified_listener: Option<VerifiedListener>,
    /// Another program listening on the port reserved for this process.
    pub(super) foreign_listener: Option<PortOwner>,
    pub(super) http_health: Option<HttpHealthCheckResult>,
    pub(super) http_health_receiver: Option<Receiver<HttpHealthCheckResult>>,
    pub(super) last_http_health_check: Option<Instant>,
    /// Asks the system which ports this process (or a child) listens on, for servers that
    /// do not print their address or were not started on a known port.
    pub(super) listener_probe: Option<Receiver<Vec<Port>>>,
    pub(super) last_listener_probe: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HttpHealthCheckResult {
    pub(super) url: String,
    pub(super) status: HttpHealthStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HttpHealthStatus {
    Healthy {
        status_code: u16,
        response_time_ms: u128,
    },
    Attention {
        status_code: u16,
        response_time_ms: u128,
    },
    Unreachable {
        error: String,
    },
}

#[derive(Debug, Clone)]
pub(super) struct PortConflict {
    pub(super) port: Port,
    pub(super) owner: Option<PortOwner>,
}

/// Why a process failed, kept so the project card still explains it after the notification
/// is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProcessFailure {
    FolderMissing(PathBuf),
    ProgramMissing(String),
    /// Another program holds the port, found before starting or while waiting for it.
    PortTaken {
        port: Port,
        owner: Option<PortOwner>,
    },
    NoResponse(Port),
    ClosedBeforeReady,
    PortLost(Port),
    Exited,
    SpawnFailed(String),
    WaitFailed(String),
    /// A step before the program (configure, compile) failed (T028.02).
    BuildFailed(String),
}

impl ProcessFailure {
    /// One sentence for the card: no PID, command or path.
    pub(super) fn sentence(&self) -> String {
        match self {
            Self::FolderMissing(_) => "A pasta do projeto não existe mais.".into(),
            Self::ProgramMissing(program) => {
                format!("O programa {program} não foi encontrado neste computador.")
            }
            Self::PortTaken { port, .. } => format!(
                "Outro programa está usando a porta {}. Este projeto não iniciou.",
                port.value()
            ),
            Self::NoResponse(port) => format!(
                "O servidor não respondeu na porta {} a tempo. Veja os logs para investigar.",
                port.value()
            ),
            Self::ClosedBeforeReady => {
                "O servidor fechou antes de ficar pronto. Veja os logs para investigar.".into()
            }
            Self::PortLost(port) => {
                format!("O servidor parou de responder na porta {}.", port.value())
            }
            Self::Exited => "O processo fechou inesperadamente.".into(),
            Self::SpawnFailed(_) => "Não foi possível iniciar o processo.".into(),
            Self::WaitFailed(_) => "Não foi possível acompanhar o processo.".into(),
            Self::BuildFailed(_) => "Não foi possível compilar o projeto. Veja os logs.".into(),
        }
    }

    /// Lines for "Detalhes técnicos".
    pub(super) fn details(&self) -> Vec<String> {
        match self {
            Self::FolderMissing(path) => vec![format!("Pasta procurada: {}", display_path(path))],
            Self::ProgramMissing(program) => vec![format!("Programa procurado no PATH: {program}")],
            Self::PortTaken {
                port,
                owner: Some(owner),
            } => vec![port_owner_detail(*port, owner)],
            Self::PortTaken { port, owner: None } => vec![format!(
                "A porta {} está em uso; o sistema não informou por qual programa.",
                port.value()
            )],
            Self::NoResponse(_) => vec![format!(
                "Tempo de espera pela porta: {} segundos",
                PROCESS_START_TIMEOUT.as_secs()
            )],
            Self::PortLost(_) => vec![format!(
                "Sem resposta na porta por {} segundos; o processo foi encerrado.",
                PROCESS_PORT_LOSS_GRACE.as_secs()
            )],
            Self::ClosedBeforeReady | Self::Exited => vec![],
            Self::SpawnFailed(error) | Self::WaitFailed(error) | Self::BuildFailed(error) => {
                vec![error.clone()]
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PortTerminationRequest {
    pub(super) process_id: String,
    pub(super) conflict: PortConflict,
}
