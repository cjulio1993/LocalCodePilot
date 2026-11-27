//! "LocalCodePilot atualizado para {versão}" (T035): after an update, the news of the new
//! version, once. They are built into the app from `desktop/NOVIDADES.md`, so they always
//! show, with or without internet; a test fails when the app's version has no valid news.
//! The modal can be seen again from Configurações > Atualizações ("Ver novidades").

use super::*;

/// The news of each version, built into the executable.
const NEWS: &str = include_str!("../NOVIDADES.md");
const MAX_ITEMS: usize = 3;
const ITEM_MAX_CHARS: usize = 120;
/// The official page of a release; the version completes it.
const RELEASE_PAGE: &str = "https://github.com/cjulio1993/LocalCodePilot/releases/tag/v";

/// The version this app was built as.
pub(super) fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The news of `version` in the news file: the `- ` lines of its `## {version}` section.
/// Each one is plain text of at most `ITEM_MAX_CHARS` characters, and there are 1 to 3.
pub(super) fn news_for(text: &str, version: &str) -> Result<Vec<String>, String> {
    let heading = format!("## {version}");
    let mut lines = text.lines().map(str::trim);
    if !lines.any(|line| line == heading) {
        return Err(format!("NOVIDADES.md não tem a seção {heading}"));
    }
    let items = lines
        .take_while(|line| !line.starts_with("## "))
        .filter_map(|line| line.strip_prefix("- "))
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if items.is_empty() || items.len() > MAX_ITEMS {
        return Err(format!(
            "{heading}: são {} novidades; o modal mostra de 1 a {MAX_ITEMS}",
            items.len()
        ));
    }
    if let Some(item) = items.iter().find(|item| {
        item.is_empty()
            || item.chars().count() > ITEM_MAX_CHARS
            || item.contains(['[', ']', '<', '>', '*', '_', '`'])
    }) {
        return Err(format!(
            "{heading}: \"{item}\" precisa ser texto simples (sem Markdown) de até {ITEM_MAX_CHARS} caracteres"
        ));
    }
    Ok(items)
}

/// Whether the news show when the app opens:
/// - an update by the app, confirmed by the updater's marker (`updated_by_app`), unless this
///   version was already seen;
/// - a manual update: a version newer than the last one seen, or a state saved by a version
///   that did not record it (Alpha 10 or earlier).
///
/// A clean install, the same version or an older one never shows them. A failed update
/// restores the previous version, which opens without the marker and was already seen: every
/// version with this code records `ultima_versao_vista` on its first start, and a version
/// without the field (Alpha 10 or earlier) has no news modal at all.
pub(super) fn should_show(
    current: &Version,
    last_seen: Option<&str>,
    state_existed: bool,
    updated_by_app: bool,
) -> bool {
    match last_seen.map(Version::parse) {
        Some(Ok(seen)) => current > &seen,
        // No version recorded (Alpha 10 or earlier) or a value no version wrote.
        Some(Err(_)) | None => state_existed || updated_by_app,
    }
}

/// The last version seen after this one opened: never goes back to an older version.
pub(super) fn seen_after(current: &Version, last_seen: Option<&str>) -> String {
    match last_seen.map(Version::parse) {
        Some(Ok(seen)) if seen > *current => seen.to_string(),
        _ => current.to_string(),
    }
}

pub(super) fn release_page(version: &str) -> String {
    format!("{RELEASE_PAGE}{version}")
}

/// The modal while it is open.
#[derive(Debug, Clone)]
pub(super) struct WhatsNew {
    pub version: String,
    pub items: Vec<String>,
    /// The control that opened it ("Ver novidades"), focused again when it closes.
    pub origin: Option<egui::Id>,
    pub focused: bool,
}

impl WhatsNew {
    pub(super) fn current(origin: Option<egui::Id>) -> Self {
        let version = app_version().to_owned();
        Self {
            // The test of this module guarantees the news of the app's version.
            items: news_for(NEWS, &version).unwrap_or_default(),
            version,
            origin,
            focused: false,
        }
    }
}

impl LocalCodePilot {
    pub(super) fn open_whats_new(&mut self, origin: Option<egui::Id>) {
        self.whats_new = Some(WhatsNew::current(origin));
    }

