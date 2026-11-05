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

const RELEASES_URL: &str =
    "https://api.github.com/repos/cjulio1993/LocalCodePilot/releases?per_page=20";
const RELEASE_ASSET_PREFIX: &str =
    "https://github.com/cjulio1993/LocalCodePilot/releases/download/";
const UPDATE_HELPER_FLAG: &str = "--localcodepilot-apply-update";
const UPDATED_STARTUP_FLAG: &str = "--localcodepilot-updated-startup";
const MAX_EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 1024 * 1024;
const REPLACEMENT_TIMEOUT: Duration = Duration::from_secs(90);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const STALE_STAGE_AGE: Duration = Duration::from_secs(24 * 60 * 60);
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
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    html_url: String,
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
    Failed(String),
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
    asset_name: String,
    version: String,
    target_path: PathBuf,
    stage_directory: PathBuf,
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("the workspace package version must follow semantic versioning")
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
        && update
            .executable_url
            .as_deref()
            .is_some_and(trusted_release_asset_url)
        && update
            .checksum_url
            .as_deref()
            .is_some_and(trusted_release_asset_url)
}

pub fn spawn_update_check(repaint: egui::Context) -> Receiver<Result<Option<UpdateInfo>, String>> {
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
) -> Result<Receiver<UpdateInstallEvent>, String> {
    if !cached_update_is_newer(update) {
        return Err("a versão selecionada não é mais recente que a instalada".into());
    }
    let target_path = packaged_executable_path()?;
    verify_target_directory_writable(&target_path)?;
    let executable_url = update
        .executable_url
        .clone()
        .filter(|url| trusted_release_asset_url(url))
        .ok_or_else(|| {
            "a release não oferece um executável confiável para atualização".to_owned()
        })?;
    let checksum_url = update
        .checksum_url
        .clone()
        .filter(|url| trusted_release_asset_url(url))
        .ok_or_else(|| "a release não oferece o arquivo oficial de checksums".to_owned())?;
    let asset_name = windows_update_asset_name(&update.version);
    if !executable_url.ends_with(&format!("/{asset_name}")) {
        return Err("o nome do executável da release não corresponde à versão selecionada".into());
    }
    let preparation = UpdatePreparation {
        executable_url,
        checksum_url,
        asset_name,
        version: update.version.clone(),
        target_path,
        stage_directory: new_stage_directory(&update.version)?,
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

pub fn try_run_update_helper() -> Option<i32> {
    let arguments = match parse_helper_arguments() {
        Ok(Some(arguments)) => arguments,
        Ok(None) => return None,
        Err(error) => {
            write_persistent_update_error(&error);
            return Some(2);
        }
    };
    match apply_prepared_update(&arguments) {
        Ok(()) => Some(0),
        Err(failure) => {
            // The message is saved before the previous version reopens, so
            // that version is the one that shows it.
            write_persistent_update_error(&failure.message);
            if failure.relaunch_previous {
                relaunch_after_failure(&arguments.target_path);
            }
            Some(1)
        }
    }
}

#[derive(Debug)]
struct HelperFailure {
    message: String,
    /// The previous executable is in place and should be reopened.
    relaunch_previous: bool,
}

impl HelperFailure {
    fn keep_closed(message: String) -> Self {
        Self {
            message,
            relaunch_previous: false,
        }
    }

    fn reopen_previous(message: String) -> Self {
        Self {
            message,
            relaunch_previous: true,
        }
    }
}

pub fn complete_updated_startup() -> Option<String> {
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
            spawn_update_cleanup(helper_path, stage_directory);
            spawn_confirmed_update_cleanup();
            return Some(updated_message(&version));
        }
    }
    let error = read_persistent_update_error();
    let recovered = recover_interrupted_update();
    error.or(recovered)
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

fn fetch_update(current: &Version) -> Result<Option<UpdateInfo>, String> {
    let agent = update_agent(Duration::from_secs(8));
    let mut response = agent
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "LocalCodePilot-Update-Checker")
        .call()
        .map_err(|error| format!("não foi possível consultar as releases: {error}"))?;
    let releases: Vec<GithubRelease> = response
        .body_mut()
        .read_json()
        .map_err(|error| format!("resposta de atualização inválida: {error}"))?;
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
) -> Result<PreparedUpdate, String> {
    fs::create_dir_all(&preparation.stage_directory)
        .map_err(|error| format!("não foi possível criar a pasta temporária: {error}"))?;
    let agent = download_agent();
    send_phase(sender, repaint, UpdatePhase::Checksums);
    let checksums = download_checksums(&agent, &preparation.checksum_url)?;
    let expected_sha256 = checksum_for_asset(&checksums, &preparation.asset_name)
        .ok_or_else(|| format!("o checksum de {} não foi publicado", preparation.asset_name))?;
    let candidate_path = preparation.stage_directory.join("LocalCodePilot.exe");
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
    fs::copy(&preparation.target_path, &helper_path)
        .map_err(|error| format!("não foi possível preparar o atualizador temporário: {error}"))?;
    Ok(PreparedUpdate {
        helper_path,
        candidate_path,
        target_path: preparation.target_path.clone(),
        stage_directory: preparation.stage_directory.clone(),
        expected_sha256,
        version: preparation.version.clone(),
    })
}

