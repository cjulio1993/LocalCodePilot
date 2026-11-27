//! Steps that run before a process starts, such as configuring and compiling a C/C++ project
//! (T028.02). They run in order on a worker thread, their output goes to the process logs,
//! "Parar" interrupts them, and a failure keeps the program from starting.

use super::*;
use localcodepilot_core::processes::PrepareStep;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) const BUILDING: &str = "Compilando o projeto…";

pub(super) enum BuildEvent {
    Line(String),
    Done(Result<(), String>),
}

pub(super) struct BuildJob {
    pub receiver: Receiver<BuildEvent>,
    pub cancel: Arc<AtomicBool>,
    /// The step running now, to interrupt it.
    pub current: Arc<Mutex<Option<u32>>>,
    pub logs: VecDeque<String>,
}

const MAX_BUILD_LOG_LINES: usize = 2000;

fn run_steps(
    steps: &[PrepareStep],
    directory: &Path,
    path: Option<&OsStr>,
    sender: &mpsc::Sender<BuildEvent>,
    cancel: &AtomicBool,
    current: &Mutex<Option<u32>>,
) -> Result<(), String> {
    for step in steps {
        if cancel.load(Ordering::Relaxed) {
            return Err("compilação interrompida".into());
        }
        if step
            .skip_if_exists
            .as_ref()
            .is_some_and(|marker| marker.exists())
        {
            let _ = sender.send(BuildEvent::Line(format!(
                "[LocalCodePilot] Já configurado; etapa pulada: {}",
                step.command_line()
            )));
            continue;
        }
        let _ = sender.send(BuildEvent::Line(format!("$ {}", step.command_line())));
        let program = find_executable(&step.program, path)
            .ok_or_else(|| format!("o programa '{}' não foi encontrado no PATH", step.program))?;
        let mut command = Command::new(program);
        command
            .args(&step.args)
            .current_dir(plain_path(directory))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = path {
            command.env("PATH", path);
        }
        localcodepilot_platform::installations::configure_background_command(&mut command);
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        if let Ok(mut running) = current.lock() {
            *running = Some(child.id());
        }
        let (lines_sender, lines) = mpsc::channel();
        if let Some(stdout) = child.stdout.take() {
            spawn_output_reader(stdout, lines_sender.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_output_reader(stderr, lines_sender);
        }
        let status = loop {
            while let Ok(line) = lines.try_recv() {
                let _ = sender.send(BuildEvent::Line(line));
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => return Err(error.to_string()),
            }
        };
        // The readers end with the output; a grandchild that kept it open is not waited for.
        let deadline = Instant::now() + Duration::from_secs(3);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match lines.recv_timeout(left) {
                Ok(line) => {
                    let _ = sender.send(BuildEvent::Line(line));
                }
                Err(_) => break,
            }
        }
        if let Ok(mut running) = current.lock() {
            *running = None;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("compilação interrompida".into());
        }
        if !status.success() {
            return Err(format!(
                "{} terminou com o código {}",
                step.command_line(),
                status
                    .code()
                    .map_or_else(|| "?".into(), |code| code.to_string())
            ));
        }
    }
    Ok(())
}

