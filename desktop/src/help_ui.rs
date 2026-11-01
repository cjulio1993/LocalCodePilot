//! Help page. What a junior developer needs comes first, in plain language; technical details
//! come last, in a collapsed section. A search box and shortcuts lead straight to the answer.
//! The content is data so tests can keep it on the approved vocabulary, free of features the
//! app does not have, and with every shortcut pointing at an existing answer.

use super::*;
use egui::{
    FontId,
    text::{LayoutJob, TextFormat},
};
use egui_phosphor::regular::{CARET_DOWN, CARET_RIGHT, WARNING_CIRCLE};

enum Block {
    Text(&'static str),
    /// A term in bold followed by its explanation.
    Item(&'static str, &'static str),
    /// A block that shortcuts can point to, by a fixed id instead of its text.
    Anchor(&'static str, &'static Block),
}

impl Block {
    fn content(&self) -> (Option<&'static str>, &'static str) {
        match self {
            Self::Text(text) => (None, text),
            Self::Item(term, explanation) => (Some(term), explanation),
            Self::Anchor(_, block) => block.content(),
        }
    }

    fn anchor(&self) -> Option<&'static str> {
        match self {
            Self::Anchor(id, _) => Some(id),
            _ => None,
        }
    }

    fn matches(&self, query: &str) -> bool {
        let (term, text) = self.content();
        term.is_some_and(|term| contains_folded(term, query)) || contains_folded(text, query)
    }
}

