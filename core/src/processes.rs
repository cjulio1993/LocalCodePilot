use std::path::PathBuf;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectProcess {
    pub id: String,
    pub project_name: String,
    pub project_path: PathBuf,
    pub working_directory: PathBuf,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub state: ProcessState,
    pub process_id: Option<u32>,
}

impl ProjectProcess {
    pub fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}