fn download_checksums(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let mut response = agent
        .get(url)
        .header("User-Agent", "LocalCodePilot-Updater")
        .call()
        .map_err(|error| format!("não foi possível baixar os checksums: {error}"))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_CHECKSUM_BYTES)
        .read_to_string()
        .map_err(|error| format!("o arquivo de checksums é inválido: {error}"))
}

fn download_executable(
    agent: &ureq::Agent,
    url: &str,
    destination: &Path,
    expected_sha256: &str,
    sender: &Sender<UpdateInstallEvent>,
    repaint: &egui::Context,
) -> Result<(), String> {
    let mut response = agent
        .get(url)
        .header("User-Agent", "LocalCodePilot-Updater")
        .call()
        .map_err(|error| format!("não foi possível baixar a atualização: {error}"))?;
    let total_bytes = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|size| *size <= MAX_EXECUTABLE_BYTES);
    let partial_path = destination.with_extension("exe.part");
    let mut output = File::create(&partial_path)
        .map_err(|error| format!("não foi possível criar o download temporário: {error}"))?;
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
                return Err(format!(
                    "o download da atualização foi interrompido: {error}"
                ));
            }
        };
        if read == 0 {
            break;
        }
        downloaded_bytes = downloaded_bytes.saturating_add(read as u64);
        if downloaded_bytes > MAX_EXECUTABLE_BYTES {
            let _ = fs::remove_file(&partial_path);
            return Err("o executável baixado excede o limite permitido".into());
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("não foi possível salvar a atualização: {error}"))?;
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
        .map_err(|error| format!("não foi possível concluir o download: {error}"))?;
    drop(output);
    if total_bytes.is_some_and(|total| total != downloaded_bytes) {
        let _ = fs::remove_file(&partial_path);
        return Err("o download da atualização foi interrompido antes do fim".into());
    }
    send_phase(sender, repaint, UpdatePhase::Verifying);
    let actual_sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        let _ = fs::remove_file(&partial_path);
        return Err("o SHA-256 do executável baixado não corresponde ao publicado".into());
    }
    if downloaded_bytes < 1024 || !file_starts_with_mz(&partial_path)? {
        let _ = fs::remove_file(&partial_path);
        return Err("o arquivo baixado não é um executável Windows válido".into());
    }
    fs::rename(&partial_path, destination)
        .map_err(|error| format!("não foi possível finalizar o download: {error}"))
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
) -> HelperFailure {
    match configuration_restored {
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

fn apply_prepared_update(arguments: &HelperArguments) -> Result<(), HelperFailure> {
    let to_version = &arguments.version;
    // Nothing is touched while the app that started the helper still runs;
    // relaunching it here would open a second instance.
    if !recovery::wait_for_process_exit(arguments.parent_pid, REPLACEMENT_TIMEOUT) {
        return Err(HelperFailure::keep_closed(format!(
            "o LocalCodePilot não fechou a tempo, e a atualização para a versão {to_version} não foi aplicada. Nada foi alterado: feche o LocalCodePilot e tente atualizar de novo"
        )));
    }
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
    let swapped = recovery::swap_in_candidate(
        &arguments.target_path,
        &arguments.candidate_path,
        &arguments.expected_sha256,
        sha256_file,
        |from, to| recovery::rename_with_retry(from, to, REPLACEMENT_TIMEOUT),
    );
    if let Err(error) = swapped {
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
    ))
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
    url.starts_with(RELEASE_ASSET_PREFIX) && !url.contains(['?', '#'])
}

fn persistent_update_error_path() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("LocalCodePilot")
        .join("update-error.txt")
}

fn write_persistent_update_error(error: &str) {
    let path = persistent_update_error_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, format!("A atualização automática falhou: {error}"));
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
    fn rejects_incompatible_and_release_page_cached_notifications() {
        let mut update = UpdateInfo {
            version: "99.0.0".into(),
            title: "Update".into(),
            release_url: "https://example.com/release".into(),
            download_url: "https://example.com/release".into(),
            executable_url: None,
            checksum_url: None,
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
        let server = serve(vec![
            ("/SHA256SUMS.txt", Reply::Complete(checksums.into_bytes())),
            ("/exe", Reply::Complete(executable.clone())),
        ]);
        let directory = temporary_test_directory("prepare");
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("LocalCodePilot.exe");
        fs::write(&target, b"MZ current version").unwrap();
        let preparation = UpdatePreparation {
            executable_url: format!("{server}/exe"),
            checksum_url: format!("{server}/SHA256SUMS.txt"),
            asset_name: asset.into(),
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

        assert!(error.contains("interrompido"), "{error}");
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

        assert!(error.contains("interrompido"), "{error}");
        assert!(!destination.exists());
        assert!(!destination.with_extension("exe.part").exists());
        fs::remove_dir_all(directory).unwrap();
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

        let not_restored =
            rollback_failure(failure, "0.4.0-alpha.8", "0.4.0-alpha.9", None, backup);
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
}
