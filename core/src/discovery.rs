use crate::{
    projects::{Project, ProjectCatalog},
    runtimes::RuntimeKind,
};
use std::path::{Path, PathBuf};

pub trait ProjectSource {
    type Error;
    fn candidate_paths(&self) -> Result<Vec<PathBuf>, Self::Error>;
}

pub trait RuntimeDetector {
    fn detect(&self, path: &Path) -> Vec<RuntimeKind>;
}

pub struct DiscoveryService<S, D> {
    source: S,
    detector: D,
}

impl<S, D> DiscoveryService<S, D>
where
    S: ProjectSource,
    D: RuntimeDetector,
{
    pub fn new(source: S, detector: D) -> Self {
        Self { source, detector }
    }

    pub fn discover(&self) -> Result<ProjectCatalog, S::Error> {
        let mut catalog = ProjectCatalog::default();
        for path in self.source.candidate_paths()? {
            let runtimes = self.detector.detect(&path);
            if !runtimes.is_empty() {
                catalog.add(Project::new(path, runtimes));
            }
        }
        Ok(catalog)
    }
}
