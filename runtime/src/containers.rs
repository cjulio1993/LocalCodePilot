use localcodepilot_core::containers::{ContainerProject, ContainerService};
use serde_yaml_ng::Value;
use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::Path,
};

pub const COMPOSE_FILES: &[&str] = &[
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

pub fn has_container_markers(directory: &Path) -> bool {
    ["Dockerfile", "Containerfile"]
        .iter()
        .chain(COMPOSE_FILES)
        .any(|file| directory.join(file).is_file())
}

/// Whether the WordPress site in `site` is meant to run through Compose: a Compose manifest
/// in the site folder or above it, in one of its subfolders (`docker/`, `infra/`), or anywhere
/// in the project when it declares a WordPress image. A Compose file for an unrelated sibling
/// service does not count.
pub fn wordpress_uses_compose(project_root: &Path, site: &Path) -> bool {
    detect_container_projects(project_root)
        .iter()
        .filter(|project| project.compose_file.is_some())
        .any(|project| {
            site.starts_with(&project.directory)
                || project.directory.starts_with(site)
                || project.services.iter().any(|service| {
                    service
                        .image
                        .as_deref()
                        .is_some_and(|image| image.to_ascii_lowercase().contains("wordpress"))
                })
        })
}

pub fn detect_container_projects(root: &Path) -> Vec<ContainerProject> {
    let mut projects = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0)]);
    while let Some((directory, depth)) = queue.pop_front() {
        if !seen.insert(directory.clone()) {
            continue;
        }
        if has_container_markers(&directory) {
            let mut project = ContainerProject {
                dockerfile: ["Dockerfile", "Containerfile"]
                    .iter()
                    .map(|file| directory.join(file))
                    .find(|file| file.is_file()),
                compose_file: COMPOSE_FILES
                    .iter()
                    .map(|file| directory.join(file))
                    .find(|file| file.is_file()),
                compose_overrides: [
                    "compose.override.yaml",
                    "compose.override.yml",
                    "docker-compose.override.yaml",
                    "docker-compose.override.yml",
                ]
                .iter()
                .map(|file| directory.join(file))
                .filter(|file| file.is_file())
                .collect(),
                directory: directory.clone(),
                services: vec![],
                warnings: vec![],
                error: None,
            };
            if let Some(file) = &project.compose_file {
                match parse_compose(file) {
                    Ok((services, warnings)) => {
                        project.services = services;
                        project.warnings = warnings;
                    }
                    Err(error) => project.error = Some(error),
                }
                if COMPOSE_FILES
                    .iter()
                    .filter(|name| directory.join(name).is_file())
                    .count()
                    > 1
                {
                    project.warnings.push(
                        "Há mais de um manifesto Compose; a prévia usa o arquivo indicado.".into(),
                    );
                }
                if !project.compose_overrides.is_empty() {
                    project.warnings.push("Arquivo de override encontrado. A prévia mostra somente o manifesto principal; a validação e a execução enviam os arquivos ao Compose na ordem exibida.".into());
                }
                project.warnings.push("Prévia das declarações do arquivo: variáveis, perfis e referências externas não foram resolvidos. Valide a configuração efetiva com Compose antes de iniciar.".into());
            }
            projects.push(project);
        }
        if depth >= 8 {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&directory) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                    && !name.starts_with('.')
                    && !super::IGNORED_DIRECTORIES
                        .iter()
                        .any(|ignored| name.eq_ignore_ascii_case(ignored))
                    && !entry.path().join(".localcodepilot-ignore").is_file()
                {
                    queue.push_back((entry.path(), depth + 1));
                }
            }
        }
    }
    projects.sort_by(|a, b| a.directory.cmp(&b.directory));
    projects
}

