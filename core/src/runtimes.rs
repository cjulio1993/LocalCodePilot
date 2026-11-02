use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuntimeKind {
    Rust,
    Node,
    Php,
    Python,
    Java,
    Go,
    Dart,
    Flutter,
    Dotnet,
    Ruby,
    Kotlin,
    Elixir,
    Deno,
    Cpp,
    Swift,
    Scala,
    Clojure,
    Haskell,
    Perl,
    Lua,
    R,
    Julia,
    Zig,
}

impl RuntimeKind {
    pub const ALL: [Self; 23] = [
        Self::Rust,
        Self::Node,
        Self::Php,
        Self::Python,
        Self::Java,
        Self::Go,
        Self::Dart,
        Self::Flutter,
        Self::Dotnet,
        Self::Ruby,
        Self::Kotlin,
        Self::Elixir,
        Self::Deno,
        Self::Cpp,
        Self::Swift,
        Self::Scala,
        Self::Clojure,
        Self::Haskell,
        Self::Perl,
        Self::Lua,
        Self::R,
        Self::Julia,
        Self::Zig,
    ];

    pub fn from_source_extension(extension: &str) -> Option<Self> {
        crate::languages::LANGUAGES
            .iter()
            .find(|language| {
                language
                    .source_extensions
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(extension))
            })
            .map(|language| language.kind)
    }

    /// Position in `ALL`, for per-language counters.
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|kind| *kind == self)
            .unwrap_or_default()
    }

    /// Whether the app finds commands to start projects of this language.
    pub fn starts(self) -> bool {
        crate::languages::language(self).starts
    }
}

impl fmt::Display for RuntimeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rust => "Rust",
            Self::Node => "Node",
            Self::Php => "PHP",
            Self::Python => "Python",
            Self::Java => "Java",
            Self::Go => "Go",
            Self::Dart => "Dart",
            Self::Flutter => "Flutter",
            Self::Dotnet => ".NET",
            Self::Ruby => "Ruby",
            Self::Kotlin => "Kotlin",
            Self::Elixir => "Elixir",
            Self::Deno => "Deno",
            Self::Cpp => "C/C++",
            Self::Swift => "Swift",
            Self::Scala => "Scala",
            Self::Clojure => "Clojure",
            Self::Haskell => "Haskell",
            Self::Perl => "Perl",
            Self::Lua => "Lua",
            Self::R => "R",
            Self::Julia => "Julia",
            Self::Zig => "Zig",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub kind: RuntimeKind,
    pub version: Option<String>,
}
