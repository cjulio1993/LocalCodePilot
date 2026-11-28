use eframe::egui;
use localcodepilot_platform::installations::configure_background_command;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant, SystemTime},
};

mod recovery;
pub(crate) mod session;
#[cfg(feature = "update-test-feed")]
mod test_feed;

// Simulated engine starts are only for screen tests: they must never ship.
#[cfg(all(feature = "engine-start-test", not(debug_assertions)))]
compile_error!(
    "the engine-start-test feature is only for screen tests and cannot be used in a release build"
);

// The local test feed changes where updates come from: it must never ship.
#[cfg(all(feature = "update-test-feed", not(debug_assertions)))]
compile_error!(
    "the update-test-feed feature is only for end-to-end tests and cannot be used in a release build"
);

const RELEASES_URL: &str =
    "https://api.github.com/repos/cjulio1993/LocalCodePilot/releases?per_page=20";
const RELEASE_ASSET_PREFIX: &str =
    "https://github.com/cjulio1993/LocalCodePilot/releases/download/";
const UPDATE_HELPER_FLAG: &str = "--localcodepilot-apply-update";
const UPDATED_STARTUP_FLAG: &str = "--localcodepilot-updated-startup";
const MAX_EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 1024 * 1024;
/// `SHA256SUMS.txt.sig`: a raw Ed25519 signature (64 bytes) of `SHA256SUMS.txt`.
const SIGNATURE_SUFFIX: &str = ".sig";
const MAX_SIGNATURE_BYTES: u64 = 1024;
/// Ed25519 public key that signs the checksums of every release. The private key
/// exists only as the UPDATE_SIGNING_KEY secret of the release workflow and in an
/// offline backup. Same key as `packaging/update-signing.pub.pem` (checked by a test).
const UPDATE_SIGNING_PUBLIC_KEY: [u8; 32] = [
    0x44, 0x23, 0x1e, 0xdf, 0x52, 0x25, 0x4f, 0xba, 0x79, 0x5b, 0x5c, 0x1a, 0xe0, 0xa0, 0x4c, 0x08,
    0xd0, 0x07, 0xae, 0x24, 0x90, 0x99, 0x53, 0xb5, 0x27, 0x71, 0x64, 0x53, 0x68, 0xad, 0xac, 0x3c,
];
const REPLACEMENT_TIMEOUT: Duration = Duration::from_secs(90);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const STALE_STAGE_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// File names of the downloaded update inside the preparation folder: the helper
/// tells the two kinds of update apart by them.
const EXECUTABLE_CANDIDATE_NAME: &str = "LocalCodePilot.exe";
const INSTALLER_CANDIDATE_NAME: &str = "LocalCodePilotSetup.exe";
/// `AppId` of `packaging/windows/LocalCodePilot.iss` (without the escaping brace).
const INSTALLER_APP_ID: &str = "{8B3C2F1E-6D4A-4C8E-9A51-3F2B7C1D9E40}";
/// Longest wait for the silent installer; it normally takes a few seconds.
const INSTALLER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Copy of the last installer log, kept with the configuration for "Detalhes técnicos".
const INSTALLER_LOG_FILE: &str = "update-setup.log";
/// Structured result of an installer that did not finish, read by the reopened version.
const INSTALLER_FAILURE_FILE: &str = "update-failure.json";
pub const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub title: String,
    pub release_url: String,
    pub download_url: String,
    #[serde(default)]
    pub executable_url: Option<String>,
    #[serde(default)]
    pub checksum_url: Option<String>,
    /// The short "what's new" sentence from the release notes (see `release_summary`).
    /// Caches saved by older versions have none and show the fallback text.
    #[serde(default)]
    pub summary: Option<String>,
    /// The official installer (`LocalCodePilotSetup-<version>-windows-x64.exe`). An app
    /// installed by it is updated by running it silently (T002 CA-8).
    #[serde(default)]
    pub installer_url: Option<String>,
}

/// Longest summary shown in the update notice. A longer sentence is not cut (a cut could
/// change its meaning): the notice shows the fallback text instead.
pub const SUMMARY_MAX_CHARS: usize = 140;

/// The "what's new" line of the update notice (T002 CA-2).
pub fn summary_line(update: &UpdateInfo) -> String {
    match &update.summary {
        Some(summary) => format!("O que há de novo: {summary}"),
        None => "Confira as novidades na página da versão.".to_owned(),
    }
}

/// The summary of an official release: the bold sentence that opens the first paragraph
/// after the title, as in "**Mais seguro para abrir projetos.** Versão alpha para...".
/// Only that position counts, so bold text elsewhere never becomes a summary by chance.
/// The result is plain text (no Markdown, links or control characters) of at most
/// `SUMMARY_MAX_CHARS` characters; anything else gives no summary.
fn release_summary(body: &str) -> Option<String> {
    let mut lines = body
        .lines()
        .map(str::trim)
        .skip_while(|line| line.is_empty());
    let mut first = lines.next()?;
    if first.starts_with("# ") {
        first = lines.find(|line| !line.is_empty())?;
    }
    let inner = first.strip_prefix("**")?;
    let (sentence, _) = inner.split_once("**")?;
    if sentence.contains(['[', ']', '<', '>', '*', '_']) {
        return None;
    }
    let plain = sentence
        .replace('`', "")
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    (!plain.is_empty() && plain.chars().count() <= SUMMARY_MAX_CHARS).then_some(plain)
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// Stage of the background preparation, sent before the matching work starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    Checksums,
    Downloading,
    Verifying,
}

#[derive(Debug)]
pub enum UpdateInstallEvent {
    Phase(UpdatePhase),
    Progress {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    Prepared(PreparedUpdate),
    Failed(UpdateFailure),
}

/// Why checking for or preparing an update failed (T002 CA-6). The app shows a
/// plain title, a sentence and one action per kind; the original error stays in
/// `technical`, behind "Detalhes técnicos". In every kind nothing was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateFailureKind {
    /// The releases could not be reached (no network, DNS, refused, timeout).
    NoConnection,
    /// The release service answered with an error status or something unreadable.
    InvalidResponse,
    /// The download stopped or the server failed while sending the files.
    Download,
    /// A safety check refused the files: signature, checksum, size or format.
    Verification,
    /// Something on this computer failed: folder, disk, permission, starting the updater.
    Local,
    /// The official installer ran but did not finish (blocked, failed or interrupted);
    /// the previous version was checked and reopened. Needs `versions`.
    Installer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFailure {
    pub kind: UpdateFailureKind,
    /// The original error, in the words of the component that failed.
    pub technical: String,
    /// `(current, new)` versions, for the messages that name them.
    pub versions: Option<(String, String)>,
}

impl UpdateFailure {
    pub fn new(kind: UpdateFailureKind, technical: impl Into<String>) -> Self {
        Self {
            kind,
            technical: technical.into(),
            versions: None,
        }
    }

    /// The installer did not finish and the previous version reopened (T002 CA-8).
    pub fn installer(
        current: impl Into<String>,
        new: impl Into<String>,
        technical: impl Into<String>,
    ) -> Self {
        Self {
            kind: UpdateFailureKind::Installer,
            technical: technical.into(),
            versions: Some((current.into(), new.into())),
        }
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            UpdateFailureKind::NoConnection | UpdateFailureKind::InvalidResponse => {
                "Não foi possível verificar atualizações"
            }
            UpdateFailureKind::Download => "O download foi interrompido",
            UpdateFailureKind::Verification => "Atualização recusada por segurança",
            UpdateFailureKind::Local => "Não foi possível preparar a atualização",
            UpdateFailureKind::Installer => "A instalação não foi concluída",
        }
    }

    pub fn message(&self) -> String {
        if self.kind == UpdateFailureKind::Installer {
            let (current, new) = self
                .versions
                .clone()
                .unwrap_or_else(|| ("atual".into(), "nova".into()));
            return format!(
                "Não foi possível instalar a versão {new}. A versão {current} foi reaberta e você pode continuar trabalhando. Tente novamente mais tarde ou baixe o instalador na página oficial."
            );
        }
        match self.kind {
            UpdateFailureKind::NoConnection => {
                "Confira sua conexão e tente novamente. A versão atual continua funcionando."
            }
            UpdateFailureKind::InvalidResponse => {
                "O serviço de atualizações não respondeu como esperado. Tente novamente mais tarde."
            }
            UpdateFailureKind::Download => {
                "A atualização não foi instalada. A versão atual continua funcionando."
            }
            UpdateFailureKind::Verification => {
                "O arquivo não passou na verificação. Nada foi instalado e a versão atual continua funcionando. Não instale manualmente esta atualização."
            }
            UpdateFailureKind::Local | UpdateFailureKind::Installer => {
                "Nada foi instalado e a versão atual continua funcionando. Consulte os detalhes técnicos ou tente novamente mais tarde."
            }
        }
        .to_owned()
    }

    /// A refused file is never offered again as the main action.
    pub fn can_retry(&self) -> bool {
        self.kind != UpdateFailureKind::Verification
    }

    /// Label of the link to the release page, when there is one for this kind.
    pub fn release_page_label(&self) -> Option<&'static str> {
        match self.kind {
            UpdateFailureKind::NoConnection | UpdateFailureKind::InvalidResponse => None,
            UpdateFailureKind::Verification => Some("Consultar página oficial da versão"),
            UpdateFailureKind::Download
            | UpdateFailureKind::Local
            | UpdateFailureKind::Installer => Some("Ver página oficial da versão"),
        }
    }
}

impl std::fmt::Display for UpdateFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.technical)
    }
}

fn local_failure(technical: impl Into<String>) -> UpdateFailure {
    UpdateFailure::new(UpdateFailureKind::Local, technical)
}

fn verification_failure(technical: impl Into<String>) -> UpdateFailure {
    UpdateFailure::new(UpdateFailureKind::Verification, technical)
}

fn download_failure(technical: impl Into<String>) -> UpdateFailure {
    UpdateFailure::new(UpdateFailureKind::Download, technical)
}

/// An HTTP error: an error status from the server, or no answer at all.
fn http_failure(
    error: &ureq::Error,
    on_status: UpdateFailureKind,
    on_transport: UpdateFailureKind,
    context: &str,
) -> UpdateFailure {
    let kind = if matches!(error, ureq::Error::StatusCode(_)) {
        on_status
    } else {
        on_transport
    };
    UpdateFailure::new(kind, format!("{context}: {error}"))
}

#[derive(Debug)]
pub struct PreparedUpdate {
    helper_path: PathBuf,
    candidate_path: PathBuf,
    target_path: PathBuf,
    stage_directory: PathBuf,
    expected_sha256: String,
    version: String,
}

#[derive(Debug)]
struct HelperArguments {
    target_path: PathBuf,
    candidate_path: PathBuf,
    expected_sha256: String,
    version: String,
    stage_directory: PathBuf,
    /// The app that started the helper; nothing is replaced while it runs.
    parent_pid: u32,
}

#[derive(Debug)]
struct UpdatePreparation {
    executable_url: String,
    checksum_url: String,
    /// The key that must have signed the checksums.
    signing_key: [u8; 32],
    asset_name: String,
    /// `EXECUTABLE_CANDIDATE_NAME` or `INSTALLER_CANDIDATE_NAME`.
    candidate_name: &'static str,
    version: String,
    target_path: PathBuf,
    stage_directory: PathBuf,
}

pub fn current_version() -> Version {
    #[cfg(feature = "update-test-feed")]
    if let Some(version) = test_feed::version() {
        return Version::parse(version)
            .expect("LOCALCODEPILOT_TEST_VERSION must follow semantic versioning");
    }
    Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("the workspace package version must follow semantic versioning")
}

fn releases_url() -> String {
    #[cfg(feature = "update-test-feed")]
    if let Some(base) = test_feed::base() {
        return format!("{base}/releases");
    }
    RELEASES_URL.to_owned()
}

fn signing_public_key() -> [u8; 32] {
    #[cfg(feature = "update-test-feed")]
    if let Some(key) = test_feed::signing_key() {
        return key;
    }
    UPDATE_SIGNING_PUBLIC_KEY
}

fn release_asset_prefix() -> String {
    #[cfg(feature = "update-test-feed")]
    if let Some(base) = test_feed::base() {
        return format!("{base}/download/");
    }
    RELEASE_ASSET_PREFIX.to_owned()
}

/// Startup problems of a "new" test version (test builds only): applied before
/// the window opens, and only when the updater starts this executable.
#[cfg(feature = "update-test-feed")]
pub fn simulate_test_startup() {
    if env::args_os().nth(1).as_deref() == Some(UPDATED_STARTUP_FLAG.as_ref()) {
        test_feed::simulate_updated_startup(STARTUP_TIMEOUT);
    }
}

