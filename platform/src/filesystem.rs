use localcodepilot_core::{discovery::ProjectSource, runtimes::RuntimeKind};
use std::{
    collections::{HashSet, VecDeque},
    fs, io,
    path::PathBuf,
};

const PROJECT_MARKERS: &[&str] = &[
    "Dockerfile",
    "Containerfile",
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
    "Cargo.toml",
    "package.json",
    "composer.json",
    "pyproject.toml",
    "requirements.txt",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "gradlew",
    "gradlew.bat",
    "go.mod",
    "pubspec.yaml",
    "index.html",
    "index.php",
    "wp-config.php",
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
];

#[derive(Debug, Clone)]
pub struct FilesystemProjectSource {
    roots: Vec<PathBuf>,
    max_depth: usize,
}

impl FilesystemProjectSource {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots: unique_existing(roots),
            max_depth: 8,
        }
    }

    pub fn common_locations() -> Self {
        let mut roots = Vec::new();
        if let Some(home) = home_directory() {
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
        if let Ok(current) = std::env::current_dir()
            && let Some(parent) = current.parent()
        {
            roots.push(parent.to_path_buf());
        }
        Self::new(roots)
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }
}

impl ProjectSource for FilesystemProjectSource {
    type Error = io::Error;

    fn candidate_paths(&self) -> Result<Vec<PathBuf>, Self::Error> {
        let mut projects = Vec::new();
        let mut visited = HashSet::new();
        let mut queue: VecDeque<_> = self.roots.iter().cloned().map(|path| (path, 0)).collect();

        while let Some((path, depth)) = queue.pop_front() {
            let canonical = path.canonicalize().unwrap_or(path.clone());
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
            let mut source_counts = [0_u8; 7];
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
                {
                    is_project = true;
                }
                let entry_path = entry.path();
                let Some(extension) = entry_path.extension().and_then(|value| value.to_str())
                else {
                    continue;
                };
                let index = match RuntimeKind::from_source_extension(extension) {
                    Some(RuntimeKind::Rust) => 0,
                    Some(RuntimeKind::Node) => 1,
                    Some(RuntimeKind::Php) => 2,
                    Some(RuntimeKind::Python) => 3,
                    Some(RuntimeKind::Java) => 4,
                    Some(RuntimeKind::Go) => 5,
                    Some(RuntimeKind::Dart) => 6,
                    Some(RuntimeKind::Flutter) | None => continue,
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
