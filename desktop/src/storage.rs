//! Safe persistence of the configuration files (T021).
//!
//! Writing goes through a temporary file that replaces the current one with a rename, after
//! the current one, if readable, is kept as `<file>.bak`. A crash at any step leaves the
//! current file either old and valid or new and valid, and `.bak` with the previous valid
//! copy. A file that cannot be read is never written over: it is renamed to
//! `<file>.ilegivel-<date>`, and the app restores `.bak` when that copy is valid.

use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const BACKUP_SUFFIX: &str = ".bak";
const TEMPORARY_SUFFIX: &str = ".tmp";
const UNREADABLE_MARK: &str = ".ilegivel-";
/// Preserved files kept per configuration file, the most recent ones.
const PRESERVED_TO_KEEP: usize = 5;
/// Preserved files newer than this are never removed, whatever their number.
const PRESERVED_MIN_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const RENAME_ATTEMPTS: usize = 3;
const RENAME_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Files whose unreadable content could not be moved aside: writing to them is refused so the
/// only copy is not lost.
static PROTECTED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// What happened when a configuration file was read.
#[derive(Debug)]
pub enum Loaded<T> {
    /// There is no file yet (first run).
    Missing,
    Read(T),
    /// The file could not be read; it was preserved and the valid `.bak` was used.
    Restored(T, Recovery),
    /// The file could not be read and there was no valid copy; the files were preserved.
    Lost(Recovery),
}

impl<T> Loaded<T> {
    pub fn split(self) -> (Option<T>, Option<Recovery>) {
        match self {
            Self::Missing => (None, None),
            Self::Read(value) => (Some(value), None),
            Self::Restored(value, recovery) => (Some(value), Some(recovery)),
            Self::Lost(recovery) => (None, Some(recovery)),
        }
    }
}

/// The details of a recovery, without any of the file's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    pub file: PathBuf,
    /// Why the file could not be read (an I/O or parser message, never a part of the content).
    pub cause: String,
    /// Where the unreadable files were moved. Empty if they could not be moved.
    pub preserved: Vec<PathBuf>,
    /// The copy that was used, when there was a valid one.
    pub restored_from: Option<PathBuf>,
}

pub fn backup_path(path: &Path) -> PathBuf {
    with_suffix(path, BACKUP_SUFFIX)
}