impl LocalCodePilot {
    /// Runs the steps of `process` on a worker thread; the process starts when they finish.
    pub(super) fn start_build(&mut self, index: usize, ctx: &egui::Context) {
        let process = self.managed.processes[index].clone();
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let current = Arc::new(Mutex::new(None));
        let path = self.environment_path.clone();
        let (thread_cancel, thread_current) = (cancel.clone(), current.clone());
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = run_steps(
                &process.prepare,
                &process.working_directory,
                path.as_deref(),
                &sender,
                &thread_cancel,
                &thread_current,
            );
            let _ = sender.send(BuildEvent::Done(result));
            repaint.request_repaint();
        });
        self.managed.processes[index].state = ProcessState::Starting;
        self.managed.processes[index].exit_code = None;
        self.managed
            .failures
            .remove(&self.managed.processes[index].id);
        self.managed.builds.insert(
            self.managed.processes[index].id.clone(),
            BuildJob {
                receiver,
                cancel,
                current,
                logs: VecDeque::new(),
            },
        );
        self.notify(format!(
            "{}: {BUILDING}",
            self.managed.processes[index].name
        ));
    }

    pub(super) fn poll_builds(&mut self, ctx: &egui::Context) {
        let ids = self.managed.builds.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let mut finished = None;
            if let Some(job) = self.managed.builds.get_mut(&id) {
                while let Ok(event) = job.receiver.try_recv() {
                    match event {
                        BuildEvent::Line(line) => {
                            job.logs.push_back(crate::secrets::mask_secrets(&line));
                            while job.logs.len() > MAX_BUILD_LOG_LINES {
                                job.logs.pop_front();
                            }
                        }
                        BuildEvent::Done(result) => {
                            finished = Some(result);
                            break;
                        }
                    }
                }
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            let Some(result) = finished else {
                continue;
            };
            let Some(job) = self.managed.builds.remove(&id) else {
                continue;
            };
            match result {
                Ok(()) => {
                    self.managed.build_logs.insert(id.clone(), job.logs);
                    self.managed.built.insert(id.clone());
                    self.start_process(&id);
                    // The compilation output stays at the top of the program's logs.
                    if let Some(logs) = self.managed.build_logs.remove(&id)
                        && let Some(running) = self.managed.running.get_mut(&id)
                    {
                        let mut all = logs;
                        all.extend(running.logs.drain(..));
                        running.logs = all;
                    }
                }
                Err(error) => {
                    let mut logs = job.logs;
                    logs.push_back(format!("[LocalCodePilot] {error}"));
                    self.managed.build_logs.insert(id.clone(), logs);
                    if let Some(process) = self
                        .managed
                        .processes
                        .iter_mut()
                        .find(|process| process.id == id)
                    {
                        process.state = ProcessState::Failed;
                        process.process_id = None;
                    }
                    self.managed
                        .failures
                        .insert(id.clone(), ProcessFailure::BuildFailed(error));
                    self.notify(ProcessFailure::BuildFailed(String::new()).sentence());
                }
            }
        }
    }

    /// "Parar" during the steps: the running step and everything it opened are ended.
    pub(super) fn stop_build(&mut self, id: &str) -> bool {
        let Some(job) = self.managed.builds.remove(id) else {
            return false;
        };
        job.cancel.store(true, Ordering::Relaxed);
        if let Ok(current) = job.current.lock()
            && let Some(pid) = *current
        {
            let _ = terminate_process_tree(pid);
        }
        let mut logs = job.logs;
        logs.push_back("[LocalCodePilot] Compilação interrompida.".into());
        self.managed.build_logs.insert(id.to_owned(), logs);
        if let Some(process) = self
            .managed
            .processes
            .iter_mut()
            .find(|process| process.id == id)
        {
            process.state = ProcessState::Stopped;
            process.process_id = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(args: &[&str], skip_if_exists: Option<PathBuf>) -> PrepareStep {
        PrepareStep {
            program: "cargo".into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            skip_if_exists,
        }
    }

    fn run(steps: &[PrepareStep]) -> (Result<(), String>, Vec<String>) {
        let (sender, receiver) = mpsc::channel();
        let path = std::env::var_os("PATH");
        let result = run_steps(
            steps,
            &std::env::temp_dir(),
            path.as_deref(),
            &sender,
            &AtomicBool::new(false),
            &Mutex::new(None),
        );
        drop(sender);
        let lines = receiver
            .into_iter()
            .filter_map(|event| match event {
                BuildEvent::Line(line) => Some(line),
                BuildEvent::Done(_) => None,
            })
            .collect();
        (result, lines)
    }

    #[test]
    fn steps_run_in_order_skip_what_is_configured_and_stop_at_a_failure() {
        let marker = std::env::temp_dir();
        let (result, lines) = run(&[
            step(&["--version"], Some(marker)),
            step(&["--version"], None),
        ]);
        assert!(result.is_ok());
        assert!(lines[0].contains("etapa pulada"));
        assert!(lines.iter().any(|line| line == "$ cargo --version"));
        assert!(lines.iter().any(|line| line.starts_with("cargo ")));

        let (result, lines) = run(&[
            step(&["subcomando-que-nao-existe-lcp"], None),
            step(&["--version"], None),
        ]);
        assert!(result.unwrap_err().contains("terminou com o código"));
        assert!(!lines.iter().any(|line| line == "$ cargo --version"));
    }

    #[test]
    fn an_interrupted_build_runs_no_more_steps() {
        let (sender, _receiver) = mpsc::channel();
        let result = run_steps(
            &[step(&["--version"], None)],
            &std::env::temp_dir(),
            None,
            &sender,
            &AtomicBool::new(true),
            &Mutex::new(None),
        );
        assert_eq!(result, Err("compilação interrompida".into()));
    }
}