struct HelpSection {
    id: &'static str,
    title: &'static str,
    blocks: &'static [Block],
    /// Button that opens the screen the section talks about.
    link: Option<(Page, &'static str)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Section(&'static str),
    Block(&'static str),
}

const INTRO: &str = "Encontre um projeto, confira seu estado e escolha a próxima ação. Para investigar, abra Detalhes ou Ver logs.";

const PROBLEMS: &str = "problemas";

/// Common situations, shown as buttons above the sections.
const SHORTCUTS: &[(&str, Target)] = &[
    ("O site não abre", Target::Block("site-nao-abre")),
    ("Porta ocupada", Target::Block("porta-ocupada")),
    ("Docker ou Podman parado", Target::Block("motor-parado")),
    ("Entender os estados", Target::Section("estados")),
];

const SECTIONS: &[HelpSection] = &[
    HelpSection {
        id: "primeiros-passos",
        title: "Primeiros passos",
        blocks: &[
            Block::Text(
                "Em Projetos, confira as pastas examinadas. Use Adicionar pasta se faltar uma; use Atualizar para refazer a busca.",
            ),
            Block::Text(
                "Busque pelo nome ou filtre por Rodando, Com problema e Parados. No projeto escolhido, siga a ação exibida.",
            ),
            Block::Text(
                "Antes de confirmar uma instalação ou uma ação do Compose, confira o comando e a pasta em que ele será executado.",
            ),
        ],
        link: Some((Page::Projects, "Abrir Projetos")),
    },
    HelpSection {
        id: "estados",
        title: "Entenda os estados",
        blocks: &[
            Block::Item(
                "Rodando",
                "o processo ou os containers estão ativos. Isso ainda não confirma que o site responde.",
            ),
            Block::Item(
                "Parado",
                "nada está em execução para esse projeto. Use Iniciar quando a ação estiver disponível.",
            ),
            Block::Item(
                "Iniciando",
                "o aplicativo está iniciando o projeto ou instalando dependências. Para projetos com containers, acompanhe o progresso em Containers.",
            ),
            Block::Item(
                "Com problema",
                "algo impede ou interrompe a execução. Leia o motivo mostrado no projeto e abra Detalhes para investigar.",
            ),
            Block::Text("Ao lado do endereço, um selo mostra o resultado da verificação do site:"),
            Block::Item("Site respondendo", "o endereço respondeu à verificação."),
            Block::Item(
                "Servidor respondeu; página não encontrada",
                "o servidor está ativo, mas a página não foi encontrada nesse endereço.",
            ),
            Block::Item(
                "Acesso não disponível",
                "o servidor respondeu, mas a página não está acessível.",
            ),
            Block::Item(
                "Site com erro ou Site sem resposta",
                "use Ver logs para investigar.",
            ),
            Block::Item("Sem selo", "o endereço ainda não foi verificado."),
            Block::Item(
                "Endereço provável",
                "em projetos com containers, o endereço vem de uma porta publicada e pode não ser um site.",
            ),
        ],
        link: None,
    },
    HelpSection {
        id: PROBLEMS,
        title: "Se algo não funcionar",
        blocks: &[
            Block::Anchor(
                "porta-ocupada",
                &Block::Item(
                    "Porta ocupada",
                    "outro processo pode estar usando a porta. Veja a porta e o processo em Ver logs antes de alterar a configuração.",
                ),
            ),
            Block::Anchor(
                "motor-parado",
                &Block::Item(
                    "Docker ou Podman parado",
                    "abra Containers, siga a orientação mostrada para o motor e use Verificar motores e arquivos novamente.",
                ),
            ),
            Block::Item(
                "package.json inválido",
                "corrija o arquivo na linha indicada e use Atualizar em Projetos. Se o botão estiver desativado, pare os projetos em execução primeiro.",
            ),
            Block::Anchor(
                "site-nao-abre",
                &Block::Item(
                    "O site não abre",
                    "confira se o projeto está Rodando, o selo do site e o que aparece em Ver logs.",
                ),
            ),
        ],
        link: None,
    },
    HelpSection {
        id: "site-e-logs",
        title: "Abrir site e ver logs",
        blocks: &[
            Block::Text(
                "Use Abrir site quando o projeto mostrar um endereço. Se a página não abrir, confira o estado, o endereço e a porta, depois use Ver logs.",
            ),
            Block::Text(
                "Ver logs leva a Processos, para projetos que rodam direto na máquina, ou a Containers, para projetos com Docker ou Podman.",
            ),
        ],
        link: Some((Page::Overview, "Abrir Visão geral")),
    },
    HelpSection {
        id: "containers",
        title: "Containers e Compose",
        blocks: &[
            Block::Text(
                "Docker e Podman precisam estar instalados e com o motor ativo. O LocalCodePilot mostra os passos e os comandos para preparar o motor, mas não instala nada nem inicia a máquina virtual por você.",
            ),
            Block::Text(
                "Em Containers, escolha o motor, confira os serviços e o comando antes de confirmar. Um container rodando ou uma porta publicada não prova que o site está respondendo.",
            ),
            Block::Text(
                "Dois projetos com o mesmo nome de ambiente Compose, por exemplo duas pastas docker/ com compose.yaml, ficam bloqueados para iniciar e parar até receberem nomes diferentes. Isso evita que um projeto altere os containers e os dados do outro.",
            ),
        ],
        link: Some((Page::Containers, "Abrir Containers")),
    },
    HelpSection {
        id: "bancos",
        title: "Bancos de dados",
        blocks: &[
            Block::Text(
                "O aplicativo identifica algumas configurações de banco, inclusive em projetos PHP e serviços Compose. Uma porta acessível não confirma que as credenciais funcionam nem que o banco existe.",
            ),
            Block::Text(
                "Confira a configuração do projeto e os logs do serviço de banco. Nomes como db podem funcionar somente dentro da rede Compose.",
            ),
        ],
        link: None,
    },
    HelpSection {
        id: "tecnico",
        title: "Detalhes técnicos",
        blocks: &[
            Block::Item(
                "Detalhes do projeto",
                "pasta do projeto, comandos detectados e diagnóstico completo.",
            ),
            Block::Item(
                "Processos",
                "portas, resposta HTTP, PID e logs dos processos que rodam direto na máquina.",
            ),
            Block::Item(
                "Containers",
                "motores, serviços, portas publicadas, logs e os comandos Compose propostos.",
            ),
            Block::Text("Revise o comando e o alcance de cada ação antes de confirmar."),
        ],
        link: None,
    },
];

/// How long a shortcut's answer stays highlighted.
const HIGHLIGHT_SECONDS: f64 = 2.0;

/// Help view state for this session only: nothing here is saved.
pub(super) struct HelpState {
    query: String,
    /// Sections opened by the user. A search shows its results without changing this, so
    /// clearing the search restores the page as it was.
    open: Vec<bool>,
    /// Answer to highlight, and when it was chosen.
    target: Option<(Target, f64)>,
    /// Scroll to the target on the next frame it is drawn.
    scroll_pending: bool,
}

impl Default for HelpState {
    fn default() -> Self {
        Self {
            query: String::new(),
            open: SECTIONS
                .iter()
                .map(|section| section.id == "primeiros-passos")
                .collect(),
            target: None,
            scroll_pending: false,
        }
    }
}

impl HelpState {
    fn go_to(&mut self, target: Target, now: f64) {
        self.query.clear();
        if let Some(index) = section_of(target) {
            self.open[index] = true;
        }
        self.target = Some((target, now));
        self.scroll_pending = true;
    }
}

fn section_of(target: Target) -> Option<usize> {
    SECTIONS.iter().position(|section| match target {
        Target::Section(id) => section.id == id,
        Target::Block(id) => section
            .blocks
            .iter()
            .any(|block| block.anchor() == Some(id)),
    })
}

/// Lowercase without accents, one char for one char, so match positions map back to the
/// original text.
fn fold(c: char) -> char {
    match c.to_lowercase().next().unwrap_or(c) {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        other => other,
    }
}

fn folded(text: &str) -> Vec<char> {
    text.chars().map(fold).collect()
}

/// Char ranges of `query` in `text`, ignoring case and accents.
fn match_ranges(text: &str, query: &[char]) -> Vec<(usize, usize)> {
    let text = folded(text);
    let mut ranges = Vec::new();
    if query.is_empty() {
        return ranges;
    }
    let mut start = 0;
    while start + query.len() <= text.len() {
        if text[start..start + query.len()] == *query {
            ranges.push((start, start + query.len()));
            start += query.len();
        } else {
            start += 1;
        }
    }
    ranges
}

fn contains_folded(text: &str, query: &str) -> bool {
    !match_ranges(text, &folded(query)).is_empty()
}

/// Blocks to show for a search: the whole section when its title matches, otherwise only the
/// matching blocks.
fn matching_blocks(section: &HelpSection, query: &str) -> Vec<usize> {
    if contains_folded(section.title, query) {
        return (0..section.blocks.len()).collect();
    }
    section
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| block.matches(query))
        .map(|(index, _)| index)
        .collect()
}

