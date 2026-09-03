use localcodepilot_core::discovery::ProjectSource;
use std::{
    collections::{HashSet, VecDeque},
    fs, io,
    path::PathBuf,
};

const PROJECT_MARKERS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "composer.json",
    "pyproject.toml",
    "requirements.txt",
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
            max_depth: 5,
        }
    }

    pub fn common_locations() -> Self {
        let mut roots = Vec::new();
        if let Some(home) = home_directory() {
            for relative in ["Projects", "Projetos", "dev", "code", "workspace"] {
                roots.push(home.join(relative));
            }
            roots.push(
                home.join("OneDrive")
                    .join("Área de Trabalho")
                    .join("projetos"),
            );
            roots.push(home.join("OneDrive").join("Desktop").join("projects"));
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
            if PROJECT_MARKERS
                .iter()
                .any(|marker| path.join(marker).is_file())
            {
                projects.push(path.clone());
                continue;
            }
            if depth >= self.max_depth {
                continue;
            }
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => continue,
                Err(error) => return Err(error),
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with('.') || IGNORED_DIRECTORIES.contains(&name.as_ref()) {
                    continue;
                }
                if entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                {
                    queue.push_back((entry.path(), depth + 1));
                }
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
}
