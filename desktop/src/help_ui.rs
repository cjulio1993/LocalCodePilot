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

/// Approved in T021: what happens when the configuration files cannot be read.
const CONFIG_RECOVERY_HELP: &str = "Se não for possível ler seus projetos e configurações, o aplicativo preserva os arquivos. Quando existe uma cópia válida, restaura essa cópia automaticamente. Sem cópia válida, alguns dados podem estar ausentes; use Abrir pasta no aviso para encontrar os arquivos preservados. A descoberta de projetos não recupera favoritos, histórico, comandos personalizados nem confirmações.";

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
                "Antes de executar um comando deste projeto pela primeira vez, confira a pasta e o comando na confirmação. A confirmação volta se o comando efetivo mudar. Os arquivos do projeto podem mudar depois disso. Antes de confirmar uma ação do Compose, confira também o motor e os serviços afetados.",
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
                "nada está em execução para esse projeto. Se faltarem dependências, use Instalar dependências; quando estiver pronto, use Iniciar.",
            ),
            Block::Item(
                "Iniciando",
                "o aplicativo está iniciando o projeto ou instalando dependências. Para projetos com containers, acompanhe o progresso em Containers.",
            ),
            Block::Item(
                "Com problema",
                "uma instalação ou execução falhou, ou há um impedimento detectado. O cartão mostra o motivo e a próxima ação. Em Detalhes técnicos, veja a regra e o que foi encontrado; use Ver logs quando precisar investigar a saída. Depois de resolver a causa, tente iniciar novamente. Para pedir suporte, use Exportar diagnóstico em Detalhes técnicos e confira o arquivo antes de compartilhá-lo.",
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
                    "o projeto avisa quando outro programa está usando a porta dele. Qual programa é aparece em Detalhes técnicos e em Ver logs; confira antes de alterar a configuração.",
                ),
            ),
            Block::Anchor(
                "motor-parado",
                &Block::Item(
                    "Docker ou Podman parado",
                    "clique em Iniciar no projeto ou abra Containers. Confira qual motor será ligado e confirme. O LocalCodePilot aguarda o motor responder antes de iniciar o projeto. Se não conseguir, consulte o motivo e Detalhes técnicos.",
                ),
            ),
            Block::Item(
                "Projeto sem comando para iniciar",
                "Quando o projeto tiver uma entrada clara, use Iniciar: o LocalCodePilot mostra o que será executado antes da primeira vez. Em projetos C/C++ com CMake, Meson ou um projeto executável do Visual Studio, com um único programa, Iniciar compila e executa; no macOS e no Linux, um Makefile com uma única regra run também inicia (make run), e a confirmação mostra os comandos de compilação e de execução; se a pasta tiver mais de um Makefile ou mais de uma regra run, defina o comando de início; se a compilação falhar, use Ver logs. Sem compilador C/C++, o cartão avisa: no Windows e no macOS, use Preparar ambiente (no macOS, conclua a instalação na janela da Apple); no Linux, instale o pacote indicado no cartão e use Atualizar. Se houver várias entradas ou nenhuma for identificada, use Definir comando de início no cartão. Para conferir ou alterar um comando definido por você, abra Detalhes técnicos.",
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
            Block::Anchor(
                "configuracoes-ilegiveis",
                &Block::Text(CONFIG_RECOVERY_HELP),
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
                "Ver logs leva à aba Processos ou Containers em Serviços, conforme a forma de execução do projeto.",
            ),
            Block::Text(
                "Os logs salvos ficam neste computador por até 30 dias. Senhas e tokens em formatos conhecidos, como TOKEN=valor, aparecem como •••• nos logs salvos e em Copiar logs. Limpar histórico, em Configurações, remove todos os logs salvos.",
            ),
            Block::Text(
                "Antes de colar logs ou caminhos em um relato público, confira se eles contêm senhas, tokens ou dados pessoais. A saída ao vivo pode mostrar esses dados.",
            ),
        ],
        link: Some((Page::Overview, "Abrir Visão geral")),
    },
    HelpSection {
        id: "containers",
        title: "Containers e Compose",
        blocks: &[
            Block::Text(
                "Docker e Podman precisam estar instalados e com o motor ativo. Se o Docker ou o Podman estiver instalado e parado, o LocalCodePilot pode ligá-lo após sua confirmação. Ele não instala o motor. Com os dois prontos, escolha em \"Iniciar com\" qual deles usar; a escolha vale até você trocar.",
            ),
            Block::Text(
                "Em Containers, cada projeto aparece com seus serviços e o estado de cada um: Rodando, Parado, Pausado, Com problema ou Ainda não criado. Iniciar e Parar ficam no projeto e pedem confirmação. Antes de parar, a confirmação diz quais serviços vão parar e quais continuam rodando; o Parar do cartão usa a mesma confirmação. Use Buscar projeto ou serviço quando a lista for longa; Detalhes técnicos mostra imagem, portas, ID e o comando exato. Containers que não são de um projeto da lista ficam em Outros containers. Um container rodando ou uma porta publicada não prova que o site está respondendo.",
            ),
            Block::Text(
                "Dois projetos com o mesmo nome de ambiente Compose, por exemplo duas pastas docker/ com compose.yaml, ficam bloqueados para iniciar e parar até receberem nomes diferentes. Isso evita que um projeto altere os containers e os dados do outro. No aviso do ambiente, Como resolver mostra os passos e a configuração sugerida; se os containers do outro projeto estiverem rodando, Parar containers do outro projeto para só eles, sem remover volumes.",
            ),
        ],
        link: Some((Page::Containers, "Abrir Serviços > Containers")),
    },
    HelpSection {
        id: "bancos",
        title: "Bancos de dados",
        blocks: &[
            Block::Text(
                "O LocalCodePilot testa a conexão com os dados de acesso do projeto (banco, usuário e senha do .env ou do wp-config.php). Banco de dados conectado só aparece depois desse teste; uma porta aberta ou um container ativo não bastam. Se o banco impede o início, o projeto fica Parado ou Com problema, e o cartão diz o motivo por escrito.",
            ),
            Block::Item(
                "Preparo de PostgreSQL, MySQL ou MariaDB",
                "Iniciar prepara um servidor só deste computador. Na primeira vez, a confirmação mostra o download, o espaço necessário, o banco e o usuário que serão criados; nada é baixado antes de Preparar e iniciar. O pacote é o oficial de cada banco, conferido pelo SHA-256 antes de ser usado. A senha de administração fica no Gerenciador de Credenciais do Windows (no macOS e no Linux, num arquivo que só o seu usuário lê), e o projeto só inicia depois que o banco aceita os dados de acesso dele.",
            ),
            Block::Item(
                "Onde ficam os dados",
                "na pasta do LocalCodePilot: %LOCALAPPDATA%\\LocalCodePilot\\bancos no Windows, ~/Library/Application Support/LocalCodePilot/bancos no macOS e ~/.local/share/localcodepilot/bancos no Linux. Ao fechar o aplicativo, o LocalCodePilot tenta encerrar os servidores que preparou, mesmo se o preparo ainda estiver em andamento. Os dados permanecem guardados. Se algum banco continuar rodando, você verá um aviso na próxima abertura.",
            ),
            Block::Item(
                "Servidor que você já tem",
                "se um banco já responde na porta do projeto, o LocalCodePilot só testa o login: não cria banco nem usuário, não altera o servidor e não oferece Parar para ele.",
            ),
            Block::Item(
                "Migrations",
                "Iniciar prepara o banco do projeto quando necessário. Se houver migrations pendentes que o LocalCodePilot reconhece, ele mostra o que será executado e pede sua confirmação antes de aplicá-las. O projeto só inicia depois que essa etapa termina com sucesso. O comando exato fica em Detalhes técnicos; em caso de falha, use Ver logs. Por enquanto, o LocalCodePilot reconhece as migrations de projetos Laravel com banco neste computador ou SQLite dentro do projeto.",
            ),
            Block::Item(
                "Correções",
                "quando for seguro, o cartão oferece uma saída, sempre com confirmação e com um novo teste do login antes de iniciar: Usar outra porta (muda só DB_PORT e guarda uma cópia do .env), Corrigir conexão (só no servidor do LocalCodePilot) ou Criar banco novo (os dados anteriores ficam guardados).",
            ),
            Block::Item(
                "Compose",
                "um banco que é serviço do Compose, como db, só existe na rede dos containers. Ele é iniciado em Containers, com as mesmas proteções contra conflito de nomes, e o cartão mostra Container ativo; conexão não verificada. O LocalCodePilot não prepara outro servidor para ele.",
            ),
            Block::Item(
                "SQL Server",
                "o login do projeto é testado. No Windows, com o Docker ou o Podman prontos, Iniciar pode criar um container do SQL Server 2022 Developer, gratuito para desenvolvimento e teste: antes, você precisa marcar que leu e aceita os termos de licença da Microsoft, e os dados ficam num volume do motor de containers. Sem Docker nem Podman prontos, o cartão leva à página Containers. O LocalCodePilot não instala o SQL Server direto no Windows, e a imagem funciona só em computadores x64.",
            ),
            Block::Item(
                "macOS e Linux",
                "Quando há um pacote verificado para o motor e este computador, Iniciar prepara o banco após sua confirmação. O servidor e os dados ficam na pasta do LocalCodePilot e o projeto só inicia depois de testar o login. Se o preparo não estiver disponível, o cartão informa o motivo. Onde há pacote oficial verificável, Iniciar prepara o banco com esse pacote: MySQL no macOS 15 ou mais novo e no Linux x86_64, MariaDB no Linux x86_64. MariaDB no macOS e MySQL ou MariaDB no Linux ARM usam a imagem oficial em container, fixada por digest conferido. No Windows, PostgreSQL usa um pacote oficial verificado. No macOS e no Linux, usa a imagem oficial em container, fixada por digest conferido. Os dados desse PostgreSQL ficam num volume do Docker ou do Podman. Com um motor pronto, Iniciar confirma o preparo e só inicia o projeto depois de testar o login.",
            ),
            Block::Item(
                "Banco em outro computador",
                "não recebe preparo nem mudança. Testar conexão, em Detalhes técnicos, testa o login só quando você pede.",
            ),
            Block::Text(
                "Em Detalhes técnicos, confira onde está a configuração, o endereço testado, o usuário, o banco, o resultado e, quando for o caso, o servidor do LocalCodePilot. A senha nunca é mostrada.",
            ),
        ],
        link: None,
    },
    HelpSection {
        id: "instalar-windows",
        title: "Instalar no Windows com segurança",
        blocks: &[
            Block::Text(
                "As versões atuais não têm assinatura de código, e o SmartScreen pode mostrar \"O Windows protegeu o computador\". Antes de abrir o instalador ou o zip, confira o SHA-256 do arquivo.",
            ),
            Block::Item(
                "1. Baixe pela página oficial",
                "use a página de releases do LocalCodePilot no GitHub e encontre, no arquivo SHA256SUMS.txt da mesma release, o SHA-256 publicado para esse mesmo arquivo.",
            ),
            Block::Item(
                "2. Calcule o SHA-256",
                "no PowerShell, na pasta do download, execute Get-FileHash -Algorithm SHA256 -LiteralPath '.\\NOME-DO-ARQUIVO', trocando o nome pelo do arquivo baixado.",
            ),
            Block::Item(
                "3. Compare",
                "se o valor em Hash for diferente, ou se não houver SHA-256 para esse arquivo, não execute o arquivo: apague-o e baixe de novo pela página oficial.",
            ),
            Block::Item(
                "4. Abra",
                "só se os valores coincidirem e você confiar na origem. Se aparecer \"O Windows protegeu o computador\", escolha Mais informações e depois Executar assim mesmo.",
            ),
        ],
        link: None,
    },
    HelpSection {
        id: "atualizar",
        title: "Atualizar o LocalCodePilot",
        blocks: &[
            Block::Text(
                "A atualização pelo aplicativo é experimental. Quando há uma versão nova, o aviso mostra o que mudou e Atualizar agora; a mesma ação fica em Configurações > Atualizações. Antes de baixar, o LocalCodePilot verifica a assinatura dos checksums. Depois de uma atualização concluída, as novidades da versão aparecem uma vez; numa instalação nova, não aparecem. Você pode vê-las novamente em Configurações > Atualizações, ao lado da versão instalada, ou abrir as notas completas.",
            ),
            Block::Item(
                "Etapas",
                "Baixar, Verificar, Instalar e Reiniciar. Com o app fechado, a janela Atualizando o LocalCodePilot acompanha a instalação; fechar essa janela não interrompe a atualização.",
            ),
            Block::Item(
                "Instalado ou portátil",
                "a cópia instalada pelo instalador é atualizada pelo instalador oficial da versão nova, e a lista de programas do Windows acompanha a versão. A cópia portátil (zip) troca o executável. Um .exe avulso aberto direto da pasta Downloads não se atualiza sozinho: Baixar atualização leva ao instalador, e a cópia instalada passa a se atualizar sozinha.",
            ),
            Block::Item(
                "Projetos rodando",
                "se houver projetos em execução, o LocalCodePilot pede confirmação antes de encerrá-los; Agora não mantém os projetos rodando.",
            ),
            Block::Item(
                "Se falhar",
                "o aviso informa o resultado e oferece uma próxima ação, e Detalhes técnicos guarda o erro original. A versão anterior é reaberta quando a recuperação dela é confirmada.",
            ),
        ],
        link: None,
    },
    HelpSection {
        id: "tecnico",
        title: "Detalhes técnicos",
        blocks: &[
            Block::Item(
                "Detalhes técnicos do projeto",
                "pasta, linguagens e de onde vieram (arquivos do projeto ou imagem do container), comandos detectados e, quando o projeto falha, o processo, o comando e o código de saída.",
            ),
            Block::Item(
                "Linguagens neste computador",
                "em Configurações, cada linguagem aparece com a versão instalada ou como não encontrada nos locais verificados, e Detalhes técnicos mostra onde o aplicativo procurou.",
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
        // Names from the previous interface must not come back. "Corrigir conexão" is the
        // database fix of today, not the retired generic "Corrigir".
        let text = text.replace("Corrigir conexão", "");
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
                "Instalar no Windows com segurança",
                "Atualizar o LocalCodePilot",
                "Detalhes técnicos",
            ]
        );
        // Only the first steps are open at first; the rest is one click away.
        assert_eq!(
            HelpState::default().open,
            [true, false, false, false, false, false, false, false, false]
        );
    }

    #[test]
    fn the_database_help_covers_every_case_the_app_handles() {
        let section = SECTIONS
            .iter()
            .find(|section| section.id == "bancos")
            .expect("seção de bancos");
        let terms: Vec<_> = section
            .blocks
            .iter()
            .filter_map(|block| block.content().0)
            .collect();
        for case in [
            "Preparo de PostgreSQL, MySQL ou MariaDB",
            "Onde ficam os dados",
            "Servidor que você já tem",
            "Migrations",
            "Correções",
            "Compose",
            "SQL Server",
            "macOS e Linux",
            "Banco em outro computador",
        ] {
            assert!(terms.contains(&case), "falta o item {case}");
        }
        let text = all_text();
        for fact in [
            "usa a imagem oficial em container, fixada por digest conferido",
            "~/Library/Application Support/LocalCodePilot/bancos",
            "%LOCALAPPDATA%\\LocalCodePilot\\bancos",
            "Os dados permanecem guardados",
            "mesmo se o preparo ainda estiver em andamento",
            "termos de licença da Microsoft",
            "só em computadores x64",
            "A senha nunca é mostrada.",
        ] {
            assert!(text.contains(fact), "a Ajuda não diz: {fact}");
        }
    }

    #[test]
    fn help_explains_configuration_recovery_without_promising_lost_data() {
        assert!(all_text().contains(CONFIG_RECOVERY_HELP));
        assert!(CONFIG_RECOVERY_HELP.contains("não recupera favoritos"));
        assert!(sections_found("arquivos preservados").contains(&PROBLEMS));
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