/// Appends `text`, marking every match of `query` with a background.
fn append(job: &mut LayoutJob, text: &str, query: &[char], color: Color32, mark: Color32) {
    let format = TextFormat::simple(FontId::proportional(13.0), color);
    let starts = text.char_indices().map(|(at, _)| at).collect::<Vec<_>>();
    let byte = |index: usize| starts.get(index).copied().unwrap_or(text.len());
    let mut last = 0;
    for (start, end) in match_ranges(text, query) {
        job.append(&text[byte(last)..byte(start)], 0.0, format.clone());
        job.append(
            &text[byte(start)..byte(end)],
            0.0,
            TextFormat {
                background: mark,
                ..format.clone()
            },
        );
        last = end;
    }
    job.append(&text[byte(last)..], 0.0, format);
}

fn block_label(ui: &mut egui::Ui, block: &Block, query: &[char]) -> egui::Response {
    let mark = theme::WARNING.gamma_multiply(0.45);
    let text_color = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let (term, text) = block.content();
    if let Some(term) = term {
        append(
            &mut job,
            term,
            query,
            ui.visuals().strong_text_color(),
            mark,
        );
        append(&mut job, ": ", &[], text_color, mark);
    }
    append(&mut job, text, query, text_color, mark);
    ui.label(job)
}

fn problem_count_text(count: usize) -> String {
    if count == 1 {
        "1 projeto com problema".into()
    } else {
        format!("{count} projetos com problema")
    }
}