/// Extra time the app takes to close after starting the updater: only test
/// builds ever wait, so the E2E can see the updater window waiting for it.
pub fn test_close_delay() -> Duration {
    #[cfg(feature = "update-test-feed")]
    return test_feed::close_delay();
    #[cfg(not(feature = "update-test-feed"))]
    Duration::ZERO
}

pub fn cached_update_is_newer(update: &UpdateInfo) -> bool {
    cached_update_matches_os(update, env::consts::OS)
        && parse_version(&update.version).is_some_and(|version| version > current_version())
}

fn cached_update_matches_os(update: &UpdateInfo, operating_system: &str) -> bool {
    update
        .download_url
        .rsplit('/')
        .next()
        .is_some_and(|name| compatible_update_asset(name, operating_system))
}

pub fn automatic_update_available(update: &UpdateInfo) -> bool {
    cfg!(target_os = "windows")
        && packaged_executable_path().is_ok()
        && (update
            .executable_url
            .as_deref()
            .is_some_and(trusted_release_asset_url)
            || installer_update_url(update).is_some())
        && update
            .checksum_url
            .as_deref()
            .is_some_and(trusted_release_asset_url)
}

/// What "Baixar atualização" opens when this copy cannot update itself. On Windows it is
/// the official installer, so the person ends with an installed copy that updates itself
/// from then on; the portable package is only the fallback for releases without one.
pub fn manual_download_url(update: &UpdateInfo) -> &str {
    manual_download_url_for_os(update, env::consts::OS)
}

fn manual_download_url_for_os<'a>(update: &'a UpdateInfo, operating_system: &str) -> &'a str {
    match update.installer_url.as_deref() {
        Some(installer)
            if operating_system == "windows" && trusted_release_asset_url(installer) =>
        {
            installer
        }
        _ => &update.download_url,
    }
}

/// The line under the update notice when the update has to be downloaded by hand.
pub fn manual_download_note(update: &UpdateInfo) -> &'static str {
    manual_download_note_for_os(update, env::consts::OS)
}

fn manual_download_note_for_os(update: &UpdateInfo, operating_system: &str) -> &'static str {
    if manual_download_url_for_os(update, operating_system) == update.download_url {
        "Use o pacote oficial para concluir esta atualização."
    } else {
        MANUAL_INSTALLER_NOTE
    }
}

pub const MANUAL_INSTALLER_NOTE: &str = "Esta cópia não foi instalada. Baixe o instalador para atualizar e receber as próximas atualizações automaticamente.";

/// The installer to run when this copy was installed by it and the release offers it.
fn installer_update_url(update: &UpdateInfo) -> Option<String> {
    if !installed_by_installer() {
        return None;
    }
    let asset_name = windows_installer_asset_name(&update.version);
    update
        .installer_url
        .clone()
        .filter(|url| trusted_release_asset_url(url) && url.ends_with(&format!("/{asset_name}")))
}

/// Whether the running executable was installed by the official installer: its
/// uninstaller sits next to it and the installer's uninstall entry points to this
/// folder. Without both, the copy is treated as portable. Checked once per run.
fn installed_by_installer() -> bool {
    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| {
        let Ok(executable) = packaged_executable_path() else {
            return false;
        };
        let Some(directory) = executable.parent() else {
            return false;
        };
        directory.join("unins000.exe").is_file()
            && installer_install_location()
                .is_some_and(|location| same_directory(Path::new(&location), directory))
    })
}

fn installer_app_id() -> String {
    #[cfg(feature = "update-test-feed")]
    if let Some(app_id) = test_feed::installer_app_id() {
        return app_id.to_owned();
    }
    INSTALLER_APP_ID.to_owned()
}

/// `InstallLocation` of the per-user uninstall entry written by the installer.
#[cfg(windows)]
fn installer_install_location() -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let wide = |value: &str| value.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(&format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}_is1",
        installer_app_id()
    ));
    let value = wide("InstallLocation");
    let mut buffer = vec![0_u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: both names are NUL-terminated, and `size` is the byte length of `buffer`.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let length = buffer.iter().position(|unit| *unit == 0)?;
    Some(String::from_utf16_lossy(&buffer[..length]))
}

#[cfg(not(windows))]
fn installer_install_location() -> Option<String> {
    let _ = installer_app_id();
    None
}

/// Compares two folders the way Windows does: ignoring a trailing separator and case.
fn same_directory(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        path.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    normalize(left) == normalize(right)
}

pub fn spawn_update_check(
    repaint: egui::Context,
) -> Receiver<Result<Option<UpdateInfo>, UpdateFailure>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = fetch_update(&current_version());
        let _ = sender.send(result);
        repaint.request_repaint();
    });
    receiver
}

pub fn spawn_update_download(
    update: &UpdateInfo,
    repaint: egui::Context,
) -> Result<Receiver<UpdateInstallEvent>, UpdateFailure> {
    if !cached_update_is_newer(update) {
        return Err(local_failure(
            "a versão selecionada não é mais recente que a instalada",
        ));
    }
    let target_path = packaged_executable_path().map_err(local_failure)?;
    verify_target_directory_writable(&target_path).map_err(local_failure)?;
    let checksum_url = update
        .checksum_url
        .clone()
        .filter(|url| trusted_release_asset_url(url))
        .ok_or_else(|| {
            verification_failure("a release não oferece o arquivo oficial de checksums")
        })?;
    // An installed copy runs the official installer, so the Windows program list
    // shows the new version; a portable copy swaps the executable.
    let (executable_url, asset_name, candidate_name) =
        if let Some(installer_url) = installer_update_url(update) {
            (
                installer_url,
                windows_installer_asset_name(&update.version),
                INSTALLER_CANDIDATE_NAME,
            )
        } else {
            let executable_url = update
                .executable_url
                .clone()
                .filter(|url| trusted_release_asset_url(url))
                .ok_or_else(|| {
                    verification_failure(
                        "a release não oferece um executável confiável para atualização",
                    )
                })?;
            let asset_name = windows_update_asset_name(&update.version);
            if !executable_url.ends_with(&format!("/{asset_name}")) {
                return Err(verification_failure(
                    "o nome do executável da release não corresponde à versão selecionada",
                ));
            }
            (executable_url, asset_name, EXECUTABLE_CANDIDATE_NAME)
        };
    let preparation = UpdatePreparation {
        executable_url,
        checksum_url,
        signing_key: signing_public_key(),
        asset_name,
        candidate_name,
        version: update.version.clone(),
        target_path,
        stage_directory: new_stage_directory(&update.version).map_err(local_failure)?,
    };
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = prepare_update(&preparation, &sender, &repaint);
        match result {
            Ok(prepared) => {
                let _ = sender.send(UpdateInstallEvent::Prepared(prepared));
            }
            Err(error) => {
                let _ = fs::remove_dir_all(&preparation.stage_directory);
                let _ = sender.send(UpdateInstallEvent::Failed(error));
            }
        }
        repaint.request_repaint();
    });
    Ok(receiver)
}

/// The person chose "Agora não" after the file was ready: remove the prepared copy so
/// nothing is left behind. The current version is untouched.
pub fn discard_prepared_update(update: PreparedUpdate) {
    let _ = fs::remove_dir_all(&update.stage_directory);
}

pub fn launch_prepared_update(update: &PreparedUpdate) -> Result<(), String> {
    let mut command = Command::new(&update.helper_path);
    command
        .arg(UPDATE_HELPER_FLAG)
        .arg(&update.target_path)
        .arg(&update.candidate_path)
        .arg(&update.expected_sha256)
        .arg(&update.version)
        .arg(&update.stage_directory)
        .arg(std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("não foi possível iniciar o atualizador: {error}"))
}

/// Where the updater is, for its window. The window only shows it: the update
/// runs on its own thread and goes on when the window cannot open or is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperStage {
    /// The app that started the updater has not closed yet.
    WaitingForApp { since: Instant },
    /// Replacing the executable or running the official installer.
    Installing,
    /// Opening the new version (or the previous one, after a failure).
    Restarting,
    /// Nothing left to show: the version that opened shows the outcome.
    Finished,
}

/// The wait for the app to close usually takes a second or two; past this the
/// window says it is waiting instead of installing.
pub const HELPER_SLOW_CLOSE: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub struct HelperProgress(std::sync::Mutex<HelperStage>);

impl HelperProgress {
    fn new() -> Self {
        Self(std::sync::Mutex::new(HelperStage::WaitingForApp {
            since: Instant::now(),
        }))
    }

    pub fn stage(&self) -> HelperStage {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set(&self, stage: HelperStage) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = stage;
    }
}

/// Runs the updater when this process was started as one. `show_window` runs
/// on the main thread while the update goes on in the background and may
/// return at any time (window closed, or it could not open): the update is
/// never interrupted because of the window.
pub fn try_run_update_helper(
    show_window: impl FnOnce(std::sync::Arc<HelperProgress>),
) -> Option<i32> {
    let arguments = match parse_helper_arguments() {
        Ok(Some(arguments)) => arguments,
        Ok(None) => return None,
        Err(error) => {
            write_persistent_update_error(&error);
            return Some(2);
        }
    };
    let progress = std::sync::Arc::new(HelperProgress::new());
    let worker = {
        let progress = progress.clone();
        thread::spawn(move || {
            let exit_code = run_update_helper(&arguments, &progress);
            progress.set(HelperStage::Finished);
            exit_code
        })
    };
    // A panic in the window must not end the process while the update runs.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        show_window(progress.clone())
    }));
    Some(worker.join().unwrap_or(1))
}

fn run_update_helper(arguments: &HelperArguments, progress: &HelperProgress) -> i32 {
    match apply_prepared_update(arguments, progress) {
        Ok(()) => 0,
        Err(failure) => {
            // The message is saved before the previous version reopens, so
            // that version is the one that shows it.
            match &failure.report {
                HelperReport::Error => write_persistent_update_error(&failure.message),
                HelperReport::Plain => write_persistent_update_text(&failure.message),
                HelperReport::Installer(installer) => write_installer_failure(installer),
            }
            if failure.relaunch_previous {
                progress.set(HelperStage::Restarting);
                relaunch_after_failure(&arguments.target_path);
            }
            1
        }
    }
}

#[derive(Debug)]
struct HelperFailure {
    message: String,
    /// The previous executable is in place and should be reopened.
    relaunch_previous: bool,
    report: HelperReport,
}

/// How the reopened version presents the failure.
#[derive(Debug)]
enum HelperReport {
    /// A notice prefixed with "A atualização automática falhou:".
    Error,
    /// A notice with the message as is.
    Plain,
    /// The installer did not finish: the update error with its actions and details.
    Installer(InstallerFailure),
}

/// Saved by the helper when the installer did not finish and the previous
/// version was checked; read once by that version when it reopens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InstallerFailure {
    from_version: String,
    to_version: String,
    technical: String,
}

impl HelperFailure {
    fn keep_closed(message: String) -> Self {
        Self {
            message,
            relaunch_previous: false,
            report: HelperReport::Error,
        }
    }

    fn reopen_previous(message: String) -> Self {
        Self {
            message,
            relaunch_previous: true,
            report: HelperReport::Error,
        }
    }

    fn installer_not_completed(failure: InstallerFailure) -> Self {
        Self {
            message: failure.technical.clone(),
            relaunch_previous: true,
            report: HelperReport::Installer(failure),
        }
    }
}

/// What the app shows about an update when it opens.
#[derive(Debug, Default)]
pub struct StartupUpdate {
    /// A notice about the outcome (updated, restored, not completed).
    pub notice: Option<String>,
    /// The installer did not finish: shown as an update error, with "Tentar de novo".
    pub failure: Option<UpdateFailure>,
    /// This start was confirmed by the updater's marker: an update by the app succeeded and
    /// this is the new version (T035).
    pub updated_by_app: bool,
}

pub fn complete_updated_startup() -> StartupUpdate {
    let (notice, updated_by_app) = updated_startup_notice();
    let failure = read_installer_failure().map(|failure| {
        UpdateFailure::installer(failure.from_version, failure.to_version, failure.technical)
    });
    StartupUpdate {
        notice,
        failure,
        updated_by_app,
    }
}

