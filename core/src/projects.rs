use crate::{runtimes::RuntimeKind, technologies::TechnologyKind};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

const IGNORED_DIRECTORIES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    ".venv",
    "venv",
    "_macosx",
    "__macosx",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    pub runtimes: Vec<RuntimeKind>,
    pub technologies: Vec<TechnologyKind>,
    pub modified_at: Option<SystemTime>,
}

impl Project {
    pub fn new(path: PathBuf, runtimes: Vec<RuntimeKind>) -> Self {
        let path = path.canonicalize().unwrap_or(path);
        let modified_at = latest_modified_at(&path);
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Novo projeto")
            .to_owned();
        Self {
            name,
            path,
            runtimes,
            technologies: Vec::new(),
            modified_at,
        }
    }

    pub fn with_technologies(mut self, technologies: Vec<TechnologyKind>) -> Self {
        self.technologies = technologies;
        self
    }

    pub fn shows_runtime(&self, runtime: RuntimeKind) -> bool {
        runtime != RuntimeKind::Node
            || !self
                .technologies
                .iter()
                .any(|technology| technology.uses_node_tooling())
    }

    pub fn display_stack(&self) -> String {
        let labels = self
            .technologies
            .iter()
            .map(ToString::to_string)
            .chain(
                self.runtimes
                    .iter()
                    .copied()
                    .filter(|runtime| self.shows_runtime(*runtime))
                    .map(|runtime| runtime.to_string()),
            )
            .collect::<Vec<_>>();
        if labels.is_empty() {
            "Projeto local".into()
        } else {
            labels.join(" + ")
        }
    }
}

fn latest_modified_at(root: &Path) -> Option<SystemTime> {
    let mut latest = root
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0_usize)]);

    while let Some((directory, depth)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
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
                if depth < 8
                    && !name.starts_with('.')
                    && !IGNORED_DIRECTORIES.contains(&name.as_ref())
                {
                    queue.push_back((entry.path(), depth + 1));
                }
            } else if let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) {
                latest = Some(latest.map_or(modified, |current| current.max(modified)));
            }
        }
    }

    latest
}

#[derive(Debug, Default)]
pub struct ProjectCatalog {
    projects: Vec<Project>,
}

impl ProjectCatalog {
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn into_projects(self) -> Vec<Project> {
        self.projects
    }

    pub fn add(&mut self, project: Project) -> bool {
        if self
            .projects
            .iter()
            .any(|current| same_path(&current.path, &project.path))
        {
            return false;
        }
        self.projects.push(project);
        true
    }

    pub fn search(&self, query: &str) -> Vec<&Project> {
        let query = query.trim().to_lowercase();
        self.projects
            .iter()
            .filter(|project| {
                query.is_empty()
                    || project.name.to_lowercase().contains(&query)
                    || project
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
            })
            .collect()
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::technologies::TechnologyKind;

    #[test]
    fn catalog_rejects_duplicate_paths() {
        let mut catalog = ProjectCatalog::default();
        let project = Project::new(PathBuf::from("project"), vec![RuntimeKind::Rust]);
        assert!(catalog.add(project.clone()));
        assert!(!catalog.add(project));
    }

    #[test]
    fn display_stack_prefers_detected_technologies_over_node() {
        let project = Project::new(PathBuf::from("project"), vec![RuntimeKind::Node])
            .with_technologies(vec![TechnologyKind::Vue, TechnologyKind::TypeScript]);

        assert_eq!(project.display_stack(), "Vue + TypeScript");
    }
}
