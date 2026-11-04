use localcodepilot_core::{
    discovery::RuntimeDetector,
    environments::{RequirementSource, RequirementStatus, RuntimeRequirement, VersionConstraint},
    languages,
    ports::Port,
    processes::{PortOverride, ProcessState, ProjectProcess},
    projects::Project,
    runtimes::RuntimeKind,
    technologies::TechnologyKind,
};
use semver::{Version, VersionReq};
use std::{collections::VecDeque, fs, path::Path};

pub mod databases;

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
    "ephemeral",
    // Elixir: dependencies and build output of Mix.
    "deps",
    "_build",
];
const CONTINUOUS_SCRIPT_NAMES: &[&str] = &["dev", "start", "serve", "server", "watch", "preview"];

pub fn detect_node_requirement(
    directory: &Path,
    project_root: &Path,
) -> Option<RuntimeRequirement> {
    let package_path = find_in_project_ancestors(directory, project_root, "package.json")?;
    let version_file = [".nvmrc", ".node-version"]
        .iter()
        .find_map(|name| find_in_project_ancestors(directory, project_root, name));

    let (version, source) = if let Some(path) = version_file {
        let value = fs::read_to_string(&path).ok()?.trim().to_owned();
        if value.is_empty() {
            (
                VersionConstraint::recommended("LTS"),
                RequirementSource::Recommended,
            )
        } else {
            (
                VersionConstraint::declared(value),
                RequirementSource::VersionFile(path),
            )
        }
    } else {
        let declared = fs::read_to_string(&package_path)
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
            .and_then(|document| {
                document
                    .get("engines")?
                    .get("node")?
                    .as_str()
                    .map(str::to_owned)
            });
        match declared {
            Some(value) => (
                VersionConstraint::declared(value),
                RequirementSource::Manifest(package_path),
            ),
            None => (
                VersionConstraint::recommended("LTS"),
                RequirementSource::Recommended,
            ),
        }
    };

    Some(RuntimeRequirement {
        runtime: RuntimeKind::Node,
        command: "node".into(),
        version,
        source,
        status: RequirementStatus::Pending,
    })
}

pub fn node_version_matches(constraint: &str, installed: &str) -> Option<bool> {
    let installed = parse_loose_version(installed)?;
    let constraint = constraint.trim();
    if constraint.is_empty()
        || constraint.eq_ignore_ascii_case("node")
        || constraint.to_ascii_lowercase().starts_with("lts")
        || constraint == "*"
    {
        return None;
    }

    let mut parsed_any = false;
    for alternative in constraint.split("||").map(str::trim) {
        if let Some((lower, upper)) = alternative.split_once(" - ") {
            let (Some(lower), Some(upper)) =
                (parse_loose_version(lower), parse_loose_version(upper))
            else {
                continue;
            };
            parsed_any = true;
            if installed >= lower && installed <= upper {
                return Some(true);
            }
            continue;
        }

        let normalized = normalize_node_constraint(alternative);
        if let Ok(requirement) = VersionReq::parse(&normalized) {
            parsed_any = true;
            if requirement.matches(&installed) {
                return Some(true);
            }
        }
    }
    parsed_any.then_some(false)
}

pub fn runtime_version_matches(constraint: &str, installed: &str) -> Option<bool> {
    let constraint = constraint.trim();
    if let Some(value) = constraint.strip_prefix("~=") {
        let base = parse_loose_version(value)?;
        let upper = if value.matches('.').count() >= 2 {
            format!("{}.{}.0", base.major, base.minor + 1)
        } else {
            format!("{}.0.0", base.major + 1)
        };
        return node_version_matches(&format!(">={base}, <{upper}"), installed);
    }
    node_version_matches(constraint, installed)
}

pub fn detect_python_requirement(
    directory: &Path,
    project_root: &Path,
) -> Option<RuntimeRequirement> {
    let marker = [
        "pyproject.toml",
        "requirements.txt",
        "manage.py",
        "main.py",
        "app.py",
        "index.html",
    ]
    .iter()
    .find_map(|name| find_in_project_ancestors(directory, project_root, name))?;
    if marker.file_name().is_some_and(|name| name == "index.html")
        && [
            "package.json",
            "composer.json",
            "Cargo.toml",
            "go.mod",
            "pubspec.yaml",
        ]
        .iter()
        .any(|name| find_in_project_ancestors(directory, project_root, name).is_some())
    {
        return None;
    }
    let version_file = find_in_project_ancestors(directory, project_root, ".python-version");
    let (version, source) = if let Some(path) = version_file {
        let value = fs::read_to_string(&path).ok()?.trim().to_owned();
        (
            if value.is_empty() {
                VersionConstraint::recommended("3.14")
            } else {
                VersionConstraint::declared(value)
            },
            RequirementSource::VersionFile(path),
        )
    } else if marker
        .file_name()
        .is_some_and(|name| name == "pyproject.toml")
    {
        let declared = fs::read_to_string(&marker).ok().and_then(|contents| {
            contents
                .parse::<toml::Value>()
                .ok()
                .and_then(|document| {
                    document
                        .get("project")
                        .and_then(|project| project.get("requires-python"))
                        .and_then(toml::Value::as_str)
                        .or_else(|| {
                            document
                                .get("tool")
                                .and_then(|tool| tool.get("poetry"))
                                .and_then(|poetry| poetry.get("dependencies"))
                                .and_then(|dependencies| dependencies.get("python"))
                                .and_then(toml::Value::as_str)
                        })
                        .map(str::to_owned)
                })
                .or_else(|| python_constraint_from_toml_text(&contents))
        });
        match declared {
            Some(value) => (
                VersionConstraint::declared(value),
                RequirementSource::Manifest(marker),
            ),
            None => (
                VersionConstraint::recommended("3.14"),
                RequirementSource::Recommended,
            ),
        }
    } else {
        (
            VersionConstraint::recommended("3.14"),
            RequirementSource::Recommended,
        )
    };
    Some(RuntimeRequirement {
        runtime: RuntimeKind::Python,
        command: "python".into(),
        version,
        source,
        status: RequirementStatus::Pending,
    })
}

pub fn detect_php_requirement(directory: &Path, project_root: &Path) -> Option<RuntimeRequirement> {
    let marker = [
        "composer.json",
        "artisan",
        "index.php",
        "wp-config.php",
        "wp-config-sample.php",
    ]
    .iter()
    .find_map(|name| find_in_project_ancestors(directory, project_root, name))?;
    let composer = find_in_project_ancestors(directory, project_root, "composer.json");
    let declared = composer.as_ref().and_then(|path| {
        fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
            .and_then(|document| {
                document
                    .get("require")?
                    .get("php")?
                    .as_str()
                    .map(str::to_owned)
            })
    });
    let (version, source) = match (declared, composer) {
        (Some(value), Some(path)) => (
            VersionConstraint::declared(value),
            RequirementSource::Manifest(path),
        ),
        _ => (
            VersionConstraint::recommended("8.5"),
            if marker
                .file_name()
                .is_some_and(|name| name == "composer.json")
            {
                RequirementSource::Manifest(marker)
            } else {
                RequirementSource::Recommended
            },
        ),
    };
    Some(RuntimeRequirement {
        runtime: RuntimeKind::Php,
        command: "php".into(),
        version,
        source,
        status: RequirementStatus::Pending,
    })
}

pub fn detect_php_extensions(directory: &Path, project_root: &Path) -> Vec<String> {
    let mut extensions = find_in_project_ancestors(directory, project_root, "composer.json")
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .and_then(|document| {
            document
                .get("require")
                .and_then(|value| value.as_object())
                .cloned()
        })
        .map(|requirements| {
            requirements
                .keys()
                .filter_map(|name| name.strip_prefix("ext-"))
                .map(|name| name.to_ascii_lowercase().replace('-', "_"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if find_in_project_ancestors(directory, project_root, "wp-config.php").is_some()
        && !containers::has_container_markers(project_root)
    {
        extensions.push("mysqli".into());
    }
    extensions.sort();
    extensions.dedup();
    extensions
}

pub fn detect_java_requirement(
    directory: &Path,
    project_root: &Path,
) -> Option<RuntimeRequirement> {
    let marker = [
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "gradlew",
        "gradlew.bat",
    ]
    .iter()
    .find_map(|name| find_in_project_ancestors(directory, project_root, name))?;
    let version_file = find_in_project_ancestors(directory, project_root, ".java-version");
    let declared = version_file.as_ref().and_then(|path| {
        fs::read_to_string(path)
            .ok()
            .map(|contents| contents.trim().to_owned())
            .filter(|value| !value.is_empty())
    });
    let manifest_version = declared.or_else(|| {
        let contents = fs::read_to_string(&marker).ok()?;
        [
            "java.version",
            "maven.compiler.release",
            "maven.compiler.source",
        ]
        .iter()
        .find_map(|tag| xml_tag_value(&contents, tag))
        .or_else(|| gradle_java_version(&contents))
    });
    let (version, source) = if let Some(value) = manifest_version {
        (
            VersionConstraint::declared(value),
            version_file.map_or_else(
                || RequirementSource::Manifest(marker),
                RequirementSource::VersionFile,
            ),
        )
    } else {
        (
            VersionConstraint::recommended("25"),
            RequirementSource::Recommended,
        )
    };
    Some(RuntimeRequirement {
        runtime: RuntimeKind::Java,
        command: "java".into(),
        version,
        source,
        status: RequirementStatus::Pending,
    })
}

pub fn detect_go_requirement(directory: &Path, project_root: &Path) -> Option<RuntimeRequirement> {
    let manifest = find_in_project_ancestors(directory, project_root, "go.mod")?;
    let declared = fs::read_to_string(&manifest).ok().and_then(|contents| {
        contents.lines().find_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next()? == "go")
                .then(|| parts.next().map(|version| format!(">={version}")))
                .flatten()
        })
    });
    let (version, source) = declared.map_or_else(
        || {
            (
                VersionConstraint::recommended("latest"),
                RequirementSource::Recommended,
            )
        },
        |version| {
            (
                VersionConstraint::declared(version),
                RequirementSource::Manifest(manifest),
            )
        },
    );
    Some(RuntimeRequirement {
        runtime: RuntimeKind::Go,
        command: "go".into(),
        version,
        source,
        status: RequirementStatus::Pending,
    })
}

pub fn detect_dart_requirement(
    directory: &Path,
    project_root: &Path,
) -> Option<RuntimeRequirement> {
    let manifest = find_in_project_ancestors(directory, project_root, "pubspec.yaml")?;
    let contents = fs::read_to_string(&manifest).ok()?;
    if is_flutter_pubspec(&contents) {
        return None;
    }
    let declared = dart_sdk_constraint(&contents);
    let (version, source) = declared.map_or_else(
        || {
            (
                VersionConstraint::recommended("stable"),
                RequirementSource::Recommended,
            )
        },
        |version| {
            (
                VersionConstraint::declared(version),
                RequirementSource::Manifest(manifest),
            )
        },
    );
    Some(RuntimeRequirement {
        runtime: RuntimeKind::Dart,
        command: "dart".into(),
        version,
        source,
        status: RequirementStatus::Pending,
    })
}

/// A Clojure project with `deps.edn` runs through Babashka, whose `bb clojure` is the Clojure
/// CLI and installs with winget on Windows. Leiningen projects (`project.clj`) are not covered.
pub fn detect_clojure_requirement(
    directory: &Path,
    project_root: &Path,
) -> Option<RuntimeRequirement> {
    find_in_project_ancestors(directory, project_root, "deps.edn")?;
    Some(RuntimeRequirement {
        runtime: RuntimeKind::Clojure,
        command: "bb".into(),
        version: VersionConstraint::recommended("latest"),
        source: RequirementSource::Recommended,
        status: RequirementStatus::Pending,
    })
}

/// A Haskell project with `stack.yaml` needs Stack, which installs with winget and downloads
/// the GHC compiler the project asks for.
pub fn detect_haskell_requirement(
    directory: &Path,
    project_root: &Path,
) -> Option<RuntimeRequirement> {
    find_in_project_ancestors(directory, project_root, "stack.yaml")?;
    Some(RuntimeRequirement {
        runtime: RuntimeKind::Haskell,
        command: "stack".into(),
        version: VersionConstraint::recommended("latest"),
        source: RequirementSource::Recommended,
        status: RequirementStatus::Pending,
    })
}

/// A Zig project with `build.zig` needs the `zig` compiler, which installs with winget.
pub fn detect_zig_requirement(directory: &Path, project_root: &Path) -> Option<RuntimeRequirement> {
    find_in_project_ancestors(directory, project_root, "build.zig")?;
    Some(RuntimeRequirement {
        runtime: RuntimeKind::Zig,
        command: "zig".into(),
        version: VersionConstraint::recommended("latest"),
        source: RequirementSource::Recommended,
        status: RequirementStatus::Pending,
    })
}

/// Clojure runs on the JVM: a `deps.edn` project also needs Java 17 or newer.
pub fn detect_clojure_java_requirement(
    directory: &Path,
    project_root: &Path,
) -> Option<RuntimeRequirement> {
    find_in_project_ancestors(directory, project_root, "deps.edn")?;
    Some(RuntimeRequirement {
        runtime: RuntimeKind::Java,
        command: "java".into(),
        version: VersionConstraint::recommended(">=17"),
        source: RequirementSource::Recommended,
        status: RequirementStatus::Pending,
    })
}

fn is_flutter_pubspec(contents: &str) -> bool {
    contents.lines().any(|line| {
        let line = line.trim();
        line == "flutter:" || line == "sdk: flutter"
    })
}

fn dart_sdk_constraint(contents: &str) -> Option<String> {
    let mut in_environment = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indentation = line.len() - line.trim_start().len();
        if indentation == 0 {
            in_environment = trimmed == "environment:";
            continue;
        }
        if in_environment && trimmed.starts_with("sdk:") {
            let value = trimmed
                .trim_start_matches("sdk:")
                .trim()
                .trim_matches(['\'', '"']);
            return (!value.is_empty()).then(|| value.to_owned());
        }
    }
    None
}

fn xml_tag_value(contents: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let value = contents.split_once(&start)?.1.split_once(&end)?.0.trim();
    (!value.is_empty() && !value.starts_with("${")).then(|| value.to_owned())
}

