use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TechnologyKind {
    Laravel,
    Lumen,
    WordPress,
    Vue,
    React,
    TypeScript,
    JavaScript,
    Flutter,
    StaticSite,
    Containers,
}

impl TechnologyKind {
    pub fn uses_node_tooling(self) -> bool {
        matches!(
            self,
            Self::Vue | Self::React | Self::TypeScript | Self::JavaScript
        )
    }
}

impl fmt::Display for TechnologyKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Laravel => "Laravel",
            Self::Lumen => "Lumen",
            Self::WordPress => "WordPress",
            Self::Vue => "Vue",
            Self::React => "React",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::Flutter => "Flutter",
            Self::StaticSite => "Site estático",
            Self::Containers => "Containers",
        })
    }
}
