//! What the app knows about each language in one place: how a project shows it, which
//! program answers its version, which Docker images run it and where installers put it.
//! Starting a project stays specific to each language (`runtime` crate).

use crate::runtimes::RuntimeKind;

/// A program that answers its version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Program {
    pub name: &'static str,
    pub version_args: &'static [&'static str],
}

/// Folder where an installer puts the program even when it does not reach `PATH`. One path
/// segment may end in `*`, such as `Programs/Python/Python3*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallDir {
    Home(&'static str),
    LocalAppData(&'static str),
    ProgramFiles(&'static str),
    Absolute(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    pub kind: RuntimeKind,
    pub name: &'static str,
    /// Files whose name alone marks a project of this language.
    pub manifests: &'static [&'static str],
    /// Extensions of manifests with project-specific names, such as `app.csproj`.
    pub manifest_extensions: &'static [&'static str],
    pub source_extensions: &'static [&'static str],
    /// Tried in order; the first one found is used.
    pub programs: &'static [Program],
    /// Image names: a name without `/` matches the last part of the repository
    /// (`docker.io/library/python` → `python`); one with `/` matches anywhere in it.
    pub images: &'static [&'static str],
    pub install_dirs: &'static [InstallDir],
    /// Whether the app finds commands to start projects of this language.
    pub starts: bool,
}

