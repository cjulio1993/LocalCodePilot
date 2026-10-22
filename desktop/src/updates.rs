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
    time::{Duration, SystemTime},
};

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

#[derive(Debug)]
pub enum UpdateInstallEvent {
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
    let asset_name = format!("LocalCodePilot-{}-windows-x64.exe", update.version);
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
        Err(error) => {
            write_persistent_update_error(&error);
            Some(1)
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
            return Some(format!("LocalCodePilot {version} instalado com sucesso"));
        }
    }
    read_persistent_update_error()
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
            let executable_url = release
                .assets
                .iter()
                .find(|asset| operating_system == "windows" && windows_update_asset(&asset.name))
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
    let agent = update_agent(Duration::from_secs(90));
    let checksums = download_checksums(&agent, &preparation.checksum_url)?;
    let expected_sha256 = checksum_for_asset(&checksums, &preparation.asset_name)
        .ok_or_else(|| format!("o checksum de {} não foi publicado", preparation.asset_name))?;
    let candidate_path = preparation.stage_directory.join("LocalCodePilot.exe");
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
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("o download da atualização foi interrompido: {error}"))?;
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
        let _ = sender.send(UpdateInstallEvent::Progress {
            downloaded_bytes,
            total_bytes,
        });
        repaint.request_repaint();
    }
    output
        .sync_all()
        .map_err(|error| format!("não foi possível concluir o download: {error}"))?;
    drop(output);
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

fn apply_prepared_update(arguments: &HelperArguments) -> Result<(), String> {
    if let Err(error) = validate_helper_arguments(arguments) {
        if is_packaged_executable_name(&arguments.target_path) {
            relaunch_after_failure(&arguments.target_path);
        }
        return Err(error);
    }
    let candidate_sha256 = match sha256_file(&arguments.candidate_path) {
        Ok(hash) => hash,
        Err(error) => {
            relaunch_after_failure(&arguments.target_path);
            return Err(error);
        }
    };
    if !candidate_sha256.eq_ignore_ascii_case(&arguments.expected_sha256) {
        relaunch_after_failure(&arguments.target_path);
        return Err("a atualização preparada falhou na segunda verificação SHA-256".into());
    }
    let marker_path = arguments.stage_directory.join("startup-ok");
    let helper_path = match env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            relaunch_after_failure(&arguments.target_path);
            return Err(format!(
                "não foi possível identificar o atualizador: {error}"
            ));
        }
    };
    let install_directory = match arguments.target_path.parent() {
        Some(path) => path,
        None => {
            relaunch_after_failure(&arguments.target_path);
            return Err("pasta de instalação inválida".into());
        }
    };
    let backup_path = arguments
        .target_path
        .with_file_name("LocalCodePilot.update-backup.exe");
    let _ = fs::remove_file(&backup_path);
    if let Err(error) = replace_target_with_backup(
        &arguments.target_path,
        &arguments.candidate_path,
        &backup_path,
        &arguments.expected_sha256,
    ) {
        relaunch_after_failure(&arguments.target_path);
        return Err(error);
    }
    let mut command = Command::new(&arguments.target_path);
    command
        .arg(UPDATED_STARTUP_FLAG)
        .arg(&marker_path)
        .arg(&helper_path)
        .arg(&arguments.stage_directory)
        .arg(&arguments.version)
        .current_dir(install_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            restore_backup(&arguments.target_path, &backup_path);
            relaunch_after_failure(&arguments.target_path);
            return Err(format!("não foi possível reiniciar a nova versão: {error}"));
        }
    };
    let started = std::time::Instant::now();
    while started.elapsed() < STARTUP_TIMEOUT {
        if marker_path.is_file() {
            let _ = fs::remove_file(&backup_path);
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                restore_backup(&arguments.target_path, &backup_path);
                relaunch_after_failure(&arguments.target_path);
                return Err(format!(
                    "a nova versão encerrou durante a inicialização ({status}); a versão anterior foi restaurada"
                ));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(format!("não foi possível confirmar a nova versão: {error}"));
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("a nova versão iniciou, mas não confirmou a atualização; o backup foi preservado".into())
}

fn replace_target_with_backup(
    target: &Path,
    candidate: &Path,
    backup: &Path,
    expected_sha256: &str,
) -> Result<(), String> {
    wait_and_backup_target(target, backup)?;
    if let Err(error) = fs::copy(candidate, target) {
        restore_backup(target, backup);
        return Err(format!("não foi possível instalar a nova versão: {error}"));
    }
    let installed_sha256 = match sha256_file(target) {
        Ok(hash) => hash,
        Err(error) => {
            restore_backup(target, backup);
            return Err(error);
        }
    };
    if !installed_sha256.eq_ignore_ascii_case(expected_sha256) {
        restore_backup(target, backup);
        return Err("o executável instalado falhou na verificação SHA-256".into());
    }
    Ok(())
}

fn parse_helper_arguments() -> Result<Option<HelperArguments>, String> {
    let mut arguments = env::args_os().skip(1);
    if arguments.next().as_deref() != Some(UPDATE_HELPER_FLAG.as_ref()) {
        return Ok(None);
    }
    let values: Vec<OsString> = arguments.collect();
    if values.len() != 5 {
        return Err("os argumentos internos do atualizador são inválidos".into());
    }
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

fn wait_and_backup_target(target: &Path, backup: &Path) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        match fs::rename(target, backup) {
            Ok(()) => return Ok(()),
            Err(error) if started.elapsed() < REPLACEMENT_TIMEOUT => {
                thread::sleep(Duration::from_millis(100));
                if !target.exists() {
                    return Err("o executável instalado desapareceu durante a atualização".into());
                }
                let _ = error;
            }
            Err(error) => {
                return Err(format!(
                    "não foi possível substituir o executável; confira a permissão da pasta: {error}"
                ));
            }
        }
    }
}

fn restore_backup(target: &Path, backup: &Path) {
    let _ = fs::remove_file(target);
    let _ = fs::rename(backup, target);
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

    #[test]
    fn replaces_the_target_and_keeps_a_backup_until_startup_confirmation() {
        let directory = temporary_test_directory("replace");
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("LocalCodePilot.exe");
        let candidate = directory.join("candidate.exe");
        let backup = directory.join("backup.exe");
        fs::write(&target, b"MZ old version").unwrap();
        fs::write(&candidate, b"MZ new version").unwrap();
        let expected = sha256_file(&candidate).unwrap();

        replace_target_with_backup(&target, &candidate, &backup, &expected).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"MZ new version");
        assert_eq!(fs::read(&backup).unwrap(), b"MZ old version");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn restores_the_previous_target_when_post_copy_verification_fails() {
        let directory = temporary_test_directory("rollback");
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("LocalCodePilot.exe");
        let candidate = directory.join("candidate.exe");
        let backup = directory.join("backup.exe");
        fs::write(&target, b"MZ old version").unwrap();
        fs::write(&candidate, b"MZ corrupted version").unwrap();

        let error = replace_target_with_backup(
            &target,
            &candidate,
            &backup,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap_err();

        assert!(error.contains("SHA-256"));
        assert_eq!(fs::read(&target).unwrap(), b"MZ old version");
        assert!(!backup.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
