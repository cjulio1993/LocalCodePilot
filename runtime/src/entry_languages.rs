//! Languages a project starts by running one file or one standard command (T028.01): Perl,
//! Lua, R (Shiny apps as sites), Julia, Haskell with only Cabal and Clojure with Leiningen.
//! Only an unequivocal entry is used; with several candidates the app asks for the command
//! and says why, and with none it says that nothing to start was found.

use super::push_process;
use localcodepilot_core::{
    ports::Port, processes::ProjectProcess, projects::Project, runtimes::RuntimeKind,
};
use std::{fs, path::Path};

/// Port of a Shiny app started by the app (Shiny's own default is random).
const SHINY_PORT: u16 = 3838;

/// The entry of a language that runs one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    One(String),
    /// More than one plausible file: the app does not pick one. Holds the candidates.
    Many(Vec<String>),
    None,
}

/// The file to run: one of `preferred` when exactly one exists, otherwise the only file with
/// one of `extensions` in the folder.
pub fn entry_file(folder: &Path, extensions: &[&str], preferred: &[&str]) -> Entry {
    let files = root_files(folder);
    let found = preferred
        .iter()
        .filter(|name| files.iter().any(|file| file == *name))
        .collect::<Vec<_>>();
    match found.as_slice() {
        [one] => return Entry::One((**one).to_owned()),
        [_, _, ..] => return Entry::Many(found.iter().map(|name| (**name).to_owned()).collect()),
        [] => {}
    }
    let candidates = files
        .iter()
        .filter(|file| {
            Path::new(file).extension().is_some_and(|extension| {
                extensions
                    .iter()
                    .any(|wanted| extension.eq_ignore_ascii_case(wanted))
            })
        })
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [one] => Entry::One((*one).clone()),
        [] => Entry::None,
        _ => Entry::Many(candidates.into_iter().cloned().collect()),
    }
}