/// The notice of this start and whether the updater's marker confirmed it.
fn updated_startup_notice() -> (Option<String>, bool) {
    let mut arguments = env::args_os().skip(1);
    let first = arguments.next();
    if first.as_deref() == Some(UPDATED_STARTUP_FLAG.as_ref()) {
        let marker_path = arguments.next().map(PathBuf::from);
        let helper_path = arguments.next().map(PathBuf::from);
        let stage_directory = arguments.next().map(PathBuf::from);
        let version = arguments.next().and_then(|value| value.into_string().ok());
        if arguments.next().is_none()
            && let (Some(marker_path), Some(helper_path), Some(stage_directory), Some(version)) =
                (marker_path, helper_path, stage_directory, version)
            && validate_cleanup_paths(&marker_path, &helper_path, &stage_directory).is_ok()
            && fs::write(&marker_path, b"ok").is_ok()
        {
            let _ = fs::remove_file(persistent_update_error_path());
            let _ = fs::remove_file(installer_failure_path());
            spawn_update_cleanup(helper_path, stage_directory);
            spawn_confirmed_update_cleanup();
            return (Some(updated_message(&version)), true);
        }
    }
    let error = read_persistent_update_error();
    let recovered = recover_interrupted_update();
    (error.or(recovered), false)
}

fn updated_message(version: &str) -> String {
    format!("LocalCodePilot atualizado para a versão {version}.")
}

/// The new version confirmed its startup: the previous executable and the
/// journal are no longer needed. Runs in the background because the helper
/// may still hold the backup for a moment.
fn spawn_confirmed_update_cleanup() {
    let Ok(executable) = env::current_exe() else {
        return;
    };
    let journal = configuration_directory().map(|directory| recovery::journal_path(&directory));
    thread::spawn(move || {
        recovery::remove_swap_leftovers(&executable);
        if let Some(journal) = journal {
            let _ = fs::remove_file(journal);
        }
    });
}

/// Finishes or undoes an update the helper could not see to the end (slow
/// confirmation, crash, power loss), based on the journal and on the version
/// that is actually running.
fn recover_interrupted_update() -> Option<String> {
    let journal_path = recovery::journal_path(&configuration_directory()?);
    let journal = recovery::read_journal(&journal_path)?;
    let executable = env::current_exe().ok()?;
    let message = match recovery::decide_recovery(&journal, &executable, &current_version()) {
        recovery::Recovery::NotThisInstallation => return None,
        recovery::Recovery::Completed { version } => Some(updated_message(&version)),
        recovery::Recovery::NotCompleted { from, to } => Some(not_completed_message(&from, &to)),
        recovery::Recovery::Unknown => None,
    };
    recovery::remove_swap_leftovers(&executable);
    let _ = fs::remove_file(&journal_path);
    message
}

fn configuration_directory() -> Option<PathBuf> {
    crate::config::config_directory()
}

fn fetch_update(current: &Version) -> Result<Option<UpdateInfo>, UpdateFailure> {
    let agent = update_agent(Duration::from_secs(8));
    let mut response = agent
        .get(releases_url())
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "LocalCodePilot-Update-Checker")
        .call()
        .map_err(|error| {
            http_failure(
                &error,
                UpdateFailureKind::InvalidResponse,
                UpdateFailureKind::NoConnection,
                "não foi possível consultar as releases",
            )
        })?;
    let releases: Vec<GithubRelease> = response.body_mut().read_json().map_err(|error| {
        UpdateFailure::new(
            UpdateFailureKind::InvalidResponse,
            format!("resposta de atualização inválida: {error}"),
        )
    })?;
    Ok(select_update(releases, current))
}

fn select_update(releases: Vec<GithubRelease>, current: &Version) -> Option<UpdateInfo> {
    select_update_for_os(releases, current, env::consts::OS)
}

fn select_update_for_os(
    releases: Vec<GithubRelease>,
    current: &Version,
    operating_system: &str,
) -> Option<UpdateInfo> {
    releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = parse_version(&release.tag_name)?;
            if version <= *current
                || (current.pre.is_empty() && (release.prerelease || !version.pre.is_empty()))
            {
                return None;
            }
            let download_url = release
                .assets
                .iter()
                .find(|asset| portable_asset(&asset.name, operating_system))
                .or_else(|| {
                    release
                        .assets
                        .iter()
                        .find(|asset| compatible_update_asset(&asset.name, operating_system))
                })
                .map(|asset| asset.browser_download_url.clone())?;
            // Only the exact update executable qualifies: the installer shares the
            // `LocalCodePilot-<version>-windows-x64` prefix in older releases and
            // is listed before it by the GitHub API (#97).
            let update_asset_name = windows_update_asset_name(&version);
            let executable_url = release
                .assets
                .iter()
                .find(|asset| {
                    operating_system == "windows"
                        && asset.name.eq_ignore_ascii_case(&update_asset_name)
                })
                .map(|asset| asset.browser_download_url.clone());
            let installer_asset_name = windows_installer_asset_name(&version);
            let installer_url = release
                .assets
                .iter()
                .find(|asset| {
                    operating_system == "windows"
                        && asset.name.eq_ignore_ascii_case(&installer_asset_name)
                })
                .map(|asset| asset.browser_download_url.clone());
            let checksum_url = release
                .assets
                .iter()
                .find(|asset| asset.name.eq_ignore_ascii_case("SHA256SUMS.txt"))
                .map(|asset| asset.browser_download_url.clone());
            let title = release
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("LocalCodePilot {version}"));
            Some((
                version.clone(),
                UpdateInfo {
                    version: version.to_string(),
                    title,
                    release_url: release.html_url,
                    download_url,
                    executable_url,
                    checksum_url,
                    summary: release.body.as_deref().and_then(release_summary),
                    installer_url,
                },
            ))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, update)| update)
}

fn prepare_update(
    preparation: &UpdatePreparation,
    sender: &Sender<UpdateInstallEvent>,
    repaint: &egui::Context,
) -> Result<PreparedUpdate, UpdateFailure> {
    fs::create_dir_all(&preparation.stage_directory).map_err(|error| {
        local_failure(format!(
            "não foi possível criar a pasta temporária: {error}"
        ))
    })?;
    let agent = download_agent();
    send_phase(sender, repaint, UpdatePhase::Checksums);
    let checksums = download_checksums(&agent, &preparation.checksum_url)?;
    // The checksums come from the same release as the executable; the signature
    // proves they were published with the project's key, so replacing both files
    // in a release is not enough to ship another executable.
    let signature = download_signature(
        &agent,
        &format!("{}{SIGNATURE_SUFFIX}", preparation.checksum_url),
    )?;
    verify_checksums_signature(checksums.as_bytes(), &signature, &preparation.signing_key)?;
    let expected_sha256 =
        checksum_for_asset(&checksums, &preparation.asset_name).ok_or_else(|| {
            verification_failure(format!(
                "o checksum de {} não foi publicado",
                preparation.asset_name
            ))
        })?;
    let candidate_path = preparation.stage_directory.join(preparation.candidate_name);
    send_phase(sender, repaint, UpdatePhase::Downloading);
    download_executable(
        &agent,
        &preparation.executable_url,
        &candidate_path,
        &expected_sha256,
        sender,
        repaint,
    )?;
    let helper_path = preparation
        .stage_directory
        .join("LocalCodePilot-updater.exe");
    fs::copy(&preparation.target_path, &helper_path).map_err(|error| {
        local_failure(format!(
            "não foi possível preparar o atualizador temporário: {error}"
        ))
    })?;
    Ok(PreparedUpdate {
        helper_path,
        candidate_path,
        target_path: preparation.target_path.clone(),
        stage_directory: preparation.stage_directory.clone(),
        expected_sha256,
        version: preparation.version.clone(),
    })
}

/// A missing file in a release (404) means it was not published, which the
/// safety checks refuse; any other HTTP failure is a download problem.
fn release_file_failure(error: &ureq::Error, context: &str) -> UpdateFailure {
    let kind = if matches!(error, ureq::Error::StatusCode(404)) {
        UpdateFailureKind::Verification
    } else {
        UpdateFailureKind::Download
    };
    UpdateFailure::new(kind, format!("{context}: {error}"))
}

fn download_checksums(agent: &ureq::Agent, url: &str) -> Result<String, UpdateFailure> {
    let mut response = agent
        .get(url)
        .header("User-Agent", "LocalCodePilot-Updater")
        .call()
        .map_err(|error| release_file_failure(&error, "não foi possível baixar os checksums"))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_CHECKSUM_BYTES)
        .read_to_string()
        .map_err(|error| download_failure(format!("o arquivo de checksums é inválido: {error}")))
}

fn download_signature(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, UpdateFailure> {
    let mut response = agent
        .get(url)
        .header("User-Agent", "LocalCodePilot-Updater")
        .call()
        .map_err(|error| {
            release_file_failure(
                &error,
                "a release não publicou a assinatura dos checksums (SHA256SUMS.txt.sig)",
            )
        })?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_SIGNATURE_BYTES)
        .read_to_vec()
        .map_err(|error| {
            download_failure(format!(
                "não foi possível baixar a assinatura dos checksums: {error}"
            ))
        })
}

fn verify_checksums_signature(
    checksums: &[u8],
    signature: &[u8],
    public_key: &[u8; 32],
) -> Result<(), UpdateFailure> {
    use ring::signature::{ED25519, UnparsedPublicKey};
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(checksums, signature)
        .map_err(|_| {
            verification_failure(
                "a assinatura dos checksums não confere com a chave do LocalCodePilot; a atualização foi recusada",
            )
        })
}

fn download_executable(
    agent: &ureq::Agent,
    url: &str,
    destination: &Path,
    expected_sha256: &str,
    sender: &Sender<UpdateInstallEvent>,
    repaint: &egui::Context,
) -> Result<(), UpdateFailure> {
    let mut response = agent
        .get(url)
        .header("User-Agent", "LocalCodePilot-Updater")
        .call()
        .map_err(|error| {
            http_failure(
                &error,
                UpdateFailureKind::Download,
                UpdateFailureKind::Download,
                "não foi possível baixar a atualização",
            )
        })?;
    let total_bytes = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|size| *size <= MAX_EXECUTABLE_BYTES);
    let partial_path = destination.with_extension("exe.part");
    let mut output = File::create(&partial_path).map_err(|error| {
        local_failure(format!(
            "não foi possível criar o download temporário: {error}"
        ))
    })?;
    let mut reader = response.body_mut().as_reader();
    let mut hasher = Sha256::new();
    let mut downloaded_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    let mut last_progress: Option<Instant> = None;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                let _ = fs::remove_file(&partial_path);
                return Err(download_failure(format!(
                    "o download da atualização foi interrompido: {error}"
                )));
            }
        };
        if read == 0 {
            break;
        }
        downloaded_bytes = downloaded_bytes.saturating_add(read as u64);
        if downloaded_bytes > MAX_EXECUTABLE_BYTES {
            let _ = fs::remove_file(&partial_path);
            return Err(verification_failure(
                "o executável baixado excede o limite permitido",
            ));
        }
        if let Err(error) = output.write_all(&buffer[..read]) {
            let _ = fs::remove_file(&partial_path);
            return Err(local_failure(format!(
                "não foi possível salvar a atualização: {error}"
            )));
        }
        hasher.update(&buffer[..read]);
        if last_progress.is_none_or(|sent| sent.elapsed() >= PROGRESS_INTERVAL) {
            last_progress = Some(Instant::now());
            send_progress(sender, repaint, downloaded_bytes, total_bytes);
        }
    }
    // The last chunk is always reported, even when it arrived inside the interval.
    send_progress(sender, repaint, downloaded_bytes, total_bytes);
    output
        .sync_all()
        .map_err(|error| local_failure(format!("não foi possível concluir o download: {error}")))?;
    drop(output);
    if total_bytes.is_some_and(|total| total != downloaded_bytes) {
        let _ = fs::remove_file(&partial_path);
        return Err(download_failure(
            "o download da atualização foi interrompido antes do fim",
        ));
    }
    send_phase(sender, repaint, UpdatePhase::Verifying);
    let actual_sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        let _ = fs::remove_file(&partial_path);
        return Err(verification_failure(
            "o SHA-256 do executável baixado não corresponde ao publicado",
        ));
    }
    if downloaded_bytes < 1024 || !file_starts_with_mz(&partial_path).map_err(local_failure)? {
        let _ = fs::remove_file(&partial_path);
        return Err(verification_failure(
            "o arquivo baixado não é um executável Windows válido",
        ));
    }
    fs::rename(&partial_path, destination)
        .map_err(|error| local_failure(format!("não foi possível finalizar o download: {error}")))
}