fn gradle_java_version(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        if !(line.contains("sourceCompatibility") || line.contains("languageVersion")) {
            return None;
        }
        line.split(|character: char| !character.is_ascii_digit())
            .find(|part| !part.is_empty())
            .map(str::to_owned)
    })
}

fn python_constraint_from_toml_text(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        matches!(key.trim(), "requires-python" | "python")
            .then(|| value.trim().trim_matches(['\'', '"']).trim().to_owned())
    })
}

pub fn preferred_node_package_manager(directory: &Path) -> &'static str {
    detect_node_package_manager(directory)
}

pub fn node_package_manager_spec(directory: &Path) -> String {
    let document = fs::read_to_string(directory.join("package.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok());
    if let Some(spec) = document
        .as_ref()
        .and_then(|document| document.get("packageManager"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| {
            matches!(
                value.split('@').next().unwrap_or_default(),
                "npm" | "pnpm" | "yarn" | "bun"
            )
        })
    {
        return spec.to_owned();
    }
    match detect_node_package_manager(directory) {
        "pnpm" => "pnpm@stable".into(),
        "yarn" => "yarn@stable".into(),
        "bun" => "bun@latest".into(),
        _ => "npm".into(),
    }
}

fn find_in_project_ancestors(
    directory: &Path,
    project_root: &Path,
    name: &str,
) -> Option<std::path::PathBuf> {
    let mut current = Some(directory);
    while let Some(path) = current {
        if !path.starts_with(project_root) {
            return None;
        }
        let candidate = path.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if path == project_root {
            return None;
        }
        current = path.parent();
    }
    None
}

fn parse_loose_version(value: &str) -> Option<Version> {
    let value = value.trim().trim_start_matches(['v', '=']);
    let numeric = value
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .find(|part| !part.is_empty())?;
    let mut parts = numeric.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    Some(Version::new(major, minor, patch))
}

fn normalize_node_constraint(value: &str) -> String {
    let trimmed = value.trim().trim_start_matches('v');
    if let Some(prefix) = trimmed
        .strip_suffix(".x")
        .or_else(|| trimmed.strip_suffix(".*"))
    {
        let dots = prefix.matches('.').count();
        return if dots == 0 {
            format!(
                ">={prefix}.0.0, <{}.0.0",
                prefix.parse::<u64>().unwrap_or(0) + 1
            )
        } else {
            format!("~{prefix}.0")
        };
    }
    if trimmed.chars().all(|character| character.is_ascii_digit()) {
        return format!("^{trimmed}.0.0");
    }
    trimmed
        .replace(',', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn detect(path: &Path) -> Vec<RuntimeKind> {
    let mut found = Vec::new();
    detect_in_directory(path, &mut found);

    if path.join(".git").exists() {
        let mut queue = VecDeque::from([(path.to_path_buf(), 0_usize)]);
        while let Some((directory, depth)) = queue.pop_front() {
            if depth > 0 {
                detect_in_directory(&directory, &mut found);
            }
            if depth >= 8 {
                continue;
            }
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && !name.starts_with('.')
                    && !entry.path().join(".localcodepilot-ignore").is_file()
                    && !IGNORED_DIRECTORIES
                        .iter()
                        .any(|ignored| name.eq_ignore_ascii_case(ignored))
                {
                    queue.push_back((entry.path(), depth + 1));
                }
            }
        }
    }
    found
}

pub fn detect_technologies(path: &Path) -> Vec<TechnologyKind> {
    let mut found = Vec::new();
    let runtimes = detect(path);
    if !runtimes.contains(&RuntimeKind::Php) && is_static_site_root(path, path) {
        push_unique(&mut found, TechnologyKind::StaticSite);
    }
    detect_technologies_in_directory(path, &mut found);
    // Phoenix keeps its JavaScript and TypeScript in `assets/`, built by Mix: they are part of
    // the Elixir project, not a frontend of their own.
    let mix_assets = path.join("mix.exs").is_file().then(|| path.join("assets"));

    if path.join(".git").exists() {
        let mut queue = VecDeque::from([(path.to_path_buf(), 0_usize)]);
        while let Some((directory, depth)) = queue.pop_front() {
            if mix_assets
                .as_ref()
                .is_some_and(|assets| directory.starts_with(assets))
            {
                continue;
            }
            if depth > 0 {
                detect_technologies_in_directory(&directory, &mut found);
            }
            if depth >= 8 {
                continue;
            }
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && !name.starts_with('.')
                    && !entry.path().join(".localcodepilot-ignore").is_file()
                    && !IGNORED_DIRECTORIES
                        .iter()
                        .any(|ignored| name.eq_ignore_ascii_case(ignored))
                {
                    queue.push_back((entry.path(), depth + 1));
                }
            }
        }
    }
    // Folders without Git are not walked above, so a Compose file in `docker/` would otherwise
    // leave a PHP site without its Containers action.
    if !found.contains(&TechnologyKind::Containers) && site_uses_compose(&found, path, path) {
        push_unique(&mut found, TechnologyKind::Containers);
    }
    found.sort_by_key(|technology| technology_priority(*technology));
    found
}

/// Whether the PHP site in `site` starts through Compose instead of `php -S`. Laravel and Lumen
/// keep their own servers.
fn site_uses_compose(technologies: &[TechnologyKind], project_root: &Path, site: &Path) -> bool {
    if technologies.contains(&TechnologyKind::WordPress) {
        containers::wordpress_uses_compose(project_root, site)
    } else if technologies.contains(&TechnologyKind::Laravel)
        || technologies.contains(&TechnologyKind::Lumen)
        || !site.join("index.php").is_file()
    {
        false
    } else {
        containers::php_site_uses_compose(project_root, site)
    }
}

pub fn detect_processes(project: &Project) -> Vec<ProjectProcess> {
    let mut processes = Vec::new();
    detect_processes_in_directory(project, &project.path, &mut processes);

    if project.path.join(".git").exists() {
        let mut queue = VecDeque::from([(project.path.clone(), 0_usize)]);
        while let Some((directory, depth)) = queue.pop_front() {
            if depth > 0 {
                detect_processes_in_directory(project, &directory, &mut processes);
            }
            if depth >= 8 {
                continue;
            }
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && !name.starts_with('.')
                    && !entry.path().join(".localcodepilot-ignore").is_file()
                    && !IGNORED_DIRECTORIES
                        .iter()
                        .any(|ignored| name.eq_ignore_ascii_case(ignored))
                {
                    queue.push_back((entry.path(), depth + 1));
                }
            }
        }
    }
    remove_redundant_laravel_frontend(&mut processes);
    // A project with containers starts through them (a PHP site served by Compose, for
    // example), so it gets no local fallback.
    if processes.is_empty() && containers::detect_container_projects(&project.path).is_empty() {
        detect_fallback_process(project, &mut processes);
    }
    processes
}

/// When no framework or manifest gives a command, the project still starts the plain way its
/// files allow, so "Iniciar" works without anyone typing a command: a Java file with `main`,
/// a folder of PHP pages or a folder of HTML pages.
fn detect_fallback_process(project: &Project, processes: &mut Vec<ProjectProcess>) {
    let root = &project.path;
    let files = fs::read_dir(root)
        .map(|entries| {
            let mut files = entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            files.sort();
            files
        })
        .unwrap_or_default();
    let with_extension = |extension: &str| {
        files
            .iter()
            .filter(|name| {
                Path::new(name.as_str())
                    .extension()
                    .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            })
            .cloned()
            .collect::<Vec<_>>()
    };

    // `java Main.java` runs a single-file program without a build tool (Java 11+).
    if let Some(source) = with_extension("java").into_iter().find(|name| {
        fs::read_to_string(root.join(name))
            .is_ok_and(|contents| contents.contains("static void main("))
    }) {
        push_process(
            processes,
            project,
            root,
            &format!("Executar {source}"),
            "java",
            &[&source],
        );
        return;
    }

    // PHP pages without `index.php`: the built-in server with a page that lists them.
    if !with_extension("php").is_empty()
        && let Some(router) = php_listing_router()
    {
        let router = router.to_string_lossy().into_owned();
        push_process(
            processes,
            project,
            root,
            "Servidor PHP",
            "php",
            &["-S", "localhost:8000", &router],
        );
        return;
    }

    // HTML pages: any page at the root, or an `index.html` one or two folders down. The
    // static server lists the folder, so every page can be opened.
    let html_below = || {
        fs::read_dir(root).ok().is_some_and(|entries| {
            entries.flatten().any(|entry| {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                path.is_dir()
                    && !name.starts_with('.')
                    && !IGNORED_DIRECTORIES
                        .iter()
                        .any(|ignored| name == ignored.to_ascii_lowercase())
                    && (path.join("index.html").is_file()
                        || fs::read_dir(&path).ok().is_some_and(|children| {
                            children
                                .flatten()
                                .any(|child| child.path().join("index.html").is_file())
                        }))
            })
        })
    };
    if !with_extension("html").is_empty() || !with_extension("htm").is_empty() || html_below() {
        let port = static_site_port(root).to_string();
        push_process(
            processes,
            project,
            root,
            "Site estático",
            "python",
            &["-m", "http.server", port.as_str(), "--bind", "127.0.0.1"],
        );
    }
}

/// A router for `php -S` that answers `/` with a list of the folder's PHP files and leaves
/// every other request to the server. It lives in the temporary folder, outside the project.
fn php_listing_router() -> Option<std::path::PathBuf> {
    const ROUTER: &str = r#"<?php
// LocalCodePilot: lista as páginas PHP de uma pasta sem index.php.
$path = parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH);
if ($path !== '/') {
    return false;
}
$files = glob('*.php');
sort($files, SORT_NATURAL | SORT_FLAG_CASE);
echo '<!doctype html><meta charset="utf-8"><title>Páginas PHP</title>';
echo '<style>body{font-family:system-ui,sans-serif;margin:2rem}li{margin:.4rem 0}</style>';
echo '<h1>Páginas PHP deste projeto</h1><ul>';
foreach ($files as $file) {
    echo '<li><a href="/' . rawurlencode($file) . '">' . htmlspecialchars($file) . '</a></li>';
}
echo '</ul>';
"#;
    let folder = std::env::temp_dir().join("localcodepilot");
    let router = folder.join("php-listing-router.php");
    if fs::read_to_string(&router).ok().as_deref() != Some(ROUTER) {
        fs::create_dir_all(&folder).ok()?;
        fs::write(&router, ROUTER).ok()?;
    }
    Some(router)
}

fn remove_redundant_laravel_frontend(processes: &mut Vec<ProjectProcess>) {
    let has_standalone_frontend = processes.iter().any(|process| {
        matches!(process.program.as_str(), "npm" | "pnpm" | "yarn" | "bun")
            && process.name != "Frontend"
            && !process.working_directory.join("artisan").is_file()
    });
    if has_standalone_frontend {
        processes.retain(|process| {
            process.name != "Frontend" || !process.working_directory.join("artisan").is_file()
        });
    }
}

fn detect_processes_in_directory(
    project: &Project,
    directory: &Path,
    processes: &mut Vec<ProjectProcess>,
) {
    let artisan_root = marker_ancestor(directory, &project.path, "artisan");
    let belongs_to_artisan_app = artisan_root.is_some();
    let php_framework = artisan_root.and_then(detect_php_framework);
    detect_cargo_process(project, directory, processes);
    detect_go_process(project, directory, processes);
    detect_dotnet_process(project, directory, processes);
    detect_deno_process(project, directory, processes);
    detect_elixir_process(project, directory, processes);
    detect_swift_process(project, directory, processes);
    detect_scala_process(project, directory, processes);
    detect_clojure_process(project, directory, processes);
    detect_haskell_process(project, directory, processes);
    detect_zig_process(project, directory, processes);
    detect_ruby_process(project, directory, processes);
    detect_dart_or_flutter_process(project, directory, processes);
    if belongs_to_artisan_app {
        detect_laravel_frontend_process(project, directory, processes);
    } else {
        let package_manager = detect_node_package_manager(directory);
        detect_json_scripts(
            project,
            directory,
            "package.json",
            package_manager,
            &["run"],
            processes,
        );
    }
    if !belongs_to_artisan_app {
        detect_json_scripts(
            project,
            directory,
            "composer.json",
            "composer",
            &["run-script"],
            processes,
        );
    }
    if directory.join("artisan").is_file() {
        if php_framework == Some(TechnologyKind::Lumen) {
            push_process(
                processes,
                project,
                directory,
                "Servidor Lumen",
                "php",
                &["-S", "localhost:8000", "-t", "public"],
            );
        } else {
            push_process(
                processes,
                project,
                directory,
                "Servidor Laravel",
                "php",
                &["artisan", "serve"],
            );
        }
    } else if !belongs_to_artisan_app
        && (directory.join("index.php").is_file()
            || directory.join("wp-config.php").is_file()
            || directory.join("wp-config-sample.php").is_file()
            || (directory == project.path
                && project.runtimes.contains(&RuntimeKind::Php)
                && directory.join("index.html").is_file()))
        // Checked last: it scans the project for Compose manifests.
        && !site_uses_compose(&project.technologies, &project.path, directory)
    {
        push_process(
            processes,
            project,
            directory,
            "Servidor PHP",
            "php",
            &["-S", "localhost:8000"],
        );
    }
    for (file, name, args) in [
        (
            "manage.py",
            "Servidor Django",
            vec!["manage.py", "runserver"],
        ),
        ("main.py", "Executar main.py", vec!["main.py"]),
        ("app.py", "Executar app.py", vec!["app.py"]),
    ] {
        if directory.join(file).is_file() {
            let virtual_python = if cfg!(windows) {
                directory.join(".venv").join("Scripts").join("python.exe")
            } else {
                directory.join(".venv").join("bin").join("python")
            };
            let program = if virtual_python.is_file() {
                virtual_python.to_string_lossy().into_owned()
            } else {
                "python".into()
            };
            push_process(processes, project, directory, name, &program, &args);
        }
    }
    if directory == project.path
        && !project.runtimes.contains(&RuntimeKind::Php)
        && is_static_site_root(directory, &project.path)
    {
        let port = static_site_port(&project.path).to_string();
        push_process(
            processes,
            project,
            directory,
            "Site estático",
            "python",
            &["-m", "http.server", port.as_str(), "--bind", "127.0.0.1"],
        );
    }
    detect_java_process(project, directory, processes);
}

fn static_site_port(project_path: &Path) -> u16 {
    let mut hash = 0x811c_9dc5_u32;
    for byte in project_path.to_string_lossy().to_ascii_lowercase().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    4200 + (hash % 4000) as u16
}

fn go_main_package(directory: &Path) -> bool {
    fs::read_dir(directory).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            let path = entry.path();
            path.extension().is_some_and(|extension| extension == "go")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with("_test.go"))
                && fs::read_to_string(path).ok().is_some_and(|contents| {
                    contents.lines().any(|line| line.trim() == "package main")
                })
        })
    })
}

