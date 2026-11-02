//! "Linguagens neste computador" in Configurações and the language lines of project cards:
//! where each language was found and whether this computer has it.

use super::*;
use localcodepilot_core::languages::{self, LANGUAGES, Language};
use localcodepilot_platform::installations::VersionAnswer;
use localcodepilot_platform::languages::{LanguageCheck, LanguageStatus, check_languages};

#[derive(Default)]
pub(super) struct LanguageState {
    checks: Option<Vec<LanguageCheck>>,
    pending: Option<Receiver<Vec<LanguageCheck>>>,
    started: bool,
    filter: String,
}

/// The first line a program printed for its version, short enough for a row.
fn short_version(version: &str) -> String {
    let version = version.trim();
    if version.chars().count() > 60 {
        format!("{}…", version.chars().take(60).collect::<String>())
    } else {
        version.to_owned()
    }
}

/// One phrase for the state of a language on this computer.
fn status_text(check: Option<&LanguageCheck>) -> (String, Color32) {
    match check.map(|check| &check.status) {
        None => ("Verificando...".to_owned(), theme::MUTED),
        Some(LanguageStatus::Found {
            version: VersionAnswer::Version(version),
            ..
        }) => (
            format!("Instalado · {}", short_version(version)),
            theme::SUCCESS,
        ),
        Some(LanguageStatus::Found { .. }) => {
            ("Instalado · Versão indisponível".to_owned(), theme::WARNING)
        }
        Some(LanguageStatus::StoreShortcut { .. } | LanguageStatus::NotFound) => (
            "Não encontrado nos locais verificados".to_owned(),
            theme::MUTED,
        ),
    }
}

fn check_details(ui: &mut egui::Ui, language: &Language, check: Option<&LanguageCheck>) {
    ui.spacing_mut().item_spacing.y = 2.0;
    if let Some(check) = check {
        match &check.status {
            LanguageStatus::Found {
                program,
                path,
                outside_path,
                version,
            } => {
                notice::detail_line(ui, &format!("Programa: {program} · {}", display_path(path)));
                if *outside_path {
                    notice::detail_line(
                        ui,
                        "Encontrado fora do PATH, numa pasta padrão de instalação: um terminal pode não encontrá-lo.",
                    );
                }
                match version {
                    VersionAnswer::Version(_) => {}
                    VersionAnswer::TimedOut => {
                        notice::detail_line(ui, "O programa não informou a versão em 10 segundos.")
                    }
                    VersionAnswer::Failed => notice::detail_line(
                        ui,
                        "O programa respondeu com erro ao informar a versão.",
                    ),
                }
            }
            LanguageStatus::StoreShortcut { path } => notice::detail_line(
                ui,
                &format!(
                    "Só existe o atalho da Microsoft Store ({}); a linguagem não está instalada por trás dele.",
                    display_path(path)
                ),
            ),
            LanguageStatus::NotFound => {}
        }
        notice::detail_line(
            ui,
            &format!("Locais verificados: {}", check.searched.join(", ")),
        );
    }
    let programs = language
        .programs
        .iter()
        .map(|program| format!("{} {}", program.name, program.version_args.join(" ")))
        .collect::<Vec<_>>();
    notice::detail_line(
        ui,
        &format!("Versão consultada com: {}", programs.join("; ")),
    );
    let files = language
        .manifests
        .iter()
        .map(|manifest| (*manifest).to_owned())
        .chain(
            language
                .manifest_extensions
                .iter()
                .map(|extension| format!("*.{extension}")),
        )
        .chain(
            language
                .source_extensions
                .iter()
                .map(|extension| format!(".{extension}")),
        )
        .collect::<Vec<_>>();
    if !files.is_empty() {
        notice::detail_line(ui, &format!("Reconhecido por: {}", files.join(", ")));
    }
    if !language.images.is_empty() {
        notice::detail_line(
            ui,
            &format!(
                "Imagens Docker reconhecidas: {}",
                language.images.join(", ")
            ),
        );
    }
}