/// Message after the new version failed to start. `configuration_restored`
/// is `None` when the executable could not be restored (so the
/// configuration was not touched either). Each outcome says what is installed
/// and what the person can safely do next.
fn rollback_failure(
    failure: &str,
    from_version: &str,
    to_version: &str,
    configuration_restored: Option<std::io::Result<()>>,
    config_backup: &Path,
    by_installer: bool,
) -> HelperFailure {
    match configuration_restored {
        // The installer already registered the new version: the Windows program
        // list may keep showing it (approved text, T002 CA-8).
        Some(Ok(())) if by_installer => HelperFailure {
            report: HelperReport::Plain,
            ..HelperFailure::reopen_previous(format!(
                "A versão {to_version} não abriu. Restauramos o aplicativo e as configurações da versão {from_version}; você pode continuar trabalhando. A lista de programas do Windows ainda pode mostrar a versão {to_version}. Tente novamente mais tarde ou baixe o instalador na página oficial."
            ))
        },
        Some(Ok(())) => HelperFailure::reopen_previous(format!(
            "{failure}. A versão {from_version} foi restaurada e continua em uso: você pode continuar trabalhando normalmente. Para instalar a versão {to_version}, tente de novo mais tarde ou baixe-a na página oficial"
        )),
        Some(Err(error)) => HelperFailure::reopen_previous(format!(
            "{failure}. A versão {from_version} foi restaurada, mas as configurações não puderam ser recuperadas ({error}). Confira as pastas de busca e os favoritos; a cópia guardada antes da atualização está em {}",
            config_backup.display()
        )),
        None => HelperFailure::keep_closed(format!(
            "{failure}, e não foi possível restaurar a versão {from_version}. Baixe o LocalCodePilot na página oficial e instale de novo; suas configurações foram guardadas em {}",
            config_backup.display()
        )),
    }
}

fn not_completed_message(from: &str, to: &str) -> String {
    format!(
        "A atualização para a versão {to} não foi concluída. A versão {from} continua instalada e pode ser usada normalmente. Para tentar de novo, use Atualizar agora em Configurações"
    )
}

fn apply_prepared_update(
    arguments: &HelperArguments,
    progress: &HelperProgress,
) -> Result<(), HelperFailure> {
    let to_version = &arguments.version;
    // Nothing is touched while the app that started the helper still runs;
    // relaunching it here would open a second instance.
    if !recovery::wait_for_process_exit(arguments.parent_pid, REPLACEMENT_TIMEOUT) {
        return Err(HelperFailure::keep_closed(format!(
            "o LocalCodePilot não fechou a tempo, e a atualização para a versão {to_version} não foi aplicada. Nada foi alterado: feche o LocalCodePilot e tente atualizar de novo"
        )));
    }
    progress.set(HelperStage::Installing);
    let from_version = current_version().to_string();
    let not_applied = |reason: String| {
        Err(HelperFailure::reopen_previous(format!(
            "{reason}. A versão {from_version} continua instalada e pode ser usada normalmente. Tente de novo mais tarde ou baixe a atualização na página oficial"
        )))
    };
    if let Err(error) = validate_helper_arguments(arguments) {
        if !is_packaged_executable_name(&arguments.target_path) {
            return Err(HelperFailure::keep_closed(error));
        }
        return not_applied(error);
    }
    match sha256_file(&arguments.candidate_path) {
        Ok(hash) if hash.eq_ignore_ascii_case(&arguments.expected_sha256) => {}
        Ok(_) => {
            return not_applied(
                "a atualização preparada falhou na segunda verificação SHA-256".into(),
            );
        }
        Err(error) => return not_applied(error),
    }
    let helper_path = match env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            return not_applied(format!(
                "não foi possível identificar o atualizador: {error}"
            ));
        }
    };
    let Some(install_directory) = arguments.target_path.parent() else {
        return not_applied("pasta de instalação inválida".into());
    };
    let Some(config_directory) = configuration_directory() else {
        return not_applied("a pasta de configurações não foi encontrada".into());
    };
    let config_backup = match recovery::backup_configuration(&config_directory, to_version) {
        Ok(path) => path,
        Err(error) => {
            return not_applied(format!(
                "não foi possível guardar uma cópia das configurações: {error}"
            ));
        }
    };
    let journal_path = recovery::journal_path(&config_directory);
    let mut journal = recovery::UpdateJournal {
        from_version: from_version.clone(),
        to_version: to_version.clone(),
        target: arguments.target_path.clone(),
        phase: recovery::JournalPhase::Swapping,
        config_backup: Some(config_backup.clone()),
    };
    if let Err(error) = recovery::write_journal(&journal_path, &journal) {
        return not_applied(format!(
            "não foi possível registrar o início da atualização: {error}"
        ));
    }
    let by_installer = is_installer_candidate(&arguments.candidate_path);
    if by_installer {
        let installed = install_with_installer(
            arguments,
            install_directory,
            &config_directory,
            &from_version,
        );
        if let Err(failure) = installed {
            if failure.relaunch_previous {
                let _ = fs::remove_file(&journal_path);
            }
            return Err(failure);
        }
    } else if let Err(error) = recovery::swap_in_candidate(
        &arguments.target_path,
        &arguments.candidate_path,
        &arguments.expected_sha256,
        sha256_file,
        |from, to| recovery::rename_with_retry(from, to, REPLACEMENT_TIMEOUT),
    ) {
        return match error {
            recovery::SwapError::NotApplied(reason) | recovery::SwapError::RolledBack(reason) => {
                let _ = fs::remove_file(&journal_path);
                not_applied(reason)
            }
            // The journal stays: the next startup of whichever copy runs
            // will clean up what it can.
            recovery::SwapError::Broken(reason) => Err(HelperFailure::keep_closed(format!(
                "{reason}. Baixe o LocalCodePilot {from_version} ou {to_version} na página oficial"
            ))),
        };
    }
    journal.phase = recovery::JournalPhase::Starting;
    let _ = recovery::write_journal(&journal_path, &journal);
    progress.set(HelperStage::Restarting);

    let marker_path = arguments.stage_directory.join("startup-ok");
    let mut command = Command::new(&arguments.target_path);
    command
        .arg(UPDATED_STARTUP_FLAG)
        .arg(&marker_path)
        .arg(&helper_path)
        .arg(&arguments.stage_directory)
        .arg(to_version)
        .current_dir(install_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    let failure = match command.spawn() {
        Err(error) => format!("não foi possível abrir a versão {to_version}: {error}"),
        Ok(mut child) => {
            match recovery::wait_for_startup(&mut child, &marker_path, STARTUP_TIMEOUT) {
                recovery::StartupOutcome::Confirmed => {
                    recovery::remove_swap_leftovers(&arguments.target_path);
                    let _ = fs::remove_file(&journal_path);
                    return Ok(());
                }
                // Slow is not failed: the new version confirms and cleans up
                // by itself when its window opens.
                recovery::StartupOutcome::StillRunning => {
                    journal.phase = recovery::JournalPhase::PendingConfirmation;
                    if journal_path.is_file() {
                        let _ = recovery::write_journal(&journal_path, &journal);
                    }
                    return Ok(());
                }
                recovery::StartupOutcome::Exited(status) => {
                    format!("a versão {to_version} fechou ao iniciar ({status})")
                }
            }
        }
    };
    let executable_restored =
        recovery::restore_previous_executable(&arguments.target_path, REPLACEMENT_TIMEOUT);
    let configuration_restored = executable_restored
        .then(|| recovery::restore_configuration(&config_directory, &config_backup));
    if executable_restored {
        let _ = fs::remove_file(&journal_path);
    }
    Err(rollback_failure(
        &failure,
        &from_version,
        to_version,
        configuration_restored,
        &config_backup,
        by_installer,
    ))
}

fn is_installer_candidate(candidate: &Path) -> bool {
    candidate
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(INSTALLER_CANDIDATE_NAME))
}

/// Runs the official installer over the current installation. The previous
/// executable is copied to the backup name first: if the installer does not
/// finish, the executable in the folder is checked against the previous one and
/// put back when it differs; if it finishes, the backup is what the usual
/// startup check restores when the new version does not open.
fn install_with_installer(
    arguments: &HelperArguments,
    install_directory: &Path,
    config_directory: &Path,
    from_version: &str,
) -> Result<(), HelperFailure> {
    let to_version = &arguments.version;
    let target = &arguments.target_path;
    let not_applied = |reason: String| {
        HelperFailure::reopen_previous(format!(
            "{reason}. A versão {from_version} continua instalada e pode ser usada normalmente. Tente de novo mais tarde ou baixe a atualização na página oficial"
        ))
    };
    let previous_sha256 = sha256_file(target).map_err(not_applied)?;
    let backup = install_directory.join(recovery::BACKUP_FILE_NAME);
    let _ = fs::remove_file(&backup);
    fs::copy(target, &backup).map_err(|error| {
        let _ = fs::remove_file(&backup);
        not_applied(format!(
            "não foi possível guardar uma cópia da versão atual: {error}"
        ))
    })?;
    let log = arguments.stage_directory.join("setup.log");
    let outcome = run_installer(&arguments.candidate_path, install_directory, &log);
    let saved_log = config_directory.join(INSTALLER_LOG_FILE);
    let log_note = match fs::copy(&log, &saved_log) {
        Ok(_) => format!("registro do instalador: {}", saved_log.display()),
        Err(_) => "o instalador não gravou registro".to_owned(),
    };
    let Err(reason) = outcome else {
        return Ok(());
    };
    let technical = format!("{reason}; {log_note}");
    let is_previous = |path: &Path| {
        sha256_file(path).is_ok_and(|hash| hash.eq_ignore_ascii_case(&previous_sha256))
    };
    let previous_in_place = is_previous(target)
        || (recovery::restore_previous_executable(target, REPLACEMENT_TIMEOUT)
            && is_previous(target));
    if !previous_in_place {
        return Err(HelperFailure::keep_closed(format!(
            "{technical}. Baixe o LocalCodePilot {from_version} ou {to_version} na página oficial"
        )));
    }
    recovery::remove_swap_leftovers(target);
    Err(HelperFailure::installer_not_completed(InstallerFailure {
        from_version: from_version.to_owned(),
        to_version: to_version.clone(),
        technical,
    }))
}

/// Runs the installer silently in `directory` and waits for it. The installer
/// does not close other programs: another running copy of the app makes it fail
/// instead of being closed with its projects. `scripts/test-windows-installer-upgrade.ps1`
/// runs the installer with the same switches in CI; keep both in sync.
fn run_installer(installer: &Path, directory: &Path, log: &Path) -> Result<(), String> {
    let mut command = Command::new(installer);
    command
        .args([
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/SP-",
            "/NOCANCEL",
            "/NOCLOSEAPPLICATIONS",
            "/NORESTARTAPPLICATIONS",
        ])
        .arg(format!("/DIR={}", directory.display()))
        .arg(format!("/LOG={}", log.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(parent) = installer.parent() {
        command.current_dir(parent);
    }
    configure_background_command(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("não foi possível iniciar o instalador: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(match status.code() {
                    Some(code) => format!("o instalador terminou com o código {code}"),
                    None => format!("o instalador terminou sem código de saída ({status})"),
                });
            }
            Ok(None) if started.elapsed() >= INSTALLER_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "o instalador não terminou em {} minutos e foi interrompido",
                    INSTALLER_TIMEOUT.as_secs() / 60
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(error) => return Err(format!("não foi possível acompanhar o instalador: {error}")),
        }
    }
}

fn parse_helper_arguments() -> Result<Option<HelperArguments>, String> {
    let mut arguments = env::args_os().skip(1);
    if arguments.next().as_deref() != Some(UPDATE_HELPER_FLAG.as_ref()) {
        return Ok(None);
    }
    let values: Vec<OsString> = arguments.collect();
    if values.len() != 6 {
        return Err("os argumentos internos do atualizador são inválidos".into());
    }
    let parent_pid = values[5]
        .to_str()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| "o processo informado ao atualizador é inválido".to_owned())?;
    let expected_sha256 = values[2]
        .clone()
        .into_string()
        .map_err(|_| "o SHA-256 informado ao atualizador é inválido".to_owned())?;
    let version = values[3]
        .clone()
        .into_string()
        .map_err(|_| "a versão informada ao atualizador é inválida".to_owned())?;
    Ok(Some(HelperArguments {
        target_path: PathBuf::from(&values[0]),
        candidate_path: PathBuf::from(&values[1]),
        expected_sha256,
        version,
        stage_directory: PathBuf::from(&values[4]),
        parent_pid,
    }))
}