const fn program(name: &'static str, version_args: &'static [&'static str]) -> Program {
    Program { name, version_args }
}

pub const LANGUAGES: &[Language] = &[
    Language {
        kind: RuntimeKind::Rust,
        name: "Rust",
        manifests: &["Cargo.toml"],
        manifest_extensions: &[],
        source_extensions: &["rs"],
        programs: &[program("cargo", &["--version"])],
        images: &["rust"],
        install_dirs: &[InstallDir::Home(".cargo/bin")],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Node,
        name: "JavaScript/TypeScript (Node)",
        manifests: &["package.json"],
        manifest_extensions: &[],
        source_extensions: &["js", "jsx", "mjs", "cjs", "ts", "tsx"],
        programs: &[program("node", &["--version"])],
        images: &["node", "bun"],
        install_dirs: &[InstallDir::ProgramFiles("nodejs")],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Php,
        name: "PHP",
        manifests: &[
            "composer.json",
            "index.php",
            "wp-config.php",
            "wp-config-sample.php",
        ],
        manifest_extensions: &[],
        source_extensions: &["php", "phtml"],
        programs: &[program("php", &["-r", "echo PHP_VERSION, PHP_EOL;"])],
        images: &["php", "wordpress", "composer", "laravel"],
        install_dirs: &[
            InstallDir::Absolute("C:/php"),
            InstallDir::Absolute("C:/tools/php*"),
        ],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Python,
        name: "Python",
        manifests: &["pyproject.toml", "requirements.txt", "Pipfile", "setup.py"],
        manifest_extensions: &[],
        source_extensions: &["py", "pyw"],
        programs: &[
            program("python", &["--version"]),
            program("python3", &["--version"]),
        ],
        images: &["python"],
        install_dirs: &[InstallDir::LocalAppData("Programs/Python/Python3*")],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Java,
        name: "Java",
        manifests: &[
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "gradlew",
            "gradlew.bat",
        ],
        manifest_extensions: &[],
        source_extensions: &["java"],
        programs: &[program("java", &["-version"])],
        images: &[
            "eclipse-temurin",
            "openjdk",
            "amazoncorretto",
            "maven",
            "gradle",
        ],
        install_dirs: &[
            InstallDir::ProgramFiles("Java/jdk*/bin"),
            InstallDir::ProgramFiles("Eclipse Adoptium/jdk*/bin"),
        ],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Go,
        name: "Go",
        manifests: &["go.mod"],
        manifest_extensions: &[],
        source_extensions: &["go"],
        programs: &[program("go", &["version"])],
        images: &["golang"],
        install_dirs: &[
            InstallDir::ProgramFiles("Go/bin"),
            InstallDir::Absolute("/usr/local/go/bin"),
        ],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Dart,
        name: "Dart",
        // pubspec.yaml decides between Dart and Flutter.
        manifests: &[],
        manifest_extensions: &[],
        source_extensions: &["dart"],
        programs: &[program("dart", &["--version"])],
        images: &["dart"],
        install_dirs: &[],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Flutter,
        name: "Flutter",
        manifests: &[],
        manifest_extensions: &[],
        source_extensions: &[],
        programs: &[program("flutter", &["--version"])],
        images: &[],
        install_dirs: &[InstallDir::Absolute("C:/src/flutter/bin")],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Dotnet,
        name: ".NET (C#, F#)",
        manifests: &["global.json"],
        manifest_extensions: &["csproj", "fsproj", "vbproj", "sln"],
        source_extensions: &["cs", "fs"],
        programs: &[program("dotnet", &["--version"])],
        images: &["dotnet/"],
        install_dirs: &[
            InstallDir::ProgramFiles("dotnet"),
            InstallDir::Home(".dotnet"),
        ],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Ruby,
        name: "Ruby",
        manifests: &["Gemfile", "config.ru"],
        manifest_extensions: &["gemspec"],
        source_extensions: &["rb"],
        programs: &[program("ruby", &["--version"])],
        images: &["ruby"],
        install_dirs: &[InstallDir::Absolute("C:/Ruby*/bin")],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Kotlin,
        name: "Kotlin",
        manifests: &[],
        manifest_extensions: &[],
        // Not `kts`: every Gradle build written in Kotlin would look like a Kotlin project.
        source_extensions: &["kt"],
        programs: &[
            program("kotlinc", &["-version"]),
            program("kotlin", &["-version"]),
        ],
        images: &[],
        install_dirs: &[],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Elixir,
        name: "Elixir",
        manifests: &["mix.exs"],
        manifest_extensions: &[],
        source_extensions: &["ex", "exs"],
        programs: &[program("elixir", &["--version"])],
        images: &["elixir"],
        // Elixir runs on Erlang, which winget installs in its own folder.
        install_dirs: &[
            InstallDir::ProgramFiles("Elixir/bin"),
            InstallDir::ProgramFiles("Erlang OTP/bin"),
        ],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Deno,
        name: "Deno",
        manifests: &["deno.json", "deno.jsonc"],
        manifest_extensions: &[],
        source_extensions: &[],
        programs: &[program("deno", &["--version"])],
        images: &["denoland/deno"],
        install_dirs: &[InstallDir::Home(".deno/bin")],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Cpp,
        name: "C/C++",
        manifests: &["CMakeLists.txt", "meson.build"],
        manifest_extensions: &["vcxproj"],
        source_extensions: &["c", "cc", "cpp", "cxx", "h", "hpp"],
        programs: &[
            program("gcc", &["--version"]),
            program("clang", &["--version"]),
        ],
        images: &["gcc"],
        install_dirs: &[],
        starts: false,
    },
    Language {
        kind: RuntimeKind::Swift,
        name: "Swift",
        manifests: &["Package.swift"],
        manifest_extensions: &[],
        source_extensions: &["swift"],
        programs: &[program("swift", &["--version"])],
        images: &["swift"],
        install_dirs: &[],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Scala,
        name: "Scala",
        manifests: &["build.sbt"],
        manifest_extensions: &[],
        source_extensions: &["scala"],
        programs: &[
            program("scala", &["--version"]),
            program("sbt", &["--version"]),
        ],
        images: &["scala-sbt"],
        install_dirs: &[],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Clojure,
        name: "Clojure",
        manifests: &["project.clj", "deps.edn"],
        manifest_extensions: &[],
        source_extensions: &["clj", "cljs", "cljc"],
        programs: &[
            program("clojure", &["--version"]),
            program("lein", &["version"]),
            program("bb", &["--version"]),
        ],
        images: &["clojure"],
        install_dirs: &[],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Haskell,
        name: "Haskell",
        manifests: &["stack.yaml", "cabal.project"],
        manifest_extensions: &["cabal"],
        source_extensions: &["hs"],
        programs: &[
            program("ghc", &["--version"]),
            program("stack", &["--version"]),
        ],
        images: &["haskell"],
        install_dirs: &[InstallDir::Home(".ghcup/bin")],
        starts: true,
    },
    Language {
        kind: RuntimeKind::Perl,
        name: "Perl",
        manifests: &["cpanfile", "Makefile.PL"],
        manifest_extensions: &[],
        source_extensions: &["pl", "pm"],
        programs: &[program("perl", &["--version"])],
        images: &["perl"],
        // Git for Windows ships a Perl that is not in `PATH`.
        install_dirs: &[
            InstallDir::Absolute("C:/Strawberry/perl/bin"),
            InstallDir::ProgramFiles("Git/usr/bin"),
        ],
        starts: false,
    },
    Language {
        kind: RuntimeKind::Lua,
        name: "Lua",
        manifests: &[],
        manifest_extensions: &["rockspec"],
        source_extensions: &["lua"],
        programs: &[program("lua", &["-v"])],
        images: &[],
        install_dirs: &[],
        starts: false,
    },
    Language {
        kind: RuntimeKind::R,
        name: "R",
        manifests: &["renv.lock"],
        manifest_extensions: &["Rproj"],
        source_extensions: &["r"],
        programs: &[program("Rscript", &["--version"])],
        images: &["r-base", "rocker/"],
        install_dirs: &[InstallDir::ProgramFiles("R/R-*/bin")],
        starts: false,
    },
    Language {
        kind: RuntimeKind::Julia,
        name: "Julia",
        manifests: &["Project.toml", "JuliaProject.toml"],
        manifest_extensions: &[],
        source_extensions: &["jl"],
        programs: &[program("julia", &["--version"])],
        images: &["julia"],
        install_dirs: &[InstallDir::LocalAppData("Programs/Julia*/bin")],
        starts: false,
    },
    Language {
        kind: RuntimeKind::Zig,
        name: "Zig",
        manifests: &["build.zig", "build.zig.zon"],
        manifest_extensions: &[],
        source_extensions: &["zig"],
        programs: &[program("zig", &["version"])],
        images: &[],
        install_dirs: &[],
        starts: false,
    },
];

pub fn language(kind: RuntimeKind) -> &'static Language {
    LANGUAGES
        .iter()
        .find(|language| language.kind == kind)
        .expect("every RuntimeKind has an entry in LANGUAGES")
}