/// `go run .` for a program at the module root; otherwise one command per program in the
/// common `cmd/<name>/` layout, run from the module root as `go run ./cmd/<name>`.
fn detect_go_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    if !directory.join("go.mod").is_file() {
        return;
    }
    let mut programs = Vec::new();
    if go_main_package(directory) {
        programs.push((
            "Executar aplicação Go".to_owned(),
            ".".to_owned(),
            directory.to_path_buf(),
        ));
    } else if let Ok(entries) = fs::read_dir(directory.join("cmd")) {
        let mut folders = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && go_main_package(path))
            .collect::<Vec<_>>();
        folders.sort();
        for folder in folders {
            let name = folder
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            programs.push((format!("Executar {name}"), format!("./cmd/{name}"), folder));
        }
    }
    for (name, package, source) in programs {
        push_process(
            processes,
            project,
            directory,
            &name,
            "go",
            &["run", &package],
        );
        if let Some(port) = detect_go_server_port(&source)
            && let Some(process) = processes.last_mut()
        {
            process.expected_port = Port::new(port);
        }
    }
}

fn detect_go_server_port(directory: &Path) -> Option<u16> {
    fs::read_dir(directory)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            path.extension()
                .is_some_and(|extension| extension == "go")
                .then(|| fs::read_to_string(path).ok())
                .flatten()
        })
        .find_map(|contents| {
            contents.split("ListenAndServe").skip(1).find_map(|call| {
                let address = call.split('"').nth(1)?;
                address
                    .rsplit_once(':')
                    .map_or(address, |(_, port)| port)
                    .parse()
                    .ok()
            })
        })
}

const DOTNET_PROJECT_EXTENSIONS: [&str; 3] = ["csproj", "fsproj", "vbproj"];

/// A .NET project that `dotnet run` can start: a web app (`Microsoft.NET.Sdk.Web`, Blazor) or
/// a console or worker program (`<OutputType>Exe</OutputType>`). Libraries and test projects
/// are left out.
fn dotnet_runnable(project_file: &Path) -> Option<bool> {
    let contents = fs::read_to_string(project_file).ok()?.to_ascii_lowercase();
    if contents.contains("microsoft.net.test.sdk")
        || contents.contains("<istestproject>true</istestproject>")
    {
        return None;
    }
    let web = contents.contains("microsoft.net.sdk.web")
        || contents.contains("microsoft.net.sdk.blazorwebassembly")
        || contents.contains("microsoft.net.sdk.razor");
    let executable = contents.contains("<outputtype>exe</outputtype>")
        || contents.contains("<outputtype>winexe</outputtype>")
        || contents.contains("microsoft.net.sdk.worker");
    (web || executable).then_some(web)
}

/// The first http address of the `Project` profiles in `Properties/launchSettings.json`, the
/// one `dotnet run` uses by default.
fn dotnet_launch_port(project_file: &Path) -> Option<Port> {
    let settings = project_file
        .parent()?
        .join("Properties/launchSettings.json");
    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(settings).ok()?).ok()?;
    settings
        .get("profiles")?
        .as_object()?
        .values()
        .filter(|profile| {
            profile.get("commandName").and_then(|name| name.as_str()) == Some("Project")
        })
        .filter_map(|profile| profile.get("applicationUrl")?.as_str())
        .flat_map(|urls| urls.split(';'))
        .find_map(|url| {
            let rest = url.trim().strip_prefix("http://")?;
            let port = rest.split('/').next()?.rsplit_once(':')?.1;
            Port::new(port.parse().ok()?)
        })
}

/// `dotnet run --project <file>` for every runnable project in this folder, and for the
/// projects a solution (`.sln`) lists in its subfolders. A project already started from a
/// solution higher up is not listed twice.
fn detect_dotnet_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();
    let has_extension = |path: &Path, extensions: &[&str]| {
        path.extension().is_some_and(|extension| {
            extensions
                .iter()
                .any(|wanted| extension.eq_ignore_ascii_case(wanted))
        })
    };
    let mut project_files = files
        .iter()
        .filter(|path| has_extension(path, &DOTNET_PROJECT_EXTENSIONS))
        .cloned()
        .collect::<Vec<_>>();
    for solution in files.iter().filter(|path| has_extension(path, &["sln"])) {
        let Ok(contents) = fs::read_to_string(solution) else {
            continue;
        };
        for line in contents.lines().filter(|line| line.starts_with("Project(")) {
            let Some(relative) = line.split('"').nth(5) else {
                continue;
            };
            let path = directory.join(relative.replace('\\', "/"));
            if has_extension(&path, &DOTNET_PROJECT_EXTENSIONS) && path.is_file() {
                project_files.push(path);
            }
        }
    }
    for project_file in project_files {
        let already_listed = processes.iter().any(|process| {
            process.program == "dotnet"
                && process.args.last().is_some_and(|file| {
                    localcodepilot_core::containers::same_folder(
                        &process.working_directory.join(file),
                        &project_file,
                    )
                })
        });
        let Some(web) = dotnet_runnable(&project_file).filter(|_| !already_listed) else {
            continue;
        };
        let relative = project_file
            .strip_prefix(directory)
            .unwrap_or(&project_file)
            .to_string_lossy()
            .replace('\\', "/");
        let stem = project_file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = if web {
            format!("Servidor ASP.NET ({stem})")
        } else {
            format!("Executar {stem}")
        };
        push_process(
            processes,
            project,
            directory,
            &name,
            "dotnet",
            &["run", "--project", &relative],
        );
        if web && let Some(process) = processes.last_mut() {
            process.expected_port = dotnet_launch_port(&project_file);
        }
    }
}

/// Ruby web apps and programs, in the order their conventions are checked:
/// - Rails (`bin/rails`): `ruby bin/rails server`, port 3000;
/// - Rack (`config.ru`): `bundle exec rackup`, port 9292;
/// - Sinatra (`app.rb` that requires it): `ruby app.rb`, port 4567;
/// - a program (`main.rb` or `app.rb`): `ruby <file>`.
///
/// With a Gemfile, `bundle exec` loads the project's gems.
fn detect_ruby_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    let gemfile = directory.join("Gemfile").is_file();
    let mut push = |name: &str, program: &str, args: &[&str], port: Option<u16>| {
        push_process(processes, project, directory, name, program, args);
        if let (Some(port), Some(process)) = (port, processes.last_mut()) {
            process.expected_port = Port::new(port);
        }
    };
    if directory.join("bin/rails").is_file() && directory.join("config/application.rb").is_file() {
        push(
            "Servidor Rails",
            "ruby",
            &["bin/rails", "server"],
            Some(3000),
        );
        return;
    }
    if directory.join("config.ru").is_file() {
        if gemfile {
            push("Servidor Rack", "bundle", &["exec", "rackup"], Some(9292));
        } else {
            push("Servidor Rack", "rackup", &[], Some(9292));
        }
        return;
    }
    let script = ["app.rb", "main.rb"]
        .into_iter()
        .find(|name| directory.join(name).is_file());
    let Some(script) = script else {
        return;
    };
    let sinatra = fs::read_to_string(directory.join(script)).is_ok_and(|contents| {
        contents.contains("require 'sinatra'") || contents.contains("require \"sinatra\"")
    });
    let (name, port) = if sinatra {
        ("Servidor Sinatra".to_owned(), Some(4567))
    } else {
        (format!("Executar {script}"), None)
    };
    if gemfile {
        push(&name, "bundle", &["exec", "ruby", script], port);
    } else {
        push(&name, "ruby", &[script], port);
    }
}

/// Deno: the long-running tasks of `deno.json` (`deno task dev`); without tasks, the entry
/// file (`main.ts`, `server.ts`, `main.js`) with network, read and environment access, the
/// permissions a local web server needs.
fn detect_deno_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    let Some(manifest) = ["deno.json", "deno.jsonc"]
        .into_iter()
        .find(|name| directory.join(name).is_file())
    else {
        return;
    };
    let before = processes.len();
    detect_json_commands(
        project,
        directory,
        manifest,
        "tasks",
        "deno",
        &["task"],
        processes,
    );
    if processes.len() > before {
        return;
    }
    if let Some(entry) = ["main.ts", "server.ts", "main.js", "server.js"]
        .into_iter()
        .find(|name| directory.join(name).is_file())
    {
        push_process(
            processes,
            project,
            directory,
            &format!("Executar {entry}"),
            "deno",
            &["run", "--allow-net", "--allow-read", "--allow-env", entry],
        );
    }
}

/// Elixir with Mix: Phoenix serves with `mix phx.server` (port 4000); an application with a
/// supervision tree (`mod:` in `mix.exs`, such as Plug or Bandit) runs with
/// `mix run --no-halt`. Libraries have nothing to start.
fn detect_elixir_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    let Ok(mix) = fs::read_to_string(directory.join("mix.exs")) else {
        return;
    };
    if mix.contains("{:phoenix,") {
        push_process(
            processes,
            project,
            directory,
            "Servidor Phoenix",
            "mix",
            &["phx.server"],
        );
        if let Some(process) = processes.last_mut() {
            process.expected_port = Port::new(4000);
        }
    } else if mix.contains("mod:") {
        push_process(
            processes,
            project,
            directory,
            "Executar aplicação Elixir",
            "mix",
            &["run", "--no-halt"],
        );
    }
}

/// Swift Package Manager: a package with an executable target runs with `swift run`; a Vapor
/// server listens on port 8080. Libraries have nothing to start.
fn detect_swift_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    let Ok(manifest) = fs::read_to_string(directory.join("Package.swift")) else {
        return;
    };
    if !manifest.contains(".executableTarget(") && !manifest.contains(".executable(") {
        return;
    }
    let vapor = manifest.contains("vapor/vapor");
    let name = if vapor {
        "Servidor Vapor"
    } else {
        "Executar pacote Swift"
    };
    push_process(processes, project, directory, name, "swift", &["run"]);
    if vapor && let Some(process) = processes.last_mut() {
        process.expected_port = Port::new(8080);
    }
}

/// Scala with sbt. sbt 2 runs as a thin client that leaves a server in the background, which
/// the app could not stop; `--server` keeps everything in the process the app started. sbt 1
/// already works that way and does not know the option. A Play application (its sbt plugin
/// in `project/plugins.sbt` or `PlayScala` in the build) serves on port 9000.
fn detect_scala_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    let Ok(build) = fs::read_to_string(directory.join("build.sbt")) else {
        return;
    };
    let plugins = fs::read_to_string(directory.join("project/plugins.sbt")).unwrap_or_default();
    let play = build.contains("PlayScala")
        || build.contains("PlayJava")
        || plugins.contains("com.typesafe.play")
        || plugins.contains("org.playframework");
    let name = if play {
        "Servidor Play"
    } else {
        "Executar projeto sbt"
    };
    let sbt_1 =
        fs::read_to_string(directory.join("project/build.properties")).is_ok_and(|properties| {
            properties.lines().any(|line| {
                line.split_once('=').is_some_and(|(key, value)| {
                    key.trim() == "sbt.version" && value.trim().starts_with("1.")
                })
            })
        });
    let args: &[&str] = if sbt_1 {
        &["run"]
    } else {
        &["--server", "run"]
    };
    push_process(processes, project, directory, name, "sbt", args);
    if play && let Some(process) = processes.last_mut() {
        process.expected_port = Port::new(9000);
    }
}

/// Clojure with `deps.edn`: the first alias meant to run the program (`:run`, `:dev`,
/// `:serve` or `:server`) that has `:main-opts`, through Babashka's Clojure CLI
/// (`bb clojure -M:run`). Without such an alias there is no convention to follow.
fn detect_clojure_process(
    project: &Project,
    directory: &Path,
    processes: &mut Vec<ProjectProcess>,
) {
    let Ok(deps) = fs::read_to_string(directory.join("deps.edn")) else {
        return;
    };
    let compact = deps.split_whitespace().collect::<Vec<_>>().join(" ");
    let Some(alias) = ["run", "dev", "serve", "server"].into_iter().find(|alias| {
        compact
            .split(&format!(":{alias} {{"))
            .nth(1)
            .is_some_and(|rest| {
                rest.split('}')
                    .next()
                    .is_some_and(|body| body.contains(":main-opts"))
            })
    }) else {
        return;
    };
    let option = format!("-M:{alias}");
    push_process(
        processes,
        project,
        directory,
        "Executar projeto Clojure",
        "bb",
        &["clojure", &option],
    );
}

/// Haskell with Stack: `stack run` builds and runs the project's executable. Projects with
/// only Cabal (no `stack.yaml`) are not covered.
fn detect_haskell_process(
    project: &Project,
    directory: &Path,
    processes: &mut Vec<ProjectProcess>,
) {
    if !directory.join("stack.yaml").is_file() {
        return;
    }
    let executable = fs::read_dir(directory).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            (name == "package.yaml" || name.ends_with(".cabal"))
                && fs::read_to_string(entry.path())
                    .is_ok_and(|contents| contents.contains("executable"))
        })
    });
    if executable {
        push_process(
            processes,
            project,
            directory,
            "Executar projeto Haskell",
            "stack",
            &["run"],
        );
    }
}

/// Zig: `zig build run` when `build.zig` declares the `run` step, as `zig init` does.
fn detect_zig_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    let Ok(build) = fs::read_to_string(directory.join("build.zig")) else {
        return;
    };
    if build.contains("step(\"run\"") {
        push_process(
            processes,
            project,
            directory,
            "Executar projeto Zig",
            "zig",
            &["build", "run"],
        );
    }
}

