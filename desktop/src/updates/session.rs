//! The state of an update while the app runs: the check, the offer, the download, the
//! confirmation before ending managed processes, the progress and the restart (T023).
//! The screens read it; the app feeds it the processes that are running and saves the
//! results that belong to the workspace.

use super::{
    PreparedUpdate, UpdateFailure, UpdateFailureKind, UpdateInfo, UpdateInstallEvent, UpdatePhase,
    cached_update_is_newer, spawn_update_download,
};
use crate::update_progress_ui::UpdateProgress;
use eframe::egui;
use std::{
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

/// A managed process that would end if the app closed now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveRun {
    pub id: String,
    pub pid: Option<u32>,
}

/// Runs that the last confirmation did not cover. A run confirmed while it was still
/// starting (no process id yet) stays covered once it gets one.
pub(crate) fn runs_not_confirmed(active: &[ActiveRun], confirmed: &[ActiveRun]) -> Vec<ActiveRun> {
    active
        .iter()
        .filter(|run| {
            !confirmed
                .iter()
                .any(|done| done.id == run.id && (done.pid == run.pid || done.pid.is_none()))
        })
        .cloned()
        .collect()
}

/// Where the update stopped to ask before ending managed processes.
pub(crate) enum UpdateConfirmation {
    /// "Atualizar agora" was clicked with projects running: nothing downloaded yet.
    BeforeDownload(UpdateInfo),
    /// The file is ready, but a project started after the last confirmation.
    BeforeRestart(PreparedUpdate),
}

/// What a finished check brings for the workspace to save.
pub(crate) struct CheckOutcome {
    pub cached_update: Option<UpdateInfo>,
}

#[derive(Default)]
pub(crate) struct UpdateSession {
    pub check: Option<Receiver<Result<Option<UpdateInfo>, UpdateFailure>>>,
    pub available: Option<UpdateInfo>,
    pub manual_check: bool,
    pub check_result: Option<Result<String, UpdateFailure>>,
    install: Option<Receiver<UpdateInstallEvent>>,
    pub install_version: Option<String>,
    install_progress: Option<(u64, Option<u64>)>,
    install_phase: Option<UpdatePhase>,
    pub install_error: Option<UpdateFailure>,
    /// The updater was launched; the app shows "Reiniciando" until it closes.
    pub restart_at: Option<Instant>,
    /// Where the update stopped to ask before ending managed processes.
    pub confirmation: Option<UpdateConfirmation>,
    /// Gives the safe choice ("Agora não") the keyboard focus when the confirmation opens.
    pub confirmation_focus: bool,
    /// The runs the person agreed to end in the last confirmation.
    pub confirmed_runs: Vec<ActiveRun>,
}

impl UpdateSession {
    pub(crate) fn new(
        check: Option<Receiver<Result<Option<UpdateInfo>, UpdateFailure>>>,
        available: Option<UpdateInfo>,
        install_error: Option<UpdateFailure>,
    ) -> Self {
        Self {
            check,
            available,
            install_error,
            ..Self::default()
        }
    }

    pub(crate) fn in_progress(&self) -> bool {
        self.install.is_some()
            || self.restart_at.is_some()
            || matches!(
                self.confirmation,
                Some(UpdateConfirmation::BeforeRestart(_))
            )
    }

    /// A última tentativa foi recusada por segurança (checksum, assinatura ou arquivo inválido).
    pub(crate) fn refused_for_safety(&self) -> bool {
        self.install_error
            .as_ref()
            .is_some_and(|failure| !failure.can_retry())
    }

    pub(crate) fn progress(&self) -> UpdateProgress {
        UpdateProgress::new(
            self.install_phase,
            self.install_progress,
            self.restart_at.is_some(),
        )
    }

