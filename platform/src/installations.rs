use localcodepilot_core::environments::{InstallationStep, InstallationStepKind};
#[cfg(target_os = "windows")]
use std::collections::HashSet;
use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
};

const NODE_LTS_PACKAGE_ID: &str = "OpenJS.NodeJS.LTS";
const BUN_PACKAGE_ID: &str = "Oven-sh.Bun";
const GO_PACKAGE_ID: &str = "GoLang.Go";
const DART_PACKAGE_ID: &str = "Google.DartSDK";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramInventory {
    pub command: String,
    pub path: PathBuf,
    pub version: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsWingetProvider;

impl WindowsWingetProvider {
    pub fn is_available(&self) -> bool {
        find_executable("winget", None).is_some()
    }

    pub fn resolve_node_installation(
        &self,
        version_constraint: &str,
        working_directory: &Path,
        matches: impl Fn(&str, &str) -> Option<bool>,
    ) -> Result<InstallationStep, String> {
        self.resolve_package_installation(
            NODE_LTS_PACKAGE_ID,
            "Node.js",
            version_constraint,
            working_directory,
            matches,
        )
    }

    pub fn resolve_bun_installation(
        &self,
        version_constraint: &str,
        working_directory: &Path,
    ) -> Result<InstallationStep, String> {
        self.resolve_package_installation(
            BUN_PACKAGE_ID,
            "Bun",
            version_constraint,
            working_directory,
            version_matches,
        )
    }

    pub fn resolve_go_installation(
        &self,
        version_constraint: &str,
        working_directory: &Path,
        matches: impl Fn(&str, &str) -> Option<bool>,
    ) -> Result<InstallationStep, String> {
        self.resolve_package_installation(
            GO_PACKAGE_ID,
            "Go",
            version_constraint,
            working_directory,
            matches,
        )
    }

    pub fn resolve_dart_installation(
        &self,
        version_constraint: &str,
        working_directory: &Path,
        matches: impl Fn(&str, &str) -> Option<bool>,
    ) -> Result<InstallationStep, String> {
        self.resolve_package_installation(
            DART_PACKAGE_ID,
            "Dart SDK",
            version_constraint,
            working_directory,
            matches,
        )
    }

    pub fn resolve_python_installation(
        &self,
        version_constraint: &str,
        working_directory: &Path,
        matches: impl Fn(&str, &str) -> Option<bool>,
    ) -> Result<InstallationStep, String> {
        let version = [(3, 14), (3, 13), (3, 12), (3, 11), (3, 10)]
            .into_iter()
            .find(|(major, minor)| {
                matches(version_constraint, &format!("{major}.{minor}.0")).unwrap_or(true)
            })
            .ok_or_else(|| {
                format!(
                    "Nenhuma versão Python aprovada atende a {version_constraint} (suportadas: 3.10 a 3.14)"
                )
            })?;
        let package_id = match version {
            (3, 10) => "Python.Python.3.10",
            (3, 11) => "Python.Python.3.11",
            (3, 12) => "Python.Python.3.12",
            (3, 13) => "Python.Python.3.13",
            (3, 14) => "Python.Python.3.14",
            _ => {
                return Err(format!(
                    "A versão Python {}.{} ainda não possui um pacote aprovado no LocalCodePilot",
                    version.0, version.1
                ));
            }
        };
        self.resolve_package_installation(
            package_id,
            "Python",
            version_constraint,
            working_directory,
            matches,
        )
    }

    pub fn resolve_java_installation(
        &self,
        version_constraint: &str,
        working_directory: &Path,
        matches: impl Fn(&str, &str) -> Option<bool>,
    ) -> Result<InstallationStep, String> {
        let major = [25, 21, 17]
            .into_iter()
            .find(|major| matches(version_constraint, &format!("{major}.0.0")).unwrap_or(true))
            .ok_or_else(|| {
                format!(
                    "Nenhum JDK aprovado atende a {version_constraint} (suportados: 17, 21 e 25)"
                )
            })?;
        let package_id = match major {
            17 => "Microsoft.OpenJDK.17",
            21 => "Microsoft.OpenJDK.21",
            25 => "Microsoft.OpenJDK.25",
            _ => {
                return Err(format!(
                    "O JDK {major} ainda não possui um pacote aprovado no LocalCodePilot"
                ));
            }
        };
        self.resolve_package_installation(
            package_id,
            "Microsoft OpenJDK",
            version_constraint,
            working_directory,
            matches,
        )
    }

    pub fn resolve_php_installation(
        &self,
        version_constraint: &str,
        working_directory: &Path,
        matches: impl Fn(&str, &str) -> Option<bool>,
    ) -> Result<InstallationStep, String> {
        let version = [(8, 5), (8, 4), (8, 3), (8, 2), (8, 1)]
            .into_iter()
            .find(|(major, minor)| {
                matches(version_constraint, &format!("{major}.{minor}.0")).unwrap_or(true)
            })
            .ok_or_else(|| {
                format!(
                    "Nenhuma versão PHP aprovada atende a {version_constraint} (suportadas: 8.1 a 8.5)"
                )
            })?;
        let package_id = match version {
            (8, 1) => "PHP.PHP.8.1",
            (8, 2) => "PHP.PHP.8.2",
            (8, 3) => "PHP.PHP.8.3",
            (8, 4) => "PHP.PHP.8.4",
            (8, 5) => "PHP.PHP.8.5",
            _ => {
                return Err(format!(
                    "A versão PHP {}.{} ainda não possui um pacote aprovado no LocalCodePilot",
                    version.0, version.1
                ));
            }
        };
        self.resolve_package_installation(
            package_id,
            "PHP",
            version_constraint,
            working_directory,
            matches,
        )
    }

    fn resolve_package_installation(
        &self,
        package_id: &str,
        display_name: &str,
        version_constraint: &str,
        working_directory: &Path,
        matches: impl Fn(&str, &str) -> Option<bool>,
    ) -> Result<InstallationStep, String> {
        if !cfg!(target_os = "windows") {
            return Err(
                "A instalação automática com winget está disponível apenas no Windows".into(),
            );
        }
        if !self.is_available() {
            return Err(
                "O winget não foi encontrado. Atualize ou instale o App Installer da Microsoft Store."
                    .into(),
            );
        }

        let mut command = Command::new("winget");
        command.args([
            "show",
            "--id",
            package_id,
            "--exact",
            "--versions",
            "--source",
            "winget",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]);
        configure_background_command(&mut command);
        let output = command.output().map_err(|error| {
            format!("Não foi possível consultar versões do {display_name}: {error}")
        })?;
        if !output.status.success() {
            return Err(format!(
                "O winget não conseguiu consultar o pacote {package_id}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let versions = parse_winget_versions(&String::from_utf8_lossy(&output.stdout));
        let selected = versions
            .iter()
            .find(|version| matches(version_constraint, version).unwrap_or(true))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "O winget não oferece uma versão de {display_name} compatível com {version_constraint}"
                )
            })?;

        Ok(package_installation_step(
            package_id,
            display_name,
            working_directory,
            version_constraint,
            &selected,
        ))
    }
}

pub fn inspect_program(
    program: &str,
    version_args: &[&str],
    path: Option<&OsStr>,
) -> Option<ProgramInventory> {
    let executable = find_executable(program, path)?;
    let mut command = Command::new(&executable);
    command.args(version_args);
    if let Some(path) = path {
        command.env("PATH", path);
    }
    configure_background_command(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&output.stderr).lines())
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_owned();
    Some(ProgramInventory {
        command: program.to_owned(),
        path: executable,
        version,
    })
}