fn validate_helper_arguments(arguments: &HelperArguments) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err("a atualização automática está disponível somente no Windows".into());
    }
    if !is_packaged_executable_name(&arguments.target_path) {
        return Err("o destino da atualização não é o executável distribuído".into());
    }
    if parse_version(&arguments.version).is_none() {
        return Err("a versão preparada é inválida".into());
    }
    if !valid_sha256(&arguments.expected_sha256) {
        return Err("o SHA-256 preparado é inválido".into());
    }
    let candidate_name = arguments
        .candidate_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if candidate_name != EXECUTABLE_CANDIDATE_NAME && candidate_name != INSTALLER_CANDIDATE_NAME {
        return Err("o arquivo preparado não é uma atualização reconhecida".into());
    }
    let stage = arguments
        .stage_directory
        .canonicalize()
        .map_err(|error| format!("pasta temporária inválida: {error}"))?;
    let root = updates_root_directory()
        .canonicalize()
        .map_err(|error| format!("raiz temporária inválida: {error}"))?;
    if !stage.starts_with(&root) {
        return Err("a atualização não está na pasta temporária esperada".into());
    }
    let candidate_parent = arguments
        .candidate_path
        .parent()
        .and_then(|path| path.canonicalize().ok());
    let helper_parent = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .and_then(|path| path.canonicalize().ok());
    if candidate_parent.as_deref() != Some(stage.as_path())
        || helper_parent.as_deref() != Some(stage.as_path())
    {
        return Err("os arquivos do atualizador não pertencem à mesma preparação".into());
    }
    Ok(())
}

fn relaunch_after_failure(target: &Path) {
    if !target.is_file() {
        return;
    }
    let mut command = Command::new(target);
    if let Some(parent) = target.parent() {
        command.current_dir(parent);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    let _ = command.spawn();
}

fn spawn_update_cleanup(helper_path: PathBuf, stage_directory: PathBuf) {
    thread::spawn(move || {
        for _ in 0..600 {
            match fs::remove_file(&helper_path) {
                Ok(()) => {
                    let _ = fs::remove_dir_all(&stage_directory);
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let _ = fs::remove_dir_all(&stage_directory);
                    return;
                }
                Err(_) => thread::sleep(Duration::from_millis(100)),
            }
        }
    });
}

fn validate_cleanup_paths(marker: &Path, helper: &Path, stage: &Path) -> Result<(), String> {
    let canonical_stage = stage
        .canonicalize()
        .map_err(|error| format!("pasta temporária de limpeza inválida: {error}"))?;
    let canonical_root = updates_root_directory()
        .canonicalize()
        .map_err(|error| format!("raiz temporária de limpeza inválida: {error}"))?;
    let canonical_helper = helper
        .canonicalize()
        .map_err(|error| format!("atualizador temporário inválido: {error}"))?;
    let marker_parent = marker.parent().and_then(|path| path.canonicalize().ok());
    if !canonical_stage.starts_with(&canonical_root)
        || marker_parent.as_deref() != Some(canonical_stage.as_path())
        || canonical_helper.parent() != Some(canonical_stage.as_path())
        || marker.file_name().and_then(|name| name.to_str()) != Some("startup-ok")
        || helper.file_name().and_then(|name| name.to_str()) != Some("LocalCodePilot-updater.exe")
    {
        return Err("os caminhos de limpeza da atualização são inválidos".into());
    }
    Ok(())
}

fn verify_target_directory_writable(target: &Path) -> Result<(), String> {
    let directory = target
        .parent()
        .ok_or_else(|| "a pasta do executável atual é inválida".to_owned())?;
    let probe = directory.join(format!(
        ".localcodepilot-update-write-test-{}",
        std::process::id()
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|error| {
            format!(
                "o LocalCodePilot não pode atualizar arquivos nesta pasta ({error}); use o download manual"
            )
        })?;
    fs::remove_file(&probe)
        .map_err(|error| format!("não foi possível concluir o teste da pasta: {error}"))
}

fn packaged_executable_path() -> Result<PathBuf, String> {
    let executable = env::current_exe()
        .map_err(|error| format!("não foi possível identificar o executável atual: {error}"))?;
    if !is_packaged_executable_name(&executable) {
        return Err(
            "a atualização automática só funciona no LocalCodePilot.exe distribuído oficialmente"
                .into(),
        );
    }
    Ok(executable)
}

fn is_packaged_executable_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("LocalCodePilot.exe"))
}

fn new_stage_directory(version: &str) -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| format!("relógio do sistema inválido: {error}"))?
        .as_nanos();
    let safe_version: String = version
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
        .collect();
    Ok(updates_root_directory().join(format!("{safe_version}-{}-{nonce}", std::process::id())))
}

fn updates_root_directory() -> PathBuf {
    env::temp_dir().join("LocalCodePilot").join("updates")
}

fn update_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

/// A slow but steady download must finish: there is no total limit, only short
/// limits to connect and to get the response, plus a limit per read that
/// restarts whenever data arrives. Neither the call nor the body has a cap.
fn download_agent() -> ureq::Agent {
    download_agent_with_stall_timeout(DOWNLOAD_STALL_TIMEOUT)
}

fn download_agent_with_stall_timeout(stall_timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(None)
        .timeout_resolve(Some(Duration::from_secs(15)))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_per_read(Some(stall_timeout))
        .timeout_per_call(None)
        .timeout_recv_body(None)
        .build()
        .into()
}

fn send_phase(sender: &Sender<UpdateInstallEvent>, repaint: &egui::Context, phase: UpdatePhase) {
    let _ = sender.send(UpdateInstallEvent::Phase(phase));
    repaint.request_repaint();
}

fn send_progress(
    sender: &Sender<UpdateInstallEvent>,
    repaint: &egui::Context,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    let _ = sender.send(UpdateInstallEvent::Progress {
        downloaded_bytes,
        total_bytes,
    });
    repaint.request_repaint();
}

/// Removes preparation folders left by interrupted or failed attempts. Only
/// real directories directly under the update root and older than
/// `STALE_STAGE_AGE` are removed: a recent one may belong to an update in
/// progress, and links are never followed.
pub fn spawn_stale_stage_cleanup() {
    thread::spawn(|| remove_stale_stages(&updates_root_directory(), STALE_STAGE_AGE));
}

fn remove_stale_stages(root: &Path, max_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= max_age);
        if stale && fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn checksum_for_asset(contents: &str, asset_name: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        let name = fields.next()?.trim_start_matches('*');
        (name == asset_name && fields.next().is_none() && valid_sha256(hash))
            .then(|| hash.to_ascii_lowercase())
    })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("não foi possível abrir {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("não foi possível verificar {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn file_starts_with_mz(path: &Path) -> Result<bool, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("não foi possível validar o executável: {error}"))?;
    let mut signature = [0_u8; 2];
    file.read_exact(&mut signature)
        .map_err(|error| format!("o executável baixado está incompleto: {error}"))?;
    Ok(signature == *b"MZ")
}

fn trusted_release_asset_url(url: &str) -> bool {
    url.starts_with(&release_asset_prefix()) && !url.contains(['?', '#'])
}

fn persistent_update_error_path() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("LocalCodePilot")
        .join("update-error.txt")
}

fn write_persistent_update_error(error: &str) {
    write_persistent_update_text(&format!("A atualização automática falhou: {error}"));
}

fn write_persistent_update_text(text: &str) {
    let path = persistent_update_error_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, text);
}

fn installer_failure_path() -> PathBuf {
    persistent_update_error_path().with_file_name(INSTALLER_FAILURE_FILE)
}

fn write_installer_failure(failure: &InstallerFailure) {
    let path = installer_failure_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(contents) = serde_json::to_vec_pretty(failure) {
        let _ = fs::write(path, contents);
    }
}

fn read_installer_failure() -> Option<InstallerFailure> {
    let path = installer_failure_path();
    let contents = fs::read(&path).ok()?;
    let _ = fs::remove_file(path);
    serde_json::from_slice(&contents).ok()
}

fn read_persistent_update_error() -> Option<String> {
    let path = persistent_update_error_path();
    let message = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(path);
    (!message.trim().is_empty()).then(|| message.trim().to_owned())
}

fn parse_version(value: &str) -> Option<Version> {
    Version::parse(value.trim().trim_start_matches(['v', 'V'])).ok()
}

fn windows_portable_asset(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".zip")
        && name.contains("windows")
        && (name.contains("x64") || name.contains("amd64"))
}

fn macos_portable_asset(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".dmg") && name.contains("macos") && name.contains("universal")
}

fn portable_asset(name: &str, operating_system: &str) -> bool {
    match operating_system {
        "windows" => windows_portable_asset(name),
        "macos" => macos_portable_asset(name),
        _ => false,
    }
}

fn compatible_update_asset(name: &str, operating_system: &str) -> bool {
    portable_asset(name, operating_system)
        || (operating_system == "windows" && windows_update_asset(name))
}

fn windows_update_asset(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".exe")
        && name.starts_with("localcodepilot-")
        && name.contains("windows")
        && (name.contains("x64") || name.contains("amd64"))
        && !name.contains("setup")
}

fn windows_update_asset_name(version: impl std::fmt::Display) -> String {
    format!("LocalCodePilot-{version}-windows-x64.exe")
}