    /// Receives the result of a check. Returns what the workspace must save when the check
    /// succeeded. `now` and `remind_after` decide whether the update is offered now.
    pub(crate) fn poll_check(&mut self, now: u64, remind_after: u64) -> Option<CheckOutcome> {
        let receiver = self.check.as_ref()?;
        match receiver.try_recv() {
            Ok(Ok(update)) => {
                self.check_result = Some(Ok(update.as_ref().map_or_else(
                    || "Você está usando a versão mais recente disponível para este canal.".into(),
                    |update| format!("Atualização disponível: {}", update.version),
                )));
                let manual = self.manual_check;
                self.available = update
                    .clone()
                    .filter(cached_update_is_newer)
                    .filter(|_| manual || remind_after <= now);
                // Uma consulta nova pode trazer arquivos corrigidos: a próxima instalação
                // verifica tudo de novo, então o erro anterior deixa de valer.
                // O instalador que não concluiu é outro caso: o aviso fica até a pessoa tentar de novo.
                if !self.in_progress()
                    && self
                        .install_error
                        .as_ref()
                        .is_none_or(|failure| failure.kind != UpdateFailureKind::Installer)
                {
                    self.install_error = None;
                }
                self.manual_check = false;
                self.check = None;
                Some(CheckOutcome {
                    cached_update: update,
                })
            }
            Ok(Err(error)) => {
                self.check_result = Some(Err(error));
                self.manual_check = false;
                self.check = None;
                None
            }
            Err(TryRecvError::Disconnected) => {
                self.check_result = Some(Err(UpdateFailure::new(
                    UpdateFailureKind::NoConnection,
                    "a consulta de atualizações foi interrompida",
                )));
                self.manual_check = false;
                self.check = None;
                None
            }
            Err(TryRecvError::Empty) => None,
        }
    }

    /// "Atualizar agora": with projects running, ask first (CA-5 da T002); otherwise download.
    pub(crate) fn request_install(
        &mut self,
        update: &UpdateInfo,
        active: &[ActiveRun],
        ctx: egui::Context,
    ) {
        if self.in_progress() || self.confirmation.is_some() {
            return;
        }
        if active.is_empty() {
            self.confirmed_runs.clear();
            self.start_install(update, ctx);
        } else {
            self.confirmation = Some(UpdateConfirmation::BeforeDownload(update.clone()));
            self.confirmation_focus = true;
        }
    }

    pub(crate) fn start_install(&mut self, update: &UpdateInfo, ctx: egui::Context) {
        if self.in_progress() {
            return;
        }
        self.install_error = None;
        self.install_progress = Some((0, None));
        self.install_phase = None;
        self.install_version = Some(update.version.clone());
        match spawn_update_download(update, ctx) {
            Ok(receiver) => {
                self.install = Some(receiver);
            }
            Err(error) => {
                self.install_progress = None;
                self.install_version = None;
                self.install_error = Some(error);
            }
        }
    }

    /// The updater was launched: the window closes after a short pause.
    pub(crate) fn launched(&mut self, close_delay: Duration, ctx: &egui::Context) {
        // The updater waits for this process to exit; a short pause
        // lets the person read "Instalando" before the window closes.
        // The updater window goes on from that same step.
        self.restart_at = Some(Instant::now() + Duration::from_millis(1200) + close_delay);
        ctx.request_repaint();
    }

    /// "Agora não" (or Escape) in the confirmation: nothing is downloaded or installed, and the
    /// app and its processes keep running. A file already prepared is discarded.
    pub(crate) fn decline(&mut self) {
        if let Some(UpdateConfirmation::BeforeRestart(prepared)) = self.confirmation.take() {
            super::discard_prepared_update(prepared);
            self.install_version = None;
            self.install_phase = None;
        }
        self.confirmation = None;
        self.confirmed_runs.clear();
    }

    pub(crate) fn launch_failed(&mut self, error: String) {
        self.install_version = None;
        self.install_error = Some(UpdateFailure::new(UpdateFailureKind::Local, error));
    }