fn parse_compose(file: &Path) -> Result<(Vec<ContainerService>, Vec<String>), String> {
    if fs::metadata(file)
        .map_err(|_| "Não foi possível ler o manifesto Compose")?
        .len()
        > 1_048_576
    {
        return Err("O manifesto excede o limite de 1 MiB desta prévia".into());
    }
    let contents =
        fs::read_to_string(file).map_err(|_| "Não foi possível ler o manifesto Compose")?;
    let document: Value = serde_yaml_ng::from_str(&contents).map_err(|error| {
        error.location().map_or_else(
            || "YAML inválido no manifesto Compose".into(),
            |location| {
                format!(
                    "YAML inválido na linha {}, coluna {}",
                    location.line(),
                    location.column()
                )
            },
        )
    })?;
    let services = document.get("services").and_then(Value::as_mapping)
        .filter(|services| !services.is_empty())
        .ok_or("Nenhum mapa services válido no arquivo base. Serviços externos não são resolvidos nesta prévia.")?;
    let mut warnings = Vec::new();
    if document.get("include").is_some() {
        warnings.push("Arquivos include não foram carregados nesta prévia.".into());
    }
    let mut result = Vec::new();
    for (name, service) in services {
        let name = name
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or("Nome de serviço inválido")?;
        if service.as_mapping().is_none() {
            return Err(format!("A declaração de {name} deve ser um mapa"));
        }
        if service.get("extends").is_some() || service.get("<<").is_some() {
            warnings.push(format!(
                "{name}: extends ou fragmentos herdados não foram resolvidos."
            ));
        }
        let ports = service
            .get("ports")
            .and_then(Value::as_sequence)
            .map(|ports| ports.iter().filter_map(port_label).collect())
            .unwrap_or_default();
        let profiles = service
            .get("profiles")
            .and_then(Value::as_sequence)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        result.push(ContainerService {
            name: name.into(),
            image: service
                .get("image")
                .and_then(Value::as_str)
                .map(str::to_owned),
            builds_image: service.get("build").is_some(),
            ports,
            profiles,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((result, warnings))
}

fn scalar(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|value| value.to_string()))
}

fn port_label(value: &Value) -> Option<String> {
    if let Some(short) = scalar(value) {
        return Some(short);
    }
    let target = scalar(value.get("target")?)?;
    let published = value.get("published").and_then(scalar);
    let host = value.get("host_ip").and_then(Value::as_str);
    let protocol = value
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("tcp");
    Some(match (host, published) {
        (Some(host), Some(port)) => format!("{host}:{port}:{target}/{protocol}"),
        (None, Some(port)) => format!("{port}:{target}/{protocol}"),
        (_, None) => format!("porta dinâmica → {target}/{protocol}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_container_projects_and_retains_native_stack() {
        let root = fixture("classification");
        fs::write(root.join("Dockerfile"), "FROM scratch").unwrap();
        assert_eq!(
            crate::detect_technologies(&root),
            [localcodepilot_core::technologies::TechnologyKind::Containers]
        );
        fs::write(root.join("package.json"), "{}").unwrap();
        let technologies = crate::detect_technologies(&root);
        assert!(
            technologies.contains(&localcodepilot_core::technologies::TechnologyKind::Containers)
        );
        assert!(
            technologies.contains(&localcodepilot_core::technologies::TechnologyKind::JavaScript)
        );
        assert!(crate::detect(&root).contains(&localcodepilot_core::runtimes::RuntimeKind::Node));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flags_overrides_includes_and_external_references_as_unresolved() {
        let root = fixture("references");
        fs::write(root.join("compose.yaml"), "include: [other.yaml]\nservices:\n  web:\n    extends:\n      file: base.yaml\n      service: web\n").unwrap();
        fs::write(root.join("compose.override.yaml"), "services: {}\n").unwrap();
        let projects = detect_container_projects(&root);
        assert!(
            projects[0]
                .warnings
                .iter()
                .any(|message| message.contains("override"))
        );
        assert!(
            projects[0]
                .warnings
                .iter()
                .any(|message| message.contains("include"))
        );
        assert!(
            projects[0]
                .warnings
                .iter()
                .any(|message| message.contains("extends"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("lcp-containers-{name}-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn parses_base_services_profiles_and_both_port_forms_without_secrets() {
        let root = fixture("compose");
        fs::write(root.join("compose.yaml"), "services:\n  web:\n    image: nginx:alpine\n    ports: [\"${PORT:-8080}:80\"]\n    environment:\n      PASSWORD: private-token\n  worker:\n    build: .\n    profiles: [jobs]\n    ports:\n      - target: 9000\n        published: \"9001\"\n        host_ip: 127.0.0.1\n        protocol: udp\n").unwrap();
        fs::write(root.join("docker-compose.yml"), "invalid: [").unwrap();
        let projects = detect_container_projects(&root);
        let project = &projects[0];
        assert!(project.error.is_none());
        assert_eq!(project.services.len(), 2);
        assert_eq!(project.services[0].ports, ["${PORT:-8080}:80"]);
        assert_eq!(project.services[1].ports, ["127.0.0.1:9001:9000/udp"]);
        assert_eq!(project.services[1].profiles, ["jobs"]);
        assert!(project.services[1].builds_image);
        assert!(!format!("{project:?}").contains("private-token"));
        assert!(
            project
                .compose_file
                .as_ref()
                .unwrap()
                .ends_with("compose.yaml")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn diagnoses_invalid_yaml_without_offering_commands() {
        let root = fixture("invalid");
        fs::write(root.join("compose.yml"), "services: [").unwrap();
        let projects = detect_container_projects(&root);
        assert!(
            projects[0]
                .error
                .as_ref()
                .unwrap()
                .contains("YAML inválido")
        );
        assert!(
            projects[0]
                .proposed_commands(localcodepilot_core::containers::ContainerEngine::Docker)
                .is_empty()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_dockerfile_and_skips_ignored_fixtures_and_dependencies() {
        let root = fixture("scan");
        for directory in ["app", "qa/sample", "node_modules/dependency"] {
            fs::create_dir_all(root.join(directory)).unwrap();
            fs::write(root.join(directory).join("Dockerfile"), "FROM scratch").unwrap();
        }
        fs::write(root.join("qa/.localcodepilot-ignore"), "").unwrap();
        let projects = detect_container_projects(&root);
        assert_eq!(projects.len(), 1);
        assert!(projects[0].directory.ends_with("app"));
        assert_eq!(
            projects[0].proposed_commands(localcodepilot_core::containers::ContainerEngine::Podman)
                [0]
            .args[0],
            "build"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