    pub(super) fn show_whats_new(&mut self, root_ui: &mut egui::Ui) {
        let Some(mut news) = self.whats_new.take() else {
            // The focus goes back to "Ver novidades" one frame after the modal closed: while
            // its layer is still the modal one, egui keeps the focus inside it.
            if let Some(origin) = self.whats_new_focus_return.take() {
                root_ui
                    .ctx()
                    .memory_mut(|memory| memory.request_focus(origin));
            }
            return;
        };
        let title = format!("LocalCodePilot atualizado para {}", news.version);
        let mut close = false;
        let mut open_notes = false;
        // A modal layer: the rest of the app takes no input, and Tab stays inside it.
        let modal = egui::Modal::new(egui::Id::unique("whats-new")).show(root_ui.ctx(), |ui| {
            ui.set_max_width(480.0);
            ui.label(RichText::new(&title).strong().size(16.0));
            ui.add_space(8.0);
            for item in &news.items {
                ui.add(egui::Label::new(RichText::new(format!("•  {item}")).size(13.0)).wrap());
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.link("Ver notas completas").clicked() {
                    open_notes = true;
                }
                let close_button =
                    ui.add(egui::Button::new("Fechar").fill(theme::PRIMARY.gamma_multiply(0.72)));
                // The keyboard focus starts inside the modal.
                if !news.focused {
                    close_button.request_focus();
                    news.focused = true;
                }
                if close_button.clicked() {
                    close = true;
                }
            });
        });
        if open_notes {
            root_ui
                .ctx()
                .open_url(egui::OpenUrl::new_tab(release_page(&news.version)));
        }
        // Esc or a click outside closes it too.
        // Screen readers name the modal by its title when the focus enters it.
        announced(modal.response.clone(), egui::Role::Dialog, &title);
        if close || modal.should_close() {
            // Seen: the modal does not come back for this version.
            if let Ok(current) = Version::parse(app_version()) {
                self.workspace_state.last_seen_version = Some(seen_after(
                    &current,
                    self.workspace_state.last_seen_version.as_deref(),
                ));
                self.workspace_dirty = true;
            }
            self.whats_new_focus_return = news.origin;
            root_ui.ctx().request_repaint();
        } else {
            self.whats_new = Some(news);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_version_has_its_news_built_in() {
        let items = news_for(NEWS, app_version()).unwrap_or_else(|error| {
            panic!("{error}. Acrescente as novidades da versão em desktop/NOVIDADES.md.")
        });
        assert!((1..=MAX_ITEMS).contains(&items.len()));
    }

    #[test]
    fn every_version_in_the_news_file_is_valid() {
        for line in NEWS.lines().filter(|line| line.starts_with("## ")) {
            let version = line.trim_start_matches("## ").trim();
            assert!(
                Version::parse(version).is_ok(),
                "versão inválida: {version}"
            );
            news_for(NEWS, version).unwrap();
        }
    }

    #[test]
    fn the_news_of_a_version_are_plain_and_at_most_three() {
        let text = "## 2.0.0\n\n- Um.\n- Dois.\n\n## 1.0.0\n\n- Antigo.\n";
        assert_eq!(news_for(text, "2.0.0").unwrap(), vec!["Um.", "Dois."]);
        assert_eq!(news_for(text, "1.0.0").unwrap(), vec!["Antigo."]);
        assert!(news_for(text, "3.0.0").is_err());
        assert!(news_for("## 1.0.0\n", "1.0.0").is_err());
        assert!(news_for("## 1.0.0\n- a\n- b\n- c\n- d\n", "1.0.0").is_err());
        assert!(news_for("## 1.0.0\n- **negrito**\n", "1.0.0").is_err());
        assert!(news_for("## 1.0.0\n- [link](https://x)\n", "1.0.0").is_err());
        assert!(news_for(&format!("## 1.0.0\n- {}\n", "a".repeat(121)), "1.0.0").is_err());
    }

    #[test]
    fn the_news_show_once_after_an_update_and_never_on_a_clean_install() {
        let current = Version::parse("1.0.0-alpha.1").unwrap();
        let alpha10 = Version::parse("0.4.0-alpha.10").unwrap();
        // Clean install: no saved state, no marker.
        assert!(!should_show(&current, None, false, false));
        // Update by the app, confirmed by the marker: from Alpha 10 (no version recorded)
        // or from a version that recorded it.
        assert!(should_show(&current, None, true, true));
        assert!(should_show(&current, Some("0.4.0-alpha.10"), true, true));
        // Manual update: state of Alpha 10 or earlier, or a newer version than the one seen.
        assert!(should_show(&current, None, true, false));
        assert!(should_show(&current, Some("0.4.0-alpha.10"), true, false));
        // Failed update by the app: the previous version is restored and opens without the
        // marker, with its own version already seen.
        assert!(!should_show(&alpha10, Some("0.4.0-alpha.10"), true, false));
        // Already seen (the same version opens again, or is reinstalled), or an older one.
        assert!(!should_show(&current, Some("1.0.0-alpha.1"), true, false));
        assert!(!should_show(&current, Some("1.0.0-alpha.1"), true, true));
        assert!(!should_show(&alpha10, Some("1.0.0-alpha.1"), true, false));
        // A later version shows its own news once.
        assert!(should_show(
            &Version::parse("1.0.0-alpha.2").unwrap(),
            Some("1.0.0-alpha.1"),
            true,
            false
        ));
        // Closing records the version, never going back to an older one.
        assert_eq!(
            seen_after(&current, Some("0.4.0-alpha.10")),
            "1.0.0-alpha.1"
        );
        assert_eq!(seen_after(&current, Some("2.0.0")), "2.0.0");
        assert_eq!(seen_after(&current, None), "1.0.0-alpha.1");
    }

    #[test]
    fn the_full_notes_open_the_official_release_page() {
        assert_eq!(
            release_page("1.0.0-alpha.1"),
            "https://github.com/cjulio1993/LocalCodePilot/releases/tag/v1.0.0-alpha.1"
        );
    }
}
