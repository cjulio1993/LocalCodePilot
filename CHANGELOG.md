# Changelog

Todas as mudanças relevantes do LocalCodePilot serão registradas neste arquivo.

O projeto usa [Versionamento Semântico](https://semver.org/lang/pt-BR/) a partir das versões públicas.

## [Não publicado]

### Adicionado

- Projetos Scala com sbt iniciam sem pedir comando. No sbt 2, o app usa `sbt --server run`: sem essa opção, o `sbt` é um cliente que deixa um servidor rodando em segundo plano, que o app não conseguiria parar. Projetos com sbt 1.x seguem com `sbt run`. Aplicações Play usam a porta 9000.
- Pacotes Swift com alvo executável iniciam sem pedir comando, com `swift run` (servidores Vapor na porta 8080). No Windows, o Swift só compila no ambiente de desenvolvedor do Visual Studio: o app localiza o Visual Studio pelo `vswhere`, aplica o ambiente do `vcvars64` e o `SDKROOT` salvo pelo instalador só aos processos `swift`, sem que seja preciso abrir um prompt especial.
- Projetos Elixir iniciam sem pedir comando: Phoenix com `mix phx.server` (porta 4000) e aplicações com árvore de supervisão (`mod:` no `mix.exs`) com `mix run --no-halt`. **Instalar dependências** roda `mix deps.get` e, antes, instala o Hex e o Rebar quando faltam, sem a pergunta que travaria a instalação.
- Projetos Kotlin e Java com o plugin `application` do Gradle, como os servidores Ktor, iniciam sem pedir comando, com `gradlew run --no-daemon` (porta 8080 no Ktor). Projetos Gradle com o plugin Kotlin aparecem como Kotlin.
- Projetos Deno iniciam sem pedir comando: as tarefas contínuas do `deno.json` ou `deno.jsonc` (`deno task dev`) ou, sem tarefas, o arquivo de entrada (`main.ts`, `server.ts`) com `deno run --allow-net --allow-read --allow-env`.
- Projetos Ruby iniciam sem pedir comando: Rails com `ruby bin/rails server` (porta 3000), apps Rack com `bundle exec rackup` (9292), Sinatra com `ruby app.rb` (4567) e programas `main.rb`/`app.rb`. Com Gemfile, **Instalar dependências** roda `bundle install` dentro de `vendor/bundle` do próprio projeto, sem misturar com as gems de outros projetos.
- Projetos .NET (C#, F#, VB) iniciam sem pedir comando: `dotnet run --project` para apps web (ASP.NET, Blazor) e programas de console ou worker, inclusive os listados num `.sln` em subpastas. Bibliotecas e projetos de teste ficam de fora, e a porta vem do perfil do `launchSettings.json`.
- No aviso de conflito do Compose, quando os containers do outro projeto estão rodando, o botão **Parar containers do outro projeto** os para sem sair do app. A confirmação lista os containers, avisa que os volumes não serão removidos, e o app confere de novo, logo antes de executar, que eles continuam os mesmos e rodando. Antes, era preciso usar `docker compose -p <nome> stop` no terminal.
- **Definir comando de início**: um projeto sem comando detectado (de qualquer linguagem, como C/C++, Perl, Lua, R ou Julia) pode receber o comando que o inicia, com nome, programa, argumentos, pasta de execução e porta opcional. A janela mostra exatamente o que será executado, recusa recursos de terminal (`&&`, `|`, `;`, redirecionamentos, variáveis) e confere se o programa e a pasta existem. O comando passa a funcionar como um detectado, fica salvo, aparece no cartão como "Comando definido por você" e pode ser editado ou removido em Detalhes técnicos quando não está rodando.
- Proteção contra conflitos de nome Compose: o aplicativo identifica cada ambiente pelo motor, pelo nome que o provedor Compose resolve (`name:`, `COMPOSE_PROJECT_NAME` no ambiente ou no `.env`, nome da pasta) e pela pasta de origem dos containers. Quando dois projetos resolvem o mesmo nome (por exemplo, dois `docker/compose.yaml`), quando há containers desse nome criados a partir de outra pasta, quando há volumes sem container que comprove a origem ou quando não é possível confirmar, iniciar e parar o ambiente, e iniciar, parar e reiniciar seus containers, ficam bloqueados, com explicação e a configuração `name:` sugerida, com o botão **Copiar configuração sugerida**. A verificação é refeita logo antes de executar. Validar, consultar estado e logs continuam disponíveis.
- Volumes deixados sem containers (por exemplo, após `compose down`) podem ser confirmados como do projeto; a confirmação fica salva por motor e pasta e vale só para aqueles volumes (nome e data de criação).
- A CLI `containers` mostra o nome Compose efetivo de cada ambiente e os conflitos encontrados.
- Preparação guiada de Docker e Podman: quando um motor não está pronto, a aba Containers e a CLI `containers` explicam o que falta (WSL, instalação, motor parado, máquina Podman ou provedor Compose) e mostram comandos oficiais para o usuário copiar e rodar, links de documentação e condições de licença. O aplicativo não executa esses comandos.
- **Verificar motores e arquivos** relê o `PATH` do sistema enquanto um motor está ausente, encontrando um motor recém-instalado sem reiniciar o aplicativo.

### Alterado

- Num projeto Laravel recém-clonado, **Instalar dependências** também cria o `.env` a partir do `.env.example`, gera a chave da aplicação e, quando o projeto usa SQLite, cria as tabelas com `php artisan migrate`; cada etapa aparece no plano antes de executar.
- A matriz de suporte separa **Inicia** e **Abre no navegador**. Rust, Node.js, PHP, Laravel, Python/Flask, Django, Spring Boot com Maven wrapper, site estático, Lumen, Spring Boot com Gradle wrapper, Go, Dart e Flutter (web e Windows) foram validados de ponta a ponta no Windows 11 e passaram para "Sim".
- A Visão geral mostra o que está rodando, o que precisa de atenção e o que está parado, com uma ação principal por projeto (Iniciar, Abrir site, Ver logs ou Detalhes). Os estados usam os mesmos nomes em todo o aplicativo: Rodando, Parado, Iniciando e Com problema.
- Projetos ganhou busca por nome e filtros por estado, com visualização em cartões ou lista; filtro e visualização ficam salvos.
- O selo do site descreve o resultado da verificação HTTP em linguagem simples (por exemplo, "Servidor respondeu; página não encontrada" para um 404) e mostra o código ao passar o mouse; sem verificação, não aparece selo. Em projetos com containers, o endereço aparece como "Endereço provável".
- Em projetos com containers, **Parar** abre a confirmação no próprio cartão quando o aplicativo sabe o que a parada alcança: um único ambiente rodando num único motor, liberado pelas checagens de Compose. A confirmação nomeia o projeto, lista os serviços que serão parados, os que já estão parados e os que continuarão rodando (serviços de perfil opcional e containers que não estão mais no arquivo, que `compose stop` não alcança), e o aviso final diz o que parou e o que continua rodando. Quando não dá para afirmar o que será parado, o botão diz **Parar em Containers** e abre o projeto em Containers com **Outras ações** aberto. Toda confirmação de container nomeia o projeto ou o container; comando e pasta ficam em **Detalhes técnicos**.
- Um `package.json` inválido mostra o que fazer: corrigir o arquivo para iniciar o projeto.
- O aplicativo e a CLI apresentam sites WordPress como projetos PHP; a detecção continua decidindo como o site inicia e onde está o banco.
- A Ajuda foi reescrita para quem está começando: primeiros passos, estados, o que fazer quando algo não funciona, sites e logs, containers e bancos, com os detalhes técnicos num bloco recolhido.
- O menu lateral pode ser recolhido para mostrar só os ícones, com o nome de cada página na dica; a escolha fica salva. O menu rola quando não cabe na janela, e seus itens e os filtros de Projetos funcionam pelo teclado (Tab e Enter) e informam o nome e a página atual a leitores de tela.
- Projetos ficou mais compacto: "Ver 1 pasta examinada" abre as pastas onde o aplicativo procura projetos, com "Adicionar pasta" e "Restaurar pastas padrão"; busca, filtros e a escolha entre cartões e lista ficam numa linha. Em janelas de 760×520, o nome, o estado e a ação principal do primeiro projeto aparecem sem rolar.
- Saíram da interface itens que não faziam nada ou não eram verdadeiros: a página Plugins, o sino de notificações e o rodapé "Todos os serviços online", que não verificava nenhum serviço. Containers ganhou um ícone próprio.
- Botões com ícone, a estrela de favoritos e os campos de busca passaram a ter nomes claros para leitores de tela.
- A Ajuda ficou interativa: o campo **Qual é a sua dúvida?** mostra só os trechos que respondem, ignorando acentos e maiúsculas e destacando o termo; atalhos levam direto a "O site não abre", "Porta ocupada", "Docker ou Podman parado" e aos estados; as seções abrem e fecham, e um aviso leva à Visão geral quando há projetos com problema.
- Avisos em duas camadas: o bloqueio de um ambiente Compose e um motor que não está pronto mostram primeiro uma frase e uma ação; os passos ficam em **Como resolver** (ou **Como instalar**, quando o motor não foi encontrado) e nomes, pastas, volumes e o estado do motor ficam em **Detalhes técnicos**. Um motor cuja consulta falha aparece como "encontrado, mas não foi possível acessá-lo", sem afirmar que está parado. Em Processos, o aviso de um container bloqueado tem o link **Como resolver**, que abre o aviso do ambiente em Containers.
- Linguagens: além de Rust, JavaScript/TypeScript, PHP, Python, Java, Go, Dart e Flutter, o aplicativo reconhece projetos .NET (C#, F#), Ruby, Kotlin, Elixir, Deno, C/C++, Swift, Scala, Clojure, Haskell, Perl, Lua, R, Julia e Zig, por manifests e arquivos de código, e também pela imagem dos serviços do Compose e pelo `FROM` do Dockerfile. Os detalhes técnicos do cartão dizem de onde cada linguagem veio ("nos arquivos do projeto" ou "na imagem do serviço api") e se ela está instalada. Para uma linguagem que o aplicativo ainda não inicia, o cartão diz isso em vez de "nenhum comando encontrado".
- Configurações ganhou **Linguagens neste computador**, no lugar da lista "Runtimes": cada linguagem aparece como "Instalado · versão", "Instalado · Versão indisponível" ou "Não encontrado nos locais verificados", com busca, os projetos que a usam e, em Detalhes técnicos, o programa, o caminho e os locais verificados (`PATH` e pastas padrão de instalação). O atalho do Python da Microsoft Store sem Python instalado conta como não encontrado.
- Containers mostra cada projeto com seus serviços e o estado de cada um (Rodando, Reiniciando, Pausado, Parado, Com problema ou Ainda não criado), juntando o que o arquivo Compose declara com o que o motor informa. Cada ambiente tem uma ação principal (**Iniciar** ou **Parar**) e **Ver logs**; validar, consultar estado e as ações por container ficam em **Outras ações**, e arquivos, imagem, portas, ID e comandos em **Detalhes técnicos**. Projetos bloqueados, com problema e rodando aparecem primeiro, e os parados começam recolhidos. **Buscar projeto ou serviço** filtra a lista. Containers que não pertencem com certeza a um projeto da lista ficam em **Outros containers**, com as mesmas ações. Os motores aparecem numa linha; o aviso de um motor que não está pronto abre quando o aplicativo depende dele. A escolha entre Docker e Podman só aparece quando os dois estão prontos. **Parar** usa o motor em que o ambiente já está rodando. Ações que só leem (validar, consultar estado e **Ver logs**) rodam sem pedir confirmação; as que mudam o ambiente continuam pedindo, com o comando e a pasta. O resultado de uma ação aparece no topo da página, que rola até ele. Quando só restam rodando serviços que a parada do ambiente não alcança, o cabeçalho oferece **Iniciar** e indica **Outras ações**.
- Processos mostra só os processos que rodam direto na máquina; os containers ficam em Containers, e Processos indica quantos estão rodando, com o link **Ver em Containers**.
- Nos cartões de projeto, **Detalhes** passou a se chamar **Detalhes técnicos** e não repete o motivo já mostrado no cartão.
- Visão geral, Projetos e Processos atualizam o estado dos containers a cada 30 segundos, consultando apenas a lista de containers, para manter cartões e diagnóstico de banco atualizados.
- URLs candidatas de containers reconhecem mais portas de servidores de desenvolvimento, como 3001, 4000, 4200, 5000 e 8025, e não repetem a mesma URL publicada em IPv4 e IPv6.

### Corrigido

- Uma linguagem instalada numa pasta padrão, mas fora do PATH (como o Elixir e o Erlang instalados pelo winget), passa a iniciar e a instalar dependências: as pastas de instalação conhecidas entram no fim do PATH dos processos. A verificação antes de iniciar também usa o PATH atualizado, então um programa instalado depois de abrir o app é encontrado sem reiniciá-lo.
- As pastas `deps` e `_build` do Elixir não são mais examinadas como projetos; antes, os pacotes em `deps/` (como `deps/phoenix`, com `package.json`) apareciam como subprojetos Node.js. O TypeScript da pasta `assets/` do Phoenix não faz mais o projeto aparecer como TypeScript.
- O comando de projetos com Gradle Wrapper ou Maven Wrapper aparece com o caminho normal (`C:\...\gradlew.bat`), sem o prefixo interno `\\?\` do Windows.
- Depois de uma falha ao iniciar (porta ocupada, servidor que fechou antes de ficar pronto, programa que parou com erro), o projeto mostra **Tentar de novo** ao lado de Ver logs. Antes, o cartão só oferecia Ver logs, e não havia como iniciar de novo pelo próprio projeto depois de resolver a causa.
- **Abrir no VS Code** não abre mais o projeto em modo restrito. O app passava ao VS Code o caminho no formato interno do Windows, com o prefixo `\\?\`, que o VS Code trata como uma pasta diferente da que você marcou como confiável; agora passa o caminho normal.
- Em Containers, a escolha "Iniciar com: Podman" não volta mais para Docker na verificação automática a cada 15 segundos. Antes, o ambiente podia subir no Docker mesmo com o Podman escolhido. Com essa correção, Containers com Podman foram validados de ponta a ponta no Windows 11 (iniciar, endereço, logs e parar) e deixam de ser experimentais.
- Projetos sem framework não pedem mais um comando para iniciar: um arquivo Java com `main` roda com `java Main.java`, uma pasta de páginas PHP sem `index.php` abre com uma página que lista os arquivos, e páginas HTML sem `index.html` na raiz (como um jogo com `teste.html` e `source_code/index.html`) abrem como site estático. Antes, esses projetos mostravam "Reconhecemos Node, mas precisamos do comando para iniciar este projeto".
- A busca pela porta de um servidor só adota portas que respondem como servidor web. Antes, um projeto Spring Boot com Gradle ficava preso na porta do daemon do Gradle e mostrava "Site sem resposta".
- Spring Boot com Gradle inicia com `--no-daemon`: uma partida que falha, por exemplo com a porta ocupada, não deixa mais um processo Java do Gradle rodando em segundo plano.
- Num projeto Lumen recém-clonado, **Instalar dependências** também cria o `.env` a partir do `.env.example`.
- O Perl que acompanha o Git para Windows é encontrado em **Linguagens neste computador**.
- Go no layout `cmd/<nome>/main.go` passa a ter comando: um processo por programa, com `go run ./cmd/<nome>` na raiz do módulo. Antes, o cartão dizia que nenhum comando foi identificado.
- Dart encontra os programas criados por `dart create` (`bin/server.dart` e `bin/<pacote>.dart`). Antes, um servidor Dart com Dockerfile, como o do modelo oficial, só podia iniciar pelos containers.
- Um app Flutter para desktop não mostra mais **Abrir site** com o endereço do depurador; endereços do DevTools nunca são tratados como o site de nenhum projeto.
- Projetos em pastas com espaço no nome (como "Área de Trabalho") voltaram a iniciar comandos `.cmd` e `.bat`, como `npm`, `mvnw` e `gradlew`: o aplicativo passava ao `cmd.exe` o caminho no formato `\\?\`, que ele não aceita, e o comando rodava na pasta do Windows ou era cortado no espaço.
- Servidores que não escrevem o endereço no log (Express, Spring Boot, Go, servidores Rust próprios) passam a mostrar **Abrir site** e o selo do site: o aplicativo consulta no sistema a porta em que o processo do projeto, ou um processo filho, está ouvindo.
- Quando um servidor fecha porque a porta já está em uso, o cartão diz "Outro programa está usando a porta" em vez de "O processo fechou inesperadamente", em Node, Spring Boot, Flask, PHP, Rust, Go e Python. Se outro programa ocupa a porta durante a execução, o motivo também é esse, e não "parou de responder".
- No Windows, o dono de uma porta passa a ser lido da tabela TCP do sistema em vez do texto do `netstat`, que podia vir traduzido, com estados errados ou sem algumas linhas; um projeto que ouve na mesma porta que outro programa, em outro endereço, é reconhecido.
- Laravel 11 ou mais novo com SQLite não ficava mais preso em "Arquivo SQLite será preparado ao iniciar o ambiente" sem botão para iniciar: o arquivo `database/database.sqlite`, usado quando `DB_DATABASE` está comentado, é criado ao iniciar.
- A consulta de versão de cada linguagem tem tempo limite de 10 segundos; antes, um programa que não respondia prendia a verificação do ambiente.
- Um site PHP com o Compose numa subpasta (como `docker/`) inicia pelos containers quando o Compose serve o site (serviço com `build:` próprio ou imagem de PHP, Apache, Nginx ou Caddy), não só no WordPress. Um Compose só com o banco ou com phpMyAdmin mantém o servidor PHP local.
- Um `compose.yaml` salvo com BOM, comum em editores no Windows, não aparece mais como "YAML inválido".
- No Windows, quando o sistema informa qual programa usa a porta, um projeto só é marcado como Rodando, e só recebe o selo do site, se a porta for do processo dele ou de um processo filho. Se outro programa ocupar a porta, o motivo diz isso, e o log informa qual programa é.
- Quando um projeto falha ao iniciar ou para de funcionar, o cartão mostra o motivo (porta ocupada por outro programa, servidor que não respondeu a tempo, que fechou antes de ficar pronto ou que parou de responder, programa ou pasta não encontrados) em vez de "parou com erro". Processo, comando, código de saída e, quando o sistema informa, o programa que ocupa a porta ficam em **Detalhes técnicos**. O motivo some quando o projeto é iniciado de novo ou parado.
- Em Containers e em Processos, a verificação periódica dos motores não empurra mais a página para baixo enquanto roda; antes, os botões mudavam de lugar sob o cursor.
- Containers aparecem no projeto correto no Windows mesmo quando o Compose registrou a pasta com letra de unidade ou nomes em outra caixa, outro separador, prefixo `\\?\` ou caminho WSL `/mnt/<unidade>`.
- O banco em container é associado ao projeto quando o Compose está em uma subpasta e quando o `DB_HOST` usa o nome do container em vez do serviço.
- Containers Podman mostram nome, portas e URLs; antes os campos em lista retornados pelo Podman eram ignorados.
- Copiar texto do aplicativo para a área de transferência voltou a funcionar; o suporte a clipboard do `eframe` estava desativado no build.
- Com o motor parado, o Compose aparece como "será verificado quando o motor estiver ativo" em vez de ausente.
- Um site PHP com o arquivo Compose numa subpasta (como `docker/`) ou com um Compose que declara a imagem do site em outra pasta do projeto não oferece mais "Servidor PHP" (`php -S`): o cartão mostra **Iniciar com Containers**. Um Compose de outro serviço na mesma pasta pai continua sem afetar o site.
- `DB_HOST` com o nome de um serviço Compose do projeto, como `db`, aparece como banco em container, e não como servidor remoto. A CLI `databases` informa se o container está em execução, parado ou ainda não criado.
- Caminhos de projetos aparecem sem o prefixo `\\?\` na CLI e no aplicativo.
- A CLI aceita pastas relativas, como `.`, em `inspect`, `containers` e `databases`; antes, `databases .` não encontrava o container do banco em execução.

## [0.4.0-alpha.7] - 2026-10-22

### Adicionado

- Reconhecimento de WordPress e leitura segura do `DB_HOST` literal para diagnóstico MySQL.
- Listagem de containers reais Docker/Podman em Processos, com estado, portas, URLs candidatas, logs e ações de iniciar, parar ou reiniciar.
- Navegação de Projetos para Processos e Containers filtrada pelo projeto selecionado.

### Alterado

- Cartões de projeto destacam o próximo passo, mostram a resposta HTTP confirmada separadamente do estado dos serviços e agrupam opções secundárias em **Mais ações**.
- WordPress com Compose abre o fluxo de Containers em vez de iniciar um servidor PHP local sem acesso ao host do banco na rede Compose.
- README e Ajuda integrada descrevem os fluxos atuais, os limites do diagnóstico de bancos e a recuperação de falhas.

## [0.4.0-alpha.6] - 2026-10-21

### Adicionado

- Página **Ajuda** no aplicativo com tutorial de primeiros passos, explicação dos estados, orientação para bancos e containers e solução de problemas comuns.
- Diagnóstico inicial de PostgreSQL, MySQL, MariaDB, SQL Server e SQLite em `.env`/`.env.local`, com teste de porta local no desktop e na CLI sem exibir credenciais.
- Health checks HTTP assíncronos para serviços locais, com código de resposta, tempo de resposta, timeout e estados de saúde exibidos na tela Processos.
- Execução controlada de Compose com Docker ou Podman para validar, iniciar, parar, consultar estado e ler logs, além da construção de Dockerfiles isolados.
- Smoke test no CI para validar o fluxo real de Docker Compose e a construção de Dockerfile sem depender da máquina do mantenedor.
- Smoke test equivalente para Podman Compose e compatibilidade com provedores antigos sem suporte a `config --quiet` ou `ps --all`.
- Confirmação com comando e pasta antes de cada ação de container, saída rolável dentro do app, cancelamento e timeout de 30 minutos.
- Empacotamento macOS universal para Intel e Apple Silicon em DMG, com ícone, bundle nativo, assinatura ad hoc e verificações automatizadas.
- Workflow de validação macOS que gera um artefato temporário sem criar release.

### Alterado

- A matriz de suporte e a documentação de Containers agora distinguem o diagnóstico da execução confirmada e deixam claro que o app não instala nem inicia os motores.
- Arquivos Compose de override detectados agora são enviados ao provedor junto do manifesto principal.
- O verificador de atualização seleciona o DMG universal no macOS em vez de oferecer o pacote do Windows.
- A release por tag da Alpha 6 publica os artefatos Windows e o DMG macOS experimental somente depois que os checks das duas plataformas passam.
- A Alpha 6 tem suporte principal no Windows e inclui um DMG macOS universal experimental para validação em Macs reais.
- A visão geral usa cartões com altura e posição de ações consistentes, reduzindo mudanças visuais entre projetos.
- Projetos salvos aparecem a partir do cache na abertura, enquanto descoberta, processos e diagnósticos são atualizados em segundo plano.

### Corrigido

- A detecção preserva os runtimes independentes de Node, Dart e Go em repositórios que também contêm Flutter.
- O botão **Abrir aplicação** desaparece quando o processo do servidor é encerrado; a indicação de banco indisponível tem prioridade sobre **Disponível**.
- Serviços só são considerados disponíveis depois que a porta esperada responde; uma URL anunciada nos logs não basta.
- No Windows, executáveis nativos são iniciados diretamente, scripts BAT/CMD continuam pelo interpretador e a parada de processos gerenciados alcança seus descendentes mesmo quando `taskkill` falha.

- Avisos de atualização agora exigem um pacote para o sistema em uso e descartam notificações incompatíveis em cache, evitando oferecer releases exclusivas do macOS no Windows e vice-versa.

- A verificação do binário universal usa a ordem de argumentos aceita pelo `lipo` do Xcode.

## [0.4.0-alpha.5.1] - 2026-10-16

### Corrigido

- O atualizador exige um pacote para o sistema em uso: Windows recebe somente pacotes Windows e macOS recebe somente DMGs macOS.
- Releases sem pacote compatível são ignoradas, inclusive quando forem mais recentes; o app continua procurando a versão compatível mais recente.
- Avisos incompatíveis salvos em cache são descartados.
- Correção baseada na alpha 5, sem antecipar as funcionalidades da alpha 6 em validação.

## [0.4.0-alpha.5] - 2026-10-13

### Adicionado

- Atualização do executável pelo próprio aplicativo no Windows, com progresso, verificação SHA-256, reinício automático e restauração do backup quando a nova versão não inicia.
- Asset `.exe` dedicado à atualização, publicado junto com o ZIP e incluído no `SHA256SUMS.txt`.

### Alterado

- O download manual do ZIP permanece disponível como alternativa para builds de desenvolvimento, sistemas não Windows e pastas sem permissão de escrita.

## [0.4.0-alpha.4] - 2026-10-13

### Adicionado

- Diagnóstico inicial de Docker/Podman e Compose em segundo plano, com timeout por consulta.
- Descoberta de projetos por Dockerfile, Containerfile e manifests Compose.
- Aba Containers com prévia de serviços, imagens, portas, perfis e comandos propostos, sem execução de containers.
- CLI containers <pasta> para diagnóstico e inspeção de manifests.

### Alterado

- O fluxo de contribuição agora exige a atualização do README e do changelog junto com toda correção, mudança de comportamento ou funcionalidade nova.
- O procedimento administrativo de assinatura do Windows foi retirado da documentação pública e preservado somente no manual local do mantenedor, ignorado pelo Git.


## [0.4.0-alpha.3] - 2026-10-13

### Corrigido

- Executável desktop usa o subsistema gráfico do Windows; comandos auxiliares são iniciados sem janelas de console externas.

### Adicionado

- Botão Verificar se há atualizações em Configurações, com consulta imediata, progresso, resultado e mensagens de falha.
- A consulta manual ignora a espera diária e o lembrete adiado, mantendo a verificação automática existente.

## [0.4.0-alpha.2] - 2026-10-13

### Corrigido

- Descoberta automática de projetos Java por manifests Maven/Gradle e wrappers, mesmo com apenas um arquivo fonte.
- Projetos Node sem dependências externas não exigem a criação de node_modules para serem considerados prontos.
- Resolução de npm e outros comandos no Windows prioriza extensões executáveis do PATHEXT em vez de scripts Unix sem extensão.
- Erros ao iniciar a preparação passam a constar nos logs persistidos da instalação.
- Aplicações de terminal que encerram com código 0 e sem porta monitorada recebem o estado Concluído e histórico de sucesso, em vez de erro de processo inesperado.
- Localização do Dart embutido no Flutter e do SDK portátil do winget, evitando reinstalações causadas por lançadores ou PATH incompleto.
- Pacotes winget já instalados sem atualização seguem para atualização do ambiente e verificação obrigatória da versão.
- Rolagem dos requisitos e etapas na confirmação de preparação, mantendo os botões acessíveis.
- Verificação de versão de runtimes distribuídos como lançadores `.bat`/`.cmd` no Windows.
- Isolamento de pastas com `.localcodepilot-ignore` durante a descoberta recursiva, evitando agregar fixtures de QA ao ambiente do projeto pai.

## [0.4.0-alpha.1] - 2026-10-06

### Adicionado

- Detecção da porta esperada para Laravel, Django, PHP e servidores Node.js comuns.
- Verificação de conflitos de porta antes de iniciar um serviço ou ambiente.
- Seleção automática da próxima porta livre para comandos compatíveis, com ajuste seguro dos argumentos de inicialização.
- Exibição da porta escolhida na interface e registro da troca nos logs do processo.
- Estado de inicialização mantido até a porta do serviço ficar disponível.
- Monitoramento periódico da porta, com detecção de timeout, encerramento inesperado e perda do serviço.
- Verificação de listeners IPv4 e IPv6 para evitar falsos alertas em servidores que usam `::1`, como o Vite no Windows.
- Sugestões de correção nos cartões e no terminal a partir dos erros encontrados nos logs.
- Persistência local da tela selecionada, projeto em foco, favoritos e histórico dos serviços.
- Restauração das portas, URLs, resultados e dos 200 logs mais recentes de cada serviço.
- Ordenação por uso recente e ação para retomar ambientes sem iniciá-los automaticamente.
- Verificação diária de novas versões em segundo plano, incluindo releases alpha.
- Aviso de atualização com acesso às novidades, ao pacote oficial e opção de lembrar depois.
- Identificação do nome e PID do processo que ocupa uma porta no Windows.
- Confirmação explícita para encerrar o processo conflitante e tentar novamente.

## [0.3.0-alpha.1] - 2026-09-28

### Adicionado

- Inicialização e interrupção do ambiente completo de um projeto com um clique.
- Instalação de dependências ausentes com o gerenciador detectado.
- Logs ao vivo de processos e instalações em uma janela semelhante a um terminal.
- Diagnóstico de falhas de instalação com sugestões de correção e nova tentativa.
- Detecção de URLs locais e atalho para abrir a aplicação no navegador.
- Priorização dos projetos e serviços ativos na tela de processos.
- Detecção proativa de versões legadas do Laravel.
- Migração assistida do Laravel com simulação isolada, confirmação e backup dos manifests do Composer.
- Seleção automática entre npm, pnpm, Yarn e Bun.
- Iniciais do usuário do computador na interface.
- Layout responsivo para os cartões de projeto.

### Distribuição

- Primeiro pacote alpha portátil para Windows 10/11 x64.
- Metadados de versão incorporados ao executável do Windows.
- Automação de release com pacote ZIP e arquivo de verificação SHA-256.
- Etapa opcional de assinatura Authenticode por Microsoft Artifact Signing.

[Não publicado]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.4.0-alpha.5...HEAD
[0.4.0-alpha.1]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.3.0-alpha.1...v0.4.0-alpha.1
[0.3.0-alpha.1]: https://github.com/cjulio1993/LocalCodePilot/releases/tag/v0.3.0-alpha.1

[0.4.0-alpha.2]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.4.0-alpha.1...v0.4.0-alpha.2

[0.4.0-alpha.3]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.4.0-alpha.2...v0.4.0-alpha.3
[0.4.0-alpha.4]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.4.0-alpha.3...v0.4.0-alpha.4
[0.4.0-alpha.5]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.4.0-alpha.4...v0.4.0-alpha.5