    /// Follows the download and the restart. Returns the prepared update when it is time to
    /// launch it: every running project was covered by the last confirmation.
    pub(crate) fn poll_install(
        &mut self,
        active: &[ActiveRun],
        ctx: &egui::Context,
    ) -> Option<PreparedUpdate> {
        if let Some(restart_at) = self.restart_at {
            let now = Instant::now();
            if now >= restart_at {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(restart_at - now);
            }
        }
        let mut events = Vec::new();
        let mut disconnected = false;
        if let Some(receiver) = &self.install {
            loop {
                match receiver.try_recv() {
                    Ok(event) => events.push(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        let mut launch = None;
        for event in events {
            match event {
                UpdateInstallEvent::Phase(phase) => {
                    self.install_phase = Some(phase);
                }
                UpdateInstallEvent::Progress {
                    downloaded_bytes,
                    total_bytes,
                } => {
                    self.install_progress = Some((downloaded_bytes, total_bytes));
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                UpdateInstallEvent::Prepared(prepared) => {
                    self.install = None;
                    self.install_progress = None;
                    // A project started during the download was not part of the choice:
                    // ask again before closing.
                    if runs_not_confirmed(active, &self.confirmed_runs).is_empty() {
                        launch = Some(prepared);
                    } else {
                        self.confirmation = Some(UpdateConfirmation::BeforeRestart(prepared));
                        self.confirmation_focus = true;
                        ctx.request_repaint();
                    }
                }
                UpdateInstallEvent::Failed(error) => {
                    self.install = None;
                    self.install_progress = None;
                    self.install_version = None;
                    self.install_error = Some(error);
                }
            }
        }
        if disconnected && self.install.is_some() {
            self.install = None;
            self.install_progress = None;
            self.install_version = None;
            self.install_error = Some(UpdateFailure::new(
                UpdateFailureKind::Download,
                "o download da atualização foi interrompido",
            ));
        }
        launch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn run(id: &str, pid: Option<u32>) -> ActiveRun {
        ActiveRun {
            id: id.to_owned(),
            pid,
        }
    }

    fn update(version: &str) -> UpdateInfo {
        UpdateInfo {
            version: version.into(),
            title: format!("LocalCodePilot {version}"),
            release_url: "https://example.com/release".into(),
            download_url: "https://example.com/app.zip".into(),
            executable_url: None,
            checksum_url: None,
            summary: None,
            installer_url: None,
        }
    }

    #[test]
    fn a_project_started_after_the_confirmation_is_asked_about_again() {
        let confirmed = vec![run("loja", Some(10))];
        // Nothing new: the update goes on.
        assert!(runs_not_confirmed(&[run("loja", Some(10))], &confirmed).is_empty());
        // Nothing running any more: also fine.
        assert!(runs_not_confirmed(&[], &confirmed).is_empty());
        // Another project started during the download.
        assert_eq!(
            runs_not_confirmed(&[run("loja", Some(10)), run("blog", Some(20))], &confirmed),
            vec![run("blog", Some(20))]
        );
        // The same project stopped and started again: a new run, so it is asked again.
        assert_eq!(
            runs_not_confirmed(&[run("loja", Some(11))], &confirmed),
            vec![run("loja", Some(11))]
        );
        // Without any confirmation (nothing was running at the click), every run is new.
        assert_eq!(
            runs_not_confirmed(&[run("blog", None)], &[]),
            vec![run("blog", None)]
        );
    }

    #[test]
    fn a_run_confirmed_while_starting_stays_covered_once_it_runs() {
        // Confirmed in "Iniciando", before the operating system gave it an id.
        let confirmed = vec![run("loja", None)];
        assert!(runs_not_confirmed(&[run("loja", Some(42))], &confirmed).is_empty());
    }

    #[test]
    fn a_finished_check_reports_the_result_and_what_the_workspace_saves() {
        let (sender, receiver) = mpsc::channel();
        let mut session = UpdateSession::new(Some(receiver), None, None);
        assert!(session.poll_check(10, 0).is_none(), "nothing arrived yet");
        sender.send(Ok(None)).unwrap();
        let outcome = session.poll_check(10, 0).expect("the check finished");
        assert!(outcome.cached_update.is_none());
        assert_eq!(
            session.check_result,
            Some(Ok(
                "Você está usando a versão mais recente disponível para este canal.".into()
            ))
        );
        assert!(session.check.is_none());
        assert!(!session.manual_check);
    }

    #[test]
    fn a_failed_or_interrupted_check_keeps_the_error_and_saves_nothing() {
        let (sender, receiver) = mpsc::channel();
        let mut session = UpdateSession::new(Some(receiver), None, None);
        session.manual_check = true;
        let failure = UpdateFailure::new(UpdateFailureKind::NoConnection, "sem rede");
        sender.send(Err(failure.clone())).unwrap();
        assert!(session.poll_check(10, 0).is_none());
        assert_eq!(session.check_result, Some(Err(failure)));
        assert!(!session.manual_check);

        let (sender, receiver) = mpsc::channel::<Result<Option<UpdateInfo>, UpdateFailure>>();
        let mut session = UpdateSession::new(Some(receiver), None, None);
        drop(sender);
        assert!(session.poll_check(10, 0).is_none());
        assert!(matches!(
            session.check_result,
            Some(Err(UpdateFailure {
                kind: UpdateFailureKind::NoConnection,
                ..
            }))
        ));
    }

    #[test]
    fn a_new_check_clears_a_retryable_error_but_not_an_installer_failure() {
        for (kind, cleared) in [
            (UpdateFailureKind::Download, true),
            (UpdateFailureKind::Installer, false),
        ] {
            let (sender, receiver) = mpsc::channel();
            let mut session = UpdateSession::new(
                Some(receiver),
                None,
                Some(UpdateFailure::new(kind, "falhou")),
            );
            sender.send(Ok(Some(update("0.0.1")))).unwrap();
            session.poll_check(10, 0);
            assert_eq!(session.install_error.is_none(), cleared, "{kind:?}");
        }
    }

    #[test]
    fn with_projects_running_the_update_asks_before_downloading() {
        let mut session = UpdateSession::default();
        session.request_install(
            &update("99.0.0"),
            &[run("loja", Some(10))],
            egui::Context::default(),
        );
        assert!(matches!(
            session.confirmation,
            Some(UpdateConfirmation::BeforeDownload(_))
        ));
        assert!(session.confirmation_focus);
        assert!(
            session.install.is_none(),
            "nothing is downloaded before the answer"
        );
        assert!(!session.in_progress());

        // "Agora não": nothing downloaded, nothing confirmed.
        session.confirmed_runs = vec![run("loja", Some(10))];
        session.decline();
        assert!(session.confirmation.is_none());
        assert!(session.confirmed_runs.is_empty());
        assert!(session.install.is_none());
    }

    #[test]
    fn a_refused_update_is_told_apart_from_one_that_can_be_retried() {
        let mut session = UpdateSession::default();
        assert!(!session.refused_for_safety());
        session.install_error = Some(UpdateFailure::new(UpdateFailureKind::Download, "caiu"));
        assert!(!session.refused_for_safety());
        session.install_error = Some(UpdateFailure::new(UpdateFailureKind::Verification, "hash"));
        assert!(session.refused_for_safety());
    }

    #[test]
    fn a_launch_failure_is_a_local_error_and_a_launch_closes_after_a_pause() {
        let mut session = UpdateSession {
            install_version: Some("99.0.0".into()),
            ..UpdateSession::default()
        };
        session.launch_failed("sem permissão".into());
        assert!(session.install_version.is_none());
        assert_eq!(
            session.install_error.as_ref().map(|failure| failure.kind),
            Some(UpdateFailureKind::Local)
        );
        let ctx = egui::Context::default();
        session.launched(Duration::ZERO, &ctx);
        assert!(session.in_progress());
        assert!(session.restart_at.is_some_and(|at| at > Instant::now()));
    }
}