fn detect_dart_or_flutter_process(
    project: &Project,
    directory: &Path,
    processes: &mut Vec<ProjectProcess>,
) {
    let Ok(contents) = fs::read_to_string(directory.join("pubspec.yaml")) else {
        return;
    };
    if is_flutter_pubspec(&contents) {
        if directory.join("web").is_dir() {
            push_process(
                processes,
                project,
                directory,
                "Flutter Web",
                "flutter",
                &[
                    "run",
                    "-d",
                    "web-server",
                    "--web-hostname",
                    "127.0.0.1",
                    "--web-port",
                    "4180",
                ],
            );
        } else if cfg!(target_os = "windows") && directory.join("windows").is_dir() {
            push_process(
                processes,
                project,
                directory,
                "Flutter para Windows",
                "flutter",
                &["run", "-d", "windows"],
            );
        }
        return;
    }

    // `dart create` writes `bin/<package>.dart`, and its server templates `bin/server.dart`.
    let package = contents.lines().find_map(|line| {
        line.strip_prefix("name:")
            .map(|name| name.trim().trim_matches(['\'', '"']).to_owned())
    });
    let package_entrypoint = package.map(|name| format!("bin/{name}.dart"));
    let entrypoint = ["bin/main.dart", "bin/server.dart"]
        .into_iter()
        .map(str::to_owned)
        .chain(package_entrypoint)
        .chain(["main.dart".to_owned(), "lib/main.dart".to_owned()])
        .find(|path| directory.join(path).is_file());
    if let Some(entrypoint) = entrypoint {
        push_process(
            processes,
            project,
            directory,
            "Executar aplicação Dart",
            "dart",
            &["run", &entrypoint],
        );
    }
}

fn is_static_site_root(directory: &Path, project_root: &Path) -> bool {
    if !directory.join("index.html").is_file()
        || [
            "package.json",
            "composer.json",
            "Cargo.toml",
            "go.mod",
            "pubspec.yaml",
        ]
        .iter()
        .any(|marker| has_marker_in_ancestors(directory, project_root, marker))
    {
        return false;
    }
    directory == project_root
        || !directory
            .parent()
            .is_some_and(|parent| has_marker_in_ancestors(parent, project_root, "index.html"))
}

fn detect_java_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    let pom = fs::read_to_string(directory.join("pom.xml")).ok();
    if pom
        .as_deref()
        .is_some_and(|contents| contents.contains("spring-boot"))
    {
        let wrapper = if cfg!(windows) { "mvnw.cmd" } else { "mvnw" };
        let program = if directory.join(wrapper).is_file() {
            localcodepilot_core::paths::display_path(&directory.join(wrapper))
        } else {
            "mvn".into()
        };
        push_process(
            processes,
            project,
            directory,
            "Servidor Spring Boot",
            &program,
            &["spring-boot:run"],
        );
        return;
    }

    let Some(gradle) = ["build.gradle", "build.gradle.kts"]
        .iter()
        .find_map(|name| fs::read_to_string(directory.join(name)).ok())
    else {
        return;
    };
    let wrapper = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    };
    let program = if directory.join(wrapper).is_file() {
        localcodepilot_core::paths::display_path(&directory.join(wrapper))
    } else {
        "gradle".into()
    };
    // Without the daemon the build ends with the server: a start that fails would
    // otherwise leave a Gradle daemon running for hours.
    if gradle.contains("org.springframework.boot") || gradle.contains("bootRun") {
        push_process(
            processes,
            project,
            directory,
            "Servidor Spring Boot",
            &program,
            &["bootRun", "--no-daemon"],
        );
    } else if let Some(name) = gradle_application(&gradle) {
        push_process(
            processes,
            project,
            directory,
            name,
            &program,
            &["run", "--no-daemon"],
        );
        if name == "Servidor Ktor"
            && let Some(process) = processes.last_mut()
        {
            process.expected_port = Port::new(8080);
        }
    }
}

/// A Gradle build with the `application` plugin (Kotlin or Java) starts with `gradle run`;
/// Ktor adds its own plugin and serves on port 8080 by default.
fn gradle_application(build: &str) -> Option<&'static str> {
    let compact = build.replace([' ', '\t'], "");
    if compact.contains("io.ktor.plugin") || compact.contains("io.ktor:ktor-server") {
        return Some("Servidor Ktor");
    }
    [
        "id(\"application\")",
        "id'application'",
        "id\"application\"",
        "applyplugin:'application'",
        "applyplugin:\"application\"",
        "\napplication\n",
    ]
    .iter()
    .any(|pattern| compact.contains(pattern))
    .then_some("Executar aplicação")
}

fn detect_laravel_frontend_process(
    project: &Project,
    directory: &Path,
    processes: &mut Vec<ProjectProcess>,
) {
    let Ok(contents) = fs::read_to_string(directory.join("package.json")) else {
        return;
    };
    let Ok(document) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return;
    };
    let Some(command) = document
        .get("scripts")
        .and_then(|scripts| scripts.get("dev"))
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let normalized_command = command.to_ascii_lowercase();
    let uses_laravel_frontend_tooling = normalized_command.contains("vite")
        || normalized_command.contains("mix")
        || normalized_command.contains("webpack")
        || package_has_dependency(&document, "laravel-vite-plugin")
        || package_has_dependency(&document, "laravel-mix");

    if uses_laravel_frontend_tooling {
        let package_manager = node_package_manager(directory, &document);
        push_process_with_source(
            processes,
            project,
            directory,
            "Frontend",
            package_manager,
            &["run", "dev"],
            Some(command),
        );
    }
}

fn detect_node_package_manager(directory: &Path) -> &'static str {
    let document = fs::read_to_string(directory.join("package.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok());
    document.as_ref().map_or_else(
        || node_package_manager(directory, &serde_json::Value::Null),
        |document| node_package_manager(directory, document),
    )
}

fn node_package_manager(directory: &Path, document: &serde_json::Value) -> &'static str {
    if let Some(manager) = document
        .get("packageManager")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.split('@').next())
    {
        match manager.to_ascii_lowercase().as_str() {
            "pnpm" => return "pnpm",
            "yarn" => return "yarn",
            "bun" => return "bun",
            "npm" => return "npm",
            _ => {}
        }
    }

    if directory.join("bun.lock").is_file() || directory.join("bun.lockb").is_file() {
        "bun"
    } else if directory.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if directory.join("yarn.lock").is_file() {
        "yarn"
    } else {
        "npm"
    }
}

fn package_has_dependency(document: &serde_json::Value, dependency: &str) -> bool {
    ["dependencies", "devDependencies"]
        .iter()
        .filter_map(|section| document.get(section).and_then(serde_json::Value::as_object))
        .any(|dependencies| dependencies.contains_key(dependency))
}

fn detect_cargo_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    let Some(manifest) = read_cargo_manifest(directory) else {
        return;
    };

    // Um membro de workspace possui seu próprio Cargo.toml, mas deve ser
    // executado por meio do manifesto que coordena o workspace.
    if has_cargo_workspace_ancestor(directory, &project.path) {
        return;
    }

    if let Some(workspace) = manifest.get("workspace").and_then(toml::Value::as_table) {
        if workspace_has_runnable_default_member(directory, workspace) {
            push_process(
                processes,
                project,
                directory,
                "Executar workspace",
                "cargo",
                &["run"],
            );
        } else if workspace.get("default-members").is_none()
            && cargo_package_is_runnable(directory, &manifest)
        {
            push_process(
                processes,
                project,
                directory,
                "Executar projeto",
                "cargo",
                &["run"],
            );
        }
        return;
    }

    if cargo_package_is_runnable(directory, &manifest) {
        push_process(
            processes,
            project,
            directory,
            "Executar projeto",
            "cargo",
            &["run"],
        );
    }
}

fn read_cargo_manifest(directory: &Path) -> Option<toml::Value> {
    let contents = fs::read_to_string(directory.join("Cargo.toml")).ok()?;
    toml::from_str(&contents).ok()
}

fn cargo_package_is_runnable(directory: &Path, manifest: &toml::Value) -> bool {
    let Some(package) = manifest.get("package").and_then(toml::Value::as_table) else {
        return false;
    };
    let has_explicit_binary = manifest
        .get("bin")
        .and_then(toml::Value::as_array)
        .is_some_and(|binaries| !binaries.is_empty());
    let automatic_binaries_enabled = package
        .get("autobins")
        .and_then(toml::Value::as_bool)
        .unwrap_or(true);

    has_explicit_binary || (automatic_binaries_enabled && directory.join("src/main.rs").is_file())
}

fn workspace_has_runnable_default_member(
    workspace_root: &Path,
    workspace: &toml::value::Table,
) -> bool {
    let Some(default_members) = workspace
        .get("default-members")
        .and_then(toml::Value::as_array)
    else {
        return false;
    };
    let [member] = default_members.as_slice() else {
        return false;
    };
    let Some(member) = member.as_str() else {
        return false;
    };
    if member.contains(['*', '?', '[', ']']) {
        return false;
    }

    let member_directory = workspace_root.join(member);
    read_cargo_manifest(&member_directory)
        .is_some_and(|manifest| cargo_package_is_runnable(&member_directory, &manifest))
}

fn has_cargo_workspace_ancestor(directory: &Path, project_root: &Path) -> bool {
    let mut current = directory.parent();
    while let Some(path) = current {
        if !path.starts_with(project_root) {
            break;
        }
        if read_cargo_manifest(path).is_some_and(|manifest| manifest.get("workspace").is_some()) {
            return true;
        }
        if path == project_root {
            break;
        }
        current = path.parent();
    }
    false
}

fn detect_json_scripts(
    project: &Project,
    directory: &Path,
    manifest: &str,
    program: &str,
    prefix: &[&str],
    processes: &mut Vec<ProjectProcess>,
) {
    detect_json_commands(
        project, directory, manifest, "scripts", program, prefix, processes,
    );
}

/// One process per long-running entry of `key` in a JSON manifest: `scripts` in
/// `package.json`, `tasks` in `deno.json`. Whole-line `//` comments (`deno.jsonc`) are
/// ignored.
fn detect_json_commands(
    project: &Project,
    directory: &Path,
    manifest: &str,
    key: &str,
    program: &str,
    prefix: &[&str],
    processes: &mut Vec<ProjectProcess>,
) {
    let Ok(contents) = fs::read_to_string(directory.join(manifest)) else {
        return;
    };
    let contents = contents
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(document) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return;
    };
    let Some(scripts) = document.get(key).and_then(|value| value.as_object()) else {
        return;
    };
    let mut names: Vec<_> = scripts
        .iter()
        .filter(|(name, _)| !name.starts_with("pre") && !name.starts_with("post"))
        .filter(|(name, _)| {
            name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':' | '.')
            })
        })
        .filter(|(name, command)| is_continuous_script(name, command))
        .map(|(name, _)| name)
        .collect();
    names.sort();
    for name in names {
        let mut args = prefix.to_vec();
        args.push(name);
        let source_command = scripts.get(name).and_then(serde_json::Value::as_str);
        push_process_with_source(
            processes,
            project,
            directory,
            name,
            program,
            &args,
            source_command,
        );
    }
}

fn is_continuous_script(name: &str, command: &serde_json::Value) -> bool {
    let normalized_name = name.to_ascii_lowercase();
    if CONTINUOUS_SCRIPT_NAMES.iter().any(|candidate| {
        normalized_name == *candidate
            || normalized_name.starts_with(&format!("{candidate}:"))
            || normalized_name.starts_with(&format!("{candidate}-"))
    }) {
        return true;
    }

    command.as_str().is_some_and(|command| {
        let command = command.to_ascii_lowercase();
        command.contains("--watch") || command.contains("webpack serve")
    })
}

fn has_marker_in_ancestors(directory: &Path, project_root: &Path, marker: &str) -> bool {
    marker_ancestor(directory, project_root, marker).is_some()
}

fn marker_ancestor<'a>(directory: &'a Path, project_root: &Path, marker: &str) -> Option<&'a Path> {
    let mut current = Some(directory);
    while let Some(path) = current {
        if !path.starts_with(project_root) {
            break;
        }
        if path.join(marker).is_file() {
            return Some(path);
        }
        if path == project_root {
            break;
        }
        current = path.parent();
    }
    None
}

fn push_process(
    processes: &mut Vec<ProjectProcess>,
    project: &Project,
    working_directory: &Path,
    name: &str,
    program: &str,
    args: &[&str],
) {
    push_process_with_source(
        processes,
        project,
        working_directory,
        name,
        program,
        args,
        None,
    );
}

fn push_process_with_source(
    processes: &mut Vec<ProjectProcess>,
    project: &Project,
    working_directory: &Path,
    name: &str,
    program: &str,
    args: &[&str],
    source_command: Option<&str>,
) {
    let args: Vec<_> = args.iter().map(|value| (*value).to_owned()).collect();
    let command = std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let expected_port = infer_expected_port(program, &args, source_command);
    let port_override = infer_port_override(program, &args, source_command, expected_port);
    processes.push(ProjectProcess {
        id: format!("{}::{command}", working_directory.display()),
        project_name: project.name.clone(),
        project_path: project.path.clone(),
        working_directory: working_directory.to_path_buf(),
        name: name.to_owned(),
        program: program.to_owned(),
        args,
        state: ProcessState::Stopped,
        process_id: None,
        exit_code: None,
        expected_port,
        port_override,
    });
}