impl LocalCodePilot {
    /// Checks every language in the background; once at startup and on request.
    pub(super) fn check_languages(&mut self, ctx: egui::Context) {
        if self.languages.pending.is_some() {
            return;
        }
        self.languages.started = true;
        let path = self.environment_path.clone();
        let (sender, receiver) = mpsc::channel();
        self.languages.pending = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(check_languages(path.as_deref()));
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_languages(&mut self, ctx: &egui::Context) {
        if !self.languages.started {
            self.check_languages(ctx.clone());
        }
        let Some(receiver) = &self.languages.pending else {
            return;
        };
        match receiver.try_recv() {
            Ok(checks) => {
                self.languages.checks = Some(checks);
                self.languages.pending = None;
            }
            Err(TryRecvError::Disconnected) => self.languages.pending = None,
            Err(TryRecvError::Empty) => {}
        }
    }

    fn language_check(&self, kind: RuntimeKind) -> Option<&LanguageCheck> {
        self.languages
            .checks
            .as_ref()?
            .iter()
            .find(|check| check.kind == kind)
    }

    /// Lines for a project card: each language, where it was found and whether this computer
    /// has it. A language seen only in an image says so, without claiming the files use it.
    pub(super) fn language_lines(&self, project: &Project) -> Vec<String> {
        let installed = |kind: RuntimeKind| match self
            .language_check(kind)
            .map(|check| &check.status)
        {
            Some(LanguageStatus::Found {
                version: VersionAnswer::Version(version),
                ..
            }) => format!(" · instalado: {}", short_version(version)),
            Some(LanguageStatus::Found { .. }) => " · instalado, versão indisponível".to_owned(),
            Some(_) => " · não encontrado nos locais verificados".to_owned(),
            None => String::new(),
        };
        let mut lines = project
            .runtimes
            .iter()
            .map(|kind| format!("{kind}: nos arquivos do projeto{}", installed(*kind)))
            .collect::<Vec<_>>();
        for environment in self
            .container_state_projects()
            .iter()
            .filter(|environment| {
                localcodepilot_core::containers::path_is_within(
                    &environment.directory,
                    &project.path,
                )
            })
        {
            let services = environment.services.iter().filter_map(|service| {
                let image = service.image.as_deref()?;
                Some((
                    languages::from_image(image)?,
                    format!("na imagem do serviço {} ({image})", service.name),
                ))
            });
            let bases = environment.base_images.iter().filter_map(|image| {
                Some((
                    languages::from_image(image)?,
                    format!("na imagem base do Dockerfile ({image})"),
                ))
            });
            for (kind, origin) in services.chain(bases) {
                lines.push(format!("{kind}: {origin}"));
            }
        }
        lines
    }

    pub(super) fn languages_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Linguagens neste computador");
        ui.label(
            RichText::new(
                "O LocalCodePilot procura cada linguagem no PATH e nas pastas padrão de instalação. Nada é instalado por aqui.",
            )
            .color(theme::MUTED),
        );
        ui.add_space(10.0);
        let checking = self.languages.pending.is_some();
        let check = ui
            .horizontal_wrapped(|ui| {
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.languages.filter)
                        .hint_text("Buscar linguagem")
                        .desired_width(260.0),
                );
                announced(search, egui::Role::SearchInput, "Buscar linguagem");
                let clicked = ui
                    .add_enabled(!checking, egui::Button::new("Verificar de novo"))
                    .clicked();
                if checking {
                    ui.spinner();
                    ui.label("Verificando...");
                }
                clicked
            })
            .inner;
        if check {
            self.check_languages(ui.ctx().clone());
        }
        if checking {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        ui.add_space(8.0);
        let filter = self.languages.filter.trim().to_lowercase();
        let mut rows = LANGUAGES
            .iter()
            .filter(|language| {
                filter.is_empty()
                    || language.name.to_lowercase().contains(&filter)
                    || language.kind.to_string().to_lowercase().contains(&filter)
            })
            .collect::<Vec<_>>();
        // What this computer has comes first; the table order is kept within each part.
        rows.sort_by_key(|language| {
            !matches!(
                self.language_check(language.kind)
                    .map(|check| &check.status),
                Some(LanguageStatus::Found { .. })
            )
        });
        if rows.is_empty() {
            ui.label(
                RichText::new("Nenhuma linguagem encontrada. Tente outro termo.")
                    .color(theme::MUTED),
            );
        }
        for language in rows {
            let check = self.language_check(language.kind);
            let (status, color) = status_text(check);
            let used_by = self
                .projects
                .iter()
                .filter(|project| project.runtimes.contains(&language.kind))
                .map(|project| project.name.as_str())
                .collect::<Vec<_>>();
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(8)
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let details = ui
                        .horizontal_wrapped(|ui| {
                            ui.label(RichText::new(language.name).strong());
                            ui.label(RichText::new(&status).color(color));
                            notice::disclosure(
                                ui,
                                egui::Id::unique(("language-details", language.name)),
                                "Detalhes técnicos",
                                false,
                            )
                        })
                        .inner;
                    if !language.starts {
                        ui.label(
                            RichText::new(
                                "O LocalCodePilot reconhece projetos desta linguagem, mas ainda não os inicia.",
                            )
                            .color(theme::MUTED)
                            .size(12.0),
                        );
                    }
                    if !used_by.is_empty() {
                        ui.label(
                            RichText::new(format!("Nos seus projetos: {}", used_by.join(", ")))
                                .color(theme::MUTED)
                                .size(12.0),
                        );
                    }
                    if details {
                        check_details(ui, language, check);
                    }
                });
            ui.add_space(6.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn statuses_never_claim_a_language_is_absent() {
        let check = |status| LanguageCheck {
            kind: RuntimeKind::Python,
            status,
            searched: vec!["PATH".into()],
        };
        let found = |version| LanguageStatus::Found {
            program: "python",
            path: PathBuf::from("python"),
            outside_path: false,
            version,
        };
        assert_eq!(
            status_text(Some(&check(found(VersionAnswer::Version(
                "Python 3.12.1".into()
            )))))
            .0,
            "Instalado · Python 3.12.1"
        );
        assert_eq!(
            status_text(Some(&check(found(VersionAnswer::TimedOut)))).0,
            "Instalado · Versão indisponível"
        );
        for missing in [
            LanguageStatus::NotFound,
            LanguageStatus::StoreShortcut {
                path: PathBuf::from("python.exe"),
            },
        ] {
            assert_eq!(
                status_text(Some(&check(missing))).0,
                "Não encontrado nos locais verificados"
            );
        }
        assert_eq!(status_text(None).0, "Verificando...");
    }
}
