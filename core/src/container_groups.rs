//! Puts the Compose environments and Dockerfiles found in catalogued projects together with
//! the containers the engines report, so a screen can show each project's services and
//! their state in one place.

use crate::compose_safety::ComposeName;
use crate::containers::{
    ContainerEngine, ContainerProject, ContainerService, RuntimeContainer, path_is_within,
    same_folder,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    Restarting,
    Paused,
    Stopped,
    Problem,
    NotCreated,
}

impl ServiceState {
    pub fn of(container: &RuntimeContainer) -> Self {
        match container.state.to_ascii_lowercase().as_str() {
            "running" => Self::Running,
            "restarting" => Self::Restarting,
            "paused" => Self::Paused,
            "dead" => Self::Problem,
            "exited" if exit_code(&container.status).is_some_and(is_failure) => Self::Problem,
            _ => Self::Stopped,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Rodando",
            Self::Restarting => "Reiniciando",
            Self::Paused => "Pausado",
            Self::Stopped => "Parado",
            Self::Problem => "Com problema",
            Self::NotCreated => "Ainda não criado",
        }
    }

    /// Whether the service is up, so stopping the environment would affect it.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Restarting)
    }
}

/// Exit code in Docker's `Exited (1) 2 minutes ago`. Podman leaves the status empty.
fn exit_code(status: &str) -> Option<i32> {
    let rest = status.trim().strip_prefix("Exited (")?;
    rest.split_once(')')?.0.trim().parse().ok()
}

/// Codes of a normal stop or an interruption: 130 (Ctrl+C), 137 (killed after the stop
/// timeout, which may also be out of memory) and 143 (terminated) do not prove a failure.
fn is_failure(code: i32) -> bool {
    !matches!(code, 0 | 130 | 137 | 143)
}

/// A service declared in the file, a container the engine reports, or both.
#[derive(Debug, Clone)]
pub struct ServiceRow<'a> {
    pub name: String,
    pub declared: Option<&'a ContainerService>,
    pub container: Option<&'a RuntimeContainer>,
}

impl ServiceRow<'_> {
    pub fn state(&self) -> ServiceState {
        self.container
            .map_or(ServiceState::NotCreated, ServiceState::of)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentState {
    /// The file could not be read or a service failed.
    Problem,
    /// Some services are up.
    Running {
        active: usize,
        total: usize,
    },
    Stopped,
    /// A Dockerfile without Compose: there is only an image to build.
    ImageOnly,
}

/// Why a running service is out of reach of `compose stop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    OptionalProfile,
    NotInFile,
}

impl KeepReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::OptionalProfile => "perfil opcional",
            Self::NotInFile => "não está mais no arquivo",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StopScope {
    pub will_stop: Vec<String>,
    pub already_stopped: Vec<String>,
    pub keeps_running: Vec<(String, KeepReason)>,
}

#[derive(Debug, Clone)]
pub struct EnvironmentGroup<'a> {
    pub project: &'a ContainerProject,
    pub name: Option<&'a ComposeName>,
    pub services: Vec<ServiceRow<'a>>,
}