/// The official installer. Its name must not start with `LocalCodePilot-`, or older
/// clients would take it for the update executable (#97).
fn windows_installer_asset_name(version: impl std::fmt::Display) -> String {
    format!("LocalCodePilotSetup-{version}-windows-x64.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn releases(json: &str) -> Vec<GithubRelease> {
        serde_json::from_str(json).unwrap()
    }

    fn temporary_test_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "localcodepilot-update-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn selects_the_newest_alpha_and_all_windows_assets() {
        let update = select_update_for_os(
            releases(
                r#"[
                    {
                        "tag_name":"v0.4.0-alpha.5",
                        "name":"LocalCodePilot 0.4.0-alpha.5",
                        "html_url":"https://example.com/release",
                        "prerelease":true,
                        "assets":[
                            {
                                "name":"LocalCodePilot-0.4.0-alpha.5-windows-x64.zip",
                                "browser_download_url":"https://example.com/windows.zip"
                            },
                            {
                                "name":"LocalCodePilot-0.4.0-alpha.5-windows-x64.exe",
                                "browser_download_url":"https://example.com/windows.exe"
                            },
                            {
                                "name":"SHA256SUMS.txt",
                                "browser_download_url":"https://example.com/SHA256SUMS.txt"
                            }
                        ]
                    },
                    {
                        "tag_name":"v0.3.0-alpha.2",
                        "html_url":"https://example.com/older",
                        "prerelease":true
                    }
                ]"#,
            ),
            &Version::parse("0.4.0-alpha.4").unwrap(),
            "windows",
        )
        .unwrap();

        assert_eq!(update.version, "0.4.0-alpha.5");
        assert_eq!(update.download_url, "https://example.com/windows.zip");
        assert_eq!(
            update.executable_url.as_deref(),
            Some("https://example.com/windows.exe")
        );
        assert_eq!(
            update.checksum_url.as_deref(),
            Some("https://example.com/SHA256SUMS.txt")
        );
    }

    fn release_with_assets(version: &str, assets: &[&str]) -> Vec<GithubRelease> {
        let assets = assets
            .iter()
            .map(|name| {
                format!(
                    r#"{{"name":"{name}","browser_download_url":"https://github.com/cjulio1993/LocalCodePilot/releases/download/v{version}/{name}"}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        releases(&format!(
            r#"[{{"tag_name":"v{version}","html_url":"https://example.com/release","prerelease":true,"assets":[{assets}]}}]"#
        ))
    }

    #[test]
    fn the_alpha_10_sees_the_first_1_0_alpha() {
        // The jump from 0.4 to 1.0 (versioning decided on 2026-11-25): a prerelease client
        // takes any newer version, prerelease or not, with the same asset names.
        let assets = [
            "LocalCodePilot-1.0.0-alpha.1-macos-universal.dmg",
            "LocalCodePilot-1.0.0-alpha.1-windows-x64.exe",
            "LocalCodePilot-1.0.0-alpha.1-windows-x64.zip",
            "LocalCodePilotSetup-1.0.0-alpha.1-windows-x64.exe",
            "SHA256SUMS.txt",
            "SHA256SUMS.txt.sig",
        ];
        let update = select_update_for_os(
            release_with_assets("1.0.0-alpha.1", &assets),
            &Version::parse("0.4.0-alpha.10").unwrap(),
            "windows",
        )
        .unwrap();
        assert_eq!(update.version, "1.0.0-alpha.1");
        assert!(
            update
                .executable_url
                .as_deref()
                .unwrap()
                .ends_with("/LocalCodePilot-1.0.0-alpha.1-windows-x64.exe")
        );
        assert!(
            update
                .installer_url
                .as_deref()
                .unwrap()
                .ends_with("/LocalCodePilotSetup-1.0.0-alpha.1-windows-x64.exe")
        );
    }

    #[test]
    fn picks_the_update_executable_from_the_real_alpha_8_asset_list() {
        // Asset order as returned by the GitHub API for v0.4.0-alpha.8 (#97):
        // the installer comes before the update executable.
        let update = select_update_for_os(
            release_with_assets(
                "0.4.0-alpha.8",
                &[
                    "LocalCodePilot-0.4.0-alpha.8-macos-universal.dmg",
                    "LocalCodePilot-0.4.0-alpha.8-windows-x64-setup.exe",
                    "LocalCodePilot-0.4.0-alpha.8-windows-x64.exe",
                    "LocalCodePilot-0.4.0-alpha.8-windows-x64.zip",
                    "SHA256SUMS.txt",
                ],
            ),
            &Version::parse("0.4.0-alpha.7").unwrap(),
            "windows",
        )
        .unwrap();

        let executable_url = update.executable_url.as_deref().unwrap();
        assert!(executable_url.ends_with("/LocalCodePilot-0.4.0-alpha.8-windows-x64.exe"));
        assert!(trusted_release_asset_url(executable_url));
        assert!(update.download_url.ends_with("-windows-x64.zip"));
    }

    #[test]
    fn a_manual_download_on_windows_gets_the_installer_not_the_zip() {
        // Asset list of the published v0.4.0-alpha.10. A copy run from the loose
        // update executable (for example straight from Downloads) cannot update itself
        // and must be sent to the installer, never to another loose file or the zip.
        let assets = [
            "LocalCodePilot-0.4.0-alpha.10-macos-universal.dmg",
            "LocalCodePilot-0.4.0-alpha.10-windows-x64.exe",
            "LocalCodePilot-0.4.0-alpha.10-windows-x64.zip",
            "LocalCodePilotSetup-0.4.0-alpha.10-windows-x64.exe",
            "SHA256SUMS.txt",
            "SHA256SUMS.txt.sig",
        ];
        let current = Version::parse("0.4.0-alpha.9").unwrap();

        let windows = select_update_for_os(
            release_with_assets("0.4.0-alpha.10", &assets),
            &current,
            "windows",
        )
        .unwrap();
        assert!(
            manual_download_url_for_os(&windows, "windows")
                .ends_with("/LocalCodePilotSetup-0.4.0-alpha.10-windows-x64.exe")
        );
        assert_eq!(
            manual_download_note_for_os(&windows, "windows"),
            MANUAL_INSTALLER_NOTE
        );

        let macos = select_update_for_os(
            release_with_assets("0.4.0-alpha.10", &assets),
            &current,
            "macos",
        )
        .unwrap();
        assert!(manual_download_url_for_os(&macos, "macos").ends_with("-macos-universal.dmg"));
        assert_eq!(
            manual_download_note_for_os(&macos, "macos"),
            "Use o pacote oficial para concluir esta atualização."
        );

        // A release without the installer, or one whose installer link is not an
        // official release asset, falls back to the portable package.
        let without_installer = select_update_for_os(
            release_with_assets("0.4.0-alpha.10", &assets[..3]),
            &current,
            "windows",
        )
        .unwrap();
        assert!(manual_download_url_for_os(&without_installer, "windows").ends_with(".zip"));
        let mut foreign = windows.clone();
        foreign.installer_url = Some("https://example.com/LocalCodePilotSetup.exe".into());
        assert!(manual_download_url_for_os(&foreign, "windows").ends_with(".zip"));
    }

    #[test]
    fn ignores_installers_and_releases_without_the_exact_update_executable() {
        let renamed = select_update_for_os(
            release_with_assets(
                "0.4.0-alpha.9",
                &[
                    "LocalCodePilotSetup-0.4.0-alpha.9-windows-x64.exe",
                    "LocalCodePilot-0.4.0-alpha.9-windows-x64.exe",
                    "LocalCodePilot-0.4.0-alpha.9-windows-x64.zip",
                    "SHA256SUMS.txt",
                ],
            ),
            &Version::parse("0.4.0-alpha.8").unwrap(),
            "windows",
        )
        .unwrap();
        assert!(
            renamed
                .executable_url
                .as_deref()
                .unwrap()
                .ends_with("/LocalCodePilot-0.4.0-alpha.9-windows-x64.exe")
        );

        let installer_only = select_update_for_os(
            release_with_assets(
                "0.4.0-alpha.9",
                &[
                    "LocalCodePilot-0.4.0-alpha.9-windows-x64-setup.exe",
                    "LocalCodePilot-0.4.0-alpha.9-windows-x64.zip",
                ],
            ),
            &Version::parse("0.4.0-alpha.8").unwrap(),
            "windows",
        )
        .unwrap();
        assert!(installer_only.executable_url.is_none());
        assert!(!windows_update_asset(
            "LocalCodePilot-0.4.0-alpha.9-windows-x64-setup.exe"
        ));
        assert!(!windows_update_asset(
            "LocalCodePilotSetup-0.4.0-alpha.9-windows-x64.exe"
        ));
    }

    #[test]
    fn selects_the_universal_dmg_on_macos() {
        let update = select_update_for_os(
            releases(
                r#"[{
                    "tag_name":"v0.4.0-alpha.6",
                    "html_url":"https://example.com/release",
                    "prerelease":true,
                    "assets":[
                        {
                            "name":"LocalCodePilot-0.4.0-alpha.6-windows-x64.zip",
                            "browser_download_url":"https://example.com/windows.zip"
                        },
                        {
                            "name":"LocalCodePilot-0.4.0-alpha.6-macos-universal.dmg",
                            "browser_download_url":"https://example.com/macos.dmg"
                        }
                    ]
                }]"#,
            ),
            &Version::parse("0.4.0-alpha.5").unwrap(),
            "macos",
        )
        .unwrap();

        assert_eq!(update.download_url, "https://example.com/macos.dmg");
    }

    #[test]
    fn only_offers_releases_with_assets_for_the_requested_os() {
        let json = r#"[
            {
                "tag_name":"v0.4.0-alpha.6-rc.1",
                "html_url":"https://example.com/mac-release",
                "assets":[{
                    "name":"LocalCodePilot-0.4.0-alpha.6-rc.1-macos-universal.dmg",
                    "browser_download_url":"https://example.com/macos.dmg"
                }]
            },
            {
                "tag_name":"v0.4.0-alpha.5",
                "html_url":"https://example.com/windows-release",
                "assets":[{
                    "name":"LocalCodePilot-0.4.0-alpha.5-windows-x64.zip",
                    "browser_download_url":"https://example.com/windows.zip"
                }]
            },
            {
                "tag_name":"v0.4.0-alpha.7",
                "html_url":"https://example.com/empty-release"
            }
        ]"#;
        let current = Version::parse("0.4.0-alpha.4").unwrap();
        let windows = select_update_for_os(releases(json), &current, "windows").unwrap();
        assert_eq!(windows.version, "0.4.0-alpha.5");
        let macos = select_update_for_os(releases(json), &current, "macos").unwrap();
        assert_eq!(macos.version, "0.4.0-alpha.6-rc.1");
        assert!(macos.executable_url.is_none());
        assert!(
            select_update_for_os(
                releases(json),
                &Version::parse("0.4.0-alpha.5").unwrap(),
                "windows"
            )
            .is_none()
        );
        assert!(select_update_for_os(releases(json), &current, "linux").is_none());
    }

    #[test]
    fn the_summary_is_the_bold_sentence_that_opens_the_release_notes() {
        let alpha9 = "# LocalCodePilot 0.4.0-alpha.9\n\n**Mais seguro para abrir projetos de terceiros e para atualizar.** Versão alpha para Windows 10/11 x64.\n\n## Novidades\n\n- **Confirmação antes de executar.** ...";
        assert_eq!(
            release_summary(alpha9).as_deref(),
            Some("Mais seguro para abrir projetos de terceiros e para atualizar.")
        );
        // Without a title, the first paragraph still counts; CRLF and extra spaces are fine.
        assert_eq!(
            release_summary("\r\n**Atualização  com `etapas` na tela.**\r\nMais texto").as_deref(),
            Some("Atualização com etapas na tela.")
        );
    }

    #[test]
    fn without_an_opening_bold_sentence_there_is_no_summary() {
        for body in [
            "",
            "# LocalCodePilot 1.0",
            // The first paragraph does not start in bold; bold text later never counts.
            "# LocalCodePilot 1.0\n\nVersão para Windows.\n\n- **Novidade.** Algo",
            "Texto **em negrito no meio.**",
            // Unclosed bold, empty bold.
            "**Sem fechamento",
            "****",
            // Links, HTML or nested emphasis are not turned into plain text by guessing.
            "**Veja [a página](https://example.com).**",
            "**<b>Novo</b>**",
            "**Novo _recurso_.**",
        ] {
            assert_eq!(release_summary(body), None, "{body:?}");
        }
    }

    #[test]
    fn a_summary_longer_than_the_limit_is_not_cut() {
        let exact = "a".repeat(SUMMARY_MAX_CHARS);
        assert_eq!(
            release_summary(&format!("**{exact}**")),
            Some(exact.clone())
        );
        let longer = "é".repeat(SUMMARY_MAX_CHARS + 1);
        assert_eq!(release_summary(&format!("**{longer}**")), None);
    }

    #[test]
    fn the_notice_line_falls_back_to_the_release_page_and_old_caches_still_load() {
        // A cache written before the summary existed has no "summary" field.
        let cached: UpdateInfo = serde_json::from_str(
            r#"{"version":"0.4.0","title":"T","release_url":"https://example.com/r","download_url":"https://example.com/d"}"#,
        )
        .unwrap();
        assert_eq!(cached.summary, None);
        assert_eq!(
            summary_line(&cached),
            "Confira as novidades na página da versão."
        );
        let with_summary = UpdateInfo {
            summary: Some("Mais seguro.".into()),
            ..cached
        };
        assert_eq!(
            summary_line(&with_summary),
            "O que há de novo: Mais seguro."
        );
    }

    #[test]
    fn the_release_notes_body_reaches_the_update_summary() {
        let json = r##"[{"tag_name":"v99.0.0","name":"LocalCodePilot 99","html_url":"https://github.com/cjulio1993/LocalCodePilot/releases/tag/v99.0.0","body":"# LocalCodePilot 99\n\n**Tudo mais rápido.** Detalhes.","assets":[{"name":"LocalCodePilot-99.0.0-windows-x64.zip","browser_download_url":"https://github.com/cjulio1993/LocalCodePilot/releases/download/v99.0.0/LocalCodePilot-99.0.0-windows-x64.zip"}]},{"tag_name":"v98.0.0","html_url":"https://github.com/cjulio1993/LocalCodePilot/releases/tag/v98.0.0","body":null,"assets":[]}]"##;
        let current = Version::parse("1.0.0").unwrap();
        let update = select_update_for_os(releases(json), &current, "windows").unwrap();
        assert_eq!(update.summary.as_deref(), Some("Tudo mais rápido."));
    }

    #[test]
    fn rejects_incompatible_and_release_page_cached_notifications() {
        let mut update = UpdateInfo {
            version: "99.0.0".into(),
            title: "Update".into(),
            release_url: "https://example.com/release".into(),
            download_url: "https://example.com/release".into(),
            executable_url: None,
            checksum_url: None,
            summary: None,
            installer_url: None,
        };
        assert!(!cached_update_is_newer(&update));
        update.download_url = "https://example.com/LocalCodePilot-windows-x64.zip".into();
        assert!(cached_update_matches_os(&update, "windows"));
        assert!(!cached_update_matches_os(&update, "macos"));
        update.download_url = "https://example.com/LocalCodePilot-macos-universal.dmg".into();
        assert!(cached_update_matches_os(&update, "macos"));
        assert!(!cached_update_matches_os(&update, "windows"));
    }

    #[test]
    fn stable_channel_ignores_prereleases() {
        let update = select_update_for_os(
            releases(
                r#"[
                    {
                        "tag_name":"v0.5.0-alpha.1",
                        "html_url":"https://example.com/alpha",
                        "prerelease":true
                    },
                    {
                        "tag_name":"v0.4.1",
                        "html_url":"https://example.com/stable",
                        "prerelease":false,
                        "assets":[{
                            "name":"LocalCodePilot-0.4.1-windows-x64.zip",
                            "browser_download_url":"https://example.com/windows.zip"
                        }]
                    }
                ]"#,
            ),
            &Version::parse("0.4.0").unwrap(),
            "windows",
        )
        .unwrap();

        assert_eq!(update.version, "0.4.1");
    }

    #[test]
    fn ignores_drafts_and_versions_that_are_not_newer() {
        let update = select_update(
            releases(
                r#"[
                    {
                        "tag_name":"v0.4.0-alpha.2",
                        "html_url":"https://example.com/draft",
                        "draft":true,
                        "prerelease":true
                    },
                    {
                        "tag_name":"v0.4.0-alpha.1",
                        "html_url":"https://example.com/current",
                        "prerelease":true
                    }
                ]"#,
            ),
            &Version::parse("0.4.0-alpha.1").unwrap(),
        );

        assert!(update.is_none());
    }

    #[test]
    fn reads_only_the_exact_sha256_asset_entry() {
        let checksums = concat!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other.exe\n",
            "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789  LocalCodePilot-0.4.0-alpha.5-windows-x64.exe\n"
        );

        assert_eq!(
            checksum_for_asset(checksums, "LocalCodePilot-0.4.0-alpha.5-windows-x64.exe")
                .as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
        assert!(checksum_for_asset(checksums, "LocalCodePilot.exe").is_none());
    }

    #[test]
    fn accepts_only_assets_from_the_official_release_path() {
        assert!(trusted_release_asset_url(
            "https://github.com/cjulio1993/LocalCodePilot/releases/download/v0.5.0/LocalCodePilot.exe"
        ));
        assert!(!trusted_release_asset_url(
            "https://example.com/LocalCodePilot.exe"
        ));
        assert!(!trusted_release_asset_url(
            "https://github.com/cjulio1993/LocalCodePilot/releases/download/v0.5.0/file.exe?redirect=example"
        ));
    }

    /// Normal and release builds have no local feed: whatever the environment
    /// says, updates come only from the official releases.
    #[cfg(not(feature = "update-test-feed"))]
    #[test]
    fn without_the_test_feed_feature_updates_come_only_from_github() {
        assert_eq!(releases_url(), RELEASES_URL);
        assert_eq!(release_asset_prefix(), RELEASE_ASSET_PREFIX);
        assert!(!trusted_release_asset_url(
            "http://127.0.0.1:8765/download/v0.5.0/LocalCodePilot-0.5.0-windows-x64.exe"
        ));
        assert_eq!(
            current_version(),
            Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
        );
    }

    #[test]
    fn cleanup_rejects_a_stage_that_escapes_the_update_root() {
        let stage = new_stage_directory("cleanup-test").unwrap();
        fs::create_dir_all(&stage).unwrap();
        let helper = stage.join("LocalCodePilot-updater.exe");
        fs::write(&helper, b"MZ helper").unwrap();
        assert!(validate_cleanup_paths(&stage.join("startup-ok"), &helper, &stage).is_ok());

        let escaped_stage = updates_root_directory()
            .join("..")
            .join(stage.file_name().unwrap());
        fs::create_dir_all(&escaped_stage).unwrap();
        let escaped_helper = escaped_stage.join("LocalCodePilot-updater.exe");
        fs::write(&escaped_helper, b"MZ helper").unwrap();

        assert!(
            validate_cleanup_paths(
                &escaped_stage.join("startup-ok"),
                &escaped_helper,
                &escaped_stage,
            )
            .is_err()
        );
        fs::remove_dir_all(stage).unwrap();
        fs::remove_dir_all(escaped_stage).unwrap();
    }

    #[test]
    fn restores_cached_updates_written_before_automatic_installation() {
        let update: UpdateInfo = serde_json::from_str(
            r#"{
                "version":"0.4.0-alpha.4",
                "title":"LocalCodePilot 0.4.0-alpha.4",
                "release_url":"https://example.com/release",
                "download_url":"https://example.com/windows.zip"
            }"#,
        )
        .unwrap();

        assert_eq!(update.executable_url, None);
        assert_eq!(update.checksum_url, None);
    }

    enum Reply {
        Complete(Vec<u8>),
        /// Announces `announced` bytes, sends only `body` and keeps the
        /// connection open for `hold` before closing it.
        Partial {
            announced: u64,
            body: Vec<u8>,
            hold: Duration,
        },
        /// Sends the whole body in `chunk`-sized pieces, pausing `pause`
        /// between them.
        Slow {
            body: Vec<u8>,
            chunk: usize,
            pause: Duration,
        },
    }

    /// Minimal HTTP/1.1 server on localhost: one connection per reply.
    fn serve(replies: Vec<(&'static str, Reply)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming().take(replies.len()) {
                let mut stream = stream.unwrap();
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or_default();
                let header = |length: u64| {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
                    )
                };
                match replies.iter().find(|(served, _)| *served == path) {
                    Some((_, Reply::Complete(body))) => {
                        let _ = stream.write_all(header(body.len() as u64).as_bytes());
                        let _ = stream.write_all(body);
                    }
                    Some((_, Reply::Slow { body, chunk, pause })) => {
                        let _ = stream.write_all(header(body.len() as u64).as_bytes());
                        for piece in body.chunks(*chunk) {
                            let _ = stream.write_all(piece);
                            let _ = stream.flush();
                            thread::sleep(*pause);
                        }
                    }
                    Some((
                        _,
                        Reply::Partial {
                            announced,
                            body,
                            hold,
                        },
                    )) => {
                        let _ = stream.write_all(header(*announced).as_bytes());
                        let _ = stream.write_all(body);
                        let _ = stream.flush();
                        thread::sleep(*hold);
                    }
                    None => {
                        let _ = stream.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                }
            }
        });
        address
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// A throwaway signing key for the tests (never the release key).
    fn test_signer(seed: u8) -> (ring::signature::Ed25519KeyPair, [u8; 32]) {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
        let public = pair.public_key().as_ref().try_into().unwrap();
        (pair, public)
    }

    fn sign(pair: &ring::signature::Ed25519KeyPair, contents: &[u8]) -> Vec<u8> {
        pair.sign(contents).as_ref().to_vec()
    }

    #[test]
    fn the_embedded_signing_key_is_the_published_public_key() {
        // packaging/update-signing.pub.pem is an Ed25519 SubjectPublicKeyInfo:
        // a fixed 12-byte prefix followed by the 32-byte key.
        let pem = include_str!("../../packaging/update-signing.pub.pem");
        let base64: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let der = decode_base64(&base64);
        assert_eq!(der.len(), 44);
        assert_eq!(
            der[..12],
            [
                0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00
            ]
        );
        assert_eq!(der[12..], UPDATE_SIGNING_PUBLIC_KEY);
    }

    fn decode_base64(text: &str) -> Vec<u8> {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bits = 0u32;
        let mut count = 0;
        let mut bytes = Vec::new();
        for character in text.bytes().filter(|byte| *byte != b'=') {
            let value = ALPHABET.iter().position(|item| *item == character).unwrap() as u32;
            bits = (bits << 6) | value;
            count += 6;
            if count >= 8 {
                count -= 8;
                bytes.push((bits >> count) as u8);
                bits &= (1 << count) - 1;
            }
        }
        bytes
    }

    #[test]
    fn checksums_are_trusted_only_with_a_valid_signature_of_the_expected_key() {
        let (pair, public) = test_signer(7);
        let (_, other_public) = test_signer(8);
        let checksums = b"abc  LocalCodePilot-9.9.9-windows-x64.exe\n";
        let signature = sign(&pair, checksums);

        assert_eq!(
            verify_checksums_signature(checksums, &signature, &public),
            Ok(())
        );
        let mut altered = checksums.to_vec();
        altered[0] = b'f';
        for (contents, signature, key) in [
            (altered.as_slice(), signature.clone(), public),
            (checksums.as_slice(), signature.clone(), other_public),
            (checksums.as_slice(), signature[..63].to_vec(), public),
            (checksums.as_slice(), Vec::new(), public),
        ] {
            let error = verify_checksums_signature(contents, &signature, &key).unwrap_err();
            assert!(error.technical.contains("não confere"), "{error}");
            assert_eq!(error.kind, UpdateFailureKind::Verification);
        }
    }

    fn signed_preparation(
        server: &str,
        directory: &Path,
        signing_key: [u8; 32],
    ) -> UpdatePreparation {
        fs::create_dir_all(directory).unwrap();
        let target = directory.join("LocalCodePilot.exe");
        fs::write(&target, b"MZ current version").unwrap();
        UpdatePreparation {
            executable_url: format!("{server}/exe"),
            checksum_url: format!("{server}/SHA256SUMS.txt"),
            signing_key,
            asset_name: "LocalCodePilot-9.9.9-windows-x64.exe".into(),
            candidate_name: EXECUTABLE_CANDIDATE_NAME,
            version: "9.9.9".into(),
            target_path: target,
            stage_directory: directory.join("stage"),
        }
    }

    #[test]
    fn an_update_without_a_valid_signature_is_refused_before_the_download() {
        let executable = b"MZ new version".to_vec();
        let checksums = format!(
            "{}  LocalCodePilot-9.9.9-windows-x64.exe\n",
            sha256_hex(&executable)
        );
        let (pair, public) = test_signer(7);
        let (other_pair, _) = test_signer(8);
        let cases = [
            ("ausente", None, "não publicou a assinatura"),
            (
                "outra-chave",
                Some(sign(&other_pair, checksums.as_bytes())),
                "não confere",
            ),
            (
                "conteudo-trocado",
                Some(sign(&pair, b"outro conteudo")),
                "não confere",
            ),
        ];
        for (name, signature, expected) in cases {
            let mut replies = vec![(
                "/SHA256SUMS.txt",
                Reply::Complete(checksums.clone().into_bytes()),
            )];
            match signature {
                Some(signature) => {
                    replies.push(("/SHA256SUMS.txt.sig", Reply::Complete(signature)))
                }
                // The request for the signature gets a 404.
                None => replies.push(("/nothing", Reply::Complete(Vec::new()))),
            }
            replies.push(("/exe", Reply::Complete(executable.clone())));
            let server = serve(replies);
            let directory = temporary_test_directory(&format!("signature-{name}"));
            let preparation = signed_preparation(&server, &directory, public);
            let (sender, receiver) = mpsc::channel();

            let error =
                prepare_update(&preparation, &sender, &egui::Context::default()).unwrap_err();
            drop(sender);

            assert!(error.technical.contains(expected), "{name}: {error}");
            // A missing or wrong signature is a refusal, never a retryable download error.
            assert_eq!(error.kind, UpdateFailureKind::Verification, "{name}");
            assert!(!error.can_retry(), "{name}");
            // Nothing was downloaded: the phase never reached the executable.
            assert!(
                !receiver.iter().any(|event| matches!(
                    event,
                    UpdateInstallEvent::Phase(UpdatePhase::Downloading)
                )),
                "{name}"
            );
            assert!(
                !preparation
                    .stage_directory
                    .join("LocalCodePilot.exe")
                    .exists()
            );
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn the_download_has_no_total_time_limit_only_stall_limits() {
        let agent = download_agent();
        let config = agent.config();
        assert_eq!(config.timeouts().global, None);
        assert_eq!(config.timeout_per_read(), Some(DOWNLOAD_STALL_TIMEOUT));
        assert_eq!(config.timeouts().per_call, None);
        assert_eq!(config.timeouts().recv_body, None);
    }

    #[test]
    fn prepares_an_update_reporting_phases_in_order_and_throttled_progress() {
        let mut executable = b"MZ".to_vec();
        executable.resize(2 * 1024 * 1024, 7);
        let size = executable.len() as u64;
        let asset = "LocalCodePilot-9.9.9-windows-x64.exe";
        let checksums = format!("{}  {asset}\n", sha256_hex(&executable));
        let (pair, public) = test_signer(7);
        let signature = sign(&pair, checksums.as_bytes());
        let server = serve(vec![
            ("/SHA256SUMS.txt", Reply::Complete(checksums.into_bytes())),
            ("/SHA256SUMS.txt.sig", Reply::Complete(signature)),
            ("/exe", Reply::Complete(executable.clone())),
        ]);
        let directory = temporary_test_directory("prepare");
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("LocalCodePilot.exe");
        fs::write(&target, b"MZ current version").unwrap();
        let preparation = UpdatePreparation {
            executable_url: format!("{server}/exe"),
            checksum_url: format!("{server}/SHA256SUMS.txt"),
            signing_key: public,
            asset_name: asset.into(),
            candidate_name: EXECUTABLE_CANDIDATE_NAME,
            version: "9.9.9".into(),
            target_path: target,
            stage_directory: directory.join("stage"),
        };
        let (sender, receiver) = mpsc::channel();

        let started = Instant::now();
        let prepared = prepare_update(&preparation, &sender, &egui::Context::default()).unwrap();
        let elapsed = started.elapsed();
        drop(sender);
        let events: Vec<_> = receiver.iter().collect();

        let phases: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                UpdateInstallEvent::Phase(phase) => Some(*phase),
                _ => None,
            })
            .collect();
        assert_eq!(
            phases,
            [
                UpdatePhase::Checksums,
                UpdatePhase::Downloading,
                UpdatePhase::Verifying
            ]
        );
        let progress: Vec<_> = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| match event {
                UpdateInstallEvent::Progress {
                    downloaded_bytes,
                    total_bytes,
                } => Some((index, *downloaded_bytes, *total_bytes)),
                _ => None,
            })
            .collect();
        let (last_index, last_bytes, last_total) = *progress.last().unwrap();
        assert_eq!((last_bytes, last_total), (size, Some(size)));
        let verifying_index = events
            .iter()
            .position(|event| matches!(event, UpdateInstallEvent::Phase(UpdatePhase::Verifying)))
            .unwrap();
        assert!(last_index < verifying_index);
        // At most one event per interval, plus the first and the final one.
        let allowed = 2 + (elapsed.as_millis() / PROGRESS_INTERVAL.as_millis()) as usize;
        assert!(
            progress.len() <= allowed,
            "{} progress events in {elapsed:?}",
            progress.len()
        );
        assert_eq!(fs::read(&prepared.candidate_path).unwrap(), executable);
        assert_eq!(prepared.expected_sha256, sha256_hex(&executable));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_slow_steady_download_outlasting_the_stall_limit_completes() {
        let mut executable = b"MZ".to_vec();
        executable.resize(20 * 1024, 3);
        let server = serve(vec![(
            "/exe",
            Reply::Slow {
                body: executable.clone(),
                chunk: 2 * 1024,
                pause: Duration::from_millis(300),
            },
        )]);
        let directory = temporary_test_directory("slow");
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("LocalCodePilot.exe");
        let (sender, _receiver) = mpsc::channel();
        let stall_limit = Duration::from_secs(1);

        let started = Instant::now();
        download_executable(
            &download_agent_with_stall_timeout(stall_limit),
            &format!("{server}/exe"),
            &destination,
            &sha256_hex(&executable),
            &sender,
            &egui::Context::default(),
        )
        .unwrap();

        // Ten pieces 300 ms apart: the whole download takes about three times
        // the stall limit and still finishes, because data keeps arriving.
        assert!(started.elapsed() > stall_limit * 2);
        assert_eq!(fs::read(&destination).unwrap(), executable);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_stalled_download_fails_as_interrupted_without_waiting_for_a_total_limit() {
        let server = serve(vec![(
            "/exe",
            Reply::Partial {
                announced: 1_000_000,
                body: b"MZ".repeat(1000),
                hold: Duration::from_secs(6),
            },
        )]);
        let directory = temporary_test_directory("stalled");
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("LocalCodePilot.exe");
        let (sender, _receiver) = mpsc::channel();

        let started = Instant::now();
        let error = download_executable(
            &download_agent_with_stall_timeout(Duration::from_secs(1)),
            &format!("{server}/exe"),
            &destination,
            &"a".repeat(64),
            &sender,
            &egui::Context::default(),
        )
        .unwrap_err();

        assert!(error.technical.contains("interrompido"), "{error}");
        assert_eq!(error.kind, UpdateFailureKind::Download);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!destination.exists());
        assert!(!destination.with_extension("exe.part").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_download_closed_before_the_announced_size_is_rejected() {
        let server = serve(vec![(
            "/exe",
            Reply::Partial {
                announced: 500_000,
                body: b"MZ".repeat(1000),
                hold: Duration::ZERO,
            },
        )]);
        let directory = temporary_test_directory("truncated");
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("LocalCodePilot.exe");
        let (sender, _receiver) = mpsc::channel();

        let error = download_executable(
            &download_agent(),
            &format!("{server}/exe"),
            &destination,
            &"a".repeat(64),
            &sender,
            &egui::Context::default(),
        )
        .unwrap_err();

        assert!(error.technical.contains("interrompido"), "{error}");
        assert_eq!(error.kind, UpdateFailureKind::Download);
        assert!(!destination.exists());
        assert!(!destination.with_extension("exe.part").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    /// T002 CA-6: each kind has a plain title, a sentence that says the current
    /// version keeps working, and the action agreed with the founder.
    #[test]
    fn each_failure_kind_has_a_plain_message_and_a_safe_action() {
        let kinds = [
            UpdateFailureKind::NoConnection,
            UpdateFailureKind::InvalidResponse,
            UpdateFailureKind::Download,
            UpdateFailureKind::Verification,
            UpdateFailureKind::Local,
        ];
        for kind in kinds {
            let failure = UpdateFailure::new(kind, "erro técnico de exemplo");
            assert!(!failure.title().is_empty());
            // The technical text never reaches the first layer.
            assert!(!failure.title().contains("erro técnico"));
            assert!(!failure.message().contains("erro técnico"));
            assert_eq!(failure.to_string(), "erro técnico de exemplo");
        }
        let verification = UpdateFailure::new(UpdateFailureKind::Verification, "x");
        assert_eq!(verification.title(), "Atualização recusada por segurança");
        assert!(!verification.can_retry());
        assert_eq!(
            verification.release_page_label(),
            Some("Consultar página oficial da versão")
        );
        assert!(verification.message().contains("Não instale manualmente"));
        for kind in [UpdateFailureKind::Download, UpdateFailureKind::Local] {
            let failure = UpdateFailure::new(kind, "x");
            assert!(failure.can_retry());
            assert_eq!(
                failure.release_page_label(),
                Some("Ver página oficial da versão")
            );
        }
        for kind in [
            UpdateFailureKind::NoConnection,
            UpdateFailureKind::InvalidResponse,
        ] {
            let failure = UpdateFailure::new(kind, "x");
            assert_eq!(failure.title(), "Não foi possível verificar atualizações");
            assert!(failure.can_retry());
            assert_eq!(failure.release_page_label(), None);
        }
        assert!(
            UpdateFailure::new(UpdateFailureKind::NoConnection, "x")
                .message()
                .starts_with("Confira sua conexão")
        );
        assert!(
            UpdateFailure::new(UpdateFailureKind::InvalidResponse, "x")
                .message()
                .starts_with("O serviço de atualizações não respondeu")
        );
    }

    #[test]
    fn http_failures_split_no_connection_from_an_error_status() {
        let status = ureq::Error::StatusCode(403);
        let failure = http_failure(
            &status,
            UpdateFailureKind::InvalidResponse,
            UpdateFailureKind::NoConnection,
            "consulta",
        );
        assert_eq!(failure.kind, UpdateFailureKind::InvalidResponse);
        let transport = ureq::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "recusada",
        ));
        let failure = http_failure(
            &transport,
            UpdateFailureKind::InvalidResponse,
            UpdateFailureKind::NoConnection,
            "consulta",
        );
        assert_eq!(failure.kind, UpdateFailureKind::NoConnection);
        assert!(failure.technical.starts_with("consulta: "));
        // A file that the release did not publish is refused; other errors are downloads.
        assert_eq!(
            release_file_failure(&ureq::Error::StatusCode(404), "x").kind,
            UpdateFailureKind::Verification
        );
        assert_eq!(
            release_file_failure(&ureq::Error::StatusCode(503), "x").kind,
            UpdateFailureKind::Download
        );
    }

    #[test]
    fn removes_only_old_preparation_folders() {
        let root = temporary_test_directory("stale");
        fs::create_dir_all(root.join("0.4.0-old").join("nested")).unwrap();
        fs::write(root.join("0.4.0-old").join("nested").join("file"), b"x").unwrap();
        fs::write(root.join("loose-file.txt"), b"x").unwrap();

        assert_eq!(remove_stale_stages(&root, Duration::from_secs(3600)), 0);
        assert!(root.join("0.4.0-old").is_dir());

        assert_eq!(remove_stale_stages(&root, Duration::ZERO), 1);
        assert!(!root.join("0.4.0-old").exists());
        assert!(root.join("loose-file.txt").is_file());
        assert_eq!(
            remove_stale_stages(&root.join("missing"), Duration::ZERO),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stale_cleanup_never_follows_links_out_of_the_update_root() {
        let root = temporary_test_directory("stale-link");
        let outside = temporary_test_directory("stale-outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();

        assert_eq!(remove_stale_stages(&root, Duration::ZERO), 0);
        assert!(outside.join("keep").is_file());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn rollback_messages_tell_what_is_installed_and_what_to_do_next() {
        let backup = Path::new("C:\\config\\backups\\antes-de-0.4.0-alpha.9");
        let failure = "a versão 0.4.0-alpha.9 fechou ao iniciar (exit code: 1)";

        let restored = rollback_failure(
            failure,
            "0.4.0-alpha.8",
            "0.4.0-alpha.9",
            Some(Ok(())),
            backup,
            false,
        );
        assert!(restored.relaunch_previous);
        assert!(
            restored
                .message
                .contains("A versão 0.4.0-alpha.8 foi restaurada e continua em uso")
        );
        assert!(
            restored
                .message
                .contains("tente de novo mais tarde ou baixe-a na página oficial")
        );

        let configuration_lost = rollback_failure(
            failure,
            "0.4.0-alpha.8",
            "0.4.0-alpha.9",
            Some(Err(std::io::Error::other("acesso negado"))),
            backup,
            true,
        );
        assert!(configuration_lost.relaunch_previous);
        assert!(
            configuration_lost
                .message
                .contains("as configurações não puderam ser recuperadas (acesso negado)")
        );
        assert!(
            configuration_lost
                .message
                .contains("antes-de-0.4.0-alpha.9")
        );
        assert!(!configuration_lost.message.contains("continua em uso"));

        let not_restored = rollback_failure(
            failure,
            "0.4.0-alpha.8",
            "0.4.0-alpha.9",
            None,
            backup,
            false,
        );
        assert!(!not_restored.relaunch_previous);
        assert!(
            not_restored
                .message
                .contains("não foi possível restaurar a versão 0.4.0-alpha.8")
        );
        assert!(
            not_restored
                .message
                .contains("Baixe o LocalCodePilot na página oficial")
        );
        assert!(!not_restored.message.contains("foi restaurada"));

        let interrupted = not_completed_message("0.4.0-alpha.8", "0.4.0-alpha.9");
        assert!(interrupted.contains("A versão 0.4.0-alpha.8 continua instalada"));
        assert!(interrupted.contains("use Atualizar agora em Configurações"));
    }

    #[test]
    fn installed_copy_that_did_not_open_uses_the_approved_text() {
        let backup = Path::new("C:\\config\\backups\\antes-de-0.4.0-beta.2");
        let restored = rollback_failure(
            "a versão 0.4.0-beta.2 fechou ao iniciar (exit code: 3)",
            "0.4.0-beta.1",
            "0.4.0-beta.2",
            Some(Ok(())),
            backup,
            true,
        );
        assert!(restored.relaunch_previous);
        assert!(matches!(restored.report, HelperReport::Plain));
        assert_eq!(
            restored.message,
            "A versão 0.4.0-beta.2 não abriu. Restauramos o aplicativo e as configurações da versão 0.4.0-beta.1; você pode continuar trabalhando. A lista de programas do Windows ainda pode mostrar a versão 0.4.0-beta.2. Tente novamente mais tarde ou baixe o instalador na página oficial."
        );
    }

    #[test]
    fn installer_failure_has_the_approved_title_message_and_actions() {
        let failure = UpdateFailure::installer(
            "0.4.0-beta.1",
            "0.4.0-beta.2",
            "o instalador terminou com o código 5; registro do instalador: C:\\x\\update-setup.log",
        );
        assert_eq!(failure.title(), "A instalação não foi concluída");
        assert_eq!(
            failure.message(),
            "Não foi possível instalar a versão 0.4.0-beta.2. A versão 0.4.0-beta.1 foi reaberta e você pode continuar trabalhando. Tente novamente mais tarde ou baixe o instalador na página oficial."
        );
        assert!(failure.can_retry());
        assert_eq!(
            failure.release_page_label(),
            Some("Ver página oficial da versão")
        );
        assert!(failure.technical.contains("código 5"));
    }

    #[test]
    fn installer_failure_survives_the_reopen_once() {
        let saved = InstallerFailure {
            from_version: "0.4.0-beta.1".into(),
            to_version: "0.4.0-beta.2".into(),
            technical: "o instalador terminou com o código 1".into(),
        };
        let json = serde_json::to_vec(&saved).unwrap();
        let read: InstallerFailure = serde_json::from_slice(&json).unwrap();
        assert_eq!(read, saved);
    }

    #[test]
    fn helper_accepts_only_the_two_candidate_names() {
        let stage = Path::new("stage");
        assert!(is_installer_candidate(
            &stage.join("LocalCodePilotSetup.exe")
        ));
        assert!(!is_installer_candidate(&stage.join("LocalCodePilot.exe")));
        assert!(!is_installer_candidate(&stage.join("setup.exe")));
    }

    #[test]
    fn same_directory_ignores_case_and_trailing_separator() {
        assert!(same_directory(
            Path::new("C:\\Users\\Dev\\AppData\\Local\\Programs\\LocalCodePilot\\"),
            Path::new("c:\\users\\dev\\appdata\\local\\programs\\localcodepilot"),
        ));
        assert!(!same_directory(
            Path::new("C:\\Programs\\LocalCodePilot"),
            Path::new("C:\\Programs\\LocalCodePilot-portatil"),
        ));
    }
}