fn root_files(folder: &Path) -> Vec<String> {
    let mut files = fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// Executables declared in the only `.cabal` file of a Cabal project (no `stack.yaml`).
fn cabal_entry(folder: &Path) -> Entry {
    if folder.join("stack.yaml").is_file() {
        return Entry::None;
    }
    let cabal_files = root_files(folder)
        .into_iter()
        .filter(|file| file.to_ascii_lowercase().ends_with(".cabal"))
        .collect::<Vec<_>>();
    let [cabal] = cabal_files.as_slice() else {
        return if cabal_files.is_empty() {
            Entry::None
        } else {
            Entry::Many(cabal_files)
        };
    };
    let Ok(contents) = fs::read_to_string(folder.join(cabal)) else {
        return Entry::None;
    };
    let executables = contents
        .lines()
        .filter_map(|line| {
            let rest = line.to_ascii_lowercase();
            rest.strip_prefix("executable")
                .filter(|rest| rest.starts_with(char::is_whitespace))
                .map(|rest| format!("executable {}", rest.trim()))
        })
        .collect::<Vec<_>>();
    match executables.len() {
        0 => Entry::None,
        1 => Entry::One("cabal run".into()),
        _ => Entry::Many(executables),
    }
}

/// C/C++ with CMake or Meson: the only executable target (T028.02).
fn cpp_entry(folder: &Path) -> Entry {
    let targets = crate::build_languages::build_targets(folder);
    let names = targets
        .iter()
        .map(|target| match target {
            crate::build_languages::BuildTarget::CMake(name)
            | crate::build_languages::BuildTarget::Meson(name) => name.clone(),
        })
        .collect::<Vec<_>>();
    match names.as_slice() {
        [] => Entry::None,
        [one] => Entry::One(one.clone()),
        _ => Entry::Many(names),
    }
}

/// Leiningen runs the namespace in `:main`.
fn leiningen_entry(folder: &Path) -> Entry {
    match fs::read_to_string(folder.join("project.clj")) {
        Ok(contents) if contents.contains(":main") => Entry::One("lein run".into()),
        _ => Entry::None,
    }
}

fn perl_entry(folder: &Path) -> Entry {
    entry_file(folder, &["pl"], &["main.pl", "app.pl", "script.pl"])
}

fn lua_entry(folder: &Path) -> Entry {
    entry_file(folder, &["lua"], &["main.lua"])
}

fn r_entry(folder: &Path) -> Entry {
    entry_file(folder, &["r"], &["main.R", "app.R"])
}

fn julia_entry(folder: &Path) -> Entry {
    entry_file(folder, &["jl"], &["main.jl", "app.jl", "run.jl"])
}

/// The entry of each language this module starts, for the languages the project has.
fn entries(project: &Project) -> Vec<(RuntimeKind, Entry)> {
    let folder = &project.path;
    project
        .runtimes
        .iter()
        .filter_map(|runtime| {
            let entry = match runtime {
                RuntimeKind::Perl => perl_entry(folder),
                RuntimeKind::Lua => lua_entry(folder),
                RuntimeKind::R => r_entry(folder),
                RuntimeKind::Julia => julia_entry(folder),
                RuntimeKind::Haskell => cabal_entry(folder),
                RuntimeKind::Clojure => leiningen_entry(folder),
                RuntimeKind::Cpp => cpp_entry(folder),
                _ => return None,
            };
            Some((*runtime, entry))
        })
        .collect()
}

/// Several plausible entries for a language of the project: the card says so instead of
/// picking one.
pub fn has_ambiguous_entry(project: &Project) -> bool {
    ambiguous_entries(project).is_some()
}

/// The candidates of each ambiguous language, for "Detalhes técnicos": "Perl: a.pl, b.pl".
pub fn ambiguous_entries(project: &Project) -> Option<String> {
    let lines = entries(project)
        .into_iter()
        .filter_map(|(runtime, entry)| match entry {
            Entry::Many(candidates) => Some(format!("{runtime}: {}", candidates.join(", "))),
            _ => None,
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("; "))
}

/// Adds the process of the first language with an unequivocal entry. Called only when no
/// framework or manifest gave a command, so a helper script never replaces the real one.
pub(super) fn detect_entry_process(project: &Project, processes: &mut Vec<ProjectProcess>) {
    let folder = &project.path;
    // With an ambiguous language the app asks instead of starting another one.
    if has_ambiguous_entry(project) {
        return;
    }
    for (runtime, entry) in entries(project) {
        let Entry::One(entry) = entry else {
            continue;
        };
        match runtime {
            RuntimeKind::Perl => {
                push_process(
                    processes,
                    project,
                    folder,
                    "Executar script Perl",
                    "perl",
                    &[&entry],
                );
            }
            RuntimeKind::Lua => {
                push_process(
                    processes,
                    project,
                    folder,
                    "Executar script Lua",
                    "lua",
                    &[&entry],
                );
            }
            RuntimeKind::R => {
                let shiny = fs::read_to_string(folder.join(&entry)).is_ok_and(|contents| {
                    contents.contains("shinyApp(") || contents.contains("shinyServer(")
                });
                if shiny {
                    let expression = format!(
                        "shiny::runApp('{entry}', port={SHINY_PORT}, launch.browser=FALSE)"
                    );
                    push_process(
                        processes,
                        project,
                        folder,
                        "Servidor Shiny",
                        "Rscript",
                        &["-e", &expression],
                    );
                    if let Some(process) = processes.last_mut() {
                        process.expected_port = Port::new(SHINY_PORT);
                    }
                } else {
                    push_process(
                        processes,
                        project,
                        folder,
                        "Executar script R",
                        "Rscript",
                        &[&entry],
                    );
                }
            }
            RuntimeKind::Julia => {
                if folder.join("Project.toml").is_file() {
                    push_process(
                        processes,
                        project,
                        folder,
                        "Executar programa Julia",
                        "julia",
                        &["--project=.", &entry],
                    );
                } else {
                    push_process(
                        processes,
                        project,
                        folder,
                        "Executar programa Julia",
                        "julia",
                        &[&entry],
                    );
                }
            }
            RuntimeKind::Haskell => {
                push_process(
                    processes,
                    project,
                    folder,
                    "Executar projeto Haskell",
                    "cabal",
                    &["run"],
                );
            }
            RuntimeKind::Cpp => {
                let Ok([target]) =
                    <[_; 1]>::try_from(crate::build_languages::build_targets(folder))
                else {
                    continue;
                };
                let Some(build) = crate::build_languages::build_folder(folder) else {
                    continue;
                };
                let (steps, program) = crate::build_languages::build_plan(folder, &target, &build);
                let name = match target {
                    crate::build_languages::BuildTarget::CMake(_) => "Compilar e executar (CMake)",
                    crate::build_languages::BuildTarget::Meson(_) => "Compilar e executar (Meson)",
                };
                push_process(
                    processes,
                    project,
                    folder,
                    name,
                    &program.to_string_lossy(),
                    &[],
                );
                if let Some(process) = processes.last_mut() {
                    process.prepare = steps;
                }
            }
            RuntimeKind::Clojure => {
                push_process(
                    processes,
                    project,
                    folder,
                    "Executar projeto Clojure",
                    "lein",
                    &["run"],
                );
            }
            _ => continue,
        }
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("lcp-t028-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str, contents: &str) -> &Self {
            fs::write(self.0.join(name), contents).unwrap();
            self
        }

        fn project(&self, runtimes: &[RuntimeKind]) -> Project {
            Project::new(self.0.clone(), runtimes.to_vec())
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn commands(project: &Project) -> Vec<String> {
        let mut processes = Vec::new();
        detect_entry_process(project, &mut processes);
        processes.iter().map(ProjectProcess::command_line).collect()
    }

    #[test]
    fn one_obvious_file_starts_and_more_than_one_asks() {
        let perl = Folder::new("perl");
        perl.file("main.pl", "print 1;").file("util.pl", "1;");
        assert_eq!(
            commands(&perl.project(&[RuntimeKind::Perl])),
            vec!["perl main.pl"]
        );

        let single = Folder::new("lua-unico");
        single.file("jogo.lua", "print(1)");
        assert_eq!(
            commands(&single.project(&[RuntimeKind::Lua])),
            vec!["lua jogo.lua"]
        );

        let many = Folder::new("lua-varios");
        many.file("a.lua", "").file("b.lua", "");
        let project = many.project(&[RuntimeKind::Lua]);
        assert!(commands(&project).is_empty());
        assert!(has_ambiguous_entry(&project));
        assert_eq!(
            ambiguous_entries(&project).as_deref(),
            Some("Lua: a.lua, b.lua")
        );

        let two_preferred = Folder::new("perl-dois");
        two_preferred.file("main.pl", "").file("app.pl", "");
        assert!(has_ambiguous_entry(
            &two_preferred.project(&[RuntimeKind::Perl])
        ));

        let none = Folder::new("julia-nada");
        none.file("README.md", "");
        let project = none.project(&[RuntimeKind::Julia]);
        assert!(commands(&project).is_empty());
        assert!(!has_ambiguous_entry(&project));
    }

    #[test]
    fn r_scripts_run_and_shiny_apps_open_as_sites() {
        let script = Folder::new("r-script");
        script.file("main.R", "print(1)");
        assert_eq!(
            commands(&script.project(&[RuntimeKind::R])),
            vec!["Rscript main.R"]
        );

        let shiny = Folder::new("r-shiny");
        shiny.file("app.R", "library(shiny)\nshinyApp(ui, server)\n");
        let mut processes = Vec::new();
        detect_entry_process(&shiny.project(&[RuntimeKind::R]), &mut processes);
        assert_eq!(
            processes[0].command_line(),
            "Rscript -e shiny::runApp('app.R', port=3838, launch.browser=FALSE)"
        );
        assert_eq!(processes[0].expected_port, Port::new(SHINY_PORT));
    }

    #[test]
    fn julia_uses_the_project_environment_when_there_is_one() {
        let plain = Folder::new("julia");
        plain.file("main.jl", "println(1)");
        assert_eq!(
            commands(&plain.project(&[RuntimeKind::Julia])),
            vec!["julia main.jl"]
        );
        plain.file("Project.toml", "name = \"App\"\n");
        assert_eq!(
            commands(&plain.project(&[RuntimeKind::Julia])),
            vec!["julia --project=. main.jl"]
        );
    }

    #[test]
    fn cabal_and_leiningen_need_one_declared_executable() {
        let cabal = Folder::new("cabal");
        cabal.file(
            "app.cabal",
            "cabal-version: 3.0\nname: app\nexecutable app\n  main-is: Main.hs\n",
        );
        assert_eq!(
            commands(&cabal.project(&[RuntimeKind::Haskell])),
            vec!["cabal run"]
        );

        let two = Folder::new("cabal-dois");
        two.file(
            "app.cabal",
            "executable app\n  main-is: A.hs\nexecutable outra\n  main-is: B.hs\n",
        );
        assert!(has_ambiguous_entry(&two.project(&[RuntimeKind::Haskell])));

        let library = Folder::new("cabal-lib");
        library.file("lib.cabal", "library\n  exposed-modules: Lib\n");
        assert!(commands(&library.project(&[RuntimeKind::Haskell])).is_empty());

        // Stack projects keep `stack run` (detected elsewhere).
        let stack = Folder::new("stack");
        stack
            .file("stack.yaml", "")
            .file("app.cabal", "executable app\n");
        assert!(commands(&stack.project(&[RuntimeKind::Haskell])).is_empty());

        let lein = Folder::new("lein");
        lein.file("project.clj", "(defproject app \"0.1.0\" :main app.core)");
        assert_eq!(
            commands(&lein.project(&[RuntimeKind::Clojure])),
            vec!["lein run"]
        );
        let no_main = Folder::new("lein-sem-main");
        no_main.file("project.clj", "(defproject app \"0.1.0\")");
        assert!(commands(&no_main.project(&[RuntimeKind::Clojure])).is_empty());
    }

    #[test]
    fn a_cmake_or_meson_project_with_one_executable_compiles_then_runs() {
        let cmake = Folder::new("cmake");
        cmake.file(
            "CMakeLists.txt",
            "project(loja C)\nadd_executable(loja main.c)\n",
        );
        let project = cmake.project(&[RuntimeKind::Cpp]);
        let mut processes = Vec::new();
        detect_entry_process(&project, &mut processes);
        if crate::build_languages::build_folder(&cmake.0).is_some() {
            assert_eq!(processes.len(), 1);
            let process = &processes[0];
            assert_eq!(process.name, "Compilar e executar (CMake)");
            assert_eq!(process.prepare.len(), 2);
            assert_eq!(process.prepare[0].program, "cmake");
            // The build folder is never inside the project.
            assert!(!Path::new(&process.program).starts_with(&cmake.0));
        }

        let two = Folder::new("cmake-dois");
        two.file(
            "CMakeLists.txt",
            "add_executable(servidor s.c)\nadd_executable(cliente c.c)\n",
        );
        assert!(has_ambiguous_entry(&two.project(&[RuntimeKind::Cpp])));
        assert!(commands(&two.project(&[RuntimeKind::Cpp])).is_empty());

        let library = Folder::new("cmake-lib");
        library.file("CMakeLists.txt", "add_library(util STATIC u.c)\n");
        assert!(commands(&library.project(&[RuntimeKind::Cpp])).is_empty());
        assert!(!has_ambiguous_entry(&library.project(&[RuntimeKind::Cpp])));
    }

    #[test]
    fn a_language_the_project_does_not_have_is_never_started() {
        let node = Folder::new("node-com-perl");
        node.file("main.pl", "print 1;");
        assert!(commands(&node.project(&[RuntimeKind::Node])).is_empty());
    }
}