impl EnvironmentGroup<'_> {
    pub fn state(&self) -> EnvironmentState {
        if self.project.error.is_some()
            || self
                .services
                .iter()
                .any(|row| row.state() == ServiceState::Problem)
        {
            return EnvironmentState::Problem;
        }
        if self.project.compose_file.is_none() {
            return EnvironmentState::ImageOnly;
        }
        let active = self
            .services
            .iter()
            .filter(|row| row.state().is_active())
            .count();
        if active > 0 {
            EnvironmentState::Running {
                active,
                total: self.services.len(),
            }
        } else {
            EnvironmentState::Stopped
        }
    }

    pub fn paused(&self) -> usize {
        self.services
            .iter()
            .filter(|row| row.state() == ServiceState::Paused)
            .count()
    }

    /// Engines that run at least one container of this environment, in a stable order.
    pub fn engines_in_use(&self) -> Vec<ContainerEngine> {
        let mut engines = Vec::new();
        for engine in [ContainerEngine::Docker, ContainerEngine::Podman] {
            if self.services.iter().any(|row| {
                row.container.is_some_and(|container| {
                    container.engine == engine
                        && matches!(
                            ServiceState::of(container),
                            ServiceState::Running | ServiceState::Restarting | ServiceState::Paused
                        )
                })
            }) {
                engines.push(engine);
            }
        }
        engines
    }

    /// What `compose stop` on `engine` reaches, as tested on Docker Compose with both engines:
    /// only services of the file without a profile. Running services of an optional profile
    /// and containers the file no longer declares keep running. `None` when that cannot be
    /// stated: no name confirmed by Compose, a paused service, or containers of this
    /// environment up on the other engine.
    pub fn stop_scope(&self, engine: ContainerEngine) -> Option<StopScope> {
        if !matches!(self.name, Some(ComposeName::Resolved(_)))
            || self.project.compose_file.is_none()
        {
            return None;
        }
        let mut scope = StopScope::default();
        for row in &self.services {
            let Some(container) = row.container else {
                continue;
            };
            let state = ServiceState::of(container);
            if container.engine != engine {
                if state != ServiceState::Stopped && state != ServiceState::Problem {
                    return None;
                }
                continue;
            }
            if state == ServiceState::Paused {
                return None;
            }
            let reached = row
                .declared
                .is_some_and(|service| service.profiles.is_empty());
            match (state.is_active(), reached) {
                (true, true) => scope.will_stop.push(row.name.clone()),
                (false, true) => scope.already_stopped.push(row.name.clone()),
                (true, false) => scope.keeps_running.push((
                    row.name.clone(),
                    if row.declared.is_some() {
                        KeepReason::OptionalProfile
                    } else {
                        KeepReason::NotInFile
                    },
                )),
                (false, false) => {}
            }
        }
        Some(scope)
    }

    fn matches(&self, filter: &str) -> bool {
        contains(&self.project.directory.to_string_lossy(), filter)
            || self.name.is_some_and(|name| contains(name.value(), filter))
            || self.services.iter().any(|row| {
                contains(&row.name, filter)
                    || row
                        .container
                        .is_some_and(|container| contains(&container.name, filter))
            })
    }
}

#[derive(Debug, Clone)]
pub struct ProjectGroup<'a> {
    pub root: &'a Path,
    pub name: &'a str,
    pub environments: Vec<EnvironmentGroup<'a>>,
}

impl ProjectGroup<'_> {
    /// Folder of an environment relative to the project, such as `docker`; empty for the
    /// project folder itself.
    pub fn relative_folder(&self, environment: &EnvironmentGroup) -> String {
        let directory = &environment.project.directory;
        if same_folder(directory, self.root) {
            return String::new();
        }
        // Engines and the catalog may spell the same folder differently; the last component
        // still names it.
        directory
            .strip_prefix(self.root)
            .ok()
            .or_else(|| directory.file_name().map(Path::new))
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    }

    /// Case-insensitive match on the project, folder, Compose name, service or container.
    pub fn matches(&self, filter: &str) -> bool {
        let filter = filter.trim().to_lowercase();
        filter.is_empty()
            || contains(self.name, &filter)
            || self
                .environments
                .iter()
                .any(|environment| environment.matches(&filter))
    }
}

fn contains(text: &str, filter: &str) -> bool {
    text.to_lowercase().contains(filter)
}

#[derive(Debug, Clone, Default)]
pub struct ContainerOverview<'a> {
    pub projects: Vec<ProjectGroup<'a>>,
    /// Containers that cannot be tied to an environment of the catalog with certainty.
    pub others: Vec<&'a RuntimeContainer>,
}

/// A container belongs to an environment only when the engine recorded the same folder and
/// the same Compose project name; anything less certain goes to `others`.
fn belongs_to_environment(
    container: &RuntimeContainer,
    environment: &ContainerProject,
    name: Option<&ComposeName>,
) -> bool {
    let Some(name) = name else {
        return false;
    };
    container
        .project_directory
        .as_deref()
        .is_some_and(|folder| same_folder(folder, &environment.directory))
        && container.compose_project.as_deref() == Some(name.value())
}