fn infer_port_override(
    program: &str,
    args: &[String],
    source_command: Option<&str>,
    expected_port: Option<Port>,
) -> Option<PortOverride> {
    expected_port?;
    if program.eq_ignore_ascii_case("php")
        && args.first().is_some_and(|argument| argument == "artisan")
        && args.get(1).is_some_and(|argument| argument == "serve")
    {
        return Some(PortOverride::LongOption { separator: false });
    }
    if program.eq_ignore_ascii_case("python")
        && args.first().is_some_and(|argument| argument == "manage.py")
        && args.get(1).is_some_and(|argument| argument == "runserver")
    {
        return Some(PortOverride::Positional);
    }
    if program.eq_ignore_ascii_case("php")
        && args.first().is_some_and(|argument| argument == "-S")
        && args.get(1).is_some_and(|argument| argument.contains(':'))
    {
        return Some(PortOverride::PhpServerAddress { argument_index: 1 });
    }
    if program.eq_ignore_ascii_case("python")
        && args.first().is_some_and(|argument| argument == "-m")
        && args
            .get(1)
            .is_some_and(|argument| argument == "http.server")
    {
        return Some(PortOverride::ArgumentValue { argument_index: 2 });
    }
    if program.eq_ignore_ascii_case("flutter")
        && let Some(argument_index) = args.iter().position(|argument| argument == "--web-port")
        && args.get(argument_index + 1).is_some()
    {
        return Some(PortOverride::ArgumentValue {
            argument_index: argument_index + 1,
        });
    }
    if source_command.is_some() && matches!(program.to_ascii_lowercase().as_str(), "npm" | "pnpm") {
        return Some(PortOverride::LongOption { separator: true });
    }
    if source_command.is_some() && matches!(program.to_ascii_lowercase().as_str(), "yarn" | "bun") {
        return Some(PortOverride::LongOption { separator: false });
    }
    None
}

fn infer_expected_port(
    program: &str,
    args: &[String],
    source_command: Option<&str>,
) -> Option<Port> {
    let invocation = std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(port) = source_command
        .and_then(explicit_port)
        .or_else(|| explicit_port(&invocation))
    {
        return Port::new(port);
    }

    let detected_command = source_command.unwrap_or(&invocation).to_ascii_lowercase();
    let static_server_port = (program.eq_ignore_ascii_case("python")
        && args.first().is_some_and(|argument| argument == "-m")
        && args
            .get(1)
            .is_some_and(|argument| argument == "http.server"))
    .then(|| args.get(2)?.parse().ok())
    .flatten();
    let uses_default_port_8000 = (program.eq_ignore_ascii_case("php")
        && args.first().is_some_and(|argument| argument == "artisan")
        && args.get(1).is_some_and(|argument| argument == "serve"))
        || (program.eq_ignore_ascii_case("python")
            && args.first().is_some_and(|argument| argument == "manage.py")
            && args.get(1).is_some_and(|argument| argument == "runserver"));
    let default_port = if static_server_port.is_some() {
        static_server_port
    } else if uses_default_port_8000 {
        Some(8000)
    } else if detected_command.contains("vite") {
        Some(5173)
    } else if detected_command.contains("next dev")
        || detected_command.contains("react-scripts start")
        || detected_command.contains("nuxt")
    {
        Some(3000)
    } else if detected_command.contains("astro dev") {
        Some(4321)
    } else if detected_command.contains("webpack serve") {
        Some(8080)
    } else {
        None
    };
    default_port.and_then(Port::new)
}

fn explicit_port(command: &str) -> Option<u16> {
    let tokens: Vec<_> = command
        .split_whitespace()
        .map(|token| token.trim_matches(['"', '\'', ',', ';']))
        .collect();
    for (index, token) in tokens.iter().enumerate() {
        if let Some(port) = token
            .strip_prefix("--port=")
            .or_else(|| token.strip_prefix("-p="))
            .and_then(|value| value.parse().ok())
        {
            return Some(port);
        }
        if matches!(*token, "--port" | "--web-port" | "-p")
            && let Some(port) = tokens.get(index + 1).and_then(|value| value.parse().ok())
        {
            return Some(port);
        }
        if let Some((_, value)) = token.rsplit_once(':')
            && let Ok(port) = value.trim_end_matches('/').parse()
        {
            return Some(port);
        }
    }
    None
}

/// Languages of one folder: manifests (by name or extension, from the language table) and
/// source files. A language the app cannot start yet needs a manifest or two source files,
/// so a stray script does not label a project.
fn detect_in_directory(path: &Path, found: &mut Vec<RuntimeKind>) {
    let pubspec = fs::read_to_string(path.join("pubspec.yaml")).ok();
    let is_flutter = pubspec.as_deref().is_some_and(is_flutter_pubspec);
    if pubspec.is_some() {
        let runtime = if is_flutter {
            RuntimeKind::Flutter
        } else {
            RuntimeKind::Dart
        };
        if !found.contains(&runtime) {
            found.push(runtime);
        }
    }
    // Kotlin builds with Gradle, like Java; the Kotlin plugin in the build tells them apart.
    let kotlin_build = ["build.gradle.kts", "build.gradle"].iter().any(|name| {
        fs::read_to_string(path.join(name)).is_ok_and(|build| {
            build.contains("kotlin(\"jvm\")") || build.contains("org.jetbrains.kotlin")
        })
    });
    if kotlin_build && !found.contains(&RuntimeKind::Kotlin) {
        found.push(RuntimeKind::Kotlin);
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    let mut manifests = [false; RuntimeKind::ALL.len()];
    let mut sources = [0_u8; RuntimeKind::ALL.len()];
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(runtime) = languages::from_manifest(&name) {
            manifests[runtime.index()] = true;
        } else if let Some(runtime) = Path::new(name.as_ref())
            .extension()
            .and_then(|value| value.to_str())
            .and_then(RuntimeKind::from_source_extension)
        {
            let count = &mut sources[runtime.index()];
            *count = count.saturating_add(1);
        }
    }
    // Manifests first, then source files, each in the order of the language table, so the
    // result does not depend on how the folder lists its files.
    let by_manifest = languages::LANGUAGES
        .iter()
        .map(|language| language.kind)
        .filter(|runtime| manifests[runtime.index()]);
    let by_source = languages::LANGUAGES
        .iter()
        .map(|language| language.kind)
        .filter(|runtime| {
            let count = sources[runtime.index()];
            count >= 2 || (count == 1 && runtime.starts())
        });
    for runtime in by_manifest.chain(by_source) {
        if !(is_flutter && runtime == RuntimeKind::Dart) && !found.contains(&runtime) {
            found.push(runtime);
        }
    }
}

fn detect_technologies_in_directory(path: &Path, found: &mut Vec<TechnologyKind>) {
    if path.join("wp-config.php").is_file()
        || path.join("wp-config-sample.php").is_file()
        || (path.join("wp-content").is_dir() && path.join("wp-includes").is_dir())
        || fs::read_to_string(path.join("composer.json"))
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
            .and_then(|document| document.get("require").cloned())
            .and_then(|requirements| requirements.as_object().cloned())
            .is_some_and(|requirements| {
                requirements.keys().any(|package| {
                    matches!(
                        package.as_str(),
                        "johnpbloch/wordpress" | "roots/wordpress" | "wordpress/wordpress"
                    )
                })
            })
    {
        push_unique(found, TechnologyKind::WordPress);
    }
    if containers::has_container_markers(path) {
        push_unique(found, TechnologyKind::Containers);
    }
    if let Some(framework) = detect_php_framework(path) {
        push_unique(found, framework);
    }
    if fs::read_to_string(path.join("pubspec.yaml"))
        .ok()
        .is_some_and(|contents| is_flutter_pubspec(&contents))
    {
        push_unique(found, TechnologyKind::Flutter);
    }

    let package_path = path.join("package.json");
    let package = fs::read_to_string(&package_path)
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok());
    if let Some(package) = &package {
        let has_dependency = |candidate: &str| {
            ["dependencies", "devDependencies"]
                .iter()
                .filter_map(|section| package.get(section).and_then(|value| value.as_object()))
                .any(|dependencies| dependencies.contains_key(candidate))
        };
        if has_dependency("vue") || has_dependency("nuxt") || has_dependency("@vitejs/plugin-vue") {
            push_unique(found, TechnologyKind::Vue);
        }
        if has_dependency("react")
            || has_dependency("react-dom")
            || has_dependency("next")
            || has_dependency("@vitejs/plugin-react")
        {
            push_unique(found, TechnologyKind::React);
        }
    }

    let mut has_javascript = package.is_some();
    let mut has_typescript = path.join("tsconfig.json").is_file()
        || package.as_ref().is_some_and(|package| {
            ["dependencies", "devDependencies"]
                .iter()
                .filter_map(|section| package.get(section).and_then(|value| value.as_object()))
                .any(|dependencies| dependencies.contains_key("typescript"))
        });
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            match entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("ts" | "tsx") => has_typescript = true,
                Some("js" | "jsx" | "mjs" | "cjs" | "vue") => has_javascript = true,
                _ => {}
            }
        }
    }
    if has_typescript {
        push_unique(found, TechnologyKind::TypeScript);
    } else if has_javascript {
        push_unique(found, TechnologyKind::JavaScript);
    }
}

fn detect_php_framework(path: &Path) -> Option<TechnologyKind> {
    let contents = fs::read_to_string(path.join("composer.json")).ok();
    let document = contents
        .as_deref()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(contents).ok());
    let requirements = document
        .as_ref()
        .and_then(|document| document.get("require"))
        .and_then(serde_json::Value::as_object);

    if requirements.is_some_and(|requirements| requirements.contains_key("laravel/lumen-framework"))
    {
        Some(TechnologyKind::Lumen)
    } else if requirements
        .is_some_and(|requirements| requirements.contains_key("laravel/framework"))
        || path.join("artisan").is_file()
    {
        Some(TechnologyKind::Laravel)
    } else {
        None
    }
}

fn push_unique(found: &mut Vec<TechnologyKind>, technology: TechnologyKind) {
    if !found.contains(&technology) {
        found.push(technology);
    }
}

