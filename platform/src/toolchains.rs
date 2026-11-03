//! Environment some toolchains need beyond the PATH.
//!
//! Swift on Windows links with the Microsoft linker, so it only builds inside the Visual
//! Studio developer environment (`vcvars64.bat`) and with the `SDKROOT` its installer saves
//! for the user. The app sets both up for `swift`, so "Iniciar" works without opening a
//! developer prompt.

use std::{collections::HashMap, path::PathBuf, sync::OnceLock};
#[cfg(windows)]
use std::{env, path::Path};

/// Variables to set and folders to put before the PATH of a process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolchainEnvironment {
    pub variables: Vec<(String, String)>,
    pub path_prefix: Vec<PathBuf>,
}

/// The extra environment `program` needs on this computer, if any. Computed once.
pub fn environment_for(program: &str) -> Option<&'static ToolchainEnvironment> {
    if !cfg!(windows) || !program.eq_ignore_ascii_case("swift") {
        return None;
    }
    static SWIFT: OnceLock<Option<ToolchainEnvironment>> = OnceLock::new();
    SWIFT.get_or_init(swift_windows_environment).as_ref()
}

#[cfg(windows)]
fn swift_windows_environment() -> Option<ToolchainEnvironment> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let mut environment = ToolchainEnvironment::default();
    if env::var_os("SDKROOT").is_none()
        && let Some(sdkroot) = saved_variable("SDKROOT")
    {
        environment.variables.push(("SDKROOT".into(), sdkroot));
    }
    let vswhere = PathBuf::from(env::var_os("ProgramFiles(x86)")?)
        .join("Microsoft Visual Studio/Installer/vswhere.exe");
    let mut query = Command::new(&vswhere);
    query.args([
        "-latest",
        "-products",
        "*",
        "-requires",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "-property",
        "installationPath",
    ]);
    crate::installations::configure_background_command(&mut query);
    let output = query.output().ok()?;
    let installation = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let vcvars = Path::new(&installation).join("VC/Auxiliary/Build/vcvars64.bat");
    if installation.is_empty() || !vcvars.is_file() {
        return Some(environment);
    }
    let mut capture = Command::new("cmd");
    capture.raw_arg(format!("/d /s /c \"\"{}\" >nul && set\"", vcvars.display()));
    crate::installations::configure_background_command(&mut capture);
    let output = capture.output().ok()?;
    let current = env::vars().collect::<HashMap<_, _>>();
    let developer = environment_changes(&current, &String::from_utf8_lossy(&output.stdout));
    environment.variables.extend(developer.variables);
    environment.path_prefix = developer.path_prefix;
    Some(environment)
}

#[cfg(not(windows))]
fn swift_windows_environment() -> Option<ToolchainEnvironment> {
    None
}

/// A variable the installers save for the user or the machine, read from the registry so
/// that an install made after the app opened counts.
#[cfg(windows)]
fn saved_variable(name: &str) -> Option<String> {
    use std::process::Command;

    let script = format!(
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8;$u=[Environment]::GetEnvironmentVariable('{name}','User');if($u){{$u}}else{{[Environment]::GetEnvironmentVariable('{name}','Machine')}}"
    );
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]);
    crate::installations::configure_background_command(&mut command);
    let output = command.output().ok()?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

/// What `set` printed after the developer script, compared with the app's environment:
/// changed variables, and the PATH folders the script added, in its order.
#[cfg_attr(not(windows), allow(dead_code))]
fn environment_changes(
    current: &HashMap<String, String>,
    set_output: &str,
) -> ToolchainEnvironment {
    let current_path = current
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.to_ascii_lowercase())
        .unwrap_or_default();
    let known = current_path.split(';').collect::<Vec<_>>();
    let mut changes = ToolchainEnvironment::default();
    for line in set_output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        if key.eq_ignore_ascii_case("PATH") {
            changes.path_prefix = value
                .split(';')
                .filter(|folder| {
                    !folder.is_empty() && !known.contains(&folder.to_ascii_lowercase().as_str())
                })
                .map(PathBuf::from)
                .collect();
        } else if current.get(key).map(String::as_str) != Some(value) {
            changes.variables.push((key.to_owned(), value.to_owned()));
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_what_the_developer_script_changed() {
        let current = HashMap::from([
            ("PATH".to_owned(), r"C:\Windows;C:\Tools".to_owned()),
            ("USERNAME".to_owned(), "julio".to_owned()),
        ]);
        let output = "INCLUDE=C:\\VS\\include\r\nPath=C:\\VS\\bin;C:\\Windows;C:\\Tools\r\nUSERNAME=julio\r\n";
        let changes = environment_changes(&current, output);
        assert_eq!(
            changes.variables,
            [("INCLUDE".to_owned(), r"C:\VS\include".to_owned())]
        );
        assert_eq!(changes.path_prefix, [PathBuf::from(r"C:\VS\bin")]);
    }

    #[test]
    fn only_swift_gets_an_extra_environment() {
        assert!(environment_for("node").is_none());
        if !cfg!(windows) {
            assert!(environment_for("swift").is_none());
        }
    }
}
