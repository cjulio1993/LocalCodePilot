use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TechnologyKind {
    Laravel,
    Vue,
    React,
    TypeScript,
    JavaScript,
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
            Self::Vue => "Vue",
            Self::React => "React",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
        })
    }
}