fn technology_priority(technology: TechnologyKind) -> usize {
    match technology {
        TechnologyKind::Laravel => 0,
        TechnologyKind::Lumen => 1,
        TechnologyKind::WordPress => 2,
        TechnologyKind::Flutter => 2,
        TechnologyKind::StaticSite => 3,
        TechnologyKind::Vue => 4,
        TechnologyKind::React => 5,
        TechnologyKind::TypeScript => 6,
        TechnologyKind::JavaScript => 7,
        TechnologyKind::Containers => 8,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ManifestRuntimeDetector;

impl RuntimeDetector for ManifestRuntimeDetector {
    fn detect(&self, path: &Path) -> Vec<RuntimeKind> {
        detect(path)
    }

    fn detect_technologies(&self, path: &Path) -> Vec<TechnologyKind> {
        detect_technologies(path)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_independent_runtimes_in_flutter_monorepos() {
        let root = std::env::temp_dir().join(format!(
            "localcodepilot-flutter-monorepo-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("tools")).unwrap();
        fs::create_dir_all(root.join("mobile")).unwrap();
        fs::create_dir_all(root.join("api")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(
            root.join("pubspec.yaml"),
            "name: root\ndependencies:\n  flutter:\n    sdk: flutter\n",
        )
        .unwrap();
        fs::write(root.join("package.json"), "{}").unwrap();
        fs::write(
            root.join("mobile/pubspec.yaml"),
            "name: mobile\ndependencies:\n  flutter:\n    sdk: flutter\n",
        )
        .unwrap();
        fs::write(root.join("mobile/main.dart"), "void main() {}").unwrap();
        fs::write(root.join("tools/pubspec.yaml"), "name: tools\n").unwrap();
        fs::write(root.join("api/go.mod"), "module example.com/api\n").unwrap();

        let detected = detect(&root);
        assert!(detected.contains(&RuntimeKind::Flutter));
        assert!(detected.contains(&RuntimeKind::Node));
        assert!(detected.contains(&RuntimeKind::Dart));
        assert!(detected.contains(&RuntimeKind::Go));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_qa_fixtures_in_parent_but_allows_direct_inspection() {
        let root = std::env::temp_dir().join(format!("lcp-ignored-qa-{}", std::process::id()));
        let fixture = root.join("qa").join("node");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(&fixture).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "qa-root"
version = "0.1.0"
"#,
        )
        .unwrap();
        fs::write(root.join("qa/.localcodepilot-ignore"), "").unwrap();
        fs::write(
            fixture.join("package.json"),
            r#"{"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        assert_eq!(detect(&root), vec![RuntimeKind::Rust]);
        assert!(!detect_technologies(&root).contains(&TechnologyKind::JavaScript));
        assert!(detect_processes(&Project::new(root.clone(), detect(&root))).is_empty());
        assert_eq!(detect(&fixture), vec![RuntimeKind::Node]);
        assert_eq!(
            detect_processes(&Project::new(fixture.clone(), detect(&fixture))).len(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }

    use super::*;
    use std::{fs, time::SystemTime};

    #[test]
    fn recognizes_wordpress_from_config_or_composer() {
        let path = std::env::temp_dir().join(format!(
            "lcp-wordpress-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("wp-config-sample.php"), "<?php").unwrap();
        let mut found = vec![];
        detect_technologies_in_directory(&path, &mut found);
        assert!(found.contains(&TechnologyKind::WordPress));
        fs::remove_file(path.join("wp-config-sample.php")).unwrap();
        fs::write(
            path.join("composer.json"),
            r#"{"require":{"roots/wordpress":"*"}}"#,
        )
        .unwrap();
        found.clear();
        detect_technologies_in_directory(&path, &mut found);
        assert!(found.contains(&TechnologyKind::WordPress));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn wordpress_compose_does_not_start_php_outside_its_database_network() {
        let path = std::env::temp_dir().join(format!(
            "lcp-wordpress-compose-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("index.php"), "<?php").unwrap();
        fs::write(path.join("wp-config.php"), "<?php").unwrap();
        let project = Project::new(path.clone(), vec![RuntimeKind::Php])
            .with_technologies(vec![TechnologyKind::WordPress, TechnologyKind::Containers]);
        assert_eq!(detect_processes(&project).len(), 1);
        fs::write(
            path.join("docker-compose.yml"),
            "services:\n  db:\n    image: mysql:8\n",
        )
        .unwrap();
        assert!(detect_processes(&project).is_empty());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn wordpress_with_compose_in_a_subfolder_or_sibling_runs_through_containers() {
        let root = std::env::temp_dir().join(format!(
            "lcp-wordpress-compose-layouts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let php_servers = |project: &Project| {
            detect_processes(project)
                .iter()
                .filter(|process| process.name == "Servidor PHP")
                .count()
        };

        // Site with its Compose file in docker/.
        let site = root.join("blog");
        fs::create_dir_all(site.join("docker")).unwrap();
        fs::write(site.join("wp-config.php"), "<?php").unwrap();
        let blog = Project::new(site.clone(), vec![RuntimeKind::Php])
            .with_technologies(vec![TechnologyKind::WordPress]);
        assert_eq!(php_servers(&blog), 1);
        fs::write(
            site.join("docker").join("compose.yaml"),
            "services:\n  db:\n    image: mariadb:11\n",
        )
        .unwrap();
        assert_eq!(php_servers(&blog), 0);
        // Without Git the subfolders are not walked, but the card still needs Containers.
        assert!(detect_technologies(&site).contains(&TechnologyKind::Containers));

        // Monorepo: a sibling Compose for another service does not hijack the site...
        let monorepo = root.join("monorepo");
        fs::create_dir_all(monorepo.join(".git")).unwrap();
        fs::create_dir_all(monorepo.join("wordpress")).unwrap();
        fs::create_dir_all(monorepo.join("api")).unwrap();
        fs::create_dir_all(monorepo.join("infra")).unwrap();
        fs::write(monorepo.join("wordpress").join("wp-config.php"), "<?php").unwrap();
        fs::write(
            monorepo.join("api").join("compose.yaml"),
            "services:\n  api:\n    image: node:22\n",
        )
        .unwrap();
        let project = Project::new(monorepo.clone(), vec![RuntimeKind::Php])
            .with_technologies(vec![TechnologyKind::WordPress]);
        assert_eq!(php_servers(&project), 1);
        // ...but one that declares a WordPress image does.
        fs::write(
            monorepo.join("infra").join("compose.yaml"),
            "services:\n  site:\n    image: docker.io/library/WordPress:6\n",
        )
        .unwrap();
        assert_eq!(php_servers(&project), 0);
        assert!(containers::wordpress_uses_compose(
            &monorepo,
            &monorepo.join("wordpress")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_php_site_runs_through_compose_only_when_compose_serves_it() {
        let root = std::env::temp_dir().join(format!(
            "lcp-php-compose-layouts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let php_servers = |project: &Project| {
            detect_processes(project)
                .iter()
                .filter(|process| process.name == "Servidor PHP")
                .count()
        };
        let site = root.join("loja");
        fs::create_dir_all(site.join("docker")).unwrap();
        fs::write(site.join("index.php"), "<?php echo 'loja';").unwrap();
        let loja = Project::new(site.clone(), vec![RuntimeKind::Php]);
        assert_eq!(php_servers(&loja), 1);
        assert!(!detect_technologies(&site).contains(&TechnologyKind::Containers));

        // Only the database in Compose: the site still runs with PHP on the machine.
        let compose = site.join("docker").join("compose.yaml");
        fs::write(
            &compose,
            "services:\n  db:\n    image: mariadb:11\n  admin:\n    image: phpmyadmin:5\n",
        )
        .unwrap();
        assert_eq!(php_servers(&loja), 1);
        assert!(!detect_technologies(&site).contains(&TechnologyKind::Containers));

        // A web server in Compose serves the site: start it through Containers.
        fs::write(
            &compose,
            "services:\n  web:\n    image: docker.io/library/nginx:alpine\n  db:\n    image: mariadb:11\n",
        )
        .unwrap();
        assert_eq!(php_servers(&loja), 0);
        assert!(detect_technologies(&site).contains(&TechnologyKind::Containers));

        // So does a service that builds its own image.
        fs::write(&compose, "services:\n  app:\n    build: ..\n").unwrap();
        assert_eq!(php_servers(&loja), 0);

        // A PHP image in an unrelated sibling folder does not hijack the site.
        let monorepo = root.join("monorepo");
        fs::create_dir_all(monorepo.join(".git")).unwrap();
        fs::create_dir_all(monorepo.join("site")).unwrap();
        fs::create_dir_all(monorepo.join("outro")).unwrap();
        fs::write(monorepo.join("site").join("index.php"), "<?php").unwrap();
        fs::write(
            monorepo.join("outro").join("compose.yaml"),
            "services:\n  app:\n    image: php:8.3-apache\n",
        )
        .unwrap();
        let project = Project::new(monorepo.clone(), vec![RuntimeKind::Php]);
        assert_eq!(php_servers(&project), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_node_requirement_from_version_files_and_manifest() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-node-requirement-{nonce}"));
        let app = root.join("apps").join("web");
        fs::create_dir_all(&app).unwrap();
        fs::write(
            app.join("package.json"),
            r#"{"engines":{"node":">=20 <23"}}"#,
        )
        .unwrap();

        let manifest_requirement = detect_node_requirement(&app, &root).unwrap();
        assert_eq!(manifest_requirement.version.value, ">=20 <23");
        assert_eq!(
            manifest_requirement.source,
            RequirementSource::Manifest(app.join("package.json"))
        );

        fs::write(root.join(".nvmrc"), "22\n").unwrap();
        let version_file_requirement = detect_node_requirement(&app, &root).unwrap();
        assert_eq!(version_file_requirement.version.value, "22");
        assert_eq!(
            version_file_requirement.source,
            RequirementSource::VersionFile(root.join(".nvmrc"))
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn matches_common_node_version_constraints() {
        assert_eq!(node_version_matches("22", "v22.12.0"), Some(true));
        assert_eq!(node_version_matches("20.x", "v20.18.1"), Some(true));
        assert_eq!(node_version_matches(">=20 <23", "v22.12.0"), Some(true));
        assert_eq!(node_version_matches("^20.0.0", "v22.12.0"), Some(false));
        assert_eq!(
            node_version_matches("18 || 20 || 22", "v22.12.0"),
            Some(true)
        );
        assert_eq!(node_version_matches("lts/*", "v22.12.0"), None);
    }

    #[test]
    fn detects_python_php_and_java_requirements() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("localcodepilot-runtime-requirements-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("pyproject.toml"),
            "[project]\nrequires-python = \">=3.11,<3.14\"\n",
        )
        .unwrap();
        fs::write(
            root.join("composer.json"),
            r#"{"require":{"php":"^8.2","ext-mbstring":"*","ext-pdo-mysql":"*"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("pom.xml"),
            "<project><properties><java.version>21</java.version></properties><build><plugins><spring-boot/></plugins></build></project>",
        )
        .unwrap();

        assert_eq!(
            detect_python_requirement(&root, &root)
                .unwrap()
                .version
                .value,
            ">=3.11,<3.14"
        );
        assert_eq!(
            detect_php_requirement(&root, &root).unwrap().version.value,
            "^8.2"
        );
        assert_eq!(
            detect_php_extensions(&root, &root),
            ["mbstring", "pdo_mysql"]
        );
        assert_eq!(
            detect_java_requirement(&root, &root).unwrap().version.value,
            "21"
        );
        assert_eq!(
            detect(&root),
            [RuntimeKind::Php, RuntimeKind::Python, RuntimeKind::Java]
        );
        let project = Project::new(root.clone(), detect(&root));
        assert!(
            detect_processes(&project)
                .iter()
                .any(|process| process.args == ["spring-boot:run"])
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn matches_python_compatible_release_constraints() {
        assert_eq!(
            runtime_version_matches("~=3.10", "Python 3.13.2"),
            Some(true)
        );
        assert_eq!(
            runtime_version_matches("~=3.10.2", "Python 3.11.0"),
            Some(false)
        );
    }

    #[test]
    fn detects_multiple_runtimes() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-runtime-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("Cargo.toml"), "").unwrap();
        fs::write(path.join("package.json"), "{}").unwrap();
        assert_eq!(detect(&path), vec![RuntimeKind::Rust, RuntimeKind::Node]);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn recognizes_languages_the_app_does_not_start_yet_without_stray_files() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-languages-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("Gemfile"), "source 'https://rubygems.org'\n").unwrap();
        fs::write(path.join("Api.csproj"), "<Project />").unwrap();
        // One Lua script is not a Lua project; two source files are.
        fs::write(path.join("build.lua"), "").unwrap();
        assert_eq!(detect(&path), [RuntimeKind::Dotnet, RuntimeKind::Ruby]);
        fs::write(path.join("init.lua"), "").unwrap();
        assert_eq!(
            detect(&path),
            [RuntimeKind::Dotnet, RuntimeKind::Ruby, RuntimeKind::Lua]
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_laravel_vue_and_typescript() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-technologies-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("artisan"), "").unwrap();
        fs::write(path.join("tsconfig.json"), "{}").unwrap();
        fs::write(
            path.join("package.json"),
            r#"{"dependencies":{"vue":"^3.0.0"},"devDependencies":{"typescript":"^5.0.0"}}"#,
        )
        .unwrap();

        assert_eq!(
            detect_technologies(&path),
            [
                TechnologyKind::Laravel,
                TechnologyKind::Vue,
                TechnologyKind::TypeScript
            ]
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_lumen_and_uses_the_php_builtin_server() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-lumen-{nonce}"));
        fs::create_dir_all(path.join("public")).unwrap();
        fs::write(path.join("artisan"), "").unwrap();
        fs::write(
            path.join("composer.json"),
            r#"{"require":{"php":"^8.2","laravel/lumen-framework":"^10.0"}}"#,
        )
        .unwrap();
        fs::write(path.join("public/index.php"), "<?php").unwrap();

        let project = Project::new(path.clone(), detect(&path));
        let technologies = detect_technologies(&path);
        let commands = detect_processes(&project);

        assert_eq!(technologies, [TechnologyKind::Lumen]);
        assert!(!technologies.contains(&TechnologyKind::Laravel));
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "Servidor Lumen");
        assert_eq!(
            commands[0].command_line(),
            "php -S localhost:8000 -t public"
        );
        assert_eq!(commands[0].expected_port, Port::new(8000));
        assert_eq!(
            commands[0].port_override,
            Some(PortOverride::PhpServerAddress { argument_index: 1 })
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_react_with_javascript() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-react-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(
            path.join("package.json"),
            r#"{"dependencies":{"react":"^19.0.0","react-dom":"^19.0.0"}}"#,
        )
        .unwrap();

        assert_eq!(
            detect_technologies(&path),
            [TechnologyKind::React, TechnologyKind::JavaScript]
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_plain_javascript_without_a_manifest() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-javascript-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("app.js"), "console.log('hello')").unwrap();

        assert_eq!(detect_technologies(&path), [TechnologyKind::JavaScript]);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_runtime_inside_git_repository_modules() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-repository-{nonce}"));
        fs::create_dir_all(path.join(".git")).unwrap();
        fs::create_dir_all(path.join("painel-controle")).unwrap();
        fs::write(path.join("painel-controle").join("composer.json"), "{}").unwrap();

        assert_eq!(detect(&path), vec![RuntimeKind::Php]);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_php_from_source_files_without_manifest() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-loose-php-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("exercicio1.php"), "<?php").unwrap();

        assert_eq!(detect(&path), vec![RuntimeKind::Php]);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_node_and_composer_scripts() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-processes-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(
            path.join("package.json"),
            r#"{"scripts":{"build":"vite build","dev":"vite","test":"vitest","predev":"prepare"}}"#,
        )
        .unwrap();
        fs::write(
            path.join("composer.json"),
            r#"{"scripts":{"serve":"php -S localhost:8000","post-install":"setup"}}"#,
        )
        .unwrap();
        let project = Project::new(path.clone(), vec![RuntimeKind::Node, RuntimeKind::Php]);

        let commands: Vec<_> = detect_processes(&project)
            .iter()
            .map(ProjectProcess::command_line)
            .collect();

        assert_eq!(commands, ["npm run dev", "composer run-script serve"]);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn chooses_the_node_package_manager_without_configuration() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-managers-{nonce}"));
        fs::create_dir(&root).unwrap();

        for (directory, marker, expected) in [
            ("pnpm-project", "pnpm-lock.yaml", "pnpm"),
            ("yarn-project", "yarn.lock", "yarn"),
            ("bun-project", "bun.lock", "bun"),
        ] {
            let path = root.join(directory);
            fs::create_dir(&path).unwrap();
            fs::write(path.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();
            fs::write(path.join(marker), "").unwrap();
            let project = Project::new(path, vec![RuntimeKind::Node]);

            assert_eq!(
                detect_processes(&project)[0].command_line(),
                format!("{expected} run dev")
            );
        }

        let path = root.join("declared-project");
        fs::create_dir(&path).unwrap();
        fs::write(
            path.join("package.json"),
            r#"{"packageManager":"pnpm@10.0.0","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::write(path.join("yarn.lock"), "").unwrap();
        let project = Project::new(path, vec![RuntimeKind::Node]);
        assert_eq!(detect_processes(&project)[0].command_line(), "pnpm run dev");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn infers_default_and_explicit_development_ports() {
        let node_args = vec!["run".into(), "dev".into()];
        let laravel_args = vec!["artisan".into(), "serve".into()];
        let django_args = vec!["manage.py".into(), "runserver".into()];

        assert_eq!(
            infer_expected_port("npm", &node_args, Some("vite"))
                .unwrap()
                .value(),
            5173
        );
        assert_eq!(
            infer_expected_port("npm", &node_args, Some("vite --port 4173"))
                .unwrap()
                .value(),
            4173
        );
        assert_eq!(
            infer_expected_port("npm", &node_args, Some("next dev -p=3100"))
                .unwrap()
                .value(),
            3100
        );
        assert_eq!(
            infer_expected_port("php", &laravel_args, None)
                .unwrap()
                .value(),
            8000
        );
        assert_eq!(
            infer_expected_port("python", &django_args, None)
                .unwrap()
                .value(),
            8000
        );
        assert!(infer_expected_port("cargo", &["run".into()], None).is_none());

        assert_eq!(
            infer_port_override("npm", &node_args, Some("vite"), Port::new(5173),),
            Some(PortOverride::LongOption { separator: true })
        );
        assert_eq!(
            infer_port_override("php", &laravel_args, None, Port::new(8000)),
            Some(PortOverride::LongOption { separator: false })
        );
        assert_eq!(
            infer_port_override("python", &django_args, None, Port::new(8000)),
            Some(PortOverride::Positional)
        );
        assert_eq!(
            infer_port_override("composer", &[], Some("php artisan serve"), Port::new(8000)),
            None
        );
    }

    #[test]
    fn keeps_only_development_servers_in_a_laravel_project() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-laravel-{nonce}"));
        fs::create_dir_all(path.join("public")).unwrap();
        fs::write(path.join("artisan"), "").unwrap();
        fs::write(path.join("public").join("index.php"), "<?php").unwrap();
        fs::write(
            path.join("package.json"),
            r#"{"scripts":{"build":"vite build","dev":"vite","test":"vitest"}}"#,
        )
        .unwrap();
        fs::write(
            path.join("composer.json"),
            r#"{"scripts":{"dev":"composer run-all","test":"phpunit"}}"#,
        )
        .unwrap();
        fs::create_dir(path.join(".git")).unwrap();
        let project = Project::new(path.clone(), vec![RuntimeKind::Node, RuntimeKind::Php]);

        let commands: Vec<_> = detect_processes(&project)
            .iter()
            .map(ProjectProcess::command_line)
            .collect();

        assert_eq!(commands, ["npm run dev", "php artisan serve"]);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn ignores_a_node_server_that_is_not_laravel_frontend_tooling() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-laravel-node-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("artisan"), "").unwrap();
        fs::write(
            path.join("package.json"),
            r#"{"scripts":{"dev":"node server.js"}}"#,
        )
        .unwrap();
        let project = Project::new(path.clone(), vec![RuntimeKind::Node, RuntimeKind::Php]);

        let commands: Vec<_> = detect_processes(&project)
            .iter()
            .map(ProjectProcess::command_line)
            .collect();

        assert_eq!(commands, ["php artisan serve"]);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_common_rust_php_and_python_commands() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-commands-{nonce}"));
        fs::create_dir_all(path.join("src")).unwrap();
        fs::write(
            path.join("Cargo.toml"),
            "[package]\nname = \"example\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(path.join("src/main.rs"), "fn main() {}\n").unwrap();
        for file in ["artisan", "manage.py"] {
            fs::write(path.join(file), "").unwrap();
        }
        let project = Project::new(path.clone(), vec![]);

        let commands: Vec<_> = detect_processes(&project)
            .iter()
            .map(ProjectProcess::command_line)
            .collect();

        assert_eq!(
            commands,
            [
                "cargo run",
                "php artisan serve",
                "python manage.py runserver"
            ]
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_only_the_default_program_in_a_cargo_workspace() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-workspace-{nonce}"));
        fs::create_dir_all(path.join(".git")).unwrap();
        fs::write(
            path.join("Cargo.toml"),
            "[workspace]\nmembers = [\"core\", \"cli\", \"desktop\"]\ndefault-members = [\"desktop\"]\n",
        )
        .unwrap();

        for member in ["core", "cli", "desktop"] {
            let member_path = path.join(member);
            fs::create_dir_all(member_path.join("src")).unwrap();
            fs::write(
                member_path.join("Cargo.toml"),
                format!("[package]\nname = \"{member}\"\nversion = \"0.1.0\"\n"),
            )
            .unwrap();
        }
        fs::write(path.join("core/src/lib.rs"), "").unwrap();
        fs::write(path.join("cli/src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(path.join("desktop/src/main.rs"), "fn main() {}\n").unwrap();

        let project = Project::new(path.clone(), vec![RuntimeKind::Rust]);
        let commands = detect_processes(&project);

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "Executar workspace");
        assert_eq!(commands[0].command_line(), "cargo run");
        assert_eq!(commands[0].working_directory, path.canonicalize().unwrap());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn does_not_offer_cargo_run_for_a_library_package() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-library-{nonce}"));
        fs::create_dir_all(path.join("src")).unwrap();
        fs::write(
            path.join("Cargo.toml"),
            "[package]\nname = \"library\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(path.join("src/lib.rs"), "").unwrap();

        let project = Project::new(path.clone(), vec![RuntimeKind::Rust]);

        assert!(detect_processes(&project).is_empty());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_commands_in_git_repository_modules() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-module-commands-{nonce}"));
        let module = path.join("painel-controle");
        fs::create_dir_all(path.join(".git")).unwrap();
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("artisan"), "").unwrap();
        let project = Project::new(path.clone(), vec![RuntimeKind::Php]);

        let commands = detect_processes(&project);

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].command_line(), "php artisan serve");
        assert_eq!(
            commands[0].working_directory,
            module.canonicalize().unwrap()
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn prefers_a_standalone_frontend_over_laravel_vite() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-split-app-{nonce}"));
        let backend = root.join("backend");
        let frontend = root.join("frontend");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(&backend).unwrap();
        fs::create_dir_all(&frontend).unwrap();
        fs::write(backend.join("artisan"), "").unwrap();
        fs::write(
            backend.join("package.json"),
            r#"{"scripts":{"dev":"vite"},"devDependencies":{"laravel-vite-plugin":"*"}}"#,
        )
        .unwrap();
        fs::write(
            frontend.join("package.json"),
            r#"{"scripts":{"dev":"vite"},"dependencies":{"vue":"*"}}"#,
        )
        .unwrap();

        let commands = detect_processes(&Project::new(root.clone(), detect(&root)));

        assert_eq!(commands.len(), 2);
        assert!(
            commands
                .iter()
                .any(|process| process.name == "Servidor Laravel")
        );
        assert!(commands.iter().any(|process| {
            process.name == "dev" && process.working_directory.ends_with("frontend")
        }));
        assert!(!commands.iter().any(|process| process.name == "Frontend"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn serves_a_static_site_from_its_repository_root() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-static-{nonce}"));
        fs::create_dir_all(path.join(".git")).unwrap();
        fs::create_dir_all(path.join("en")).unwrap();
        fs::write(path.join("index.html"), "<h1>LocalCodePilot</h1>").unwrap();
        fs::write(path.join("en/index.html"), "<h1>LocalCodePilot</h1>").unwrap();
        fs::write(path.join("navigation.js"), "").unwrap();

        let project = Project::new(path.clone(), detect(&path));
        let commands = detect_processes(&project);
        let expected_port = static_site_port(&project.path);

        assert!(detect_technologies(&path).contains(&TechnologyKind::StaticSite));
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].command_line(),
            format!("python -m http.server {expected_port} --bind 127.0.0.1")
        );
        assert_eq!(commands[0].expected_port, Port::new(expected_port));
        assert_eq!(
            commands[0].port_override,
            Some(PortOverride::ArgumentValue { argument_index: 2 })
        );
        assert_ne!(
            static_site_port(&path),
            static_site_port(&path.with_file_name("another-static-site"))
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn treats_html_with_php_backend_as_php_web_project() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-php-web-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("index.html"), "<form action=\"send.php\"></form>").unwrap();
        fs::write(path.join("send.php"), "<?php echo 'ok';").unwrap();

        let project = Project::new(path.clone(), detect(&path));
        let commands = detect_processes(&project);

        assert_eq!(project.runtimes, [RuntimeKind::Php]);
        assert!(!detect_technologies(&path).contains(&TechnologyKind::StaticSite));
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].command_line(), "php -S localhost:8000");
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn projects_without_a_framework_still_start_the_plain_way() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-fallback-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };

        // A Java program in one file, even with a pom.xml that declares no plugin.
        fs::create_dir_all(root.join("java")).unwrap();
        fs::write(root.join("java/pom.xml"), "<project></project>").unwrap();
        fs::write(
            root.join("java/Main.java"),
            "public class Main { public static void main(String[] args) {} }",
        )
        .unwrap();
        assert_eq!(
            detect_processes(&project("java"))[0].command_line(),
            "java Main.java"
        );

        // PHP exercises without index.php: the server with the listing router.
        fs::create_dir_all(root.join("php")).unwrap();
        fs::write(root.join("php/exercicio1.php"), "<?php echo 1;").unwrap();
        fs::write(root.join("php/exercicio2.php"), "<?php echo 2;").unwrap();
        let php = detect_processes(&project("php"));
        assert_eq!(php.len(), 1);
        assert_eq!(php[0].program, "php");
        assert_eq!(&php[0].args[..2], ["-S", "localhost:8000"]);
        assert!(php[0].args[2].ends_with("php-listing-router.php"));
        assert!(Path::new(&php[0].args[2]).is_file());
        assert_eq!(php[0].expected_port, Port::new(8000));

        // A browser game: `teste.html` at the root and `source_code/index.html` below.
        fs::create_dir_all(root.join("game/source_code/js")).unwrap();
        fs::write(root.join("game/teste.html"), "<html></html>").unwrap();
        fs::write(root.join("game/Source_1.js"), "").unwrap();
        fs::write(root.join("game/source_code/index.html"), "<html></html>").unwrap();
        fs::write(root.join("game/source_code/js/main.js"), "").unwrap();
        let game = detect_processes(&project("game"));
        assert_eq!(game.len(), 1);
        assert_eq!(game[0].name, "Site estático");

        // A project with a real command never gets a fallback next to it.
        fs::create_dir_all(root.join("flask/templates")).unwrap();
        fs::write(root.join("flask/app.py"), "print('ok')").unwrap();
        fs::write(root.join("flask/templates/index.html"), "<html></html>").unwrap();
        let flask = detect_processes(&project("flask"));
        assert_eq!(flask.len(), 1);
        assert_eq!(flask[0].command_line(), "python app.py");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_ruby_web_apps_and_programs() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-ruby-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };
        let only = |name: &str| {
            let processes = detect_processes(&project(name));
            assert_eq!(processes.len(), 1, "{name}");
            (processes[0].command_line(), processes[0].expected_port)
        };

        fs::create_dir_all(root.join("rails/bin")).unwrap();
        fs::create_dir_all(root.join("rails/config")).unwrap();
        fs::write(root.join("rails/Gemfile"), "gem 'rails'\n").unwrap();
        fs::write(root.join("rails/bin/rails"), "#!/usr/bin/env ruby\n").unwrap();
        fs::write(root.join("rails/config/application.rb"), "").unwrap();
        fs::write(root.join("rails/config.ru"), "").unwrap();
        assert_eq!(
            only("rails"),
            ("ruby bin/rails server".into(), Port::new(3000))
        );

        fs::create_dir_all(root.join("rack")).unwrap();
        fs::write(root.join("rack/Gemfile"), "gem 'rack'\n").unwrap();
        fs::write(
            root.join("rack/config.ru"),
            "run ->(env) { [200, {}, ['ok']] }\n",
        )
        .unwrap();
        assert_eq!(only("rack"), ("bundle exec rackup".into(), Port::new(9292)));

        fs::create_dir_all(root.join("sinatra")).unwrap();
        fs::write(root.join("sinatra/Gemfile"), "gem 'sinatra'\n").unwrap();
        fs::write(
            root.join("sinatra/app.rb"),
            "require 'sinatra'\nget('/') { 'ok' }\n",
        )
        .unwrap();
        assert_eq!(
            only("sinatra"),
            ("bundle exec ruby app.rb".into(), Port::new(4567))
        );

        fs::create_dir_all(root.join("script")).unwrap();
        fs::write(root.join("script/main.rb"), "puts 'oi'\n").unwrap();
        assert_eq!(only("script"), ("ruby main.rb".into(), None));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_zig_projects_with_a_run_step() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-zig-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };

        fs::create_dir_all(root.join("app")).unwrap();
        fs::write(
            root.join("app/build.zig"),
            "const run_step = b.step(\"run\", \"Run the app\");\n",
        )
        .unwrap();
        let app = detect_processes(&project("app"));
        assert_eq!(app.len(), 1);
        assert_eq!(app[0].command_line(), "zig build run");
        let app_path = root.join("app");
        assert_eq!(
            detect_zig_requirement(&app_path, &app_path)
                .unwrap()
                .command,
            "zig"
        );

        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(
            root.join("lib/build.zig"),
            "const lib = b.addLibrary(.{});\n",
        )
        .unwrap();
        assert!(detect_processes(&project("lib")).is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_stack_projects() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-haskell-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };

        fs::create_dir_all(root.join("app")).unwrap();
        fs::write(root.join("app/stack.yaml"), "resolver: lts-24.0\n").unwrap();
        fs::write(
            root.join("app/package.yaml"),
            "name: app\nexecutables:\n  app:\n    main: Main.hs\n",
        )
        .unwrap();
        let app = detect_processes(&project("app"));
        assert_eq!(app.len(), 1);
        assert_eq!(app[0].command_line(), "stack run");
        let app_path = root.join("app");
        assert_eq!(
            detect_haskell_requirement(&app_path, &app_path)
                .unwrap()
                .command,
            "stack"
        );

        // A library has nothing to run.
        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(root.join("lib/stack.yaml"), "resolver: lts-24.0\n").unwrap();
        fs::write(
            root.join("lib/package.yaml"),
            "name: lib\nlibrary:\n  source-dirs: src\n",
        )
        .unwrap();
        assert!(detect_processes(&project("lib")).is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_clojure_deps_projects_through_babashka() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-clojure-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };

        fs::create_dir_all(root.join("api")).unwrap();
        fs::write(
            root.join("api/deps.edn"),
            "{:deps {ring/ring {:mvn/version \"1.12.2\"}}\n :aliases {:test {:extra-paths [\"test\"]}\n           :run {:main-opts [\"-m\" \"api.core\"]}}}\n",
        )
        .unwrap();
        let api = detect_processes(&project("api"));
        assert_eq!(api.len(), 1);
        assert_eq!(api[0].command_line(), "bb clojure -M:run");
        let api_path = root.join("api");
        assert_eq!(
            detect_clojure_requirement(&api_path, &api_path)
                .unwrap()
                .command,
            "bb"
        );
        assert_eq!(
            detect_clojure_java_requirement(&api_path, &api_path)
                .unwrap()
                .runtime,
            RuntimeKind::Java
        );

        // Without an alias to run, nothing is guessed.
        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(root.join("lib/deps.edn"), "{:deps {}}\n").unwrap();
        assert!(detect_processes(&project("lib")).is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_sbt_projects_and_play() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-scala-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };

        fs::create_dir_all(root.join("http4s")).unwrap();
        fs::write(root.join("http4s/build.sbt"), "scalaVersion := \"3.6.4\"\n").unwrap();
        let http4s = detect_processes(&project("http4s"));
        assert_eq!(http4s.len(), 1);
        assert_eq!(http4s[0].command_line(), "sbt --server run");
        fs::create_dir_all(root.join("http4s/project")).unwrap();
        fs::write(
            root.join("http4s/project/build.properties"),
            "sbt.version=1.10.7\n",
        )
        .unwrap();
        assert_eq!(
            detect_processes(&project("http4s"))[0].command_line(),
            "sbt run"
        );

        fs::create_dir_all(root.join("play/project")).unwrap();
        fs::write(
            root.join("play/build.sbt"),
            "lazy val root = (project in file(\".\")).enablePlugins(PlayScala)\n",
        )
        .unwrap();
        fs::write(
            root.join("play/project/plugins.sbt"),
            "addSbtPlugin(\"org.playframework\" % \"sbt-plugin\" % \"3.0.6\")\n",
        )
        .unwrap();
        let play = detect_processes(&project("play"));
        assert_eq!(play[0].name, "Servidor Play");
        assert_eq!(play[0].expected_port, Port::new(9000));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_swift_executables_and_vapor() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-swift-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };
        let write = |name: &str, manifest: &str| {
            fs::create_dir_all(root.join(name)).unwrap();
            fs::write(root.join(name).join("Package.swift"), manifest).unwrap();
        };

        write(
            "cli",
            "let package = Package(targets: [.executableTarget(name: \"Cli\")])\n",
        );
        let cli = detect_processes(&project("cli"));
        assert_eq!(cli.len(), 1);
        assert_eq!(cli[0].command_line(), "swift run");

        write(
            "api",
            ".package(url: \"https://github.com/vapor/vapor.git\", from: \"4.0.0\")\n.executableTarget(name: \"App\")\n",
        );
        let api = detect_processes(&project("api"));
        assert_eq!(api[0].name, "Servidor Vapor");
        assert_eq!(api[0].expected_port, Port::new(8080));

        write(
            "lib",
            "let package = Package(targets: [.target(name: \"Lib\")])\n",
        );
        assert!(detect_processes(&project("lib")).is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_phoenix_and_elixir_applications() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-elixir-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };
        let write = |name: &str, mix: &str| {
            fs::create_dir_all(root.join(name)).unwrap();
            fs::write(root.join(name).join("mix.exs"), mix).unwrap();
        };

        write(
            "phoenix",
            "def application, do: [mod: {Loja.Application, []}]\ndefp deps, do: [{:phoenix, \"~> 1.8\"}]\n",
        );
        let phoenix = detect_processes(&project("phoenix"));
        assert_eq!(phoenix.len(), 1);
        assert_eq!(phoenix[0].command_line(), "mix phx.server");
        assert_eq!(phoenix[0].expected_port, Port::new(4000));

        write(
            "plug",
            "def application, do: [mod: {Api.Application, []}]\ndefp deps, do: [{:bandit, \"~> 1.5\"}]\n",
        );
        assert_eq!(
            detect_processes(&project("plug"))[0].command_line(),
            "mix run --no-halt"
        );

        write(
            "lib",
            "def application, do: [extra_applications: [:logger]]\n",
        );
        assert!(detect_processes(&project("lib")).is_empty());

        // Phoenix: its `assets/` TypeScript does not make it a TypeScript project, and the
        // packages in `deps/` are not projects of their own.
        fs::create_dir_all(root.join("phoenix/.git")).unwrap();
        fs::create_dir_all(root.join("phoenix/assets/js")).unwrap();
        fs::write(root.join("phoenix/assets/tsconfig.json"), "{}").unwrap();
        fs::write(root.join("phoenix/assets/js/app.ts"), "").unwrap();
        fs::create_dir_all(root.join("phoenix/deps/phoenix")).unwrap();
        fs::write(
            root.join("phoenix/deps/phoenix/package.json"),
            "{\"scripts\":{\"dev\":\"x\"}}",
        )
        .unwrap();
        assert!(!detect_technologies(&root.join("phoenix")).contains(&TechnologyKind::TypeScript));
        assert_eq!(
            detect_processes(&project("phoenix"))
                .iter()
                .map(|process| process.command_line())
                .collect::<Vec<_>>(),
            ["mix phx.server"]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_gradle_applications_and_ktor() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-gradle-run-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };

        fs::create_dir_all(root.join("ktor/src/main/kotlin")).unwrap();
        fs::write(
            root.join("ktor/build.gradle.kts"),
            "plugins {\n    kotlin(\"jvm\") version \"2.1.0\"\n    id(\"io.ktor.plugin\") version \"3.0.0\"\n}\napplication { mainClass.set(\"ApplicationKt\") }\n",
        )
        .unwrap();
        fs::write(
            root.join("ktor/src/main/kotlin/Application.kt"),
            "fun main() {}\n",
        )
        .unwrap();
        let ktor = detect_processes(&project("ktor"));
        assert_eq!(ktor.len(), 1);
        assert_eq!(ktor[0].name, "Servidor Ktor");
        assert_eq!(&ktor[0].args, &["run", "--no-daemon"]);
        assert_eq!(ktor[0].expected_port, Port::new(8080));
        assert!(detect(&root.join("ktor")).contains(&RuntimeKind::Kotlin));

        fs::create_dir_all(root.join("cli")).unwrap();
        fs::write(
            root.join("cli/build.gradle"),
            "plugins {\n    id 'application'\n}\napplication { mainClass = 'app.Main' }\n",
        )
        .unwrap();
        let cli = detect_processes(&project("cli"));
        assert_eq!(cli.len(), 1);
        assert_eq!(cli[0].name, "Executar aplicação");

        // A library build has nothing to run.
        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(
            root.join("lib/build.gradle.kts"),
            "plugins { `java-library` }\n",
        )
        .unwrap();
        assert!(detect_processes(&project("lib")).is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_deno_tasks_and_entry_files() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-deno-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };
        let commands = |name: &str| {
            detect_processes(&project(name))
                .iter()
                .map(|process| process.command_line())
                .collect::<Vec<_>>()
        };

        // Tasks in deno.jsonc, with comments: only the long-running one starts.
        fs::create_dir_all(root.join("fresh")).unwrap();
        fs::write(
            root.join("fresh/deno.jsonc"),
            "{\n  // tarefas do projeto\n  \"tasks\": { \"dev\": \"deno run -A --watch main.ts\", \"check\": \"deno check main.ts\" }\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("fresh/main.ts"),
            "Deno.serve(() => new Response('ok'));\n",
        )
        .unwrap();
        assert_eq!(commands("fresh"), ["deno task dev"]);

        // No tasks: the entry file, with the permissions of a local server.
        fs::create_dir_all(root.join("api")).unwrap();
        fs::write(root.join("api/deno.json"), "{ \"imports\": {} }\n").unwrap();
        fs::write(
            root.join("api/main.ts"),
            "Deno.serve(() => new Response('ok'));\n",
        )
        .unwrap();
        assert_eq!(
            commands("api"),
            ["deno run --allow-net --allow-read --allow-env main.ts"]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starts_dotnet_web_and_console_projects() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-dotnet-{nonce}"));
        let project = |name: &str| {
            let path = root.join(name);
            Project::new(path.clone(), detect(&path)).with_technologies(detect_technologies(&path))
        };

        // A web API at the root, with the port of its launch profile.
        fs::create_dir_all(root.join("api/Properties")).unwrap();
        fs::write(
            root.join("api/Api.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk.Web"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>"#,
        )
        .unwrap();
        fs::write(
            root.join("api/Properties/launchSettings.json"),
            r#"{"profiles":{"http":{"commandName":"Project","applicationUrl":"http://localhost:5123"},"IIS Express":{"commandName":"IISExpress"}}}"#,
        )
        .unwrap();
        let api = detect_processes(&project("api"));
        assert_eq!(api.len(), 1);
        assert_eq!(api[0].command_line(), "dotnet run --project Api.csproj");
        assert_eq!(api[0].name, "Servidor ASP.NET (Api)");
        assert_eq!(api[0].expected_port, Port::new(5123));

        // A solution: the web app and the console program start, the library and tests do not.
        fs::create_dir_all(root.join("loja/src/Loja.Web")).unwrap();
        fs::create_dir_all(root.join("loja/src/Loja.Dominio")).unwrap();
        fs::create_dir_all(root.join("loja/tools/Importador")).unwrap();
        fs::create_dir_all(root.join("loja/tests/Loja.Testes")).unwrap();
        fs::write(
            root.join("loja/src/Loja.Web/Loja.Web.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk.Web"></Project>"#,
        )
        .unwrap();
        fs::write(
            root.join("loja/src/Loja.Dominio/Loja.Dominio.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk"></Project>"#,
        )
        .unwrap();
        fs::write(
            root.join("loja/tools/Importador/Importador.fsproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType></PropertyGroup></Project>"#,
        )
        .unwrap();
        fs::write(
            root.join("loja/tests/Loja.Testes/Loja.Testes.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType></PropertyGroup><ItemGroup><PackageReference Include="Microsoft.NET.Test.Sdk" /></ItemGroup></Project>"#,
        )
        .unwrap();
        let solution = [
            r#"Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "Loja.Web", "src\Loja.Web\Loja.Web.csproj", "{1}""#,
            r#"Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "Loja.Dominio", "src\Loja.Dominio\Loja.Dominio.csproj", "{2}""#,
            r#"Project("{F2A71F9B-5D33-465A-A702-920D77279786}") = "Importador", "tools\Importador\Importador.fsproj", "{3}""#,
            r#"Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "Loja.Testes", "tests\Loja.Testes\Loja.Testes.csproj", "{4}""#,
        ]
        .join("\n");
        fs::write(root.join("loja/Loja.sln"), solution).unwrap();
        // With Git the subfolders are visited too, and nothing is listed twice.
        fs::create_dir_all(root.join("loja/.git")).unwrap();
        let loja = detect_processes(&project("loja"))
            .iter()
            .map(|process| process.command_line())
            .collect::<Vec<_>>();
        assert_eq!(
            loja,
            [
                "dotnet run --project src/Loja.Web/Loja.Web.csproj",
                "dotnet run --project tools/Importador/Importador.fsproj",
            ]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_go_dart_and_flutter_projects() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("localcodepilot-new-runtimes-{nonce}"));
        let go = root.join("go-app");
        let dart = root.join("dart-app");
        let flutter = root.join("flutter-app");
        fs::create_dir_all(&go).unwrap();
        fs::create_dir_all(dart.join("bin")).unwrap();
        fs::create_dir_all(flutter.join("lib")).unwrap();
        fs::create_dir_all(flutter.join("web")).unwrap();
        fs::write(go.join("go.mod"), "module example.com/app\ngo 1.23\n").unwrap();
        fs::write(
            go.join("main.go"),
            "package main\nfunc main() { http.ListenAndServe(\":8000\", nil) }\n",
        )
        .unwrap();
        fs::write(
            dart.join("pubspec.yaml"),
            "name: sample\nenvironment:\n  sdk: '>=3.3.0 <4.0.0'\n",
        )
        .unwrap();
        fs::write(dart.join("bin/main.dart"), "void main() {}\n").unwrap();
        fs::write(
            flutter.join("pubspec.yaml"),
            "name: mobile\ndependencies:\n  flutter:\n    sdk: flutter\n",
        )
        .unwrap();
        fs::write(flutter.join("lib/main.dart"), "void main() {}\n").unwrap();
        fs::write(flutter.join("web/index.html"), "<html></html>\n").unwrap();

        assert_eq!(detect(&go), [RuntimeKind::Go]);
        assert_eq!(
            detect_go_requirement(&go, &go).unwrap().version.value,
            ">=1.23"
        );
        let go_processes = detect_processes(&Project::new(go.clone(), detect(&go)));
        assert_eq!(go_processes[0].command_line(), "go run .");
        assert_eq!(go_processes[0].expected_port, Port::new(8000));

        // The `cmd/<name>/main.go` layout: one command per program, from the module root.
        let layout = root.join("go-layout");
        fs::create_dir_all(layout.join("cmd/api")).unwrap();
        fs::create_dir_all(layout.join("cmd/worker")).unwrap();
        fs::create_dir_all(layout.join("internal")).unwrap();
        fs::write(
            layout.join("go.mod"),
            "module example.com/layout\ngo 1.23\n",
        )
        .unwrap();
        fs::write(
            layout.join("cmd/api/main.go"),
            "package main\nfunc main() { http.ListenAndServe(\":8091\", nil) }\n",
        )
        .unwrap();
        fs::write(
            layout.join("cmd/worker/main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();
        fs::write(layout.join("internal/lib.go"), "package internal\n").unwrap();
        let layout_processes = detect_processes(&Project::new(layout.clone(), detect(&layout)));
        assert_eq!(
            layout_processes
                .iter()
                .map(|process| (process.command_line(), process.expected_port))
                .collect::<Vec<_>>(),
            [
                ("go run ./cmd/api".to_owned(), Port::new(8091)),
                ("go run ./cmd/worker".to_owned(), None),
            ]
        );
        assert!(
            layout_processes
                .iter()
                .all(|process| process.working_directory.ends_with("go-layout"))
        );

        assert_eq!(detect(&dart), [RuntimeKind::Dart]);
        assert_eq!(
            detect_dart_requirement(&dart, &dart).unwrap().version.value,
            ">=3.3.0 <4.0.0"
        );
        assert_eq!(
            detect_processes(&Project::new(dart.clone(), detect(&dart)))[0].command_line(),
            "dart run bin/main.dart"
        );
        // Templates of `dart create`: the server and the package-named console program.
        fs::remove_file(dart.join("bin/main.dart")).unwrap();
        fs::write(dart.join("bin/server.dart"), "void main() {}\n").unwrap();
        assert_eq!(
            detect_processes(&Project::new(dart.clone(), detect(&dart)))[0].command_line(),
            "dart run bin/server.dart"
        );
        fs::remove_file(dart.join("bin/server.dart")).unwrap();
        fs::write(dart.join("bin/sample.dart"), "void main() {}\n").unwrap();
        assert_eq!(
            detect_processes(&Project::new(dart.clone(), detect(&dart)))[0].command_line(),
            "dart run bin/sample.dart"
        );

        assert_eq!(detect(&flutter), [RuntimeKind::Flutter]);
        assert_eq!(detect_technologies(&flutter), [TechnologyKind::Flutter]);
        assert!(detect_dart_requirement(&flutter, &flutter).is_none());
        let flutter_processes = detect_processes(&Project::new(flutter.clone(), detect(&flutter)));
        assert_eq!(flutter_processes.len(), 1);
        assert_eq!(
            flutter_processes[0].command_line(),
            "flutter run -d web-server --web-hostname 127.0.0.1 --web-port 4180"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
pub mod containers;
