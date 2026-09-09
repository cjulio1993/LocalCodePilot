use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeKind {
    Rust,
    Node,
    Php,
    Python,
}

impl RuntimeKind {
    pub fn from_source_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => Some(Self::Node),
            "php" | "phtml" => Some(Self::Php),
            "py" | "pyw" => Some(Self::Python),
            _ => None,
        }
    }
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
