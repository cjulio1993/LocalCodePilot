//! Pieces of the in-app update that must survive a crash or a slow start:
//! the update journal, the executable swap, the wait for processes and the
//! copy of the user's configuration taken before the update.

use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, ExitStatus},
    thread,
    time::{Duration, Instant},
};

/// Previous executable, kept until the new version confirms its startup.
pub(super) const BACKUP_FILE_NAME: &str = "LocalCodePilot.update-backup.exe";
/// New executable copied next to the target before the swap.
pub(super) const NEW_FILE_NAME: &str = "LocalCodePilot.update-new.exe";
const JOURNAL_FILE_NAME: &str = "update-state.json";
const CONFIG_BACKUPS_DIRECTORY: &str = "backups";
const CONFIG_BACKUP_PREFIX: &str = "antes-de-";
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum JournalPhase {
    Swapping,
    Starting,
    PendingConfirmation,
}

/// Written by the helper before it touches the installation and removed once
/// the outcome is known. Any later startup uses it to finish or undo the work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct UpdateJournal {
    pub from_version: String,
    pub to_version: String,
    pub target: PathBuf,
    pub phase: JournalPhase,
    #[serde(default)]
    pub config_backup: Option<PathBuf>,
}

pub(super) fn journal_path(config_directory: &Path) -> PathBuf {
    config_directory.join(JOURNAL_FILE_NAME)
}

pub(super) fn write_journal(path: &Path, journal: &UpdateJournal) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("json.part");
    fs::write(
        &partial,
        serde_json::to_vec_pretty(journal).map_err(io::Error::other)?,
    )?;
    fs::rename(partial, path)
}

pub(super) fn read_journal(path: &Path) -> Option<UpdateJournal> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Recovery {
    /// The journal belongs to another copy of the app: leave it alone.
    NotThisInstallation,
    /// The running executable is the new version: the update is done.
    Completed { version: String },
    /// The running executable is still the previous version.
    NotCompleted { from: String, to: String },
    /// Neither version matches: only the journal is discarded.
    Unknown,
}

pub(super) fn decide_recovery(
    journal: &UpdateJournal,
    running_executable: &Path,
    running_version: &Version,
) -> Recovery {
    if !same_file_path(&journal.target, running_executable) {
        return Recovery::NotThisInstallation;
    }
    let parse = |value: &str| Version::parse(value.trim_start_matches(['v', 'V'])).ok();
    if parse(&journal.to_version).as_ref() == Some(running_version) {
        Recovery::Completed {
            version: journal.to_version.clone(),
        }
    } else if parse(&journal.from_version).as_ref() == Some(running_version) {
        Recovery::NotCompleted {
            from: journal.from_version.clone(),
            to: journal.to_version.clone(),
        }
    } else {
        Recovery::Unknown
    }
}

/// Removes the updater's own leftovers next to the running executable. Only
/// the two fixed names are touched, never anything taken from the journal.
pub(super) fn remove_swap_leftovers(running_executable: &Path) {
    if let Some(directory) = running_executable.parent() {
        for name in [BACKUP_FILE_NAME, NEW_FILE_NAME] {
            let _ = remove_file_with_retry(&directory.join(name), Duration::from_secs(5));
        }
    }
}