/// The language of a project file, by name or by manifest extension.
pub fn from_manifest(file_name: &str) -> Option<RuntimeKind> {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or_default();
    LANGUAGES
        .iter()
        .find(|language| {
            language
                .manifests
                .iter()
                .any(|manifest| manifest.eq_ignore_ascii_case(file_name))
                || language
                    .manifest_extensions
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
        .map(|language| language.kind)
}

/// The language a Docker image runs, when it is an image of a language. Databases, web
/// servers and other images give `None`.
pub fn from_image(image: &str) -> Option<RuntimeKind> {
    // `registry:5000/name:tag@digest` → `registry:5000/name`.
    let image = image.split('@').next().unwrap_or(image);
    let repository = match image.rsplit_once(':') {
        Some((repository, tag)) if !tag.contains('/') => repository,
        _ => image,
    }
    .to_ascii_lowercase();
    let last = repository.rsplit('/').next().unwrap_or(&repository);
    LANGUAGES
        .iter()
        .find(|language| {
            language.images.iter().any(|name| {
                if name.contains('/') {
                    repository.contains(name)
                } else {
                    last == *name
                }
            })
        })
        .map(|language| language.kind)
}

/// Base images named by `FROM` lines of a Dockerfile, without build-stage names or
/// variables.
pub fn dockerfile_base_images(contents: &str) -> Vec<String> {
    let mut stages = Vec::new();
    let mut images = Vec::new();
    for line in contents.lines() {
        let mut words = line.split_whitespace();
        if !words
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("FROM"))
        {
            continue;
        }
        let Some(image) = words.find(|word| !word.starts_with("--")) else {
            continue;
        };
        let lowered = image.to_ascii_lowercase();
        // A stage built earlier in the same file is not a base image.
        let base = !image.contains('$') && lowered != "scratch" && !stages.contains(&lowered);
        if words
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("AS"))
            && let Some(stage) = words.next()
        {
            stages.push(stage.to_ascii_lowercase());
        }
        if base && !images.iter().any(|known: &String| known == image) {
            images.push(image.to_owned());
        }
    }
    images
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_one_entry_and_a_program() {
        for kind in RuntimeKind::ALL {
            assert_eq!(
                LANGUAGES
                    .iter()
                    .filter(|language| language.kind == kind)
                    .count(),
                1,
                "{kind:?}"
            );
            assert!(!language(kind).programs.is_empty(), "{kind:?}");
        }
        assert_eq!(LANGUAGES.len(), RuntimeKind::ALL.len());
    }

    #[test]
    fn source_extensions_and_manifests_name_one_language_each() {
        let mut extensions = Vec::new();
        let mut manifests = Vec::new();
        for language in LANGUAGES {
            for extension in language.source_extensions {
                assert!(!extensions.contains(extension), "{extension}");
                extensions.push(*extension);
            }
            for manifest in language.manifests {
                assert!(!manifests.contains(manifest), "{manifest}");
                manifests.push(*manifest);
            }
        }
    }

    #[test]
    fn images_of_languages_are_recognized_and_others_are_not() {
        for (image, kind) in [
            ("python:3.12-slim", Some(RuntimeKind::Python)),
            ("docker.io/library/node:20", Some(RuntimeKind::Node)),
            ("oven/bun:1", Some(RuntimeKind::Node)),
            (
                "docker.io/library/wordpress:6-php8.3-apache",
                Some(RuntimeKind::Php),
            ),
            (
                "mcr.microsoft.com/dotnet/aspnet:8.0",
                Some(RuntimeKind::Dotnet),
            ),
            ("eclipse-temurin:21-jre", Some(RuntimeKind::Java)),
            ("golang:1.22@sha256:abc", Some(RuntimeKind::Go)),
            ("denoland/deno:alpine", Some(RuntimeKind::Deno)),
            ("rocker/r-ver:4", Some(RuntimeKind::R)),
            ("registry.local:5000/ruby", Some(RuntimeKind::Ruby)),
            ("mariadb:11", None),
            ("nginx:alpine", None),
            ("redis", None),
            ("my-node-app:latest", None),
        ] {
            assert_eq!(from_image(image), kind, "{image}");
        }
    }

    #[test]
    fn manifests_match_by_name_or_extension() {
        assert_eq!(from_manifest("Gemfile"), Some(RuntimeKind::Ruby));
        assert_eq!(from_manifest("Api.csproj"), Some(RuntimeKind::Dotnet));
        assert_eq!(from_manifest("cargo.toml"), Some(RuntimeKind::Rust));
        assert_eq!(from_manifest("README.md"), None);
    }

    #[test]
    fn dockerfile_bases_skip_stages_variables_and_scratch() {
        let dockerfile = "\
# syntax=docker/dockerfile:1
FROM --platform=$BUILDPLATFORM golang:1.22 AS build
RUN go build
FROM build AS test
FROM ${BASE}
FROM scratch
from python:3.12-slim
";
        assert_eq!(
            dockerfile_base_images(dockerfile),
            ["golang:1.22", "python:3.12-slim"]
        );
    }
}
