use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Path as the user knows it. Project paths are canonicalized, which on Windows adds the
/// `\\?\` verbatim prefix; it is only removed for display so saved favorites and caches keep
/// matching.
pub fn display_path(path: &Path) -> String {
    let shown = path.display().to_string();
    if let Some(network) = shown.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{network}")
    } else if let Some(local) = shown.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        shown
    }
}

/// Path without the `\\?\` verbatim prefix, for programs the app starts. `cmd.exe` does not
/// accept verbatim paths: it runs from the Windows folder instead and splits the script path
/// at spaces, so `npm`, `mvnw.cmd` and `gradlew.bat` failed in folders like "Área de
/// Trabalho". Saved paths keep the prefix so favorites and caches still match.
pub fn plain_path(path: &Path) -> PathBuf {
    PathBuf::from(display_path(path))
}

/// For projects that share a name, the shortest ending of each path that tells it apart from
/// the others, such as `acme\web` and `beta\web`; the full path when only the whole path
/// differs.
/// Projects with a name of their own are left out, so their screens do not change. Names
/// and paths are compared without case.
pub fn distinguishing_folders<'a>(
    projects: impl IntoIterator<Item = (&'a str, &'a Path)>,
) -> HashMap<PathBuf, String> {
    let mut by_name: HashMap<String, Vec<&Path>> = HashMap::new();
    for (name, path) in projects {
        by_name.entry(name.to_lowercase()).or_default().push(path);
    }
    let mut folders = HashMap::new();
    for paths in by_name.into_values().filter(|paths| paths.len() > 1) {
        let parts: Vec<Vec<String>> = paths
            .iter()
            .map(|path| {
                display_path(path)
                    .split(['\\', '/'])
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .collect();
        let ending = |parts: &[String], length: usize| {
            parts[parts.len().saturating_sub(length)..]
                .join(std::path::MAIN_SEPARATOR_STR)
                .to_lowercase()
        };
        for (index, path) in paths.iter().enumerate() {
            let own = &parts[index];
            // At least the parent and the folder: the folder alone is the shared name.
            let shown = (2..=own.len())
                .find(|&length| {
                    let mine = ending(own, length);
                    parts
                        .iter()
                        .enumerate()
                        .all(|(other, theirs)| other == index || ending(theirs, length) != mine)
                })
                .filter(|&length| length < own.len())
                .map(|length| own[own.len() - length..].join(std::path::MAIN_SEPARATOR_STR))
                .unwrap_or_else(|| display_path(path));
            folders.insert(path.to_path_buf(), shown);
        }
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(projects: &[(&str, &str)]) -> Vec<Option<String>> {
        let found = distinguishing_folders(
            projects
                .iter()
                .map(|(name, path)| (*name, Path::new(*path))),
        );
        projects
            .iter()
            .map(|(_, path)| found.get(Path::new(*path)).cloned())
            .collect()
    }

    fn sep(text: &str) -> String {
        text.replace('/', std::path::MAIN_SEPARATOR_STR)
    }

    #[test]
    fn tells_projects_with_the_same_name_apart_by_the_shortest_ending() {
        assert_eq!(
            folders(&[
                ("web", "/clientes/acme/web"),
                ("web", "/clientes/beta/web"),
                ("api", "/clientes/acme/api"),
            ]),
            [Some(sep("acme/web")), Some(sep("beta/web")), None]
        );
    }

    #[test]
    fn grows_the_ending_until_it_differs() {
        assert_eq!(
            folders(&[
                ("web", "/x/a/site/web"),
                ("web", "/x/b/site/web"),
                ("web", "/x/b/outro/web"),
            ]),
            [
                Some(sep("a/site/web")),
                Some(sep("b/site/web")),
                Some(sep("outro/web"))
            ]
        );
    }

    #[test]
    fn shows_the_full_path_when_only_the_root_differs() {
        let found = folders(&[("Web", r"\\?\C:\web"), ("web", r"\\?\D:\web")]);
        assert_eq!(found[0].as_deref(), Some(r"C:\web"));
        assert_eq!(found[1].as_deref(), Some(r"D:\web"));
    }

    #[test]
    fn removes_only_the_verbatim_prefix() {
        assert_eq!(
            display_path(Path::new(r"\\?\C:\Projetos\Área de Trabalho\blog")),
            r"C:\Projetos\Área de Trabalho\blog"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\servidor\share\site")),
            r"\\servidor\share\site"
        );
        assert_eq!(
            display_path(Path::new(r"\\servidor\share")),
            r"\\servidor\share"
        );
        assert_eq!(display_path(Path::new("/home/dev/site")), "/home/dev/site");
        assert_eq!(
            plain_path(Path::new(r"\\?\C:\Users\Ana\Área de Trabalho\api")),
            PathBuf::from(r"C:\Users\Ana\Área de Trabalho\api")
        );
    }
}
