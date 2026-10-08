use localcodepilot_core::{
    discovery::RuntimeDetector,
    environments::{RequirementSource, RequirementStatus, RuntimeRequirement, VersionConstraint},
    ports::Port,
    processes::{PortOverride, ProcessState, ProjectProcess},
    projects::Project,
    runtimes::RuntimeKind,
    technologies::TechnologyKind,
};
use semver::{Version, VersionReq};
use std::{collections::VecDeque, fs, path::Path};

const CANDIDATES: &[(&str, RuntimeKind)] = &[
    ("Cargo.toml", RuntimeKind::Rust),
    ("package.json", RuntimeKind::Node),
    ("composer.json", RuntimeKind::Php),
    ("index.php", RuntimeKind::Php),
    ("wp-config.php", RuntimeKind::Php),
    ("pyproject.toml", RuntimeKind::Python),
    ("requirements.txt", RuntimeKind::Python),
    ("pom.xml", RuntimeKind::Java),
    ("build.gradle", RuntimeKind::Java),
    ("build.gradle.kts", RuntimeKind::Java),
    ("gradlew", RuntimeKind::Java),
    ("gradlew.bat", RuntimeKind::Java),
    ("go.mod", RuntimeKind::Go),
];
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
    let marker = ["composer.json", "artisan", "index.php", "wp-config.php"]
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
    let Some(path) = find_in_project_ancestors(directory, project_root, "composer.json") else {
        return Vec::new();
    };
    let Some(requirements) = fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .and_then(|document| document.get("require").cloned())
        .and_then(|requirements| requirements.as_object().cloned())
    else {
        return Vec::new();
    };
    let mut extensions = requirements
        .keys()
        .filter_map(|name| name.strip_prefix("ext-"))
        .map(|name| name.to_ascii_lowercase().replace('-', "_"))
        .collect::<Vec<_>>();
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
    if fs::read_to_string(path.join("pubspec.yaml"))
        .ok()
        .is_some_and(|contents| is_flutter_pubspec(&contents))
    {
        return vec![RuntimeKind::Flutter];
    }
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
                    && !IGNORED_DIRECTORIES
                        .iter()
                        .any(|ignored| name.eq_ignore_ascii_case(ignored))
                {
                    queue.push_back((entry.path(), depth + 1));
                }
            }
        }
    }
    if found.contains(&RuntimeKind::Flutter) {
        found.retain(|runtime| *runtime != RuntimeKind::Dart);
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

    if path.join(".git").exists() {
        let mut queue = VecDeque::from([(path.to_path_buf(), 0_usize)]);
        while let Some((directory, depth)) = queue.pop_front() {
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
                    && !IGNORED_DIRECTORIES
                        .iter()
                        .any(|ignored| name.eq_ignore_ascii_case(ignored))
                {
                    queue.push_back((entry.path(), depth + 1));
                }
            }
        }
    }
    found.sort_by_key(|technology| technology_priority(*technology));
    found
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
    processes
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
            || (directory == project.path
                && project.runtimes.contains(&RuntimeKind::Php)
                && directory.join("index.html").is_file()))
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

fn detect_go_process(project: &Project, directory: &Path, processes: &mut Vec<ProjectProcess>) {
    if !directory.join("go.mod").is_file() {
        return;
    }
    let has_main_package = fs::read_dir(directory).ok().is_some_and(|entries| {
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
    });
    if has_main_package {
        push_process(
            processes,
            project,
            directory,
            "Executar aplicação Go",
            "go",
            &["run", "."],
        );
        if let Some(port) = detect_go_server_port(directory)
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

    let entrypoint = ["bin/main.dart", "main.dart", "lib/main.dart"]
        .into_iter()
        .find(|path| directory.join(path).is_file());
    if let Some(entrypoint) = entrypoint {
        push_process(
            processes,
            project,
            directory,
            "Executar aplicação Dart",
            "dart",
            &["run", entrypoint],
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
            directory.join(wrapper).to_string_lossy().into_owned()
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

    let gradle = ["build.gradle", "build.gradle.kts"]
        .iter()
        .find_map(|name| fs::read_to_string(directory.join(name)).ok());
    if gradle.as_deref().is_some_and(|contents| {
        contents.contains("org.springframework.boot") || contents.contains("bootRun")
    }) {
        let wrapper = if cfg!(windows) {
            "gradlew.bat"
        } else {
            "gradlew"
        };
        let program = if directory.join(wrapper).is_file() {
            directory.join(wrapper).to_string_lossy().into_owned()
        } else {
            "gradle".into()
        };
        push_process(
            processes,
            project,
            directory,
            "Servidor Spring Boot",
            &program,
            &["bootRun"],
        );
    }
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
    let Ok(contents) = fs::read_to_string(directory.join(manifest)) else {
        return;
    };
    let Ok(document) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return;
    };
    let Some(scripts) = document.get("scripts").and_then(|value| value.as_object()) else {
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

fn detect_in_directory(path: &Path, found: &mut Vec<RuntimeKind>) {
    for &(marker, runtime) in CANDIDATES {
        if path.join(marker).is_file() && !found.contains(&runtime) {
            found.push(runtime);
        }
    }
    if let Ok(contents) = fs::read_to_string(path.join("pubspec.yaml")) {
        let runtime = if is_flutter_pubspec(&contents) {
            RuntimeKind::Flutter
        } else {
            RuntimeKind::Dart
        };
        if !found.contains(&runtime) {
            found.push(runtime);
        }
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let entry_path = entry.path();
        let Some(extension) = entry_path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if let Some(runtime) = RuntimeKind::from_source_extension(extension)
            && !found.contains(&runtime)
        {
            found.push(runtime);
        }
    }
}

fn detect_technologies_in_directory(path: &Path, found: &mut Vec<TechnologyKind>) {
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
        TechnologyKind::Flutter => 2,
        TechnologyKind::StaticSite => 3,
        TechnologyKind::Vue => 4,
        TechnologyKind::React => 5,
        TechnologyKind::TypeScript => 6,
        TechnologyKind::JavaScript => 7,
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
    use super::*;
    use std::{fs, time::SystemTime};

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

        assert_eq!(detect(&dart), [RuntimeKind::Dart]);
        assert_eq!(
            detect_dart_requirement(&dart, &dart).unwrap().version.value,
            ">=3.3.0 <4.0.0"
        );
        assert_eq!(
            detect_processes(&Project::new(dart.clone(), detect(&dart)))[0].command_line(),
            "dart run bin/main.dart"
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
