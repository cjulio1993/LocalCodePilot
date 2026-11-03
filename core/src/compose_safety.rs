//! Protection for Compose environments that resolve to the same project name.
//!
//! Docker and Podman treat every Compose file with the same project name as one
//! environment: `up` in one folder recreates the containers and reuses the volumes created
//! from another folder. An environment is identified by the engine, the project name the
//! Compose provider resolves and the folder the containers were created from. Anything that
//! cannot be confirmed blocks the actions that change the environment (fail-closed).

use crate::containers::{
    ContainerAction, ContainerCommand, ContainerEngine, RuntimeContainer, same_folder,
};
use std::path::{Path, PathBuf};

/// Project name of a Compose environment in the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeName {
    /// Reported by `compose config`, with the same rules `up` applies: `name:`,
    /// `COMPOSE_PROJECT_NAME` (environment or `.env`) and the folder name.
    Resolved(String),
    /// The provider could not answer. Only used to find conflicts, never to allow actions.
    Estimated(String),
}

impl ComposeName {
    pub fn value(&self) -> &str {
        match self {
            Self::Resolved(name) | Self::Estimated(name) => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeEnvironment {
    /// Folder where the app runs Compose (the folder of the main manifest).
    pub directory: PathBuf,
    pub name: ComposeName,
}

/// A volume labelled by Compose. Volumes record the project name but not the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeVolume {
    pub name: String,
    pub project: String,
    /// Creation time reported by the engine. A volume removed and recreated under the same
    /// name gets a new one, so a confirmation never carries over to someone else's data.
    pub created_at: Option<String>,
}

impl ComposeVolume {
    /// What a confirmation remembers: name and creation time. `None` when the engine did not
    /// report the creation time, in which case the volume cannot be confirmed.
    pub fn identity(&self) -> Option<String> {
        self.created_at
            .as_ref()
            .map(|created_at| format!("{}@{created_at}", self.name))
    }
}

/// Compose's project-name normalization: lowercase; only letters, digits, `-` and `_`; no
/// leading `-` or `_`.
pub fn normalize_compose_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
        .collect::<String>()
        .trim_start_matches(['-', '_'])
        .to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeSafety {
    Safe,
    /// Another catalog environment resolves to the same name.
    SharedName {
        name: String,
        other: PathBuf,
    },
    /// Containers with this name were created from another folder.
    UsedByAnotherFolder {
        name: String,
        folder: PathBuf,
    },
    /// Volumes with this name exist and no container shows where they came from.
    UnconfirmedData {
        name: String,
        volumes: Vec<ComposeVolume>,
    },
    Unverifiable {
        reason: String,
    },
}

/// Classifies `environment` on `engine`. `containers` and `volumes` are that engine's
/// query results; `None` means the query failed. `confirmed_volumes` lists the
/// [`ComposeVolume::identity`] of volumes the user confirmed as belonging to this environment.
pub fn classify_environment(
    environment: &ComposeEnvironment,
    catalog: &[ComposeEnvironment],
    engine: ContainerEngine,
    containers: Option<&[RuntimeContainer]>,
    volumes: Option<&[ComposeVolume]>,
    confirmed_volumes: &[String],
) -> ComposeSafety {
    let ComposeName::Resolved(name) = &environment.name else {
        return ComposeSafety::Unverifiable {
            reason: "O provedor Compose não informou o nome deste ambiente.".into(),
        };
    };
    if let Some(other) = catalog.iter().find(|other| {
        other.name.value() == name && !same_folder(&other.directory, &environment.directory)
    }) {
        return ComposeSafety::SharedName {
            name: name.clone(),
            other: other.directory.clone(),
        };
    }
    let Some(containers) = containers else {
        return ComposeSafety::Unverifiable {
            reason: format!("Não foi possível consultar os containers do {engine}."),
        };
    };
    let mut owns_containers = false;
    for container in containers
        .iter()
        .filter(|container| container.engine == engine)
        .filter(|container| container.compose_project.as_deref() == Some(name.as_str()))
    {
        match &container.project_directory {
            None => {
                return ComposeSafety::Unverifiable {
                    reason: format!(
                        "O container {} do ambiente \"{name}\" não registra a pasta de origem.",
                        container.name
                    ),
                };
            }
            Some(folder) if !same_folder(folder, &environment.directory) => {
                return ComposeSafety::UsedByAnotherFolder {
                    name: name.clone(),
                    folder: folder.clone(),
                };
            }
            Some(_) => owns_containers = true,
        }
    }
    if owns_containers {
        return ComposeSafety::Safe;
    }
    let Some(volumes) = volumes else {
        return ComposeSafety::Unverifiable {
            reason: format!("Não foi possível consultar os volumes do {engine}."),
        };
    };
    let unconfirmed = volumes
        .iter()
        .filter(|volume| {
            volume.project == *name
                && !volume
                    .identity()
                    .is_some_and(|identity| confirmed_volumes.contains(&identity))
        })
        .cloned()
        .collect::<Vec<_>>();
    if unconfirmed.is_empty() {
        ComposeSafety::Safe
    } else {
        ComposeSafety::UnconfirmedData {
            name: name.clone(),
            volumes: unconfirmed,
        }
    }
}

/// Why an action is blocked and how to unblock it, in user-facing language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedAction {
    /// One sentence for anyone: what may happen, without names, paths or commands.
    pub summary: &'static str,
    pub title: String,
    pub explanation: String,
    pub steps: Vec<String>,
    /// Line to add to the Compose file, offered for copying.
    pub snippet: Option<String>,
    /// The user may confirm that the listed volumes belong to this environment.
    pub can_confirm_volumes: bool,
}

/// Actions on a whole Compose environment. Read-only actions are always allowed; `up` and
/// `stop` only when the environment is safe.
pub fn guard_environment_action(
    action: ContainerAction,
    safety: &ComposeSafety,
    directory: &Path,
) -> Result<(), BlockedAction> {
    if !matches!(action, ContainerAction::Start | ContainerAction::Stop) {
        return Ok(());
    }
    match safety {
        ComposeSafety::Safe => Ok(()),
        other => Err(explain(other, directory)),
    }
}

/// Actions on one existing container. An exact ID does not make the impact clear when the
/// environment it belongs to is shared, so the same rules as the environment apply.
/// `environment_safety` is the classification of the catalog environment created from the
/// container's folder, when there is one.
pub fn guard_container_action(
    action: ContainerAction,
    container: &RuntimeContainer,
    containers: &[RuntimeContainer],
    catalog: &[ComposeEnvironment],
    environment_safety: Option<&ComposeSafety>,
) -> Result<(), BlockedAction> {
    if !matches!(
        action,
        ContainerAction::StartExisting
            | ContainerAction::StopExisting
            | ContainerAction::RestartExisting
    ) {
        return Ok(());
    }
    let Some(name) = container.compose_project.as_deref() else {
        // Not created by Compose: the ID is the whole environment.
        return Ok(());
    };
    let Some(folder) = &container.project_directory else {
        return Err(explain(
            &ComposeSafety::Unverifiable {
                reason: format!(
                    "O container {} não registra a pasta de origem.",
                    container.name
                ),
            },
            Path::new(""),
        ));
    };
    if let Some(safety) = environment_safety
        && *safety != ComposeSafety::Safe
    {
        return Err(explain(safety, folder));
    }
    if let Some(other) = catalog.iter().find(|environment| {
        environment.name.value() == name && !same_folder(&environment.directory, folder)
    }) {
        return Err(explain(
            &ComposeSafety::SharedName {
                name: name.to_owned(),
                other: other.directory.clone(),
            },
            folder,
        ));
    }
    if let Some(sibling) = containers.iter().find(|sibling| {
        sibling.engine == container.engine
            && sibling.compose_project.as_deref() == Some(name)
            && sibling
                .project_directory
                .as_deref()
                .is_none_or(|sibling_folder| !same_folder(sibling_folder, folder))
    }) {
        return Err(explain(
            &match &sibling.project_directory {
                Some(other) => ComposeSafety::UsedByAnotherFolder {
                    name: name.to_owned(),
                    folder: other.clone(),
                },
                None => ComposeSafety::Unverifiable {
                    reason: format!(
                        "O container {} do mesmo ambiente não registra a pasta de origem.",
                        sibling.name
                    ),
                },
            },
            folder,
        ));
    }
    Ok(())
}

const SHARED_SUMMARY: &str =
    "Este ambiente pode compartilhar containers ou dados com outro projeto.";

/// User-facing explanation for an unsafe environment.
pub fn explain(safety: &ComposeSafety, directory: &Path) -> BlockedAction {
    let suggestion = match safety {
        ComposeSafety::SharedName { name, other } => {
            suggested_project_name(directory, Some(other), name)
        }
        ComposeSafety::UsedByAnotherFolder { name, folder } => {
            suggested_project_name(directory, Some(folder), name)
        }
        ComposeSafety::UnconfirmedData { name, .. } => {
            suggested_project_name(directory, None, name)
        }
        ComposeSafety::Safe | ComposeSafety::Unverifiable { .. } => String::new(),
    };
    let rename_steps = |name: &str| {
        vec![
            "Adicione no topo do arquivo Compose deste projeto o trecho abaixo, com um nome que só ele use. O nome também vale quando você usa o terminal.".to_owned(),
            format!("Ou defina COMPOSE_PROJECT_NAME={suggestion} no arquivo .env da pasta do Compose."),
            format!("Containers e volumes já criados com o nome \"{name}\" continuam existindo com esse nome e não são reaproveitados pelo nome novo. Se os containers do outro projeto estiverem rodando, use Parar containers do outro projeto, abaixo: só eles param, e nenhum volume é removido."),
            "Depois, clique em Verificar motores e arquivos.".to_owned(),
        ]
    };
    match safety {
        ComposeSafety::Safe => BlockedAction {
            summary: "",
            title: String::new(),
            explanation: String::new(),
            steps: vec![],
            snippet: None,
            can_confirm_volumes: false,
        },
        ComposeSafety::SharedName { name, other } => BlockedAction {
            summary: SHARED_SUMMARY,
            title: "Nome de ambiente compartilhado".into(),
            explanation: format!(
                "O Compose chama este ambiente de \"{name}\", o mesmo nome do ambiente em {}. Para o Docker e o Podman, os dois são um único ambiente: iniciar ou parar um pode recriar, parar ou reaproveitar containers e dados do outro.",
                crate::paths::display_path(other)
            ),
            steps: rename_steps(name),
            snippet: Some(format!("name: {suggestion}")),
            can_confirm_volumes: false,
        },
        ComposeSafety::UsedByAnotherFolder { name, folder } => BlockedAction {
            summary: SHARED_SUMMARY,
            title: "Ambiente em uso por outra pasta".into(),
            explanation: format!(
                "Existem containers do ambiente \"{name}\" criados a partir de {}, possivelmente pelo terminal ou por outra ferramenta. Iniciar ou parar aqui afetaria esses containers.",
                crate::paths::display_path(folder)
            ),
            steps: rename_steps(name),
            snippet: Some(format!("name: {suggestion}")),
            can_confirm_volumes: false,
        },
        ComposeSafety::UnconfirmedData { name, volumes } => {
            let can_confirm = volumes.iter().all(|volume| volume.identity().is_some());
            let mut steps = Vec::new();
            if can_confirm {
                steps.push("Se os dados são deste projeto (por exemplo, depois de um \"compose down\"), confirme abaixo.".to_owned());
            } else {
                steps.push("O motor não informou quando esses volumes foram criados, então não é possível confirmá-los com segurança.".to_owned());
            }
            steps.push(
                "Se não forem deste projeto, dê a este ambiente um nome que só ele use, com o trecho abaixo."
                    .to_owned(),
            );
            BlockedAction {
                summary: "Existem dados deste ambiente sem origem confirmada.",
                title: "Dados de origem não confirmada".into(),
                explanation: format!(
                    "Existem volumes do ambiente \"{name}\" ({}) e nenhum container mostra de qual pasta eles vieram. Iniciar aqui faria este projeto usar esses dados.",
                    volumes
                        .iter()
                        .map(|volume| volume.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                steps,
                snippet: Some(format!("name: {suggestion}")),
                can_confirm_volumes: can_confirm,
            }
        }
        ComposeSafety::Unverifiable { reason } => BlockedAction {
            summary: "Não foi possível confirmar se este ambiente é só deste projeto.",
            title: "Não foi possível confirmar o ambiente".into(),
            explanation: format!(
                "{reason} Sem essa confirmação, iniciar ou parar pode afetar outro projeto."
            ),
            steps: vec![
                "Verifique se o motor está ativo e se o Compose está disponível.".into(),
                "Depois, clique em Verificar motores e arquivos.".into(),
            ],
            snippet: None,
            can_confirm_volumes: false,
        },
    }
}

/// Name to suggest for `name:`. Generic Compose folders (`docker/`, `infra/`) take the name of
/// the project folder above them. When `other` is the folder it collides with, the nearest
/// folder above that tells them apart is added (`cliente1/wordpress` and
/// `cliente2/wordpress` become `cliente1-wordpress`), so following the suggestion in both
/// projects cannot recreate the collision. Never returns `taken`, the colliding name.
pub fn suggested_project_name(directory: &Path, other: Option<&Path>, taken: &str) -> String {
    const GENERIC: &[&str] = &[
        "docker",
        ".docker",
        "compose",
        "infra",
        "deploy",
        "devops",
        "ops",
        "containers",
    ];
    // Folder names from the leaf up, as Compose would normalize them.
    let folders = |path: &Path| {
        path.components()
            .rev()
            .filter_map(|component| match component {
                std::path::Component::Normal(name) => {
                    Some(normalize_compose_name(&name.to_string_lossy()))
                }
                _ => None,
            })
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
    };
    let mine = folders(directory);
    let base = usize::from(
        mine.first()
            .is_some_and(|leaf| GENERIC.contains(&leaf.as_str())),
    );
    let mut suggestion = mine
        .get(base)
        .or(mine.first())
        .cloned()
        .unwrap_or_else(|| "meu-projeto".into());
    if let Some(other) = other {
        let theirs = folders(other);
        let differs_at = (0..mine.len()).find(|&depth| mine.get(depth) != theirs.get(depth));
        if let Some(depth) = differs_at.filter(|&depth| depth > base) {
            suggestion = format!("{}-{suggestion}", mine[depth]);
        }
    }
    if suggestion == normalize_compose_name(taken) {
        suggestion.push_str("-projeto");
    }
    suggestion
}

/// "Parar containers do outro projeto": stops, by their exact IDs, the running containers
/// that the conflict found in the other folder. Only those IDs are stopped and no volume is
/// removed, so the scope is known before confirming. `None` when the conflict has no other
/// folder or nothing of it is running on `engine`.
pub fn other_project_stop(
    safety: &ComposeSafety,
    engine: ContainerEngine,
    containers: &[RuntimeContainer],
    working_directory: &Path,
) -> Option<ContainerCommand> {
    let (ComposeSafety::SharedName {
        name,
        other: folder,
    }
    | ComposeSafety::UsedByAnotherFolder { name, folder }) = safety
    else {
        return None;
    };
    let ids = containers
        .iter()
        .filter(|container| {
            container.engine == engine
                && container.compose_project.as_deref() == Some(name.as_str())
                && container
                    .project_directory
                    .as_deref()
                    .is_some_and(|directory| same_folder(directory, folder))
        })
        // Validates the ID and keeps only running containers.
        .filter_map(|container| container.command(ContainerAction::StopExisting))
        .filter_map(|command| command.args.last().cloned())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return None;
    }
    Some(ContainerCommand {
        action: ContainerAction::StopOtherProject,
        label: "Parar containers do outro projeto",
        program: engine.program(),
        args: std::iter::once("stop".to_owned()).chain(ids).collect(),
        working_directory: working_directory.to_path_buf(),
    })
}

/// Checks again, right before running, that every container listed in the confirmation is
/// still running with the same Compose name and folder. `previous` is what the confirmation
/// showed; `current` is a fresh query of the engine.
pub fn recheck_other_project_stop(
    command: &ContainerCommand,
    previous: &[RuntimeContainer],
    current: &[RuntimeContainer],
) -> Result<(), String> {
    for id in command.args.iter().skip(1) {
        let shown = previous
            .iter()
            .find(|container| &container.id == id)
            .ok_or("Os containers mudaram desde a confirmação; verifique de novo.")?;
        let now = current
            .iter()
            .find(|container| &container.id == id)
            .ok_or_else(|| format!("O container {} não existe mais.", shown.name))?;
        let same_origin = now.compose_project == shown.compose_project
            && match (&now.project_directory, &shown.project_directory) {
                (Some(now), Some(shown)) => same_folder(now, shown),
                _ => false,
            };
        if !same_origin {
            return Err(format!(
                "O container {} mudou desde a confirmação; verifique de novo.",
                shown.name
            ));
        }
        if !now.state.eq_ignore_ascii_case("running") {
            return Err(format!("O container {} já está parado.", shown.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_the_other_project_lists_only_its_running_containers() {
        let mine = root().join("loja").join("docker");
        let other = root().join("site").join("docker");
        let with = |id: &str, name: &str, project: &str, folder: &Path, state: &str| {
            let mut found = container(name, Some(project), Some(folder.to_path_buf()));
            found.id = id.into();
            found.state = state.into();
            found
        };
        let containers = vec![
            with("aaaaaa111111", "docker-db-1", "docker", &other, "running"),
            with("bbbbbb222222", "docker-web-1", "docker", &other, "exited"),
            with("cccccc333333", "docker-app-1", "docker", &mine, "running"),
            with("dddddd444444", "outro-db-1", "outro", &other, "running"),
        ];
        let safety = ComposeSafety::UsedByAnotherFolder {
            name: "docker".into(),
            folder: other.clone(),
        };
        let command =
            other_project_stop(&safety, ContainerEngine::Docker, &containers, &mine).unwrap();
        assert_eq!(command.action, ContainerAction::StopOtherProject);
        assert_eq!(command.display(), "docker stop aaaaaa111111");
        assert!(command.action.changes_environment());
        // The other engine has nothing of it, and a safe environment offers nothing.
        assert!(other_project_stop(&safety, ContainerEngine::Podman, &containers, &mine).is_none());
        assert!(
            other_project_stop(
                &ComposeSafety::Safe,
                ContainerEngine::Docker,
                &containers,
                &mine
            )
            .is_none()
        );

        // Right before running, the listed container must still be the same and running.
        assert_eq!(
            recheck_other_project_stop(&command, &containers, &containers),
            Ok(())
        );
        let mut stopped = containers.clone();
        stopped[0].state = "exited".into();
        assert!(recheck_other_project_stop(&command, &containers, &stopped).is_err());
        let mut moved = containers.clone();
        moved[0].project_directory = Some(mine.clone());
        assert!(recheck_other_project_stop(&command, &containers, &moved).is_err());
        assert!(recheck_other_project_stop(&command, &containers, &containers[1..]).is_err());
    }

    fn root() -> PathBuf {
        std::env::temp_dir().join("lcp-compose-safety")
    }

    fn environment(project: &str, name: ComposeName) -> ComposeEnvironment {
        ComposeEnvironment {
            directory: root().join(project).join("docker"),
            name,
        }
    }

    fn container(name: &str, project: Option<&str>, folder: Option<PathBuf>) -> RuntimeContainer {
        RuntimeContainer {
            engine: ContainerEngine::Docker,
            id: "abcdef123456".into(),
            name: name.into(),
            image: "mariadb:11".into(),
            state: "running".into(),
            status: "Up".into(),
            ports: String::new(),
            service: Some("db".into()),
            compose_project: project.map(str::to_owned),
            project_directory: folder,
            running_for: None,
        }
    }

    const CREATED: &str = "2026-11-01T10:00:00Z";

    fn volume(name: &str, project: &str) -> ComposeVolume {
        ComposeVolume {
            name: name.into(),
            project: project.into(),
            created_at: Some(CREATED.into()),
        }
    }

    fn confirmed(name: &str) -> String {
        format!("{name}@{CREATED}")
    }

    fn resolved(name: &str) -> ComposeName {
        ComposeName::Resolved(name.into())
    }

    #[test]
    fn two_projects_with_docker_folders_share_one_compose_name() {
        let blog = environment("wp-blog", resolved("docker"));
        let shop = environment("loja", resolved("docker"));
        let catalog = [blog.clone(), shop.clone()];
        for (current, other) in [(&blog, &shop), (&shop, &blog)] {
            assert_eq!(
                classify_environment(
                    current,
                    &catalog,
                    ContainerEngine::Docker,
                    Some(&[]),
                    Some(&[]),
                    &[]
                ),
                ComposeSafety::SharedName {
                    name: "docker".into(),
                    other: other.directory.clone()
                }
            );
        }
        // An estimated name of a sibling is enough to report the conflict.
        let estimated_shop = environment("loja", ComposeName::Estimated("docker".into()));
        assert!(matches!(
            classify_environment(
                &blog,
                &[blog.clone(), estimated_shop],
                ContainerEngine::Docker,
                Some(&[]),
                Some(&[]),
                &[]
            ),
            ComposeSafety::SharedName { .. }
        ));
    }

    #[test]
    fn containers_from_another_folder_or_without_folder_block_the_environment() {
        let blog = environment("wp-blog", resolved("docker"));
        let catalog = [blog.clone()];
        let terminal = [container(
            "docker-db-1",
            Some("docker"),
            Some(root().join("fora-do-catalogo").join("docker")),
        )];
        assert!(matches!(
            classify_environment(
                &blog,
                &catalog,
                ContainerEngine::Docker,
                Some(&terminal),
                Some(&[]),
                &[]
            ),
            ComposeSafety::UsedByAnotherFolder { .. }
        ));
        let unlabeled = [container("docker-db-1", Some("docker"), None)];
        assert!(matches!(
            classify_environment(
                &blog,
                &catalog,
                ContainerEngine::Docker,
                Some(&unlabeled),
                Some(&[]),
                &[]
            ),
            ComposeSafety::Unverifiable { .. }
        ));
        // The same name on the other engine is a separate environment.
        let mut podman = terminal[0].clone();
        podman.engine = ContainerEngine::Podman;
        assert_eq!(
            classify_environment(
                &blog,
                &catalog,
                ContainerEngine::Docker,
                Some(&[podman]),
                Some(&[]),
                &[]
            ),
            ComposeSafety::Safe
        );
    }

    #[test]
    fn own_containers_and_unique_names_are_safe() {
        let blog = environment("wp-blog", resolved("wp-blog"));
        let own = [container(
            "wp-blog-db-1",
            Some("wp-blog"),
            Some(blog.directory.clone()),
        )];
        let data = [volume("wp-blog_dados", "wp-blog")];
        assert_eq!(
            classify_environment(
                &blog,
                std::slice::from_ref(&blog),
                ContainerEngine::Docker,
                Some(&own),
                Some(&data),
                &[]
            ),
            ComposeSafety::Safe
        );
        assert_eq!(
            classify_environment(
                &blog,
                std::slice::from_ref(&blog),
                ContainerEngine::Docker,
                Some(&[]),
                Some(&[]),
                &[]
            ),
            ComposeSafety::Safe
        );
    }

    #[test]
    fn volumes_without_containers_need_explicit_confirmation() {
        let blog = environment("wp-blog", resolved("docker"));
        let data = [
            volume("docker_dados", "docker"),
            volume("outro_dados", "outro"),
        ];
        assert_eq!(
            classify_environment(
                &blog,
                std::slice::from_ref(&blog),
                ContainerEngine::Docker,
                Some(&[]),
                Some(&data),
                &[]
            ),
            ComposeSafety::UnconfirmedData {
                name: "docker".into(),
                volumes: vec![volume("docker_dados", "docker")]
            }
        );
        assert_eq!(
            classify_environment(
                &blog,
                std::slice::from_ref(&blog),
                ContainerEngine::Docker,
                Some(&[]),
                Some(&data),
                &[confirmed("docker_dados")]
            ),
            ComposeSafety::Safe
        );
        // A volume removed and recreated under the same name (by another folder) is new data:
        // the earlier confirmation does not apply.
        let recreated = [ComposeVolume {
            created_at: Some("2026-12-24T08:00:00Z".into()),
            ..volume("docker_dados", "docker")
        }];
        assert!(matches!(
            classify_environment(
                &blog,
                std::slice::from_ref(&blog),
                ContainerEngine::Docker,
                Some(&[]),
                Some(&recreated),
                &[confirmed("docker_dados")]
            ),
            ComposeSafety::UnconfirmedData { .. }
        ));
        // Without a creation time the volume can never be confirmed, not even by name.
        let unknown = [ComposeVolume {
            created_at: None,
            ..volume("docker_dados", "docker")
        }];
        let safety = classify_environment(
            &blog,
            std::slice::from_ref(&blog),
            ContainerEngine::Docker,
            Some(&[]),
            Some(&unknown),
            &["docker_dados".into()],
        );
        assert!(matches!(safety, ComposeSafety::UnconfirmedData { .. }));
        assert!(!explain(&safety, &blog.directory).can_confirm_volumes);
        // A confirmation never overrides a shared name.
        let shop = environment("loja", resolved("docker"));
        assert!(matches!(
            classify_environment(
                &blog,
                &[blog.clone(), shop],
                ContainerEngine::Docker,
                Some(&[]),
                Some(&data),
                &[confirmed("docker_dados")]
            ),
            ComposeSafety::SharedName { .. }
        ));
    }

    #[test]
    fn unknown_name_or_failed_queries_fail_closed() {
        let estimated = environment("wp-blog", ComposeName::Estimated("docker".into()));
        assert!(matches!(
            classify_environment(
                &estimated,
                std::slice::from_ref(&estimated),
                ContainerEngine::Docker,
                Some(&[]),
                Some(&[]),
                &[]
            ),
            ComposeSafety::Unverifiable { .. }
        ));
        let blog = environment("wp-blog", resolved("docker"));
        assert!(matches!(
            classify_environment(
                &blog,
                std::slice::from_ref(&blog),
                ContainerEngine::Docker,
                None,
                Some(&[]),
                &[]
            ),
            ComposeSafety::Unverifiable { .. }
        ));
        assert!(matches!(
            classify_environment(
                &blog,
                std::slice::from_ref(&blog),
                ContainerEngine::Docker,
                Some(&[]),
                None,
                &[]
            ),
            ComposeSafety::Unverifiable { .. }
        ));
    }

    #[test]
    fn only_lifecycle_actions_are_blocked_on_unsafe_environments() {
        let directory = root().join("wp-blog").join("docker");
        let shared = ComposeSafety::SharedName {
            name: "docker".into(),
            other: root().join("loja").join("docker"),
        };
        for action in [ContainerAction::Start, ContainerAction::Stop] {
            let blocked = guard_environment_action(action, &shared, &directory).unwrap_err();
            assert_eq!(blocked.title, "Nome de ambiente compartilhado");
            assert_eq!(blocked.snippet.as_deref(), Some("name: wp-blog"));
            assert!(guard_environment_action(action, &ComposeSafety::Safe, &directory).is_ok());
        }
        for action in [
            ContainerAction::Validate,
            ContainerAction::Status,
            ContainerAction::Logs,
            ContainerAction::Build,
        ] {
            assert!(guard_environment_action(action, &shared, &directory).is_ok());
        }
    }

    #[test]
    fn container_actions_follow_their_environment_not_just_the_id() {
        let blog = environment("wp-blog", resolved("docker"));
        let shop = environment("loja", resolved("docker"));
        let own = container("docker-db-1", Some("docker"), Some(blog.directory.clone()));
        let catalog = [blog.clone(), shop.clone()];
        for action in [
            ContainerAction::StartExisting,
            ContainerAction::StopExisting,
            ContainerAction::RestartExisting,
        ] {
            assert!(
                guard_container_action(action, &own, std::slice::from_ref(&own), &catalog, None)
                    .is_err()
            );
            assert!(
                guard_container_action(
                    action,
                    &own,
                    std::slice::from_ref(&own),
                    std::slice::from_ref(&blog),
                    Some(&ComposeSafety::Safe)
                )
                .is_ok()
            );
        }
        assert!(
            guard_container_action(
                ContainerAction::LogsExisting,
                &own,
                std::slice::from_ref(&own),
                &catalog,
                None
            )
            .is_ok()
        );
        // Outside the catalog: siblings from another folder share the environment.
        let foreign = container(
            "docker-web-1",
            Some("docker"),
            Some(root().join("x").join("docker")),
        );
        assert!(
            guard_container_action(
                ContainerAction::StopExisting,
                &own,
                &[own.clone(), foreign],
                &[],
                None
            )
            .is_err()
        );
        // Plain containers (no Compose) are their own environment.
        let plain = container("redis", None, None);
        assert!(
            guard_container_action(
                ContainerAction::StopExisting,
                &plain,
                std::slice::from_ref(&plain),
                &catalog,
                None
            )
            .is_ok()
        );
        // Without a recorded folder the impact is unknown.
        let unlabeled = container("docker-db-1", Some("docker"), None);
        assert!(
            guard_container_action(
                ContainerAction::StopExisting,
                &unlabeled,
                std::slice::from_ref(&unlabeled),
                &[],
                None
            )
            .is_err()
        );
    }

    #[test]
    fn suggests_names_from_the_project_folder() {
        let suggest = |path: &Path| suggested_project_name(path, None, "docker");
        assert_eq!(suggest(&root().join("wp-blog").join("docker")), "wp-blog");
        assert_eq!(
            suggest(&root().join("Minha Loja").join("infra")),
            "minhaloja"
        );
        assert_eq!(suggest(&root().join("Ação_2")), "ao_2");
        assert_eq!(suggest(&root().join("api")), "api");
        assert_eq!(suggest(Path::new("")), "meu-projeto");
    }

    #[test]
    fn suggestions_never_repeat_the_colliding_name_or_each_other() {
        // Same leaf folder in two clients: the plain folder name is the colliding name.
        let first = root().join("cliente1").join("wordpress");
        let second = root().join("cliente2").join("wordpress");
        let first_suggestion = suggested_project_name(&first, Some(&second), "wordpress");
        let second_suggestion = suggested_project_name(&second, Some(&first), "wordpress");
        assert_eq!(first_suggestion, "cliente1-wordpress");
        assert_eq!(second_suggestion, "cliente2-wordpress");

        // Generic folders under projects with the same name.
        let a = root().join("a").join("site").join("docker");
        let b = root().join("b").join("site").join("docker");
        assert_eq!(suggested_project_name(&a, Some(&b), "docker"), "a-site");
        assert_eq!(suggested_project_name(&b, Some(&a), "docker"), "b-site");

        // Different project folders keep the short name.
        let blog = root().join("wp-blog").join("docker");
        let shop = root().join("loja").join("docker");
        assert_eq!(
            suggested_project_name(&blog, Some(&shop), "docker"),
            "wp-blog"
        );

        // Never the colliding name itself.
        let alone = root().join("wordpress");
        assert_eq!(
            suggested_project_name(&alone, None, "wordpress"),
            "wordpress-projeto"
        );

        // explain() uses the colliding folder.
        let blocked = explain(
            &ComposeSafety::SharedName {
                name: "wordpress".into(),
                other: second.clone(),
            },
            &first,
        );
        assert_eq!(blocked.snippet.as_deref(), Some("name: cliente1-wordpress"));
    }

    #[test]
    fn normalizes_names_like_compose() {
        assert_eq!(normalize_compose_name("-Minha Loja_2"), "minhaloja_2");
        assert_eq!(normalize_compose_name("WP.Blog"), "wpblog");
    }

    #[test]
    fn every_unsafe_state_explains_how_to_resolve_it() {
        let directory = root().join("wp-blog").join("docker");
        for safety in [
            ComposeSafety::SharedName {
                name: "docker".into(),
                other: root().join("loja"),
            },
            ComposeSafety::UsedByAnotherFolder {
                name: "docker".into(),
                folder: root().join("x"),
            },
            ComposeSafety::UnconfirmedData {
                name: "docker".into(),
                volumes: vec![volume("docker_dados", "docker")],
            },
            ComposeSafety::Unverifiable {
                reason: "Motor indisponível.".into(),
            },
        ] {
            let blocked = explain(&safety, &directory);
            assert!(!blocked.title.is_empty() && !blocked.explanation.is_empty());
            // The first layer names no folder, environment or command.
            assert!(!blocked.summary.is_empty());
            assert!(!blocked.summary.contains('"') && !blocked.summary.contains("docker"));
            assert!(!blocked.steps.is_empty());
            assert_eq!(
                blocked.can_confirm_volumes,
                matches!(safety, ComposeSafety::UnconfirmedData { .. })
            );
        }
    }
}
