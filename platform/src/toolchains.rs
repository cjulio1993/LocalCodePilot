//! Environment some toolchains need beyond the PATH.
//!
//! Swift on Windows links with the Microsoft linker, so it only builds inside the Visual
//! Studio developer environment (`vcvars64.bat`) and with the `SDKROOT` its installer saves
//! for the user. The app sets both up for `swift`, so "Iniciar" works without opening a
//! developer prompt.
//!
//! GHC, which Stack downloads for Haskell projects, cannot run its own C compiler from a
//! folder with a space, such as `C:\Users\Julio Cesar`. When the user folder has one, Stack
//! keeps its files in a folder without spaces.

use std::{collections::HashMap, ffi::OsStr, path::PathBuf, process::Command, sync::OnceLock};
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
    if !cfg!(windows) {
        return None;
    }
    static SWIFT: OnceLock<Option<ToolchainEnvironment>> = OnceLock::new();
    static STACK: OnceLock<Option<ToolchainEnvironment>> = OnceLock::new();
    match program.to_ascii_lowercase().as_str() {
        "swift" => SWIFT.get_or_init(swift_windows_environment).as_ref(),
        "stack" => STACK.get_or_init(stack_windows_environment).as_ref(),
        _ => None,
    }
}

/// Applies the extra environment of `program` to `command`, whose PATH is `path` (or the
/// app's own when `None`): the variables, and the toolchain folders before that PATH.
pub fn apply(command: &mut Command, program: &str, path: Option<&OsStr>) {
    let Some(toolchain) = environment_for(program) else {
        return;
    };
    for (key, value) in &toolchain.variables {
        command.env(key, value);
    }
    if toolchain.path_prefix.is_empty() {
        return;
    }
    let base = path
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    if let Ok(joined) = std::env::join_paths(
        toolchain
            .path_prefix
            .iter()
            .cloned()
            .chain(std::env::split_paths(&base)),
    ) {
        command.env("PATH", joined);
    }
}

#[cfg(windows)]
fn stack_windows_environment() -> Option<ToolchainEnvironment> {
    if !env::var("LOCALAPPDATA").ok()?.contains(' ') {
        return None;
    }
    let base = PathBuf::from(env::var_os("ProgramData")?)
        .join("LocalCodePilot")
        .join("stack");
    let root = base.join("root");
    let programs = base.join("programs");
    std::fs::create_dir_all(&root).ok()?;
    std::fs::create_dir_all(&programs).ok()?;
    let config = root.join("config.yaml");
    if !config.is_file() {
        std::fs::write(
            &config,
            format!("local-programs-path: {}\n", programs.display()),
        )
        .ok()?;
    }
    Some(ToolchainEnvironment {
        variables: vec![("STACK_ROOT".into(), root.display().to_string())],
        path_prefix: Vec::new(),
    })
}

#[cfg(not(windows))]
fn stack_windows_environment() -> Option<ToolchainEnvironment> {
    None
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
