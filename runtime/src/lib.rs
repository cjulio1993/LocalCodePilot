use localcodepilot_core::{
    discovery::RuntimeDetector,
    processes::{ProcessState, ProjectProcess},
    projects::Project,
    runtimes::RuntimeKind,
};
use std::{collections::VecDeque, fs, path::Path};

const CANDIDATES: &[(&str, RuntimeKind)] = &[
    ("Cargo.toml", RuntimeKind::Rust),
    ("package.json", RuntimeKind::Node),
    ("composer.json", RuntimeKind::Php),
    ("index.php", RuntimeKind::Php),
    ("wp-config.php", RuntimeKind::Php),
    ("pyproject.toml", RuntimeKind::Python),
    ("requirements.txt", RuntimeKind::Python),
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
];

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
    processes
}

fn detect_processes_in_directory(
    project: &Project,
    directory: &Path,
    processes: &mut Vec<ProjectProcess>,
) {
    if directory.join("Cargo.toml").is_file() {
        push_process(
            processes,
            project,
            directory,
            "Executar projeto",
            "cargo",
            &["run"],
        );
    }
    detect_json_scripts(
        project,
        directory,
        "package.json",
        "npm",
        &["run"],
        processes,
    );
    detect_json_scripts(
        project,
        directory,
        "composer.json",
        "composer",
        &["run-script"],
        processes,
    );
    if directory.join("artisan").is_file() {
        push_process(
            processes,
            project,
            directory,
            "Servidor Laravel",
            "php",
            &["artisan", "serve"],
        );
    } else if directory.join("index.php").is_file() || directory.join("wp-config.php").is_file() {
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
            push_process(processes, project, directory, name, "python", &args);
        }
    }
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
        .keys()
        .filter(|name| !name.starts_with("pre") && !name.starts_with("post"))
        .filter(|name| {
            name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':' | '.')
            })
        })
        .collect();
    names.sort();
    for name in names {
        let mut args = prefix.to_vec();
        args.push(name);
        push_process(processes, project, directory, name, program, &args);
    }
}

fn push_process(
    processes: &mut Vec<ProjectProcess>,
    project: &Project,
    working_directory: &Path,
    name: &str,
    program: &str,
    args: &[&str],
) {
    let args: Vec<_> = args.iter().map(|value| (*value).to_owned()).collect();
    let command = std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
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
    });
}

fn detect_in_directory(path: &Path, found: &mut Vec<RuntimeKind>) {
    for &(marker, runtime) in CANDIDATES {
        if path.join(marker).is_file() && !found.contains(&runtime) {
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

#[derive(Debug, Default, Clone, Copy)]
pub struct ManifestRuntimeDetector;

impl RuntimeDetector for ManifestRuntimeDetector {
    fn detect(&self, path: &Path) -> Vec<RuntimeKind> {
        detect(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::SystemTime};

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
            r#"{"scripts":{"dev":"vite","test":"vitest","predev":"prepare"}}"#,
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

        assert_eq!(
            commands,
            ["npm run dev", "npm run test", "composer run-script serve"]
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn detects_common_rust_php_and_python_commands() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-commands-{nonce}"));
        fs::create_dir(&path).unwrap();
        for file in ["Cargo.toml", "artisan", "manage.py"] {
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
}
