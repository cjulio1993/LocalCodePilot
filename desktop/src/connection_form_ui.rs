//! "Alterar dados de conexão" (T040): the database name, the user and the password of a
//! project, written in the project's own `.env`, `.env.local` or `wp-config.php`, the file the
//! project and the app read them from. One dialog: the fields, then what will change and the
//! confirmation. The password is never shown; empty keeps the current one. A file tracked by
//! Git never receives a password. A private copy of the file is kept outside the project, the
//! connection is tested again, and "Desfazer a alteração" restores the copy only if nobody
//! changed the file since.

use super::*;
use localcodepilot_runtime::connection_edit::{
    ConnectionField, ConnectionKey, FieldRefusal, connection_fields, write_connection_fields,
};
use localcodepilot_runtime::databases::{DatabaseEngine, DatabaseRequirement};

/// The dialog while it is open.
pub(super) struct ConnectionForm {
    project: PathBuf,
    requirement: DatabaseRequirement,
    fields: Vec<ConnectionField>,
    database: String,
    user: String,
    password: String,
    /// The file is tracked by Git: no password is written there.
    tracked: bool,
    focus_set: bool,
}

/// A saved change, for "Desfazer a alteração".
pub(super) struct CredentialChange {
    file: PathBuf,
    backup: PathBuf,
    /// SHA-256 of what the app wrote: restored only if the file is still exactly that.
    written: String,
}

/// The answer that "Testar conexão de novo" after saving shows (T040, CA-7).
fn result_sentence(test: Option<&ConnectionTest>) -> &'static str {
    match test.map(|test| &test.outcome) {
        Some(ConnectionOutcome::Connected) => "Dados salvos. A conexão com o banco funcionou.",
        Some(ConnectionOutcome::AccessDenied) => {
            "Dados salvos, mas o banco ainda recusou a conexão. Confira os detalhes técnicos."
        }
        // Not a success, and not a refused login either: the details say what happened.
        _ => {
            "Dados salvos. O teste de conexão não confirmou o acesso; confira os detalhes técnicos."
        }
    }
}

/// Whether `file` is tracked by Git in its repository. When the folder is in a repository
/// and Git cannot answer, the answer is yes: a password is never written blindly.
fn tracked_by_git(file: &Path) -> bool {
    let Some(folder) = file.parent() else {
        return false;
    };
    let in_repository = folder
        .ancestors()
        .any(|ancestor| ancestor.join(".git").exists());
    if !in_repository {
        return false;
    }
    let mut command = std::process::Command::new("git");
    command
        .arg("ls-files")
        .arg("--error-unmatch")
        .arg("--")
        .arg(file.file_name().unwrap_or_default())
        .current_dir(folder)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    match command.status() {
        Ok(status) => status.success(),
        Err(_) => true,
    }
}

impl LocalCodePilot {
    /// The database of the project whose access data the app can edit: one with a supported
    /// file and at least one field that can be changed safely.
    pub(super) fn connection_form_target(
        &self,
        project: &Path,
    ) -> Option<(DatabaseRequirement, Vec<ConnectionField>)> {
        self.project_environments
            .get(project)?
            .databases
            .iter()
            .filter(|status| {
                status.requirement.engine != DatabaseEngine::Sqlite
                    && status.requirement.compose_service.is_none()
            })
            .find_map(|status| {
                let fields = connection_fields(&status.requirement.source)?;
                fields
                    .iter()
                    .any(|field| field.refusal.is_none())
                    .then(|| (status.requirement.clone(), fields))
            })
    }

    /// The failures the form can help with (CA-1): a refused login, a database that does
    /// not exist, or access data that is missing.
    pub(super) fn connection_form_fits_failure(&self, project: &Path) -> bool {
        self.project_environments
            .get(project)
            .is_some_and(|environment| {
                environment.databases.iter().any(|status| {
                    matches!(
                        status.connection.as_ref().map(|test| &test.outcome),
                        Some(
                            ConnectionOutcome::AccessDenied
                                | ConnectionOutcome::DatabaseMissing
                                | ConnectionOutcome::InsufficientData
                        )
                    )
                })
            })
    }

    pub(super) fn open_connection_form(&mut self, project: &Path) {
        let Some((requirement, fields)) = self.connection_form_target(project) else {
            return;
        };
        let value = |key| {
            fields
                .iter()
                .find(|field| field.key == key)
                .and_then(|field| field.value.clone())
                .unwrap_or_default()
        };
        let tracked = tracked_by_git(&requirement.source);
        self.connection_form = Some(ConnectionForm {
            project: project.to_path_buf(),
            database: value(ConnectionKey::Database),
            user: value(ConnectionKey::User),
            password: String::new(),
            tracked,
            fields,
            requirement,
            focus_set: false,
        });
    }

