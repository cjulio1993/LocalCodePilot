use localcodepilot_core::{discovery::ProjectSource, languages, runtimes::RuntimeKind};
use std::{
    collections::{HashSet, VecDeque},
    fs, io,
    path::{Path, PathBuf},
};

/// Markers besides the language manifests of `languages::LANGUAGES`.
const PROJECT_MARKERS: &[&str] = &[
    "Dockerfile",
    "Containerfile",
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
    "pubspec.yaml",
    "index.html",
];
const IGNORED_DIRECTORIES: &[&str] = &[
    ".git",
    ".idea",
    ".vscode",
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    ".venv",
    "venv",
    "sodium_compat",
    "PHPMailer-master",
    "_macosx",
    "__macosx",
    // Elixir: dependencies and build output of Mix.
    "deps",
    "_build",
];

#[derive(Debug, Clone)]
pub struct FilesystemProjectSource {
    roots: Vec<PathBuf>,
    /// Folders of installed programs: the search never goes down into them, though a person
    /// may still add one as a folder of their own.
    excluded: Vec<PathBuf>,
    max_depth: usize,
}

impl FilesystemProjectSource {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots: unique_existing(roots),
            excluded: comparable_all(installation_directories()),
            max_depth: 8,
        }
    }

    /// The common folders of the person's profile. They never depend on the folder the app
    /// was started from: opened from the installer, that is the installation folder, and the
    /// one above it holds every other installed program (#98).
    pub fn common_locations() -> Self {
        Self::new(default_roots(home_directory().as_deref()))
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// `canonical` is already canonical, so this runs once per folder without touching disk.
    fn is_excluded(&self, canonical: &Path) -> bool {
        let key = comparison_key(canonical);
        self.excluded.contains(&key)
    }
}

fn default_roots(home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = home {
        for relative in [
            "Projects",
            "Projetos",
            "repos",
            "dev",
            "code",
            "workspace",
            "Desktop",
            "Documents",
            "Documentos",
        ] {
            roots.push(home.join(relative));
        }
        let one_drive = home.join("OneDrive");
        for relative in ["Área de Trabalho", "Desktop", "Documents", "Documentos"] {
            roots.push(one_drive.join(relative));
        }
    }
    roots
}

/// Where programs are installed on this system, plus the folder of this executable.
fn installation_directories() -> Vec<PathBuf> {
    let variable = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut folders = Vec::new();
    if cfg!(windows) {
        for name in [
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "ProgramData",
            "WINDIR",
        ] {
            folders.extend(variable(name));
        }
        folders.extend(variable("LOCALAPPDATA").map(|local| local.join("Programs")));
    } else if cfg!(target_os = "macos") {
        folders.extend(["/Applications", "/System", "/Library"].map(PathBuf::from));
        folders.extend(home_directory().map(|home| home.join("Applications")));
    } else {
        folders.extend(["/usr", "/opt", "/snap"].map(PathBuf::from));
    }
    folders.extend(executable_directory());
    folders
}

fn executable_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

/// The folder the Alpha 8 search added by mistake when opened from the installer: the one
/// above this app's installation folder, when it is a folder of installed programs. Older
/// versions saved it with the person's folders whenever they added or removed one (#98).
pub fn legacy_installation_root() -> Option<PathBuf> {
    let parent = executable_directory()?.parent()?.to_path_buf();
    comparable_all(installation_directories())
        .contains(&comparable(&parent))
        .then_some(parent)
}

/// Removes `legacy` from the saved folders. Returns the remaining folders and the entry
/// removed, or `None` when it was not there.
pub fn without_legacy_root(roots: &[PathBuf], legacy: &Path) -> Option<(Vec<PathBuf>, PathBuf)> {
    let legacy = comparable(legacy);
    let removed = roots
        .iter()
        .find(|root| comparable(root) == legacy)?
        .clone();
    let remaining = roots
        .iter()
        .filter(|root| comparable(root) != legacy)
        .cloned()
        .collect();
    Some((remaining, removed))
}

