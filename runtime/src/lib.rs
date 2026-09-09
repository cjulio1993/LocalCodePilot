use localcodepilot_core::{discovery::RuntimeDetector, runtimes::RuntimeKind};
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
}