    /// The changes the form would write: the fields that differ from the file, and the
    /// password when one was typed and the file may receive it.
    fn form_changes(form: &ConnectionForm) -> Vec<(ConnectionKey, String)> {
        let mut changes = Vec::new();
        for field in form.fields.iter().filter(|field| field.refusal.is_none()) {
            match field.key {
                ConnectionKey::Database if Some(&form.database) != field.value.as_ref() => {
                    changes.push((field.key, form.database.clone()));
                }
                ConnectionKey::User if Some(&form.user) != field.value.as_ref() => {
                    changes.push((field.key, form.user.clone()));
                }
                ConnectionKey::Password if !form.password.is_empty() && !form.tracked => {
                    changes.push((field.key, form.password.clone()));
                }
                _ => {}
            }
        }
        changes
    }

    pub(super) fn show_connection_form(&mut self, ctx: &egui::Context) {
        let Some(form) = self.connection_form.as_mut() else {
            return;
        };
        let file_name = form
            .requirement
            .source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut cancel = false;
        let mut confirm = false;
        let modal = egui::Modal::new(egui::Id::unique("connection-form")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(260.0, 520.0));
            ui.label(
                RichText::new("Alterar a conexão do banco?")
                    .strong()
                    .size(theme::TEXT_TITLE),
            );
            ui.add_space(8.0);
            let fields = form.fields.clone();
            for field in &fields {
                let (label, text) = match field.key {
                    ConnectionKey::Database => ("Nome do banco", &mut form.database),
                    ConnectionKey::User => ("Usuário", &mut form.user),
                    ConnectionKey::Password => ("Nova senha", &mut form.password),
                };
                let password = field.key == ConnectionKey::Password;
                let blocked_by_git = password && form.tracked;
                let editable = field.refusal.is_none() && !blocked_by_git;
                ui.label(RichText::new(label).size(theme::TEXT_BODY));
                let edit = ui.add_enabled(
                    editable,
                    egui::TextEdit::singleline(text)
                        .password(password)
                        .desired_width(f32::INFINITY),
                );
                // The field's name for screen readers; never its content when it is the password.
                edit.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::TextInput, editable, label)
                });
                if editable && !form.focus_set {
                    edit.request_focus();
                    form.focus_set = true;
                }
                let note = match (field.refusal, blocked_by_git) {
                    (Some(FieldRefusal::Variable), _) => Some(
                        "Este valor vem de uma variável. O LocalCodePilot não consegue alterá-lo aqui."
                            .to_owned(),
                    ),
                    (Some(FieldRefusal::Unsafe), _) => Some(format!(
                        "Este valor é definido de um jeito que o LocalCodePilot não consegue alterar com segurança. Edite {file_name} manualmente."
                    )),
                    (None, true) => Some(
                        "Este arquivo está no Git. Salvar uma senha nele pode enviá-la ao repositório. A senha não será alterada aqui."
                            .to_owned(),
                    ),
                    (None, false) if password => {
                        Some("Deixe em branco para manter a senha atual.".to_owned())
                    }
                    _ => None,
                };
                if let Some(note) = note {
                    ui.add(
                        egui::Label::new(
                            RichText::new(note)
                                .color(if editable { theme::MUTED } else { theme::WARNING })
                                .size(theme::TEXT_DETAIL),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(6.0);
            }
            let changes = Self::form_changes(form);
            let names = changes
                .iter()
                .filter_map(|(key, _)| form.fields.iter().find(|field| field.key == *key))
                .map(|field| field.name)
                .collect::<Vec<_>>();
            if !names.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "O LocalCodePilot vai alterar {} em {file_name}, guardar uma cópia da configuração anterior e testar a conexão novamente.",
                            names.join(", ")
                        ))
                        .color(theme::WARNING)
                        .size(theme::TEXT_BODY),
                    )
                    .wrap(),
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancelar").clicked() {
                    cancel = true;
                }
                let button = egui::Button::new(
                    RichText::new("Alterar e testar").color(Color32::WHITE),
                )
                .fill(theme::PRIMARY_FILL);
                if ui.add_enabled(!names.is_empty(), button).clicked() {
                    confirm = true;
                }
            });
        });
        announced(
            modal.response.clone(),
            egui::Role::Dialog,
            "Alterar a conexão do banco?",
        );
        if modal.should_close() {
            cancel = true;
        }
        if confirm {
            if let Some(form) = self.connection_form.take() {
                self.save_connection(form);
            }
        } else if cancel {
            // Nothing is written and no copy is made.
            self.connection_form = None;
        }
    }

    fn save_connection(&mut self, form: ConnectionForm) {
        let changes = Self::form_changes(&form);
        let file = form.requirement.source.clone();
        let result = (|| -> Result<CredentialChange, String> {
            let original = fs::read(&file).map_err(|error| {
                format!("não foi possível ler {}: {error}", display_path(&file))
            })?;
            let text = String::from_utf8(original.clone())
                .map_err(|_| "o arquivo não está em UTF-8".to_owned())?;
            let written = write_connection_fields(&text, &file, &changes)?;
            let backup = database_ui::keep_private_copy(&file, &original)?;
            fs::write(&file, written.as_bytes()).map_err(|error| {
                format!("não foi possível alterar {}: {error}", display_path(&file))
            })?;
            Ok(CredentialChange {
                file: file.clone(),
                backup,
                written: database_ui::sha256_hex(written.as_bytes()),
            })
        })();
        match result {
            Ok(change) => {
                self.credential_changes.insert(form.project.clone(), change);
                self.credential_retest.insert(form.project.clone());
                let requirement = form.requirement.clone();
                self.retest_requirement(&form.project, requirement);
            }
            Err(why) => {
                self.notify_important(format!("Os dados de conexão não foram salvos: {why}"))
            }
        }
    }

    /// After the test that follows a save, the result in words (CA-7).
    pub(super) fn credential_test_finished(
        &mut self,
        project: &Path,
        test: Option<&ConnectionTest>,
    ) {
        if self.credential_retest.remove(project) {
            self.notify_important(result_sentence(test));
        }
    }

    /// The change saved for this project, if "Desfazer a alteração" can be offered.
    pub(super) fn credential_change_file(&self, project: &Path) -> Option<String> {
        self.credential_changes.get(project).map(|change| {
            change
                .file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
    }

    /// "Desfazer a alteração": the copy goes back only if the file is still what the app wrote.
    pub(super) fn undo_connection_change(&mut self, project: &Path) {
        let Some(change) = self.credential_changes.get(project) else {
            return;
        };
        match database_ui::restore_if_unchanged(&change.file, &change.backup, &change.written) {
            Ok(()) => {
                self.credential_changes.remove(project);
                self.notify("Configuração anterior restaurada.");
                self.refresh_project_environment(project);
            }
            Err(database_ui::RestoreRefusal::Changed) => {
                let folder = change.backup.parent().map(display_path).unwrap_or_default();
                self.notify_important(format!(
                    "O arquivo mudou depois da alteração. Para não perder o que foi editado, a configuração anterior não foi restaurada. A cópia continua em {folder}."
                ));
                self.credential_changes.remove(project);
            }
            Err(database_ui::RestoreRefusal::Failed(why)) => self.notify_important(why),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_never_claims_a_connection_it_did_not_see() {
        let test = |outcome| ConnectionTest {
            outcome,
            target: "127.0.0.1:3306".into(),
            user: Some("app".into()),
            database: Some("loja".into()),
            insufficient: vec![],
            at: std::time::SystemTime::now(),
        };
        assert_eq!(
            result_sentence(Some(&test(ConnectionOutcome::Connected))),
            "Dados salvos. A conexão com o banco funcionou."
        );
        assert_eq!(
            result_sentence(Some(&test(ConnectionOutcome::AccessDenied))),
            "Dados salvos, mas o banco ainda recusou a conexão. Confira os detalhes técnicos."
        );
        for outcome in [
            ConnectionOutcome::DatabaseMissing,
            ConnectionOutcome::NotResponding,
            ConnectionOutcome::TimedOut,
        ] {
            assert!(!result_sentence(Some(&test(outcome))).contains("funcionou"));
        }
        assert!(!result_sentence(None).contains("funcionou"));
    }

    #[test]
    fn a_file_outside_a_repository_is_not_tracked() {
        let folder = std::env::temp_dir().join(format!("lcp-t040-git-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join(".env");
        std::fs::write(&file, "DB_PASSWORD=x\n").unwrap();
        // The temp folder is not inside a repository on the machines that run the tests.
        if !folder
            .ancestors()
            .any(|ancestor| ancestor.join(".git").exists())
        {
            assert!(!tracked_by_git(&file));
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_tracked_file_blocks_the_password() {
        // A throwaway repository with the .env committed.
        let folder = std::env::temp_dir().join(format!("lcp-t040-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&folder)
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        };
        if !git(&["init", "-q"]) {
            return; // No Git on this machine: nothing to check.
        }
        std::fs::write(folder.join(".env"), "DB_PASSWORD=x\n").unwrap();
        std::fs::write(folder.join(".env.local"), "DB_PASSWORD=y\n").unwrap();
        assert!(git(&["add", ".env"]));
        assert!(tracked_by_git(&folder.join(".env")));
        assert!(!tracked_by_git(&folder.join(".env.local")));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