/// A form of the path for comparing: canonical when it exists and, on Windows, without
/// differences of case.
fn comparable(path: &Path) -> PathBuf {
    comparison_key(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
}

fn comparison_key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

fn comparable_all(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.iter().map(|path| comparable(path)).collect()
}

impl ProjectSource for FilesystemProjectSource {
    type Error = io::Error;

    fn candidate_paths(&self) -> Result<Vec<PathBuf>, Self::Error> {
        let mut projects = Vec::new();
        let mut visited = HashSet::new();
        let mut queue: VecDeque<_> = self.roots.iter().cloned().map(|path| (path, 0)).collect();

        while let Some((path, depth)) = queue.pop_front() {
            let canonical = path.canonicalize().unwrap_or(path.clone());
            // A folder the person added is searched even when it is an installation folder;
            // the search only refuses to go down into one.
            if depth > 0 && self.is_excluded(&canonical) {
                continue;
            }
            if !visited.insert(canonical) || !path.is_dir() {
                continue;
            }
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => continue,
                Err(error) => return Err(error),
            };
            let mut is_project = false;
            let mut is_ignored = false;
            let mut source_counts = [0_u8; RuntimeKind::ALL.len()];
            let mut child_directories = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    if name == ".git" {
                        is_project = true;
                    } else if depth < self.max_depth
                        && !name.starts_with('.')
                        && !IGNORED_DIRECTORIES
                            .iter()
                            .any(|ignored| name.eq_ignore_ascii_case(ignored))
                    {
                        child_directories.push(entry.path());
                    }
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                if name == ".localcodepilot-ignore" {
                    is_ignored = true;
                    break;
                }
                if PROJECT_MARKERS
                    .iter()
                    .any(|marker| name.eq_ignore_ascii_case(marker))
                    || languages::from_manifest(&name).is_some()
                {
                    is_project = true;
                }
                let entry_path = entry.path();
                let Some(extension) = entry_path.extension().and_then(|value| value.to_str())
                else {
                    continue;
                };
                let Some(index) =
                    RuntimeKind::from_source_extension(extension).map(RuntimeKind::index)
                else {
                    continue;
                };
                source_counts[index] = source_counts[index].saturating_add(1);
                if source_counts[index] >= 2 {
                    is_project = true;
                }
            }
            if is_ignored {
                continue;
            }
            if is_project {
                projects.push(path);
            } else {
                queue.extend(
                    child_directories
                        .into_iter()
                        .map(|directory| (directory, depth + 1)),
                );
            }
        }
        Ok(projects)
    }
}