fn temporary_path(path: &Path) -> PathBuf {
    with_suffix(path, TEMPORARY_SUFFIX)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

/// Reads `path`, recovering from `.bak` when it cannot be parsed. `parse` must only accept
/// content the app can use; its error message must not quote the content.
pub fn read<T>(path: &Path, parse: impl Fn(&[u8]) -> Result<T, String>) -> Loaded<T> {
    remove_leftover_temporaries(path);
    let cause = match fs::read(path) {
        Ok(bytes) => match parse(&bytes) {
            Ok(value) => return Loaded::Read(value),
            Err(error) => error,
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Loaded::Missing,
        Err(error) => error.to_string(),
    };
    let mut preserved = Vec::new();
    match preserve(path) {
        Some(moved) => preserved.push(moved),
        None => protect(path),
    }
    let backup = backup_path(path);
    let backup_result = fs::read(&backup).map(|bytes| parse(&bytes));
    if let Ok(Ok(value)) = backup_result {
        return Loaded::Restored(
            value,
            Recovery {
                file: path.to_path_buf(),
                cause,
                preserved,
                restored_from: Some(backup),
            },
        );
    }
    if backup.exists()
        && let Some(moved) = preserve(&backup)
    {
        preserved.push(moved);
    }
    Loaded::Lost(Recovery {
        file: path.to_path_buf(),
        cause,
        preserved,
        restored_from: None,
    })
}

/// Writes `contents` to `path` so that a failure at any step never leaves it partially
/// written, keeping the current valid file as `.bak`. `is_valid` tells whether the current
/// file is readable: an unreadable one is preserved instead of becoming the backup.
pub fn write(path: &Path, contents: &[u8], is_valid: impl Fn(&[u8]) -> bool) -> io::Result<()> {
    if is_protected(path) {
        return Err(io::Error::other(
            "o arquivo anterior não pôde ser guardado e não será substituído",
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(path);
    write_synced(&temporary, contents, Step::WriteTemporary)?;
    match fs::read(path) {
        Ok(current) if is_valid(&current) => {
            let backup = backup_path(path);
            let backup_temporary = temporary_path(&backup);
            write_synced(&backup_temporary, &current, Step::WriteBackup)?;
            rename(&backup_temporary, &backup, Step::RenameBackup)?;
        }
        Ok(_) => {
            // It became unreadable after it was loaded: keep it before replacing it.
            if preserve(path).is_none() {
                let _ = fs::remove_file(&temporary);
                return Err(io::Error::other(
                    "o arquivo atual está ilegível e não pôde ser guardado",
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    }
    rename(&temporary, path, Step::RenameCurrent)
}

fn write_synced(path: &Path, contents: &[u8], step: Step) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    fail_point(step)?;
    file.write_all(contents)?;
    file.sync_all()
}

fn rename(from: &Path, to: &Path, step: Step) -> io::Result<()> {
    fail_point(step)?;
    let mut attempt = 1;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            // An antivirus or indexer may hold the file for a moment on Windows.
            Err(_) if attempt < RENAME_ATTEMPTS => {
                attempt += 1;
                thread::sleep(RENAME_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
}

/// Moves an unreadable file aside, under a name nothing writes to. Returns the new path.
fn preserve(path: &Path) -> Option<PathBuf> {
    let stamp = timestamp(SystemTime::now());
    let base = with_suffix(path, &format!("{UNREADABLE_MARK}{stamp}"));
    let target = (1..100)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                with_suffix(&base, &format!("-{n}"))
            }
        })
        .find(|candidate| !candidate.exists())?;
    rename(path, &target, Step::Preserve).ok()?;
    prune_preserved(path, SystemTime::now());
    Some(target)
}

/// Keeps the [`PRESERVED_TO_KEEP`] most recent preserved copies of `path` and never removes
/// one newer than [`PRESERVED_MIN_AGE`].
fn prune_preserved(path: &Path, now: SystemTime) {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let prefix = format!("{}{UNREADABLE_MARK}", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut preserved = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect::<Vec<_>>();
    preserved.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (modified, old) in preserved.into_iter().skip(PRESERVED_TO_KEEP) {
        let age = now.duration_since(modified).unwrap_or_default();
        if age > PRESERVED_MIN_AGE {
            let _ = fs::remove_file(old);
        }
    }
}

/// A crash may leave temporary files behind; they never hold the only valid copy.
fn remove_leftover_temporaries(path: &Path) {
    let _ = fs::remove_file(temporary_path(path));
    let _ = fs::remove_file(temporary_path(&backup_path(path)));
}

fn protect(path: &Path) {
    if let Ok(mut protected) = PROTECTED.lock() {
        protected
            .get_or_insert_with(HashSet::new)
            .insert(path.to_path_buf());
    }
}

fn is_protected(path: &Path) -> bool {
    PROTECTED
        .lock()
        .map(|protected| protected.as_ref().is_some_and(|set| set.contains(path)))
        .unwrap_or(false)
}

/// `AAAAMMDD-HHMMSS` in UTC.
fn timestamp(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The steps of a write where tests inject a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    WriteTemporary,
    WriteBackup,
    RenameBackup,
    RenameCurrent,
    Preserve,
}

#[cfg(not(test))]
fn fail_point(_: Step) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static FAIL_AT: std::cell::Cell<Option<Step>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn fail_point(step: Step) -> io::Result<()> {
    if FAIL_AT.with(|fail| fail.get()) == Some(step) {
        return Err(io::Error::other(format!("falha simulada em {step:?}")));
    }
    Ok(())
}

/// Opens `folder` in the system file manager.
pub fn open_folder(folder: &Path) -> io::Result<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(folder)
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "lcp-storage-{name}-{}-{:?}",
                std::process::id(),
                thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("estado.json")
        }

        fn names(&self) -> Vec<String> {
            let mut names = fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            names.sort();
            names
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn parse(bytes: &[u8]) -> Result<String, String> {
        let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
        text.starts_with("ok:")
            .then(|| text.to_owned())
            .ok_or_else(|| "conteúdo inválido".to_owned())
    }

    fn valid(bytes: &[u8]) -> bool {
        parse(bytes).is_ok()
    }

    fn fail_at(step: Option<Step>) {
        FAIL_AT.with(|fail| fail.set(step));
    }

    #[test]
    fn writes_keep_the_previous_valid_file_as_backup() {
        let folder = Folder::new("backup");
        let file = folder.file();
        write(&file, b"ok:1", valid).unwrap();
        assert!(!backup_path(&file).exists());
        write(&file, b"ok:2", valid).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"ok:2");
        assert_eq!(fs::read(backup_path(&file)).unwrap(), b"ok:1");
        assert_eq!(folder.names(), ["estado.json", "estado.json.bak"]);
    }

    #[test]
    fn a_failure_at_any_step_keeps_a_valid_file_and_backup() {
        for step in [
            Step::WriteTemporary,
            Step::WriteBackup,
            Step::RenameBackup,
            Step::RenameCurrent,
        ] {
            let folder = Folder::new(&format!("falha-{step:?}"));
            let file = folder.file();
            write(&file, b"ok:1", valid).unwrap();
            write(&file, b"ok:2", valid).unwrap();
            fail_at(Some(step));
            assert!(write(&file, b"ok:3", valid).is_err(), "{step:?}");
            fail_at(None);
            // The current file is the old or the new valid one; the backup stays valid.
            let current = fs::read(&file).unwrap();
            assert!(current == b"ok:2" || current == b"ok:3", "{step:?}");
            assert!(valid(&fs::read(backup_path(&file)).unwrap()), "{step:?}");
            // Reopening reads a valid file and cleans the leftovers.
            assert!(matches!(read(&file, parse), Loaded::Read(_)), "{step:?}");
            assert!(!temporary_path(&file).exists(), "{step:?}");
            assert!(!temporary_path(&backup_path(&file)).exists(), "{step:?}");
        }
    }

    #[test]
    fn an_unreadable_current_file_never_becomes_the_backup() {
        let folder = Folder::new("nunca-bak");
        let file = folder.file();
        write(&file, b"ok:1", valid).unwrap();
        write(&file, b"ok:2", valid).unwrap();
        fs::write(&file, b"corrompido").unwrap();
        write(&file, b"ok:3", valid).unwrap();
        assert_eq!(fs::read(backup_path(&file)).unwrap(), b"ok:1");
        assert_eq!(fs::read(&file).unwrap(), b"ok:3");
        let preserved = folder
            .names()
            .into_iter()
            .find(|name| name.contains(UNREADABLE_MARK))
            .expect("the unreadable file is preserved");
        assert_eq!(fs::read(folder.0.join(preserved)).unwrap(), b"corrompido");
    }

    #[test]
    fn an_unreadable_file_is_preserved_and_the_backup_restored() {
        let folder = Folder::new("restaura");
        let file = folder.file();
        write(&file, b"ok:1", valid).unwrap();
        write(&file, b"ok:2", valid).unwrap();
        fs::write(&file, b"corrompido").unwrap();
        let Loaded::Restored(value, recovery) = read(&file, parse) else {
            panic!("the backup should be restored");
        };
        assert_eq!(value, "ok:1");
        assert_eq!(recovery.restored_from, Some(backup_path(&file)));
        assert_eq!(recovery.preserved.len(), 1);
        assert!(!recovery.cause.contains("corrompido"));
        let preserved = recovery.preserved[0].clone();
        // Later writes never touch the preserved file.
        write(&file, b"ok:4", valid).unwrap();
        write(&file, b"ok:5", valid).unwrap();
        assert_eq!(fs::read(&preserved).unwrap(), b"corrompido");
    }

    #[test]
    fn without_a_valid_backup_every_unreadable_file_is_preserved() {
        for backup in [None, Some(b"tambem corrompido".as_slice())] {
            let folder = Folder::new(&format!("perdido-{}", backup.is_some()));
            let file = folder.file();
            fs::write(&file, b"corrompido").unwrap();
            if let Some(contents) = backup {
                fs::write(backup_path(&file), contents).unwrap();
            }
            let Loaded::Lost(recovery) = read(&file, parse) else {
                panic!("nothing valid to restore");
            };
            assert_eq!(recovery.preserved.len(), 1 + usize::from(backup.is_some()));
            assert_eq!(recovery.restored_from, None);
            write(&file, b"ok:novo", valid).unwrap();
            for path in &recovery.preserved {
                assert!(!valid(&fs::read(path).unwrap()));
            }
        }
    }

    #[test]
    fn a_missing_file_is_a_first_run() {
        let folder = Folder::new("ausente");
        assert!(matches!(read(&folder.file(), parse), Loaded::Missing));
    }

    #[test]
    fn a_file_that_cannot_be_moved_aside_is_never_replaced() {
        let folder = Folder::new("protegido");
        let file = folder.file();
        fs::write(&file, b"corrompido").unwrap();
        fail_at(Some(Step::Preserve));
        let Loaded::Lost(recovery) = read(&file, parse) else {
            panic!("nothing valid to restore");
        };
        fail_at(None);
        assert!(recovery.preserved.is_empty());
        assert!(write(&file, b"ok:1", valid).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"corrompido");
    }

    #[test]
    fn keeps_the_five_most_recent_and_anything_newer_than_thirty_days() {
        let folder = Folder::new("retencao");
        let file = folder.file();
        let now = SystemTime::now();
        let day = Duration::from_secs(24 * 60 * 60);
        for (index, age_days) in [1u64, 2, 3, 4, 5, 6, 40, 50].iter().enumerate() {
            let path = with_suffix(&file, &format!("{UNREADABLE_MARK}{index}"));
            fs::write(&path, b"x").unwrap();
            let modified = now - day * (*age_days as u32);
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(modified)
                .unwrap();
        }
        prune_preserved(&file, now);
        let left = folder.names();
        // The 6-day-old one is the sixth but too recent; the 40- and 50-day-old ones go.
        assert_eq!(left.len(), 6, "{left:?}");
        assert!(
            !left
                .iter()
                .any(|name| name.ends_with("-6") || name.ends_with("-7"))
        );
    }

    #[test]
    fn formats_the_preserved_file_date_in_utc() {
        let time = UNIX_EPOCH + Duration::from_secs(1_795_564_800 + 3_661);
        assert_eq!(timestamp(time), "20261125-010101");
        assert_eq!(timestamp(UNIX_EPOCH), "19700101-000000");
    }
}
