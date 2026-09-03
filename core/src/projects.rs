use crate::runtimes::RuntimeKind;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    pub runtimes: Vec<RuntimeKind>,
}

impl Project {
    pub fn new(path: PathBuf, runtimes: Vec<RuntimeKind>) -> Self {
        let path = path.canonicalize().unwrap_or(path);
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Novo projeto")
            .to_owned();
        Self {
            name,
            path,
            runtimes,
        }
    }

    pub fn display_stack(&self) -> String {
        if self.runtimes.is_empty() {
            "Projeto local".into()
        } else {
            self.runtimes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" + ")
        }
    }
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

    #[test]
    fn catalog_rejects_duplicate_paths() {
        let mut catalog = ProjectCatalog::default();
        let project = Project::new(PathBuf::from("project"), vec![RuntimeKind::Rust]);
        assert!(catalog.add(project.clone()));
        assert!(!catalog.add(project));
    }
}