impl LocalCodePilot {
    pub(super) fn help_page(&mut self, ui: &mut egui::Ui) {
        let now = ui.input(|input| input.time);
        ui.label(RichText::new("Ajuda").strong().size(24.0));
        ui.label(RichText::new(INTRO).color(theme::MUTED).size(14.0));
        ui.add_space(14.0);

        let mut open_page = None;
        let problems = self
            .projects
            .iter()
            .filter(|project| {
                self.project_summary(project).state == project_ui::ProjectState::Problem
            })
            .count();
        if problems > 0 {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{WARNING_CIRCLE}  {}",
                        problem_count_text(problems)
                    ))
                    .color(theme::WARNING)
                    .size(13.0),
                );
                if ui.button("Ver na Visão geral").clicked() {
                    open_page = Some(Page::Overview);
                }
            });
            ui.add_space(10.0);
        }

        let state = &mut self.help;
        let question = ui.label(RichText::new("Qual é a sua dúvida?").strong().size(15.0));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text("Ex.: porta, logs, Docker")
                    .desired_width(ui.available_width().min(420.0)),
            )
            .labelled_by(question.id);
            if !state.query.is_empty() && ui.button("Limpar").clicked() {
                state.query.clear();
            }
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (label, target) in SHORTCUTS {
                if ui.button(*label).clicked() {
                    state.go_to(*target, now);
                }
            }
        });
        ui.add_space(14.0);

        let query = state.query.trim().to_owned();
        let folded_query = folded(&query);
        let highlight = state
            .target
            .filter(|(_, since)| now - since < HIGHLIGHT_SECONDS)
            .map(|(target, since)| (target, 1.0 - ((now - since) / HIGHLIGHT_SECONDS) as f32));
        if highlight.is_some() {
            ui.ctx().request_repaint();
        }
        let mut found = false;
        for (index, section) in SECTIONS.iter().enumerate() {
            let shown = if query.is_empty() {
                None
            } else {
                let blocks = matching_blocks(section, &query);
                if blocks.is_empty() {
                    continue;
                }
                Some(blocks)
            };
            found = true;
            let open = shown.is_some() || state.open[index];
            let (header, target) = Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(10)
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let icon = if open { CARET_DOWN } else { CARET_RIGHT };
                    let header = ui.add(
                        egui::Button::new(
                            RichText::new(format!("{icon}  {}", section.title))
                                .strong()
                                .size(16.0),
                        )
                        .frame(false),
                    );
                    // While searching, results stay open; the user's choices return afterwards.
                    if header.clicked() && shown.is_none() {
                        state.open[index] = !state.open[index];
                    }
                    let mut target = highlight
                        .is_some_and(|(target, _)| target == Target::Section(section.id))
                        .then_some(header.rect);
                    if !open {
                        return (header.rect, target);
                    }
                    ui.add_space(6.0);
                    for (block_index, block) in section.blocks.iter().enumerate() {
                        if shown
                            .as_ref()
                            .is_some_and(|blocks| !blocks.contains(&block_index))
                        {
                            continue;
                        }
                        let strength = highlight.and_then(|(target, strength)| {
                            (block.anchor().map(Target::Block) == Some(target)).then_some(strength)
                        });
                        let fill = strength.map_or(Color32::TRANSPARENT, |strength| {
                            theme::PRIMARY.gamma_multiply(0.30 * strength)
                        });
                        let response = Frame::new()
                            .fill(fill)
                            .corner_radius(6)
                            .inner_margin(Margin::symmetric(6, 3))
                            .show(ui, |ui| block_label(ui, block, &folded_query))
                            .response;
                        if strength.is_some() {
                            target = Some(response.rect);
                        }
                    }
                    if let Some((page, label)) = section.link {
                        ui.add_space(6.0);
                        if ui.button(label).clicked() {
                            open_page = Some(page);
                        }
                    }
                    (header.rect, target)
                })
                .inner;
            if let (true, Some(target)) = (state.scroll_pending, target) {
                // The title stays in view with the answer when both fit; otherwise the answer
                // comes first.
                let both = header.union(target);
                if both.height() <= ui.clip_rect().height() - 24.0 {
                    ui.scroll_to_rect(both, Some(egui::Align::TOP));
                } else {
                    ui.scroll_to_rect(target, Some(egui::Align::Center));
                }
                state.scroll_pending = false;
            }
            ui.add_space(10.0);
        }
        if !found {
            ui.label(
                RichText::new(format!(
                    "Nenhuma resposta para “{query}”. Tente outra palavra ou veja Se algo não funcionar."
                ))
                .size(13.0),
            );
            ui.add_space(4.0);
            if ui.button("Ver problemas comuns").clicked() {
                state.go_to(Target::Section(PROBLEMS), now);
            }
        }
        ui.add_space(16.0);
        if let Some(page) = open_page {
            if matches!(page, Page::Processes | Page::Containers) {
                self.focused_project = None;
            }
            self.page = page;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_text() -> String {
        let mut text = INTRO.to_owned();
        for section in SECTIONS {
            for block in section.blocks {
                let (term, explanation) = block.content();
                text.push_str(term.unwrap_or_default());
                text.push_str(explanation);
            }
        }
        text
    }

    #[test]
    fn help_uses_the_approved_vocabulary() {
        let text = all_text();
        for term in [
            "Rodando",
            "Parado",
            "Iniciando",
            "Com problema",
            "Iniciar",
            "Abrir site",
            "Ver logs",
            "Detalhes",
        ] {
            assert!(text.contains(term), "missing {term}");
        }
        // Names from the previous interface must not come back.
        for retired in [
            "Abrir aplicação",
            "Iniciar ambiente",
            "Ver diagnóstico e logs",
            "Aplicação respondendo",
            "Mais ações",
            "Site não verificado",
            "Corrigir",
        ] {
            assert!(!text.contains(retired), "retired term {retired}");
        }
    }

    #[test]
    fn juniors_find_states_and_problems_before_technical_sections() {
        let titles = SECTIONS
            .iter()
            .map(|section| section.title)
            .collect::<Vec<_>>();
        assert_eq!(
            titles,
            [
                "Primeiros passos",
                "Entenda os estados",
                "Se algo não funcionar",
                "Abrir site e ver logs",
                "Containers e Compose",
                "Bancos de dados",
                "Detalhes técnicos",
            ]
        );
        // Only the first steps are open at first; the rest is one click away.
        assert_eq!(
            HelpState::default().open,
            [true, false, false, false, false, false, false]
        );
    }

    #[test]
    fn help_does_not_promise_what_the_app_does_not_do() {
        let text = all_text().to_lowercase();
        for promise in ["instala o docker", "instala docker", "em breve", "plugins"] {
            assert!(!text.contains(promise), "promises {promise}");
        }
    }

    #[test]
    fn every_shortcut_leads_to_an_existing_answer() {
        assert!(SHORTCUTS.len() <= 4, "more shortcuts become noise");
        for (label, target) in SHORTCUTS {
            assert!(section_of(*target).is_some(), "{label} points nowhere");
        }
        assert!(section_of(Target::Section(PROBLEMS)).is_some());
        let mut anchors = SECTIONS
            .iter()
            .flat_map(|section| section.blocks.iter().filter_map(Block::anchor))
            .collect::<Vec<_>>();
        let total = anchors.len();
        anchors.sort_unstable();
        anchors.dedup();
        assert_eq!(anchors.len(), total, "anchor ids must be unique");
    }

    #[test]
    fn a_shortcut_opens_its_section_and_clears_the_search() {
        let mut state = HelpState {
            query: "docker".into(),
            ..HelpState::default()
        };
        state.go_to(Target::Block("porta-ocupada"), 1.0);
        assert!(state.query.is_empty());
        assert!(state.open[section_of(Target::Block("porta-ocupada")).unwrap()]);
        assert!(state.scroll_pending);
    }

    fn sections_found(query: &str) -> Vec<&'static str> {
        SECTIONS
            .iter()
            .filter(|section| !matching_blocks(section, query).is_empty())
            .map(|section| section.id)
            .collect()
    }

    #[test]
    fn search_ignores_accents_and_case() {
        assert_eq!(sections_found("pagina"), sections_found("Página"));
        assert!(sections_found("pagina").contains(&"estados"));
        assert!(sections_found("DOCKER").contains(&"containers"));
        assert!(contains_folded("Configurações", "configuracoes"));
    }

    #[test]
    fn search_shows_only_the_matching_answers() {
        let problems = &SECTIONS[section_of(Target::Section(PROBLEMS)).unwrap()];
        assert_eq!(matching_blocks(problems, "porta").len(), 1);
        // A title match keeps the whole section.
        assert_eq!(
            matching_blocks(problems, "algo não funcionar").len(),
            problems.blocks.len()
        );
    }

    #[test]
    fn experts_find_technical_terms() {
        assert_eq!(sections_found("PID"), ["tecnico"]);
    }

    #[test]
    fn a_search_without_answers_finds_nothing() {
        assert!(sections_found("xyzzy").is_empty());
    }

    #[test]
    fn highlights_map_back_to_the_original_text() {
        let text = "Servidor respondeu; página não encontrada";
        let ranges = match_ranges(text, &folded("PAGINA"));
        assert_eq!(ranges.len(), 1);
        let (start, end) = ranges[0];
        let found = text
            .chars()
            .skip(start)
            .take(end - start)
            .collect::<String>();
        assert_eq!(found, "página");
    }

    #[test]
    fn problem_count_is_plural_aware() {
        assert_eq!(problem_count_text(1), "1 projeto com problema");
        assert_eq!(problem_count_text(2), "2 projetos com problema");
    }
}