fn same_file_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// Waits for a process to end. A zombie (ended but not yet reaped) counts as
/// ended. Returns `false` when it is still running after `timeout`.
pub(super) fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = sysinfo::System::new();
    let started = Instant::now();
    loop {
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        let running = system
            .process(pid)
            .is_some_and(|process| process.status() != sysinfo::ProcessStatus::Zombie);
        if !running {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[derive(Debug)]
pub(super) enum StartupOutcome {
    Confirmed,
    Exited(ExitStatus),
    /// Still running without confirmation when the time ran out: the new
    /// version may just be slow (an antivirus scan, for example).
    StillRunning,
}

pub(super) fn wait_for_startup(
    child: &mut Child,
    marker: &Path,
    timeout: Duration,
) -> StartupOutcome {
    let started = Instant::now();
    loop {
        if marker.is_file() {
            return StartupOutcome::Confirmed;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // The marker may have been written right before a clean exit.
                if marker.is_file() {
                    return StartupOutcome::Confirmed;
                }
                return StartupOutcome::Exited(status);
            }
            Ok(None) => {}
            Err(_) => return StartupOutcome::StillRunning,
        }
        if started.elapsed() >= timeout {
            return StartupOutcome::StillRunning;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

pub(super) fn rename_with_retry(from: &Path, to: &Path, timeout: Duration) -> io::Result<()> {
    let started = Instant::now();
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(error),
            Err(error) if started.elapsed() >= timeout => return Err(error),
            Err(_) => thread::sleep(POLL_INTERVAL),
        }
    }
}

fn remove_file_with_retry(path: &Path, timeout: Duration) -> io::Result<()> {
    let started = Instant::now();
    loop {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) if started.elapsed() >= timeout => return Err(error),
            Err(_) => thread::sleep(POLL_INTERVAL),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SwapError {
    /// Nothing changed in the installation.
    NotApplied(String),
    /// The swap failed and the previous executable is back in place.
    RolledBack(String),
    /// The swap failed and the previous executable could not be put back.
    Broken(String),
}

/// Puts `candidate` in place of `target` without a window in which `target`
/// is missing for longer than two renames: the new file is copied and
/// verified next to `target` first. The previous executable stays in
/// `BACKUP_FILE_NAME` until the new version confirms its startup.
pub(super) fn swap_in_candidate(
    target: &Path,
    candidate: &Path,
    expected_sha256: &str,
    sha256_file: impl Fn(&Path) -> Result<String, String>,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), SwapError> {
    let directory = target
        .parent()
        .ok_or_else(|| SwapError::NotApplied("pasta de instalação inválida".into()))?;
    let new_path = directory.join(NEW_FILE_NAME);
    let backup = directory.join(BACKUP_FILE_NAME);
    let _ = fs::remove_file(&new_path);
    let _ = fs::remove_file(&backup);
    if let Err(error) = fs::copy(candidate, &new_path) {
        let _ = fs::remove_file(&new_path);
        return Err(SwapError::NotApplied(format!(
            "não foi possível copiar a nova versão para a pasta do app: {error}"
        )));
    }
    match sha256_file(&new_path) {
        Ok(hash) if hash.eq_ignore_ascii_case(expected_sha256) => {}
        Ok(_) => {
            let _ = fs::remove_file(&new_path);
            return Err(SwapError::NotApplied(
                "a cópia da nova versão falhou na verificação SHA-256".into(),
            ));
        }
        Err(error) => {
            let _ = fs::remove_file(&new_path);
            return Err(SwapError::NotApplied(error));
        }
    }
    if let Err(error) = rename(target, &backup) {
        let _ = fs::remove_file(&new_path);
        return Err(SwapError::NotApplied(format!(
            "não foi possível substituir o executável; confira a permissão da pasta: {error}"
        )));
    }
    if let Err(error) = rename(&new_path, target) {
        let _ = fs::remove_file(&new_path);
        return match rename(&backup, target) {
            Ok(()) => Err(SwapError::RolledBack(format!(
                "não foi possível colocar a nova versão no lugar: {error}"
            ))),
            Err(restore_error) => Err(SwapError::Broken(format!(
                "não foi possível colocar a nova versão no lugar ({error}) nem restaurar a anterior ({restore_error})"
            ))),
        };
    }
    Ok(())
}

/// Puts the previous executable back after the new one failed to start.
/// Returns whether the previous executable is really in place again.
pub(super) fn restore_previous_executable(target: &Path, timeout: Duration) -> bool {
    let Some(directory) = target.parent() else {
        return false;
    };
    let backup = directory.join(BACKUP_FILE_NAME);
    if !backup.is_file() {
        return false;
    }
    if remove_file_with_retry(target, timeout).is_err() {
        return false;
    }
    rename_with_retry(&backup, target, timeout).is_ok()
}

/// Copies the top-level configuration files (not the updater's own files) to
/// `backups/antes-de-<version>` and keeps only that copy.
pub(super) fn backup_configuration(config_directory: &Path, version: &str) -> io::Result<PathBuf> {
    let safe_version: String = version
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
        .collect();
    let backups = config_directory.join(CONFIG_BACKUPS_DIRECTORY);
    let destination = backups.join(format!("{CONFIG_BACKUP_PREFIX}{safe_version}"));
    let _ = fs::remove_dir_all(&destination);
    fs::create_dir_all(&destination)?;
    if let Ok(entries) = fs::read_dir(config_directory) {
        for entry in entries.flatten() {
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            let name = entry.file_name();
            let is_updater_file = name.to_string_lossy().starts_with("update-");
            if metadata.is_file() && !is_updater_file {
                fs::copy(entry.path(), destination.join(&name))?;
            }
        }
    }
    if let Ok(entries) = fs::read_dir(&backups) {
        for entry in entries.flatten() {
            let older_copy = entry
                .file_name()
                .to_string_lossy()
                .starts_with(CONFIG_BACKUP_PREFIX)
                && entry.path() != destination;
            if older_copy && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(destination)
}

/// Copies a configuration backup back into the configuration directory.
pub(super) fn restore_configuration(config_directory: &Path, backup: &Path) -> io::Result<()> {
    let backups = config_directory.join(CONFIG_BACKUPS_DIRECTORY);
    let inside_backups = backup.parent() == Some(backups.as_path())
        && backup
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(CONFIG_BACKUP_PREFIX));
    if !inside_backups {
        return Err(io::Error::other(
            "cópia de configurações fora da pasta esperada",
        ));
    }
    for entry in fs::read_dir(backup)?.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            fs::copy(entry.path(), config_directory.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, process::Command, time::SystemTime};

    fn temporary_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "localcodepilot-recovery-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn fake_sha(path: &Path) -> Result<String, String> {
        fs::read(path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|error| error.to_string())
    }

    /// A process that runs for about `seconds` seconds on every platform.
    fn sleeper(seconds: u32) -> Child {
        if cfg!(windows) {
            Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!("Start-Sleep -Seconds {seconds}"),
                ])
                .spawn()
                .unwrap()
        } else {
            Command::new("sleep")
                .arg(seconds.to_string())
                .spawn()
                .unwrap()
        }
    }

    fn quick_exit() -> Child {
        if cfg!(windows) {
            Command::new("cmd").args(["/C", "exit 3"]).spawn().unwrap()
        } else {
            Command::new("sh").args(["-c", "exit 3"]).spawn().unwrap()
        }
    }

    fn journal(target: &Path) -> UpdateJournal {
        UpdateJournal {
            from_version: "0.4.0-alpha.8".into(),
            to_version: "0.4.0-alpha.9".into(),
            target: target.to_path_buf(),
            phase: JournalPhase::Starting,
            config_backup: None,
        }
    }

    #[test]
    fn swap_keeps_the_previous_executable_as_backup() {
        let directory = temporary_directory("swap");
        let target = directory.join("LocalCodePilot.exe");
        let candidate = directory.join("candidate.exe");
        fs::write(&target, "old").unwrap();
        fs::write(&candidate, "new").unwrap();

        swap_in_candidate(&target, &candidate, "new", fake_sha, |from, to| {
            fs::rename(from, to)
        })
        .unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(
            fs::read_to_string(directory.join(BACKUP_FILE_NAME)).unwrap(),
            "old"
        );
        assert!(!directory.join(NEW_FILE_NAME).exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn swap_does_not_touch_the_target_when_the_copy_fails_verification() {
        let directory = temporary_directory("swap-sha");
        let target = directory.join("LocalCodePilot.exe");
        let candidate = directory.join("candidate.exe");
        fs::write(&target, "old").unwrap();
        fs::write(&candidate, "corrupted").unwrap();

        let error = swap_in_candidate(&target, &candidate, "new", fake_sha, |from, to| {
            fs::rename(from, to)
        })
        .unwrap_err();

        assert!(matches!(error, SwapError::NotApplied(message) if message.contains("SHA-256")));
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        assert!(!directory.join(NEW_FILE_NAME).exists());
        assert!(!directory.join(BACKUP_FILE_NAME).exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn swap_puts_the_previous_executable_back_when_the_second_rename_fails() {
        let directory = temporary_directory("swap-rollback");
        let target = directory.join("LocalCodePilot.exe");
        let candidate = directory.join("candidate.exe");
        fs::write(&target, "old").unwrap();
        fs::write(&candidate, "new").unwrap();
        let new_path = directory.join(NEW_FILE_NAME);

        let error = swap_in_candidate(&target, &candidate, "new", fake_sha, |from, to| {
            if from == new_path {
                Err(io::Error::other("locked"))
            } else {
                fs::rename(from, to)
            }
        })
        .unwrap_err();

        assert!(matches!(error, SwapError::RolledBack(_)));
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        assert!(!new_path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn swap_reports_a_broken_installation_honestly() {
        let directory = temporary_directory("swap-broken");
        let target = directory.join("LocalCodePilot.exe");
        let candidate = directory.join("candidate.exe");
        fs::write(&target, "old").unwrap();
        fs::write(&candidate, "new").unwrap();
        let backup = directory.join(BACKUP_FILE_NAME);

        let error = swap_in_candidate(&target, &candidate, "new", fake_sha, |from, to| {
            if from == target {
                fs::rename(from, to)
            } else {
                Err(io::Error::other("locked"))
            }
        })
        .unwrap_err();

        assert!(matches!(error, SwapError::Broken(_)));
        assert_eq!(fs::read_to_string(&backup).unwrap(), "old");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn restoring_reports_whether_the_previous_executable_is_back() {
        let directory = temporary_directory("restore");
        let target = directory.join("LocalCodePilot.exe");
        fs::write(&target, "new").unwrap();
        assert!(!restore_previous_executable(
            &target,
            Duration::from_millis(200)
        ));
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");

        fs::write(directory.join(BACKUP_FILE_NAME), "old").unwrap();
        assert!(restore_previous_executable(
            &target,
            Duration::from_millis(200)
        ));
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        assert!(!directory.join(BACKUP_FILE_NAME).exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn waits_for_a_process_to_exit_and_gives_up_on_one_that_keeps_running() {
        let mut short = quick_exit();
        assert!(wait_for_process_exit(short.id(), Duration::from_secs(10)));
        let _ = short.wait();

        let mut long = sleeper(20);
        let started = Instant::now();
        assert!(!wait_for_process_exit(
            long.id(),
            Duration::from_millis(500)
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = long.kill();
        let _ = long.wait();
    }

    #[test]
    fn startup_wait_tells_confirmed_exited_and_still_running_apart() {
        let directory = temporary_directory("startup");
        let marker = directory.join("startup-ok");

        let mut exited = quick_exit();
        assert!(matches!(
            wait_for_startup(&mut exited, &marker, Duration::from_secs(10)),
            StartupOutcome::Exited(status) if !status.success()
        ));

        let mut slow = sleeper(20);
        assert!(matches!(
            wait_for_startup(&mut slow, &marker, Duration::from_millis(300)),
            StartupOutcome::StillRunning
        ));
        fs::write(&marker, b"ok").unwrap();
        assert!(matches!(
            wait_for_startup(&mut slow, &marker, Duration::from_secs(10)),
            StartupOutcome::Confirmed
        ));
        let _ = slow.kill();
        let _ = slow.wait();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recovery_depends_on_which_version_is_running_from_the_target() {
        let directory = temporary_directory("recovery");
        let target = directory.join("LocalCodePilot.exe");
        fs::write(&target, "exe").unwrap();
        let journal = journal(&target);

        assert_eq!(
            decide_recovery(&journal, &target, &Version::parse("0.4.0-alpha.9").unwrap()),
            Recovery::Completed {
                version: "0.4.0-alpha.9".into()
            }
        );
        assert_eq!(
            decide_recovery(&journal, &target, &Version::parse("0.4.0-alpha.8").unwrap()),
            Recovery::NotCompleted {
                from: "0.4.0-alpha.8".into(),
                to: "0.4.0-alpha.9".into()
            }
        );
        assert_eq!(
            decide_recovery(&journal, &target, &Version::parse("0.3.0").unwrap()),
            Recovery::Unknown
        );
        let other_copy = directory.join("other").join("LocalCodePilot.exe");
        assert_eq!(
            decide_recovery(
                &journal,
                &other_copy,
                &Version::parse("0.4.0-alpha.9").unwrap()
            ),
            Recovery::NotThisInstallation
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn journal_round_trips_and_leftover_cleanup_only_touches_fixed_names() {
        let directory = temporary_directory("journal");
        let target = directory.join("LocalCodePilot.exe");
        let path = journal_path(&directory);
        write_journal(&path, &journal(&target)).unwrap();
        assert_eq!(read_journal(&path), Some(journal(&target)));

        for name in [
            BACKUP_FILE_NAME,
            NEW_FILE_NAME,
            "LocalCodePilot.exe",
            "notes.txt",
        ] {
            fs::write(directory.join(name), "x").unwrap();
        }
        remove_swap_leftovers(&target);
        assert!(!directory.join(BACKUP_FILE_NAME).exists());
        assert!(!directory.join(NEW_FILE_NAME).exists());
        assert!(target.is_file());
        assert!(directory.join("notes.txt").is_file());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn configuration_backup_keeps_only_the_latest_copy_and_restores_it() {
        let config = temporary_directory("config");
        fs::write(config.join("workspace-state.json"), "before").unwrap();
        fs::write(config.join("scan-roots.txt"), "C:\\projects").unwrap();
        fs::write(config.join("update-state.json"), "journal").unwrap();

        let first = backup_configuration(&config, "0.4.0-alpha.9").unwrap();
        let second = backup_configuration(&config, "0.4.0-alpha.10").unwrap();
        assert!(!first.exists());
        assert_eq!(
            fs::read_to_string(second.join("workspace-state.json")).unwrap(),
            "before"
        );
        assert!(second.join("scan-roots.txt").is_file());
        assert!(!second.join("update-state.json").exists());

        fs::write(
            config.join("workspace-state.json"),
            "written by the new version",
        )
        .unwrap();
        restore_configuration(&config, &second).unwrap();
        assert_eq!(
            fs::read_to_string(config.join("workspace-state.json")).unwrap(),
            "before"
        );

        let outside = temporary_directory("config-outside");
        assert!(restore_configuration(&config, &outside).is_err());
        fs::remove_dir_all(config).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn a_configuration_file_that_cannot_be_written_back_is_reported() {
        let config = temporary_directory("config-fail");
        fs::write(config.join("workspace-state.json"), "before").unwrap();
        let backup = backup_configuration(&config, "0.4.0-alpha.9").unwrap();
        // A folder where the file should go makes the copy back fail.
        fs::remove_file(config.join("workspace-state.json")).unwrap();
        fs::create_dir(config.join("workspace-state.json")).unwrap();

        assert!(restore_configuration(&config, &backup).is_err());
        fs::remove_dir_all(config).unwrap();
    }
}
