use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use super::*;

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