pub fn find_executable(program: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    let program_path = Path::new(program);
    if program_path.components().count() > 1 {
        return executable_candidate(program_path).then(|| program_path.to_path_buf());
    }

    let search_path = path.map(OsString::from).or_else(|| env::var_os("PATH"))?;
    for directory in env::split_paths(&search_path) {
        let candidate = directory.join(program);
        if executable_candidate(&candidate) {
            return Some(candidate);
        }
        #[cfg(target_os = "windows")]
        if candidate.extension().is_none() {
            let extensions = env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
            for extension in extensions.to_string_lossy().split(';') {
                let extension = extension.trim().trim_start_matches('.');
                let candidate = candidate.with_extension(extension);
                if !extension.is_empty() && executable_candidate(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub fn refreshed_environment_path() -> Result<OsString, String> {
    #[cfg(target_os = "windows")]
    {
        let script = concat!(
            "[Console]::OutputEncoding=[Text.Encoding]::UTF8;",
            "$machine=[Environment]::GetEnvironmentVariable('Path','Machine');",
            "$user=[Environment]::GetEnvironmentVariable('Path','User');",
            "[Environment]::ExpandEnvironmentVariables(($machine,$user -join ';'))"
        );
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ]);
        configure_background_command(&mut command);
        let output = command
            .output()
            .map_err(|error| format!("Não foi possível atualizar o PATH: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Não foi possível atualizar o PATH: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let refreshed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let merged = merge_paths(Some(OsString::from(refreshed)), env::var_os("PATH"))?;
        merge_paths(managed_bin_directory().map(OsString::from), Some(merged))
    }

    #[cfg(not(target_os = "windows"))]
    env::var_os("PATH").ok_or_else(|| "A variável PATH não está disponível".into())
}

pub fn managed_bin_directory() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        env::var_os("LOCALAPPDATA")
            .or_else(|| env::var_os("APPDATA"))
            .map(PathBuf::from)
    } else {
        env::var_os("HOME").map(|home| PathBuf::from(home).join(".local"))
    }?;
    Some(base.join("LocalCodePilot").join("bin"))
}

fn package_installation_step(
    package_id: &str,
    display_name: &str,
    working_directory: &Path,
    version_constraint: &str,
    selected_version: &str,
) -> InstallationStep {
    InstallationStep {
        label: format!("Instalar {display_name} {selected_version}"),
        detail: format!("Versão compatível com {version_constraint}"),
        source: Some(format!("winget · {package_id}")),
        requires_elevation: true,
        kind: InstallationStepKind::Command {
            program: "winget".into(),
            args: vec![
                "install".into(),
                "--id".into(),
                package_id.into(),
                "--exact".into(),
                "--version".into(),
                selected_version.into(),
                "--source".into(),
                "winget".into(),
                "--accept-package-agreements".into(),
                "--accept-source-agreements".into(),
                "--disable-interactivity".into(),
            ],
            working_directory: working_directory.to_path_buf(),
        },
    }
}

fn version_matches(constraint: &str, candidate: &str) -> Option<bool> {
    let constraint = constraint.trim().trim_start_matches(['v', 'V']);
    if matches!(
        constraint.to_ascii_lowercase().as_str(),
        "latest" | "stable" | "*"
    ) {
        return None;
    }
    let requested = constraint
        .split(['-', '+'])
        .next()?
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let candidate = candidate
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()?
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(
        candidate.len() >= requested.len()
            && candidate
                .iter()
                .zip(&requested)
                .all(|(candidate, requested)| candidate == requested),
    )
}

fn parse_winget_versions(output: &str) -> Vec<String> {
    let mut versions = output
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.chars()
                .next()
                .is_some_and(|value| value.is_ascii_digit())
        })
        .filter_map(|line| line.split_whitespace().next())
        .filter(|value| {
            value
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    versions.sort_by_key(|value| std::cmp::Reverse(version_components(value)));
    versions.dedup();
    versions
}

fn version_components(value: &str) -> Vec<u64> {
    value
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

#[cfg(target_os = "windows")]
fn merge_paths(first: Option<OsString>, second: Option<OsString>) -> Result<OsString, String> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for value in [first, second].into_iter().flatten() {
        for path in env::split_paths(&value) {
            let key = path.to_string_lossy().to_ascii_lowercase();
            if !key.is_empty() && seen.insert(key) {
                paths.push(path);
            }
        }
    }
    env::join_paths(paths).map_err(|error| format!("Não foi possível montar o PATH: {error}"))
}

fn executable_candidate(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn configure_background_command(_command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        _command.creation_flags(CREATE_NO_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders_winget_versions() {
        let output =
            "Found Node.js (LTS) [OpenJS.NodeJS.LTS]\nVersion\n-------\n22.12.0\n24.1.0\n20.18.1\n";
        assert_eq!(
            parse_winget_versions(output),
            ["24.1.0", "22.12.0", "20.18.1"]
        );
    }

    #[test]
    fn builds_an_exact_trusted_node_installation() {
        let step = package_installation_step(
            NODE_LTS_PACKAGE_ID,
            "Node.js",
            Path::new("project"),
            ">=20 <23",
            "22.12.0",
        );
        assert!(step.requires_elevation);
        assert_eq!(step.source.as_deref(), Some("winget · OpenJS.NodeJS.LTS"));
        let InstallationStepKind::Command { program, args, .. } = step.kind else {
            panic!("expected command step");
        };
        assert_eq!(program, "winget");
        assert!(
            args.windows(2)
                .any(|values| values == ["--version", "22.12.0"])
        );
        assert!(args.contains(&"--disable-interactivity".to_owned()));
    }

    #[test]
    fn matches_an_explicit_bun_version_without_upgrading_it() {
        assert_eq!(version_matches("1.3.0", "1.3.0"), Some(true));
        assert_eq!(version_matches("1.3.0", "1.4.2"), Some(false));
        assert_eq!(version_matches("latest", "1.4.2"), None);
    }
}