/// `catalog` lists the catalogued projects as `(name, folder)`.
pub fn group_containers<'a>(
    catalog: &[(&'a str, &'a Path)],
    environments: &'a [ContainerProject],
    names: &'a HashMap<PathBuf, ComposeName>,
    containers: &'a [RuntimeContainer],
) -> ContainerOverview<'a> {
    let mut claimed = vec![false; containers.len()];
    let mut projects: Vec<ProjectGroup<'a>> = Vec::new();
    for environment in environments {
        let name = names.get(&environment.directory).or_else(|| {
            names
                .iter()
                .find(|(folder, _)| same_folder(folder, &environment.directory))
                .map(|(_, name)| name)
        });
        let mut services = Vec::new();
        let mut own = containers
            .iter()
            .enumerate()
            .filter(|(_, container)| belongs_to_environment(container, environment, name))
            .collect::<Vec<_>>();
        for declared in &environment.services {
            let before = services.len();
            own.retain(|(index, container)| {
                if container.service.as_deref() == Some(declared.name.as_str()) {
                    claimed[*index] = true;
                    services.push(ServiceRow {
                        name: declared.name.clone(),
                        declared: Some(declared),
                        container: Some(*container),
                    });
                    false
                } else {
                    true
                }
            });
            if services.len() == before {
                services.push(ServiceRow {
                    name: declared.name.clone(),
                    declared: Some(declared),
                    container: None,
                });
            }
        }
        // Containers of this environment that the file no longer declares.
        for (index, container) in own {
            claimed[index] = true;
            services.push(ServiceRow {
                name: container
                    .service
                    .clone()
                    .unwrap_or_else(|| container.name.clone()),
                declared: None,
                container: Some(container),
            });
        }
        let group = EnvironmentGroup {
            project: environment,
            name,
            services,
        };
        // The deepest catalogued folder that contains the environment.
        let owner = catalog
            .iter()
            .filter(|(_, root)| path_is_within(&environment.directory, root))
            .max_by_key(|(_, root)| root.components().count());
        let (project_name, root) = match owner {
            Some((name, root)) => (*name, *root),
            None => (
                environment
                    .directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default(),
                environment.directory.as_path(),
            ),
        };
        match projects
            .iter_mut()
            .find(|project| same_folder(project.root, root))
        {
            Some(project) => project.environments.push(group),
            None => projects.push(ProjectGroup {
                root,
                name: project_name,
                environments: vec![group],
            }),
        }
    }
    let others = containers
        .iter()
        .zip(claimed)
        .filter(|(_, claimed)| !claimed)
        .map(|(container, _)| container)
        .collect();
    ContainerOverview { projects, others }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str) -> ContainerService {
        ContainerService {
            name: name.into(),
            image: None,
            builds_image: false,
            ports: vec![],
            profiles: vec![],
        }
    }

    fn environment(directory: &str, services: &[&str]) -> ContainerProject {
        ContainerProject {
            directory: directory.into(),
            dockerfile: None,
            compose_file: Some(PathBuf::from(directory).join("compose.yaml")),
            compose_overrides: vec![],
            base_images: vec![],
            services: services.iter().map(|name| service(name)).collect(),
            warnings: vec![],
            error: None,
        }
    }

    fn container(
        engine: ContainerEngine,
        service: &str,
        project: &str,
        folder: &str,
        state: &str,
        status: &str,
    ) -> RuntimeContainer {
        RuntimeContainer {
            engine,
            id: "abcdef123456".into(),
            name: format!("{project}-{service}-1"),
            image: "image".into(),
            state: state.into(),
            status: status.into(),
            ports: String::new(),
            service: Some(service.into()),
            compose_project: Some(project.into()),
            project_directory: Some(folder.into()),
            running_for: None,
        }
    }

    #[test]
    fn services_join_the_file_and_the_engine_inside_their_project() {
        let catalog = [
            ("wp-blog", Path::new("C:/dev/wp-blog")),
            ("loja", Path::new("C:/dev/loja")),
        ];
        let environments = [
            environment("C:/dev/wp-blog/docker", &["wordpress", "db"]),
            environment("C:/dev/loja", &["api"]),
        ];
        let names = HashMap::from([
            (
                PathBuf::from("C:/dev/wp-blog/docker"),
                ComposeName::Resolved("docker".into()),
            ),
            (
                PathBuf::from("C:/dev/loja"),
                ComposeName::Resolved("loja".into()),
            ),
        ]);
        let docker = ContainerEngine::Docker;
        // Engines on Windows may record the folder with other casing and separators.
        let wordpress_folder = if cfg!(windows) {
            "c:\\dev\\wp-blog\\docker"
        } else {
            "C:/dev/wp-blog/docker"
        };
        let containers = [
            container(
                docker,
                "wordpress",
                "docker",
                wordpress_folder,
                "running",
                "Up 2 minutes",
            ),
            container(
                docker,
                "db",
                "docker",
                "C:/dev/wp-blog/docker",
                "running",
                "Up",
            ),
            // An old service the file no longer declares.
            container(
                docker,
                "cache",
                "docker",
                "C:/dev/wp-blog/docker",
                "exited",
                "Exited (0) 1 hour ago",
            ),
            // Same Compose name from a folder outside the catalog: not wp-blog's.
            container(docker, "db", "docker", "C:/other/docker", "running", "Up"),
            // Same folder, another Compose name.
            container(docker, "api", "outro", "C:/dev/loja", "running", "Up"),
        ];
        let overview = group_containers(&catalog, &environments, &names, &containers);

        assert_eq!(overview.projects.len(), 2);
        let blog = &overview.projects[0];
        assert_eq!(blog.name, "wp-blog");
        assert_eq!(blog.relative_folder(&blog.environments[0]), "docker");
        let rows = &blog.environments[0].services;
        assert_eq!(
            rows.iter()
                .map(|row| (row.name.as_str(), row.state(), row.declared.is_some()))
                .collect::<Vec<_>>(),
            [
                ("wordpress", ServiceState::Running, true),
                ("db", ServiceState::Running, true),
                ("cache", ServiceState::Stopped, false),
            ]
        );
        assert_eq!(
            blog.environments[0].state(),
            EnvironmentState::Running {
                active: 2,
                total: 3
            }
        );
        assert_eq!(blog.environments[0].engines_in_use(), [docker]);

        let shop = &overview.projects[1];
        assert_eq!(shop.relative_folder(&shop.environments[0]), "");
        assert_eq!(
            shop.environments[0].services[0].state(),
            ServiceState::NotCreated
        );
        assert_eq!(shop.environments[0].state(), EnvironmentState::Stopped);
        assert!(shop.environments[0].engines_in_use().is_empty());

        assert_eq!(
            overview
                .others
                .iter()
                .map(|container| container.name.as_str())
                .collect::<Vec<_>>(),
            ["docker-db-1", "outro-api-1"]
        );
    }

    #[test]
    fn an_environment_without_a_known_name_claims_no_container() {
        let catalog = [("app", Path::new("C:/dev/app"))];
        let environments = [environment("C:/dev/app", &["web"])];
        let containers = [container(
            ContainerEngine::Podman,
            "web",
            "app",
            "C:/dev/app",
            "running",
            "",
        )];
        let no_names = HashMap::new();
        let overview = group_containers(&catalog, &environments, &no_names, &containers);
        assert_eq!(
            overview.projects[0].environments[0].services[0].state(),
            ServiceState::NotCreated
        );
        assert_eq!(overview.others.len(), 1);
    }

    /// Mirrors a disposable run on Docker and Podman: `compose stop` without `--profile`
    /// stopped `web` only; `extra` (optional profile) and `old` (removed from the file) kept
    /// running.
    #[test]
    fn stop_reaches_only_services_of_the_file_without_a_profile() {
        let catalog = [("app", Path::new("C:/dev/app"))];
        let mut file = environment("C:/dev/app", &["web", "extra", "db"]);
        file.services[1].profiles = vec!["extra".into()];
        let environments = [file];
        let names = HashMap::from([(
            PathBuf::from("C:/dev/app"),
            ComposeName::Resolved("app".into()),
        )]);
        let docker = ContainerEngine::Docker;
        let mut containers = vec![
            container(docker, "web", "app", "C:/dev/app", "running", "Up"),
            container(docker, "extra", "app", "C:/dev/app", "running", "Up"),
            container(docker, "db", "app", "C:/dev/app", "exited", "Exited (0)"),
            container(docker, "old", "app", "C:/dev/app", "running", "Up"),
        ];
        let scope = |containers: &[RuntimeContainer], names: &HashMap<PathBuf, ComposeName>| {
            group_containers(&catalog, &environments, names, containers).projects[0].environments[0]
                .stop_scope(docker)
        };
        assert_eq!(
            scope(&containers, &names),
            Some(StopScope {
                will_stop: vec!["web".into()],
                already_stopped: vec!["db".into()],
                keeps_running: vec![
                    ("extra".into(), KeepReason::OptionalProfile),
                    ("old".into(), KeepReason::NotInFile),
                ],
            })
        );

        // Cannot be stated: a paused service, the other engine running it, or a name the
        // Compose provider did not confirm.
        containers[0].state = "paused".into();
        assert_eq!(scope(&containers, &names), None);
        containers[0].state = "running".into();
        containers[2].engine = ContainerEngine::Podman;
        containers[2].state = "running".into();
        assert_eq!(scope(&containers, &names), None);
        containers[2].state = "exited".into();
        let estimated = HashMap::from([(
            PathBuf::from("C:/dev/app"),
            ComposeName::Estimated("app".into()),
        )]);
        assert_eq!(scope(&containers, &estimated), None);
    }

    #[test]
    fn two_environments_of_one_project_stay_in_one_group() {
        let catalog = [("app", Path::new("C:/dev/app"))];
        let environments = [
            environment("C:/dev/app/docker", &["web"]),
            environment("C:/dev/app/infra", &["db"]),
        ];
        let no_names = HashMap::new();
        let overview = group_containers(&catalog, &environments, &no_names, &[]);
        assert_eq!(overview.projects.len(), 1);
        let project = &overview.projects[0];
        assert_eq!(
            project
                .environments
                .iter()
                .map(|environment| project.relative_folder(environment))
                .collect::<Vec<_>>(),
            ["docker", "infra"]
        );
        assert!(project.matches("INFRA"));
        assert!(project.matches("db"));
        assert!(project.matches("  "));
        assert!(!project.matches("loja"));
    }

    #[test]
    fn states_claim_a_failure_only_when_the_exit_code_proves_it() {
        let state = |state: &str, status: &str| {
            ServiceState::of(&container(
                ContainerEngine::Docker,
                "web",
                "app",
                "C:/app",
                state,
                status,
            ))
        };
        assert_eq!(state("running", "Up 5 seconds"), ServiceState::Running);
        assert_eq!(
            state("Restarting", "Restarting (1) 3 seconds ago"),
            ServiceState::Restarting
        );
        assert_eq!(state("paused", "Up 1 hour (Paused)"), ServiceState::Paused);
        assert_eq!(
            state("exited", "Exited (1) 2 minutes ago"),
            ServiceState::Problem
        );
        assert_eq!(
            state("exited", "Exited (0) 2 minutes ago"),
            ServiceState::Stopped
        );
        assert_eq!(
            state("exited", "Exited (137) 2 minutes ago"),
            ServiceState::Stopped
        );
        assert_eq!(state("exited", ""), ServiceState::Stopped);
        assert_eq!(state("created", "Created"), ServiceState::Stopped);
        assert_eq!(state("dead", "Dead"), ServiceState::Problem);
    }

    #[test]
    fn a_failed_service_or_a_broken_file_marks_the_environment() {
        let mut broken = environment("C:/app", &[]);
        broken.error = Some("YAML inválido".into());
        let group = EnvironmentGroup {
            project: &broken,
            name: None,
            services: vec![],
        };
        assert_eq!(group.state(), EnvironmentState::Problem);

        let mut image = environment("C:/img", &[]);
        image.compose_file = None;
        image.dockerfile = Some("C:/img/Dockerfile".into());
        let group = EnvironmentGroup {
            project: &image,
            name: None,
            services: vec![],
        };
        assert_eq!(group.state(), EnvironmentState::ImageOnly);
    }
}
