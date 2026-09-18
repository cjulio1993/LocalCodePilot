use crate::{
    projects::{Project, ProjectCatalog},
    runtimes::RuntimeKind,
    technologies::TechnologyKind,
};
use std::path::{Path, PathBuf};

pub trait ProjectSource {
    type Error;
    fn candidate_paths(&self) -> Result<Vec<PathBuf>, Self::Error>;
}

pub trait RuntimeDetector {
    fn detect(&self, path: &Path) -> Vec<RuntimeKind>;

    fn detect_technologies(&self, _path: &Path) -> Vec<TechnologyKind> {
        Vec::new()
    }
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
            let technologies = self.detector.detect_technologies(&path);
            if !runtimes.is_empty() || !technologies.is_empty() {
                catalog.add(Project::new(path, runtimes).with_technologies(technologies));
            }
        }
        Ok(catalog)
    }
}
