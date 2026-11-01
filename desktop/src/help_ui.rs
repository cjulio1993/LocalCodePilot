//! Help page. What a junior developer needs comes first, in plain language; technical details
//! come last, in a collapsed block. The content is data so tests can keep it on the approved
//! vocabulary and free of features the app does not have.

use super::*;

enum Block {
    Text(&'static str),
    /// A term in bold followed by its explanation.
    Item(&'static str, &'static str),
}

struct HelpSection {
    title: &'static str,
    blocks: &'static [Block],
    /// Button that opens the screen the section talks about.
    link: Option<(Page, &'static str)>,
}

const INTRO: &str = "Encontre um projeto, confira seu estado e escolha a próxima ação. Para investigar, abra Detalhes ou Ver logs.";

const SECTIONS: &[HelpSection] = &[
    HelpSection {
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
        title: "Se algo não funcionar",
        blocks: &[
            Block::Item(
                "Porta ocupada",
                "outro processo pode estar usando a porta. Veja a porta e o processo em Ver logs antes de alterar a configuração.",
            ),
            Block::Item(
                "Docker ou Podman parado",
                "abra Containers, siga a orientação mostrada para o motor e use Verificar motores e arquivos novamente.",
            ),
            Block::Item(
                "package.json inválido",
                "corrija o arquivo na linha indicada e use Atualizar em Projetos. Se o botão estiver desativado, pare os projetos em execução primeiro.",
            ),
            Block::Item(
                "O site não abre",
                "confira se o projeto está Rodando, o selo do site e o que aparece em Ver logs.",
            ),
        ],
        link: None,
    },
    HelpSection {
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
        title: "Bancos de dados",
        blocks: &[
            Block::Text(
                "O aplicativo identifica algumas configurações de banco, inclusive em projetos WordPress e serviços Compose. Uma porta acessível não confirma que as credenciais funcionam nem que o banco existe.",
            ),
            Block::Text(
                "Confira a configuração do projeto e os logs do serviço de banco. Nomes como db podem funcionar somente dentro da rede Compose.",
            ),
        ],
        link: None,
    },
];

const TECHNICAL_DETAILS: &[Block] = &[
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
];

fn show_blocks(ui: &mut egui::Ui, blocks: &[Block]) {
    for block in blocks {
        match block {
            Block::Text(text) => {
                ui.label(RichText::new(*text).size(13.0));
            }
            Block::Item(term, explanation) => {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new(format!("{term}:")).strong().size(13.0));
                    ui.label(RichText::new(*explanation).size(13.0));
                });
            }
        }
    }
}

impl LocalCodePilot {
    pub(super) fn help_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Ajuda").strong().size(24.0));
        ui.label(RichText::new(INTRO).color(theme::MUTED).size(14.0));
        ui.add_space(16.0);
        let mut open = None;
        for section in SECTIONS {
            Frame::new()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(10)
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(section.title).strong().size(16.0));
                    ui.add_space(6.0);
                    show_blocks(ui, section.blocks);
                    if let Some((page, label)) = section.link {
                        ui.add_space(6.0);
                        if ui.button(label).clicked() {
                            open = Some(page);
                        }
                    }
                });
            ui.add_space(10.0);
        }
        egui::CollapsingHeader::new(RichText::new("Detalhes técnicos").strong().size(15.0))
            .id_salt("help-technical-details")
            .default_open(false)
            .show(ui, |ui| show_blocks(ui, TECHNICAL_DETAILS));
        ui.add_space(16.0);
        if let Some(page) = open {
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
        let mut add = |blocks: &[Block]| {
            for block in blocks {
                match block {
                    Block::Text(line) => text.push_str(line),
                    Block::Item(term, explanation) => {
                        text.push_str(term);
                        text.push_str(explanation);
                    }
                }
            }
        };
        for section in SECTIONS {
            add(section.blocks);
        }
        add(TECHNICAL_DETAILS);
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
            ]
        );
    }

    #[test]
    fn help_does_not_promise_what_the_app_does_not_do() {
        let text = all_text().to_lowercase();
        for promise in ["instala o docker", "instala docker", "em breve", "plugins"] {
            assert!(!text.contains(promise), "promises {promise}");
        }
    }
}
