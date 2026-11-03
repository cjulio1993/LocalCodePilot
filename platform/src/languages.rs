//! Which languages this computer has: the program of each language in `PATH` and in the
//! folders its installer uses, with the version it answers.

use crate::installations::{VERSION_TIMEOUT, VersionAnswer, find_executable, read_version};
use localcodepilot_core::{
    languages::{InstallDir, LANGUAGES, Language},
    runtimes::RuntimeKind,
};
use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageStatus {
    Found {
        program: &'static str,
        path: PathBuf,
        /// Found only in an installer folder: a terminal may not find it.
        outside_path: bool,
        version: VersionAnswer,
    },
    /// Only the Microsoft Store shortcut answered, without a version: the language is not
    /// installed behind it.
    StoreShortcut {
        path: PathBuf,
    },
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageCheck {
    pub kind: RuntimeKind,
    pub status: LanguageStatus,
    /// Where the app looked, for "Detalhes técnicos".
    pub searched: Vec<String>,
}

/// Checks every language of the table in parallel. Each version answer has its own time
/// limit, so a slow program does not hold the others.
pub fn check_languages(path: Option<&OsStr>) -> Vec<LanguageCheck> {
    std::thread::scope(|scope| {
        LANGUAGES
            .iter()
            .map(|language| scope.spawn(move || check_language(language, path)))
            .collect::<Vec<_>>()
            .into_iter()
            .zip(LANGUAGES)
            .map(|(check, language)| {
                check.join().unwrap_or_else(|_| LanguageCheck {
                    kind: language.kind,
                    status: LanguageStatus::NotFound,
                    searched: vec![],
                })
            })
            .collect()
    })
}

pub fn check_language(language: &Language, path: Option<&OsStr>) -> LanguageCheck {
    let mut searched = vec!["PATH".to_owned()];
    let mut shortcut = None;
    for program in language.programs {
        let Some(executable) = find_executable(program.name, path) else {
            continue;
        };
        let version = read_version(&executable, program.version_args, path, VERSION_TIMEOUT);
        if is_store_shortcut(&executable) && !matches!(version, VersionAnswer::Version(_)) {
            shortcut.get_or_insert(executable);
            continue;
        }
        return LanguageCheck {
            kind: language.kind,
            status: LanguageStatus::Found {
                program: program.name,
                path: executable,
                outside_path: false,
                version,
            },
            searched,
        };
    }
    for folder in language.install_dirs.iter().flat_map(expand_install_dir) {
        searched.push(folder.display().to_string());
        for program in language.programs {
            let Some(executable) = find_executable(program.name, Some(folder.as_os_str())) else {
                continue;
            };
            let version = read_version(&executable, program.version_args, path, VERSION_TIMEOUT);
            return LanguageCheck {
                kind: language.kind,
                status: LanguageStatus::Found {
                    program: program.name,
                    path: executable,
                    outside_path: true,
                    version,
                },
                searched,
            };
        }
    }
    LanguageCheck {
        kind: language.kind,
        status: shortcut.map_or(LanguageStatus::NotFound, |path| {
            LanguageStatus::StoreShortcut { path }
        }),
        searched,
    }
}

/// `python.exe` in `WindowsApps` is an App Execution Alias: without Python installed from
/// the Store it only points to the Store.
fn is_store_shortcut(executable: &Path) -> bool {
    executable
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('/', "\\")
        .contains("\\microsoft\\windowsapps\\")
}

/// The standard install folders of every language that exist on this computer. They go at
/// the end of the PATH of the processes the app starts, so a language installed without
/// adding itself to the PATH (Elixir from winget, for example) still runs.
pub fn existing_install_dirs() -> Vec<PathBuf> {
    localcodepilot_core::languages::LANGUAGES
        .iter()
        .flat_map(|language| language.install_dirs)
        .flat_map(expand_install_dir)
        .filter(|folder| folder.is_dir())
        .collect()
}

/// Existing folders an install directory names, expanding one `*` segment.
fn expand_install_dir(dir: &InstallDir) -> Vec<PathBuf> {
    let (base, relative) = match dir {
        InstallDir::Home(relative) => (
            env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from),
            *relative,
        ),
        InstallDir::LocalAppData(relative) => {
            (env::var_os("LOCALAPPDATA").map(PathBuf::from), *relative)
        }
        InstallDir::ProgramFiles(relative) => {
            (env::var_os("ProgramFiles").map(PathBuf::from), *relative)
        }
        InstallDir::Absolute(path) => {
            // Drive letters only mean something on Windows.
            if !cfg!(windows) && path.as_bytes().get(1) == Some(&b':') {
                return vec![];
            }
            // The root of `/usr/local/go/bin`; a drive letter becomes the first segment.
            let root = if path.starts_with('/') {
                PathBuf::from("/")
            } else {
                PathBuf::new()
            };
            (Some(root), *path)
        }
    };
    let Some(base) = base else {
        return vec![];
    };
    let mut folders = vec![base];
    for segment in relative.split('/').filter(|segment| !segment.is_empty()) {
        folders = folders
            .into_iter()
            .flat_map(|folder| match segment.strip_suffix('*') {
                Some(prefix) => matching_children(&folder, prefix),
                None => vec![if folder.as_os_str().is_empty() {
                    PathBuf::from(format!("{segment}/"))
                } else {
                    folder.join(segment)
                }],
            })
            .collect();
    }
    folders
        .into_iter()
        .filter(|folder| folder.is_dir())
        .collect()
}

fn matching_children(folder: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        return vec![];
    };
    let mut children = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|child| {
            child
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.to_ascii_lowercase()
                        .starts_with(&prefix.to_ascii_lowercase())
                })
        })
        .collect::<Vec<_>>();
    // Newest version first, such as Python313 before Python311.
    children.sort();
    children.reverse();
    children
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn store_shortcuts_are_recognized_by_their_folder() {
        assert!(is_store_shortcut(Path::new(
            r"C:\Users\Ana\AppData\Local\Microsoft\WindowsApps\python.exe"
        )));
        assert!(!is_store_shortcut(Path::new(
            r"C:\Users\Ana\AppData\Local\Programs\Python\Python312\python.exe"
        )));
    }

    #[test]
    fn install_dirs_expand_one_wildcard_segment_newest_first() {
        let root = env::temp_dir().join(format!(
            "lcp-install-dirs-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for folder in ["Python311", "Python313", "Other"] {
            fs::create_dir_all(root.join("Programs/Python").join(folder)).unwrap();
        }
        let base = root.to_string_lossy().replace('\\', "/");
        let pattern: &'static str =
            Box::leak(format!("{base}/Programs/Python/Python3*").into_boxed_str());
        let folders = expand_install_dir(&InstallDir::Absolute(pattern));
        assert_eq!(
            folders
                .iter()
                .map(|folder| folder.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["Python313", "Python311"]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_language_says_where_it_was_looked_for() {
        let language = Language {
            kind: RuntimeKind::Zig,
            name: "Zig",
            manifests: &[],
            manifest_extensions: &[],
            source_extensions: &[],
            programs: &[localcodepilot_core::languages::Program {
                name: "lcp-program-that-does-not-exist",
                version_args: &["--version"],
            }],
            images: &[],
            install_dirs: &[],
            starts: false,
        };
        let check = check_language(&language, Some(OsStr::new("")));
        assert_eq!(check.status, LanguageStatus::NotFound);
        assert_eq!(check.searched, ["PATH"]);
    }
}
