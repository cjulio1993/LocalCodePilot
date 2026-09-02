#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Stopped,
    Starting,
    Running,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProcess {
    pub id: String,
    pub command: String,
    pub state: ProcessState,
    pub process_id: Option<u32>,
}
