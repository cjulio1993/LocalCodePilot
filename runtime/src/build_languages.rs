//! C/C++ projects built with CMake or Meson (T028.02): "Iniciar" configures, compiles and runs
//! the project's only executable. The build folder lives in the app's cache, never inside the
//! project, and the executable always lands in `<build>/bin` (CMake) or `<build>` (Meson).

use localcodepilot_core::{paths::plain_path, processes::PrepareStep};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// The executable target of a CMake or Meson project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildTarget {
    CMake(String),
    Meson(String),
}

/// Targets declared with `add_executable(<name> ...)` (CMake) or `executable('<name>', ...)`
/// (Meson) in the project's root file. Imported and alias targets are not programs to run.
pub fn build_targets(folder: &Path) -> Vec<BuildTarget> {
    if let Ok(contents) = fs::read_to_string(folder.join("CMakeLists.txt")) {
        return cmake_executables(&without_comments(&contents, true))
            .into_iter()
            .map(BuildTarget::CMake)
            .collect();
    }
    if let Ok(contents) = fs::read_to_string(folder.join("meson.build")) {
        return meson_executables(&without_comments(&contents, false))
            .into_iter()
            .map(BuildTarget::Meson)
            .collect();
    }
    Vec::new()
}

/// The file without `#` comments, so a commented-out `add_executable` or `executable` is
/// not a target. Quoted text keeps its `#`. CMake also has bracket comments (`#[[ ... ]]`,
/// `#[=[ ... ]=]`), which may span lines.
fn without_comments(contents: &str, cmake: bool) -> String {
    let mut out = String::with_capacity(contents.len());
    let mut rest = contents;
    let mut quote: Option<char> = None;
    while let Some(character) = rest.chars().next() {
        if let Some(open) = quote {
            // Meson names targets with quoted text, so it stays; in CMake a quoted
            // `add_executable(` is a message, not a call, and is blanked.
            let keep = |character: char| {
                if cmake && character != '\n' && character != open {
                    ' '
                } else {
                    character
                }
            };
            out.push(keep(character));
            rest = &rest[character.len_utf8()..];
            if character == '\\' {
                if let Some(next) = rest.chars().next() {
                    out.push(if cmake && next != '\n' { ' ' } else { next });
                    rest = &rest[next.len_utf8()..];
                }
            } else if character == open {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => {
                // CMake has no single-quoted strings.
                if character == '"' || !cmake {
                    quote = Some(character);
                }
                out.push(character);
                rest = &rest[1..];
            }
            '#' => {
                let after = &rest[1..];
                let bracket = cmake
                    .then(|| after.strip_prefix('['))
                    .flatten()
                    .and_then(|open| {
                        let equals = open.len() - open.trim_start_matches('=').len();
                        open[equals..]
                            .starts_with('[')
                            .then(|| format!("]{}]", "=".repeat(equals)))
                    });
                rest = match bracket
                    .and_then(|close| after.find(&close).map(|end| end + close.len()))
                {
                    Some(end) => &after[end..],
                    // A line comment, or a bracket comment that never closes.
                    None => after.find('\n').map_or("", |end| &after[end..]),
                };
            }
            _ => {
                out.push(character);
                rest = &rest[character.len_utf8()..];
            }
        }
    }
    out
}

fn cmake_executables(contents: &str) -> Vec<String> {
    let lower = contents.to_ascii_lowercase();
    let mut names = Vec::new();
    let mut rest = lower.as_str();
    let mut offset = 0;
    while let Some(position) = rest.find("add_executable(") {
        let start = offset + position + "add_executable(".len();
        let call = &contents[start..];
        let end = call.find(')').unwrap_or(call.len());
        let mut words = call[..end].split_whitespace();
        if let Some(name) = words.next() {
            let arguments = call[..end].to_ascii_uppercase();
            let imported = arguments.contains(" IMPORTED") || arguments.contains(" ALIAS ");
            // A name built from a variable cannot be known without configuring.
            if !imported && !name.contains("${") {
                names.push(name.to_owned());
            }
        }
        offset = start;
        rest = &lower[offset..];
    }
    names
}

fn meson_executables(contents: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = contents;
    while let Some(position) = rest.find("executable(") {
        // `shared_library(` and others end in other words; only a bare `executable(` counts.
        let before = rest[..position].chars().last();
        let call = &rest[position + "executable(".len()..];
        if !before.is_some_and(|character| character.is_alphanumeric() || character == '_') {
            let quoted = call.trim_start();
            if let Some(stripped) = quoted
                .strip_prefix('\'')
                .or_else(|| quoted.strip_prefix('"'))
                && let Some(end) = stripped.find(['\'', '"'])
            {
                names.push(stripped[..end].to_owned());
            }
        }
        rest = call;
    }
    names
}

/// The app's build folder for `project`, outside the project.
pub fn build_folder(project: &Path) -> Option<PathBuf> {
    let root = if cfg!(windows) {
        PathBuf::from(env::var_os("LOCALAPPDATA")?)
            .join("LocalCodePilot")
            .join("builds")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(env::var_os("HOME")?)
            .join("Library")
            .join("Caches")
            .join("LocalCodePilot")
            .join("builds")
    } else {
        env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?
            .join("localcodepilot")
            .join("builds")
    };
    let mut path = plain_path(project).to_string_lossy().into_owned();
    if cfg!(windows) {
        path = path.to_lowercase();
    }
    let digest = Sha256::digest(path.as_bytes());
    let hash = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let name = project
        .file_name()
        .map(|name| {
            name.to_string_lossy()
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() || character == '-' {
                        character
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .unwrap_or_default();
    Some(root.join(format!("{name}-{hash}")))
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// The steps before the program, and the program itself.
pub fn build_plan(
    project: &Path,
    target: &BuildTarget,
    build: &Path,
) -> (Vec<PrepareStep>, PathBuf) {
    // Build tools do not accept the verbatim `\\?\` prefix (MSBuild fails with it).
    let project_arg = plain_path(project).to_string_lossy().into_owned();
    let build_arg = plain_path(build).to_string_lossy().into_owned();
    match target {
        BuildTarget::CMake(name) => {
            let bin = build.join("bin");
            let bin_arg = plain_path(&bin).to_string_lossy().into_owned();
            (
                vec![
                    PrepareStep {
                        program: "cmake".into(),
                        args: vec![
                            "-S".into(),
                            project_arg,
                            "-B".into(),
                            build_arg.clone(),
                            format!("-DCMAKE_RUNTIME_OUTPUT_DIRECTORY={bin_arg}"),
                            format!("-DCMAKE_RUNTIME_OUTPUT_DIRECTORY_DEBUG={bin_arg}"),
                        ],
                        skip_if_exists: Some(build.join("CMakeCache.txt")),
                    },
                    PrepareStep {
                        program: "cmake".into(),
                        args: vec![
                            "--build".into(),
                            build_arg,
                            "--target".into(),
                            name.clone(),
                            "--config".into(),
                            "Debug".into(),
                        ],
                        skip_if_exists: None,
                    },
                ],
                bin.join(executable_name(name)),
            )
        }
        BuildTarget::Meson(name) => (
            vec![
                PrepareStep {
                    program: "meson".into(),
                    args: vec!["setup".into(), build_arg.clone(), project_arg],
                    skip_if_exists: Some(build.join("meson-private").join("coredata.dat")),
                },
                PrepareStep {
                    program: "meson".into(),
                    args: vec!["compile".into(), "-C".into(), build_arg],
                    skip_if_exists: None,
                },
            ],
            build.join(executable_name(name)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_executables_of_cmake_and_meson_projects() {
        let cmake = "cmake_minimum_required(VERSION 3.20)\nproject(loja C)\nadd_executable(loja main.c util.c)\nadd_library(util STATIC util.c)\n";
        assert_eq!(cmake_executables(cmake), vec!["loja"]);
        let two = "add_executable(servidor s.c)\nADD_EXECUTABLE(cliente c.c)\n";
        assert_eq!(cmake_executables(two), vec!["servidor", "cliente"]);
        let imported =
            "add_executable(ferramenta IMPORTED)\nadd_executable(${PROJECT_NAME} main.c)\n";
        assert!(cmake_executables(imported).is_empty());

        let meson =
            "project('loja', 'c')\nexecutable('loja', 'main.c')\nshared_library('util', 'u.c')\n";
        assert_eq!(meson_executables(meson), vec!["loja"]);
        let lib_only = "project('x', 'c')\nshared_library('x', 'x.c')\n";
        assert!(meson_executables(lib_only).is_empty());
    }

    #[test]
    fn commented_out_targets_are_not_targets() {
        let cmake = "project(loja C)\n# add_executable(antigo old.c)\nadd_executable(loja main.c) # add_executable(outro o.c)\n#[[\nadd_executable(bloco b.c)\n]]\n#[==[ add_executable(igual e.c) ]==]\nmessage(\"# add_executable(texto t.c)\")\n";
        let cleaned = without_comments(cmake, true);
        assert_eq!(cmake_executables(&cleaned), vec!["loja"]);
        assert!(cleaned.contains("message(\""));
        assert!(!cleaned.contains("texto"));

        let meson = "project('loja', 'c')\n# executable('velho', 'v.c')\nexecutable('loja', 'main.c') # executable('x', 'x.c')\nmessage('# executable(\\'texto\\')')\n";
        assert_eq!(
            meson_executables(&without_comments(meson, false)),
            vec!["loja"]
        );

        // A real second target is still ambiguous.
        let two = "# add_executable(velho v.c)\nadd_executable(a a.c)\nadd_executable(b b.c)\n";
        assert_eq!(
            cmake_executables(&without_comments(two, true)),
            vec!["a", "b"]
        );
    }

    #[test]
    fn the_build_folder_is_outside_the_project_and_stable() {
        let project = Path::new("C:/projetos/loja");
        let Some(build) = build_folder(project) else {
            return;
        };
        assert!(!build.starts_with(project));
        assert_eq!(build_folder(project), Some(build.clone()));
        assert_ne!(build_folder(Path::new("C:/projetos/outra")), Some(build));
    }

    #[test]
    fn cmake_configures_once_builds_the_target_and_runs_it_from_bin() {
        let build = Path::new("B");
        let (steps, program) =
            build_plan(Path::new("P"), &BuildTarget::CMake("loja".into()), build);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].program, "cmake");
        assert_eq!(steps[0].skip_if_exists, Some(build.join("CMakeCache.txt")));
        assert!(
            steps[1]
                .command_line()
                .contains("--build B --target loja --config Debug")
        );
        assert_eq!(program, build.join("bin").join(executable_name("loja")));

        let (steps, program) =
            build_plan(Path::new("P"), &BuildTarget::Meson("loja".into()), build);
        assert_eq!(steps[0].command_line(), "meson setup B P");
        assert_eq!(steps[1].command_line(), "meson compile -C B");
        assert_eq!(program, build.join(executable_name("loja")));
    }
}
