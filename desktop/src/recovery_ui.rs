//! The notice shown when the configuration files could not be read (T021): at the top of the
//! window until closed, and in Configurações until the person clicks "Entendi".

use super::*;
use crate::{config::ConfigRecovery, storage};

pub(super) const RESTORED_SENTENCE: &str =
    "Não foi possível ler seus projetos e configurações. Restauramos a última cópia salva.";
pub(super) const LOST_SENTENCE: &str = "Não foi possível carregar seus projetos e configurações. \
     Guardamos os arquivos para recuperação. Alguns dados podem estar ausentes.";
/// What the project scan does not bring back (CA-6).
const NOT_RECOVERED: &str = "A busca de projetos refaz só a lista de projetos. Favoritos, \
     histórico, preparos, comandos personalizados, comandos confirmados, volumes confirmados \
     e preferências de tela podem precisar ser refeitos.";

pub(super) fn sentence(recovery: &ConfigRecovery) -> &'static str {
    if recovery.restored {
        RESTORED_SENTENCE
    } else {
        LOST_SENTENCE
    }
}

/// The technical details: causes and paths, never the content of the files.
pub(super) fn detail_lines(recovery: &ConfigRecovery) -> Vec<String> {
    let mut lines = recovery
        .causes
        .iter()
        .map(|cause| format!("Causa: {cause}"))
        .collect::<Vec<_>>();
    lines.extend(
        recovery
            .preserved
            .iter()
            .map(|path| format!("Arquivo guardado: {}", display_path(path))),
    );
    lines.extend(
        recovery
            .restored_from
            .iter()
            .map(|path| format!("Cópia restaurada: {}", display_path(path))),
    );
    if !recovery.restored {
        lines.push(NOT_RECOVERED.to_owned());
    }
    lines
}

impl LocalCodePilot {
    /// The notice at the top of the window, until it is closed in this session.
    pub(super) fn config_recovery_banner(&mut self, ui: &mut egui::Ui) {
        if !self.config_recovery_notice {
            return;
        }
        let Some(recovery) = self.workspace_state.config_recovery.clone() else {
            return;
        };
        let response = self.recovery_notice(
            ui,
            &recovery,
            egui::Id::unique("config-recovery-top"),
            "Fechar",
        );
        if response.dismiss {
            self.config_recovery_notice = false;
        }
        ui.add_space(12.0);
    }

    /// The section of Configurações, while there is a recovery the person has not dismissed.
    pub(super) fn config_recovery_section(&mut self, ui: &mut egui::Ui) {
        let Some(recovery) = self.workspace_state.config_recovery.clone() else {
            return;
        };
        ui.heading("Recuperação de configurações");
        ui.add_space(12.0);
        let response = self.recovery_notice(
            ui,
            &recovery,
            egui::Id::unique("config-recovery-settings"),
            "Entendi",
        );
        if response.dismiss {
            self.workspace_state.config_recovery = None;
            self.config_recovery_notice = false;
            self.workspace_dirty = true;
            self.persist_workspace_state(true);
        }
        ui.add_space(24.0);
    }

    fn recovery_notice(
        &mut self,
        ui: &mut egui::Ui,
        recovery: &ConfigRecovery,
        id: egui::Id,
        dismiss: &str,
    ) -> notice::NoticeResponse {
        let action = if recovery.restored {
            "Detalhes"
        } else {
            "Abrir pasta"
        };
        let lines = detail_lines(recovery);
        let mut details = |ui: &mut egui::Ui| {
            for line in &lines {
                notice::detail_line(ui, line);
            }
        };
        let response = notice::Notice::new(id, sentence(recovery))
            .action(action, true)
            .dismiss(dismiss)
            .show_response(
                ui,
                notice::NoticeParts {
                    resolve: None,
                    details: Some(&mut details),
                },
            );
        if response.action {
            if recovery.restored {
                notice::open(ui.ctx(), id.with("details"));
            } else if let Some(folder) = config::config_directory()
                && let Err(error) = storage::open_folder(&folder)
            {
                self.notify(format!("Não foi possível abrir a pasta: {error}"));
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recovery(restored: bool) -> ConfigRecovery {
        ConfigRecovery {
            restored,
            at: 1,
            causes: vec!["workspace-state.json: JSON inválido (linha 1, coluna 2)".into()],
            preserved: vec![PathBuf::from(
                "pasta/workspace-state.json.ilegivel-20261125-010101",
            )],
            restored_from: restored
                .then(|| PathBuf::from("pasta/workspace-state.json.bak"))
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn the_restored_notice_says_the_copy_was_restored() {
        let restored = recovery(true);
        assert_eq!(sentence(&restored), RESTORED_SENTENCE);
        let lines = detail_lines(&restored).join("\n");
        assert!(lines.contains("Cópia restaurada"));
        assert!(lines.contains("Arquivo guardado"));
        assert!(lines.contains("JSON inválido"));
    }

    #[test]
    fn without_a_copy_the_notice_never_promises_the_lost_data_back() {
        let lost = recovery(false);
        assert_eq!(sentence(&lost), LOST_SENTENCE);
        let lines = detail_lines(&lost).join("\n");
        assert!(!lines.contains("Cópia restaurada"));
        assert!(lines.contains("podem precisar ser refeitos"));
        for text in [LOST_SENTENCE, lines.as_str()] {
            assert!(!text.contains("recuperamos"));
            assert!(!text.contains("foram recuperad"));
        }
    }
}
