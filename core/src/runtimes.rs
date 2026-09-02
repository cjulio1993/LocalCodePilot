use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeKind {
    Rust,
    Node,
    Php,
    Python,
}

impl fmt::Display for RuntimeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rust => "Rust",
            Self::Node => "Node",
            Self::Php => "PHP",
            Self::Python => "Python",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub kind: RuntimeKind,
    pub version: Option<String>,
}