fn unique_existing(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| {
                    !IGNORED_DIRECTORIES
                        .iter()
                        .any(|ignored| name.eq_ignore_ascii_case(ignored))
                })
        })
        .filter(|path| !path.join(".localcodepilot-ignore").is_file())
        .filter(|path| seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())))
        .collect()
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lcp-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn common_folders_come_only_from_the_profile() {
        let home = PathBuf::from("perfil");
        let roots = default_roots(Some(&home));
        assert!(!roots.is_empty());
        assert!(roots.iter().all(|root| root.starts_with(&home)));
        assert!(default_roots(None).is_empty());
    }

    #[test]
    fn search_skips_installed_programs_below_a_folder_but_not_a_folder_added_itself() {
        let root = scratch("install-exclusion");
        let programs = root.join("Programs");
        fs::create_dir_all(programs.join("Editor").join("resources").join("app")).unwrap();
        fs::write(
            programs
                .join("Editor")
                .join("resources")
                .join("app")
                .join("package.json"),
            "{}",
        )
        .unwrap();
        fs::create_dir_all(root.join("site")).unwrap();
        fs::write(root.join("site").join("package.json"), "{}").unwrap();

        let search = |roots: Vec<PathBuf>| {
            let mut source = FilesystemProjectSource::new(roots);
            source.excluded = comparable_all(vec![programs.clone()]);
            let mut found = source.candidate_paths().unwrap();
            found.sort();
            found
        };
        let below = search(vec![root.clone()]);
        assert_eq!(below.len(), 1, "{below:?}");
        assert!(below[0].ends_with("site"));
        let added = search(vec![programs.clone()]);
        assert_eq!(added.len(), 1, "{added:?}");
        assert!(added[0].ends_with("app"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removes_only_the_folder_saved_by_mistake() {
        let root = scratch("legacy-root");
        let programs = root.join("Programs");
        let projects = root.join("Projetos");
        fs::create_dir_all(&programs).unwrap();
        fs::create_dir_all(&projects).unwrap();
        let saved = vec![projects.clone(), programs.canonicalize().unwrap()];
        let legacy = if cfg!(windows) {
            PathBuf::from(programs.to_string_lossy().to_uppercase())
        } else {
            programs.clone()
        };
        let (remaining, removed) = without_legacy_root(&saved, &legacy).unwrap();
        assert_eq!(remaining, vec![projects.clone()]);
        assert_eq!(removed, saved[1]);
        assert!(without_legacy_root(&remaining, &legacy).is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn discovers_container_only_projects_without_native_manifests() {
        let root =
            std::env::temp_dir().join(format!("lcp-container-discovery-{}", std::process::id()));
        for (name, marker) in [
            ("image", "Dockerfile"),
            ("compose", "compose.yaml"),
            ("podman", "Containerfile"),
        ] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(marker), "").unwrap();
        }
        let candidates = FilesystemProjectSource::new(vec![root.clone()])
            .candidate_paths()
            .unwrap();
        assert_eq!(candidates.len(), 3);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn discovers_java_projects_by_their_manifest_or_wrapper() {
        let root =
            std::env::temp_dir().join(format!("localcodepilot-java-scan-{}", std::process::id()));
        let markers = [
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "gradlew",
            "gradlew.bat",
        ];
        for (index, marker) in markers.iter().enumerate() {
            let project = root.join(format!("java-{index}"));
            fs::create_dir_all(&project).unwrap();
            fs::write(project.join(marker), "").unwrap();
            fs::write(project.join("Main.java"), "class Main {}").unwrap();
        }
        let paths = FilesystemProjectSource::new(vec![root.clone()])
            .candidate_paths()
            .unwrap();
        assert_eq!(paths.len(), markers.len());
        for index in 0..markers.len() {
            assert!(paths.contains(&root.join(format!("java-{index}"))));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skips_dependency_directories() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-scan-{nonce}"));
        fs::create_dir_all(root.join("app")).unwrap();
        fs::create_dir_all(root.join("node_modules").join("dependency")).unwrap();
        fs::write(root.join("app").join("Cargo.toml"), "").unwrap();
        fs::write(
            root.join("node_modules")
                .join("dependency")
                .join("package.json"),
            "{}",
        )
        .unwrap();
        let paths = FilesystemProjectSource::new(vec![root.clone()])
            .candidate_paths()
            .unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("app"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_build_or_dependency_directory_used_as_a_scan_root() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-ignored-root-{nonce}"));
        let target = root.join("target");
        let dependency = root.join("node_modules");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&dependency).unwrap();
        fs::write(target.join("Cargo.toml"), "").unwrap();
        fs::write(dependency.join("package.json"), "{}").unwrap();

        let source = FilesystemProjectSource::new(vec![target, dependency]);

        assert!(source.roots().is_empty());
        assert!(source.candidate_paths().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn does_not_list_subprojects_inside_a_project() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-nested-{nonce}"));
        let nested = root.join("apps").join("web");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("Cargo.toml"), "").unwrap();
        fs::write(nested.join("package.json"), "{}").unwrap();

        let paths = FilesystemProjectSource::new(vec![root.clone()])
            .candidate_paths()
            .unwrap();

        assert_eq!(paths.len(), 1);
        assert!(paths.iter().any(|path| path == &root));
        assert!(!paths.iter().any(|path| path == &nested));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uses_git_repository_as_the_project_root() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-git-root-{nonce}"));
        let nested = root.join("app");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("composer.json"), "{}").unwrap();

        let paths = FilesystemProjectSource::new(vec![root.clone()])
            .candidate_paths()
            .unwrap();

        assert_eq!(paths, vec![root.clone()]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_local_exercises_without_git_or_manifest() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let container = std::env::temp_dir().join(format!("localcodepilot-exercises-{nonce}"));
        let exercises = container.join("exercios");
        fs::create_dir_all(&exercises).unwrap();
        fs::write(exercises.join("exercicio1.php"), "<?php").unwrap();
        fs::write(exercises.join("exercicio2.php"), "<?php").unwrap();

        let paths = FilesystemProjectSource::new(vec![container.clone()])
            .candidate_paths()
            .unwrap();

        assert_eq!(paths, vec![exercises]);
        fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn ignores_macos_archive_metadata_directories() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-macosx-{nonce}"));
        let metadata = root.join("__MACOSX").join("curso").join("app");
        fs::create_dir_all(&metadata).unwrap();
        fs::write(metadata.join("Controller.php"), "<?php").unwrap();
        fs::write(metadata.join("Model.php"), "<?php").unwrap();

        let paths = FilesystemProjectSource::new(vec![root.clone()])
            .candidate_paths()
            .unwrap();

        assert!(paths.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
