# LocalCodePilot

<p align="center">
  <img src="assets/branding/logo.svg" alt="LocalCodePilot" width="600">
</p>

> Clique em **Iniciar** e o projeto sobe. Sem decorar comandos, sem procurar porta, sem abrir terminal.
>
> Click **Start** and your project is up. No commands to remember, no ports to hunt, no terminal to open.

**Alpha pública / public alpha.** O LocalCodePilot ainda está em desenvolvimento e pode mudar antes da versão estável.

[Português](#português) · [English](#english)

---

## Português

### O que é

O LocalCodePilot é um aplicativo para desenvolvedores, do júnior ao especialista, que encontra os projetos do seu computador e cuida de rodá-los por você:

- **Encontra seus projetos sozinho** e reconhece a linguagem e o framework de cada um.
- **Prepara o que falta:** instala as dependências do projeto e, quando a linguagem nem está instalada, oferece instalá-la com um clique.
- **Inicia, mostra o endereço e abre o site.** Se algo der errado (uma porta ocupada, um programa que fechou), diz o motivo em português e oferece **Tentar de novo**.
- **Também cuida de containers** (Docker e Podman), com proteção para um projeto não mexer nos containers de outro.
- **Testa o banco de dados do projeto** com os dados de acesso dele e, no Windows, prepara o servidor de banco quando falta (veja [Bancos de dados](#bancos-de-dados)).

Tudo roda no seu computador. O app não envia seus projetos para lugar nenhum, e nada é instalado sem você ver o plano e confirmar.

### Baixar e instalar

**Windows 10/11 (64 bits).** Na página de [releases](https://github.com/cjulio1993/LocalCodePilot/releases):

- **Instalador (`LocalCodePilotSetup-<versão>-windows-x64.exe`; até a alpha 8, `...-windows-x64-setup.exe`), recomendado.** Instala o app e cria o atalho no Menu Iniciar. Quando você usa **Atualizar agora**, o app roda o instalador da nova versão em segundo plano, e a lista de programas do Windows passa a mostrar a versão certa. O outro `.exe` da release (`LocalCodePilot-<versão>-windows-x64.exe`) é o arquivo que a cópia portátil usa para se atualizar; não o abra direto. Aberto assim, o app não se atualiza sozinho, e **Baixar atualização** leva ao instalador.
- **Zip (portátil).** Extraia e execute `LocalCodePilot.exe`, sem instalar.

**Se você usa a Alpha 8 ou a Alpha 9:** o app avisa a nova versão; use **Atualizar agora**. Se a atualização não concluir, baixe o instalador ou o zip na [página de releases](https://github.com/cjulio1993/LocalCodePilot/releases). Na Alpha 9 instalada pelo instalador, **Atualizar agora** troca só o executável, e a lista de programas do Windows continua mostrando a Alpha 9; para a lista mostrar a versão certa, rode o instalador da Alpha 10 uma vez. A partir da Alpha 10, a atualização roda o instalador oficial.

**Se você usa a Alpha 7:** baixe o instalador ou o zip. O botão **Atualizar agora** da Alpha 7 não conclui a atualização.

**Windows: confira o arquivo antes de abrir.** As versões atuais não têm assinatura de código, e o SmartScreen pode mostrar "O Windows protegeu o computador".

1. Baixe o instalador ou o zip pela [página oficial de releases](https://github.com/cjulio1993/LocalCodePilot/releases).
2. Na mesma release, abra o `SHA256SUMS.txt` e encontre o SHA-256 publicado para **o mesmo arquivo** que você baixou.
3. Abra o PowerShell na pasta do arquivo e execute `Get-FileHash -Algorithm SHA256 -LiteralPath '.\NOME-DO-ARQUIVO'`, trocando `NOME-DO-ARQUIVO` pelo nome do instalador ou do zip baixado.
4. Compare o valor em `Hash` com o SHA-256 da release. Se forem diferentes, **não execute o arquivo**: apague o download e baixe de novo pela página oficial. Se não encontrar o SHA-256 desse arquivo, não prossiga.
5. Só se os valores coincidirem e você confiar na origem, abra o arquivo. Se o SmartScreen aparecer, clique em **Mais informações** e depois em **Executar assim mesmo**.

**macOS (experimental).** Há um pacote universal (Intel e Apple Silicon) para testes; a validação em Mac real ainda está em andamento. Veja [docs/README-MACOS.txt](docs/README-MACOS.txt).

### Primeiros passos

1. **Abra o app.** Em **Projetos**, ele mostra o que encontrou nas pastas comuns do computador. Use **Adicionar pasta** se seus projetos estiverem em outro lugar. A busca não entra em pastas de programas instalados (como `Program Files` e `%LOCALAPPDATA%\Programs`), a não ser que você adicione uma delas.
2. **Escolha um projeto.** O cartão mostra o estado do projeto, o que falta para prepará-lo e qual é o próximo passo. Dependências ainda não instaladas aparecem como **Parado**; falhas de instalação ou execução aparecem como **Com problema**. O próximo passo é:
   - **Preparar ambiente**, quando falta a linguagem;
   - **Instalar dependências**, quando faltam as bibliotecas do projeto;
   - **Iniciar**, quando está tudo pronto.

   Se dois projetos têm o mesmo nome, o cartão mostra também o trecho da pasta que os diferencia.
3. **Clique no botão.** Na primeira execução de um comando do projeto, confira a pasta e o comando antes de confirmar. A confirmação volta se o comando efetivo mudar. Para instalações e ações do Compose, confira também o plano apresentado; nas ações de containers, a confirmação mostra a pasta e o motor (Docker ou Podman).
4. **Use o projeto.** Quando o site responde, o cartão mostra o endereço e **Abrir site**. **Ver logs** mostra a saída do programa, e **Parar** encerra tudo o que o projeto abriu. Os logs salvos ficam no computador por até 30 dias, com senhas e tokens em formatos conhecidos trocados por `••••`; **Limpar histórico**, em Configurações, apaga todos. A saída ao vivo continua completa: confira se há dados pessoais antes de colar logs num relato público.

Se um dia não for possível ler seus projetos e configurações, o app guarda os arquivos e restaura sozinho a última cópia válida. Sem cópia válida, ele começa com as configurações padrão e avisa que alguns dados podem estar ausentes; **Abrir pasta** mostra os arquivos guardados, e o aviso continua em **Configurações** até você clicar em **Entendi**. A busca de projetos refaz só a lista de projetos, não favoritos, histórico, comandos personalizados nem confirmações.

A página **Ajuda**, dentro do app, explica cada estado e o que fazer em cada caso.

### Linguagens e projetos

O que acontece ao clicar em **Iniciar**, validado no Windows 11:

| Projeto | O app inicia com | O app prepara |
| --- | --- | --- |
| Node.js / JavaScript (npm, pnpm, Yarn, Bun) | o script de desenvolvimento (`dev`, `start`…) | as dependências |
| PHP, Laravel, Lumen | o servidor do PHP ou `artisan serve` | o Composer, o `.env` e a chave do Laravel |
| Python, Flask, Django | o `app.py`/`main.py` ou `runserver` | o ambiente virtual e as dependências |
| Java e Kotlin (Maven, Gradle, Spring Boot, Ktor) | o wrapper do projeto (`mvnw`, `gradlew`) | o próprio Gradle/Maven baixa as dependências |
| .NET (ASP.NET, Blazor, console) | `dotnet run` | o `dotnet` restaura os pacotes |
| Ruby (Rails, Rack, Sinatra) | `bin/rails server`, `rackup` ou `ruby app.rb` | `bundle install` dentro do projeto |
| Go | `go run` | o Go baixa os módulos |
| Rust | `cargo run` | o Cargo baixa as dependências |
| Dart e Flutter (web e Windows) | `dart run` ou `flutter run` | as dependências do `pub` |
| Deno | as tarefas do `deno.json` | o Deno baixa as dependências |
| Elixir (Phoenix) | `mix phx.server` | Hex, Rebar e `mix deps.get` |
| Swift | `swift run` | o ambiente do Visual Studio no Windows (precisa do Build Tools com C++) |
| Scala (sbt, Play) | `sbt run` | o sbt baixa as dependências |
| Clojure (`deps.edn`) | `bb clojure -M:run` | **instala o Babashka** se faltar |
| Haskell (Stack) | `stack run` | **instala o Stack**, baixa o compilador e compila |
| Haskell (só Cabal, com um executável) | `cabal run` | o Cabal baixa as dependências e compila |
| Clojure com Leiningen (`project.clj` com `:main`) | `lein run` | o Leiningen baixa as dependências |
| Perl, Lua, R, Julia | o arquivo de entrada (`main.pl`, `main.lua`, `main.R`, `main.jl`… ou o único arquivo da linguagem na pasta); um `app.R` com Shiny abre como site | — |
| C/C++ com CMake, Meson ou Visual Studio (`.vcxproj`, ou `.sln` com um único projeto executável, no Windows) | configura, compila e executa o programa; a pasta de compilação fica fora do projeto (o Visual Studio compila com o MSBuild instalado, em Debug) | **instala o CMake ou o Meson** se faltar; o compilador C/C++ precisa estar instalado |
| Zig | `zig build run` | **instala o Zig** se faltar |
| Sites HTML e projetos sem framework | um servidor local (ou `java Main.java`, ou o servidor do PHP) | — |
| Containers (Docker, Podman, Compose) | `compose up`, escolhendo o motor | não instala Docker/Podman; pode ligar o motor instalado após confirmação |
| C/C++ com um único Makefile que tem uma única regra `run` (macOS e Linux) | `make run`; a confirmação mostra as regras que ele executa (os pré-requisitos de `run` e o próprio `run`) | o `make` e o compilador precisam estar instalados |
| Outras linguagens (C/C++ com Makefile sem alvo `run`, ou no Windows…) e projetos com mais de uma entrada possível | o comando que você definir uma vez; o cartão diz por que precisa dele | — |

Quando uma linguagem não está instalada, **Preparar ambiente** usa o `winget` do Windows. Nas linhas em negrito, a instalação foi validada de ponta a ponta. Para Python, Java, PHP, Node.js, Go e Dart, a instalação pelo app ainda está em validação.

### Bancos de dados

O LocalCodePilot lê o banco de dados do projeto no `.env` (`DB_CONNECTION`, `DB_HOST`, `DB_PORT`, `DB_DATABASE`, `DB_USERNAME`, `DB_PASSWORD`) ou no `wp-config.php` e testa a conexão com esses dados de acesso. O cartão só mostra **Banco de dados conectado.** depois que o banco aceita o login do projeto; uma porta aberta ou um container ativo não bastam. Se o banco impede o início, o projeto fica **Parado** ou **Com problema**, com o motivo por escrito.

| Situação | O que o app faz |
|---|---|
| PostgreSQL, MySQL ou MariaDB neste computador, no **Windows** | **Iniciar** prepara um servidor só deste computador (`127.0.0.1`). Na primeira vez, a confirmação **Preparar banco de dados para este projeto?** mostra o download, o espaço necessário, o banco e o usuário que serão criados. O pacote é uma versão fixada e conferida por SHA-256; a senha de administração fica no Gerenciador de Credenciais do Windows. |
| SQL Server neste computador, no **Windows** | Com o Docker ou o Podman prontos, **Iniciar** pode criar um container do SQL Server 2022 Developer (gratuito para desenvolvimento e teste). Você precisa marcar que leu e aceita os termos de licença da Microsoft; o app nunca aceita por você. Sem Docker nem Podman prontos, o cartão leva à página **Containers**. |
| Servidor que você já tem, na porta do projeto | Só o login é testado. O app não cria banco nem usuário, não altera o servidor e não oferece **Parar** para ele. |
| Serviço do Compose (como `db`) | Fica no fluxo de **Containers**, com as mesmas proteções contra conflito de nomes. O cartão mostra "Container ativo; conexão não verificada."; nenhum outro servidor é preparado. |
| Banco em outro computador | Não recebe preparo nem mudança. **Testar conexão**, em **Detalhes técnicos**, testa o login só quando você pede. |
| **macOS e Linux** | O login do projeto é testado com PostgreSQL, MySQL, MariaDB e SQL Server. **Iniciar** prepara o MySQL (macOS 15+ e Linux x86_64) e o MariaDB (Linux x86_64) com o pacote oficial, após confirmação. No macOS e no Linux, Iniciar prepara PostgreSQL local em container com Docker ou Podman, após confirmação. A imagem oficial é fixada por digest, a porta fica restrita a 127.0.0.1 e os dados permanecem num volume do motor. MariaDB no macOS e MySQL/MariaDB no Linux ARM são preparados em container com Docker ou Podman, após confirmação. A imagem oficial é fixada por digest, a porta fica restrita a 127.0.0.1 e os dados permanecem num volume do motor de containers. |
| SQLite com arquivo dentro do projeto | O app cria a pasta e o arquivo quando faltam e confere a integridade. Se reconhecer migrations pendentes do projeto, pede confirmação antes de executá-las e só então inicia o projeto. |

**Onde ficam os dados.** Os servidores preparados no Windows guardam os dados em `%LOCALAPPDATA%\LocalCodePilot\bancos` (no macOS e no Linux, na pasta de dados do app); o SQL Server, e o PostgreSQL no macOS e no Linux, guardam num volume do Docker ou do Podman. Ao fechar o app, o LocalCodePilot tenta encerrar os servidores e containers que preparou, inclusive durante o primeiro preparo. Se algum continuar rodando, avisa na próxima abertura; os dados permanecem e voltam no próximo **Iniciar**.

**Correções.** Quando for seguro, o cartão oferece uma saída, sempre com confirmação e com um novo teste do login antes de iniciar: **Usar outra porta** (muda só `DB_PORT` e guarda uma cópia do `.env`), **Corrigir conexão** (só no servidor do LocalCodePilot) ou **Criar banco novo** (os dados anteriores ficam guardados). **Detalhes técnicos** mostra onde está a configuração, o endereço testado, o usuário, o banco e o resultado; a senha nunca aparece.

Mais detalhes em [docs/DATABASES.md](docs/DATABASES.md).

### Limitações desta alpha

- **Plataforma:** a validação de ponta a ponta foi feita no Windows 11. No macOS o pacote é experimental, e no Linux há só testes automáticos.
- **Assinatura:** os downloads ainda não têm assinatura digital (veja [Baixar e instalar](#baixar-e-instalar)).
- **Bancos de dados:** no macOS e no Linux, o preparo do servidor cobre o MySQL (macOS 15+ e Linux x86_64), o MariaDB (Linux x86_64) e o PostgreSQL em container (precisa do Docker ou do Podman instalado; o app liga o motor parado após confirmação, mas não o instala); o MariaDB no macOS e o MySQL/MariaDB no Linux ARM, sem pacote oficial verificável, são preparados em container, como o PostgreSQL. O SQL Server não é instalado direto no Windows (só em container), e a imagem funciona só em computadores x64. Para os formatos de migration reconhecidos (por enquanto, Laravel com banco neste computador ou SQLite dentro do projeto), **Iniciar** pede confirmação, mostra o progresso e só inicia o projeto após a conclusão. Falhas aparecem no cartão e em **Ver logs**. Migrations de Rails (`db:migrate`), Phoenix (`ecto.migrate`) e Django ainda não são reconhecidas. Bancos configurados só por `DATABASE_URL` ou por arquivos de Spring, Django e .NET ainda não são reconhecidos.
- **Formatos não cobertos:** C/C++ com Makefile ainda pede o comando de início no Windows, e no macOS e no Linux quando o Makefile não tem o alvo `run`, tem mais de uma regra `run`, usa `include`, ou quando a pasta tem mais de um Makefile (`Makefile` e `GNUmakefile`, por exemplo): o app não adivinha qual é o programa, e o cartão pede que você defina o comando de início. Projetos CMake, Meson ou Visual Studio com mais de um executável, e projetos Perl, Lua, R, Julia, Cabal ou Leiningen com mais de uma entrada possível (dois arquivos `.pl`, dois executáveis no `.cabal`) ou sem entrada também pedem, e o cartão explica o motivo. O app instala o CMake ou o Meson, mas não o compilador C/C++ (no Windows, o Visual Studio Build Tools com C++).
- **Containers:** o app não instala o Docker nem o Podman. Quando um deles está instalado e parado, **Iniciar** pergunta antes e liga o motor (no Linux, só o serviço do seu usuário; o serviço do sistema pede senha e continua com você).
- **Atualização pelo app:** é experimental até ser testada com uma versão mais nova. Durante a atualização, o app mostra as etapas Baixar, Verificar, Instalar e Reiniciar e a barra do download. Enquanto o app está fechado para atualizar, uma janela pequena, **Atualizando o LocalCodePilot**, acompanha a instalação e o reinício; fechar essa janela não interrompe a atualização, e o resultado aparece no app que reabre. Antes de baixar, ele confere a assinatura do projeto nos checksums da release e recusa a atualização se ela não conferir. Se algo falhar, o aviso diz o que aconteceu em linguagem simples, oferece **Tentar de novo** ou a página oficial da versão e guarda o erro original em **Detalhes técnicos**. Se houver projetos em execução, o app pergunta antes de fechar e encerrar os processos deles; **Agora não** mantém tudo rodando. O aviso também resume a novidade da versão numa linha ("O que há de novo: ..."), tirada das notas oficiais.

### Para desenvolvedores

O LocalCodePilot é escrito em Rust: um app desktop (`egui`) e uma CLI sobre o mesmo núcleo. Para compilar, você precisa do [rustup](https://rustup.rs/).

```powershell
cargo run                                   # abre o app desktop
cargo run -p localcodepilot-cli -- scan     # lista os projetos pela CLI
cargo test --workspace                      # testes
cargo clippy --workspace --all-targets -- -D warnings
```

O CI verifica advisories de segurança (RustSec) e licenças das dependências Rust com `cargo deny check advisories licenses`, conforme a política em [deny.toml](deny.toml). Para reproduzir localmente, instale `cargo-deny` e execute esse comando na raiz do repositório. Se a verificação falhar, consulte o pacote e o motivo no log do CI. Ela só aponta problemas já publicados no RustSec e não garante que as dependências estejam livres de falhas.

- **Como o app decide as coisas** (classificação, comandos, portas, containers, preparação do ambiente e atualização): [docs/DETALHES-TECNICOS.md](docs/DETALHES-TECNICOS.md).
- **Containers e conflitos de nome do Compose:** [docs/CONTAINERS.md](docs/CONTAINERS.md).
- **Bancos de dados:** [docs/DATABASES.md](docs/DATABASES.md).
- **Histórico de versões:** [CHANGELOG.md](CHANGELOG.md).
- **Contribuição:** antes de uma mudança grande, abra uma issue. Toda mudança de comportamento atualiza este README, o CHANGELOG e a Ajuda do app.

---

## English

### What it is

LocalCodePilot is an app for developers, from junior to expert, that finds the projects on your computer and takes care of running them for you:

- **Finds your projects on its own** and recognizes each one's language and framework.
- **Prepares what is missing:** installs the project's dependencies and, when the language itself is not installed, offers to install it with one click.
- **Starts the project, shows its address, and opens the site.** When something goes wrong (a port in use, a program that closed), it tells you why and offers **Try again**.
- **Also handles containers** (Docker and Podman), with protection so one project does not touch another project's containers.
- **Tests the project's database** with the project's own credentials and, on Windows, prepares the database server when it is missing (see [Databases](#databases)).

Everything runs on your computer. The app does not send your projects anywhere, and nothing is installed before you see the plan and confirm it.

### Download and install

**Windows 10/11 (64-bit).** On the [releases](https://github.com/cjulio1993/LocalCodePilot/releases) page:

- **Installer (`LocalCodePilotSetup-<version>-windows-x64.exe`; up to alpha 8, `...-windows-x64-setup.exe`), recommended.** Installs the app and adds a Start menu shortcut. When you use **Update now**, the app runs the new version's installer in the background, so the Windows program list shows the right version. The other `.exe` on the release (`LocalCodePilot-<version>-windows-x64.exe`) is the file the portable copy uses to update itself; do not run it directly. Run that way, the app cannot update itself, and **Baixar atualização** (download update) takes you to the installer.
- **Zip (portable).** Extract it and run `LocalCodePilot.exe`, no install needed.

**If you use Alpha 8 or Alpha 9:** the app announces the new version; use **Update now**. If the update does not complete, download the installer or zip from the [releases page](https://github.com/cjulio1993/LocalCodePilot/releases). On an Alpha 9 installed with the installer, **Update now** replaces only the executable, and the Windows program list keeps showing Alpha 9; to make the list show the right version, run the Alpha 10 installer once. Starting with Alpha 10, the update runs the official installer.

**If you use Alpha 7:** download the installer or zip. Alpha 7's **Update now** button does not complete the update.

**Windows: verify the file before opening it.** Current releases are not code signed, and SmartScreen may show "Windows protected your PC".

1. Download the installer or zip from the [official releases page](https://github.com/cjulio1993/LocalCodePilot/releases).
2. On the same release, open `SHA256SUMS.txt` and find the SHA-256 published for **the exact file** you downloaded.
3. Open PowerShell in the file's folder and run `Get-FileHash -Algorithm SHA256 -LiteralPath '.\FILE-NAME'`, replacing `FILE-NAME` with the name of the downloaded installer or zip.
4. Compare `Hash` with the SHA-256 on the release. If they differ, **do not run the file**: delete the download and download it again from the official releases page. If you cannot find the SHA-256 for that file, do not proceed.
5. Only if the values match and you trust the source, open the file. If SmartScreen appears, click **More info** and then **Run anyway**.

**macOS (experimental).** A universal package (Intel and Apple Silicon) is available for testing; validation on a real Mac is still in progress. See [docs/README-MACOS.txt](docs/README-MACOS.txt).

### Getting started

1. **Open the app.** **Projects** lists what it found in the usual folders. Use **Add folder** if your projects live elsewhere. The search does not go into folders of installed programs (such as `Program Files` and `%LOCALAPPDATA%\Programs`) unless you add one yourself.
2. **Pick a project.** Its card shows the project's state, what is missing to prepare it, and the next step. Dependencies not installed yet appear as **Parado** (stopped); failed installs or runs appear as **Com problema** (needs attention). The next step is:
   - **Prepare environment** when the language is missing;
   - **Install dependencies** when the project's libraries are missing;
   - **Start** when everything is ready.

   When two projects share a name, the card also shows the part of the folder that tells them apart.
3. **Click the button.** The first time a project command runs, check the folder and the command before you confirm. The confirmation comes back if the effective command changes. For installs and Compose actions, also check the plan shown; for container actions, the confirmation shows the folder and the engine (Docker or Podman).
4. **Use the project.** When the site answers, the card shows its address and **Open site**. **View logs** shows the program's output, and **Stop** ends everything the project opened. Saved logs stay on the computer for up to 30 days, with passwords and tokens in known formats replaced by `••••`; **Limpar histórico** (Clear history), in Settings, deletes them all. The live output stays complete: check it for personal data before pasting logs into a public report.

If your projects and settings ever cannot be read, the app keeps the files and restores the last valid copy on its own. Without a valid copy, it starts with the default settings and warns that some data may be missing; **Abrir pasta** (Open folder) shows the kept files, and the notice stays in Settings until you click **Entendi** (Got it). The project scan rebuilds only the project list, not favorites, history, custom commands or confirmations.

The **Help** page inside the app explains each state and what to do.

### Languages and projects

What happens when you click **Start**, validated on Windows 11:

| Project | The app starts it with | The app prepares |
| --- | --- | --- |
| Node.js / JavaScript (npm, pnpm, Yarn, Bun) | the development script (`dev`, `start`…) | the dependencies |
| PHP, Laravel, Lumen | PHP's server or `artisan serve` | Composer, `.env`, and the Laravel key |
| Python, Flask, Django | `app.py`/`main.py` or `runserver` | the virtual environment and dependencies |
| Java and Kotlin (Maven, Gradle, Spring Boot, Ktor) | the project's wrapper (`mvnw`, `gradlew`) | Gradle/Maven download the dependencies |
| .NET (ASP.NET, Blazor, console) | `dotnet run` | `dotnet` restores the packages |
| Ruby (Rails, Rack, Sinatra) | `bin/rails server`, `rackup`, or `ruby app.rb` | `bundle install` inside the project |
| Go | `go run` | Go downloads the modules |
| Rust | `cargo run` | Cargo downloads the dependencies |
| Dart and Flutter (web and Windows) | `dart run` or `flutter run` | the `pub` dependencies |
| Deno | the `deno.json` tasks | Deno downloads the dependencies |
| Elixir (Phoenix) | `mix phx.server` | Hex, Rebar, and `mix deps.get` |
| Swift | `swift run` | the Visual Studio environment on Windows (needs Build Tools with C++) |
| Scala (sbt, Play) | `sbt run` | sbt downloads the dependencies |
| Clojure (`deps.edn`) | `bb clojure -M:run` | **installs Babashka** if missing |
| Haskell (Stack) | `stack run` | **installs Stack**, downloads the compiler, and builds |
| Haskell (Cabal only, one executable) | `cabal run` | Cabal downloads the dependencies and builds |
| Clojure with Leiningen (`project.clj` with `:main`) | `lein run` | Leiningen downloads the dependencies |
| Perl, Lua, R, Julia | the entry file (`main.pl`, `main.lua`, `main.R`, `main.jl`… or the only file of that language in the folder); a Shiny `app.R` opens as a site | — |
| C/C++ with CMake, Meson or Visual Studio (`.vcxproj`, or a `.sln` with a single executable project, on Windows) | configures, builds, and runs the program; the build folder stays outside the project (Visual Studio builds with the installed MSBuild, in Debug) | **installs CMake or Meson** if missing; a C/C++ compiler must be installed |
| Zig | `zig build run` | **installs Zig** if missing |
| HTML sites and projects without a framework | a local server (or `java Main.java`, or PHP's server) | — |
| Containers (Docker, Podman, Compose) | `compose up`, with your choice of engine | does not install Docker/Podman; can turn on an installed engine after you confirm |
| C/C++ with a single Makefile that has a single `run` rule (macOS and Linux) | `make run`; the confirmation shows the rules it runs (the prerequisites of `run` and `run` itself) | `make` and a compiler must be installed |
| Other languages (C/C++ with a Makefile without a `run` target, or on Windows…) and projects with more than one possible entry | the command you define once; the card says why it is needed | — |

When a language is not installed, **Prepare environment** uses Windows' `winget`. Installation was validated end to end for the rows in bold. For Python, Java, PHP, Node.js, Go, and Dart, installation by the app is still being validated.

### Databases

LocalCodePilot reads the project's database settings from `.env` (`DB_CONNECTION`, `DB_HOST`, `DB_PORT`, `DB_DATABASE`, `DB_USERNAME`, `DB_PASSWORD`) or from `wp-config.php` and tests the connection with those credentials. The card shows **Banco de dados conectado.** (database connected) only after the database accepts the project's login; an open port or a running container is not enough. When the database keeps the project from starting, the project shows **Parado** (stopped) or **Com problema** (needs attention), with the reason in words.

| Situation | What the app does |
|---|---|
| PostgreSQL, MySQL or MariaDB on this computer, on **Windows** | **Start** prepares a server for this computer only (`127.0.0.1`). The first time, the confirmation **Preparar banco de dados para este projeto?** (prepare a database for this project?) shows the download, the disk space needed, and the database and user it will create. The package is a pinned version checked by SHA-256; the admin password is kept in Windows Credential Manager. |
| SQL Server on this computer, on **Windows** | With Docker or Podman ready, **Start** can create a SQL Server 2022 Developer container (free for development and testing). You must tick that you have read and accept Microsoft's license terms; the app never accepts them for you. Without Docker or Podman ready, the card leads to the **Containers** page. |
| A server you already run, on the project's port | Only the login is tested. The app does not create a database or user, does not change the server, and does not offer **Stop** for it. |
| A Compose service (such as `db`) | It stays in the **Containers** flow, with the same name-conflict protections. The card shows "Container ativo; conexão não verificada." (container running; connection not verified); no other server is prepared. |
| A database on another computer | Nothing is prepared or changed. **Testar conexão** (test connection), under **Detalhes técnicos** (technical details), tests the login only when you ask. |
| **macOS and Linux** | The project's login is tested for PostgreSQL, MySQL, MariaDB and SQL Server. **Iniciar** (Start) prepares MySQL (macOS 15+ and Linux x86_64) and MariaDB (Linux x86_64) from the official package, after you confirm. On macOS and Linux, **Iniciar** prepares a local PostgreSQL in a container with Docker or Podman, after you confirm: the official image is pinned by digest, the port is limited to 127.0.0.1, and the data stays in an engine volume. MariaDB on macOS and MySQL/MariaDB on ARM Linux are prepared in a container with Docker or Podman the same way, after you confirm. |
| SQLite with the file inside the project | The app creates the folder and the file when missing and checks its integrity. If it recognizes pending migrations of the project, it asks before running them and only then starts the project. |

**Where the data lives.** Servers prepared on Windows keep their data in `%LOCALAPPDATA%\LocalCodePilot\bancos` (on macOS and Linux, in the app's data folder); SQL Server, and PostgreSQL on macOS and Linux, keep it in a Docker or Podman volume. When the app closes, LocalCodePilot tries to stop the servers and containers it prepared, even during a first preparation. If one keeps running, it tells you at the next launch; the data stays and comes back on the next **Start**.

**Fixes.** When it is safe, the card offers a way out, always with a confirmation and a new login test before starting: **Usar outra porta** (use another port: changes only `DB_PORT` and keeps a copy of `.env`), **Corrigir conexão** (fix connection: only on LocalCodePilot's own server) or **Criar banco novo** (create a new database: the previous data is kept). **Detalhes técnicos** shows where the settings are, the address tested, the user, the database and the result; the password never appears.

More in [docs/DATABASES.md](docs/DATABASES.md).

### Limitations in this alpha

- **Platform:** end-to-end validation was done on Windows 11. The macOS package is experimental, and Linux has automated tests only.
- **Signing:** downloads are not digitally signed yet (see [Download and install](#download-and-install)).
- **Databases:** on macOS and Linux, preparing the server covers MySQL (macOS 15+ and Linux x86_64), MariaDB (Linux x86_64) and PostgreSQL in a container (Docker or Podman must be installed; the app turns a stopped engine on after you confirm, but does not install it); MariaDB on macOS and MySQL/MariaDB on ARM Linux, which have no verifiable official package, are prepared in a container, like PostgreSQL. SQL Server is not installed directly on Windows (container only), and its image runs only on x64 computers. For the migration formats it recognizes (for now, Laravel with a database on this computer or SQLite inside the project), **Iniciar** (Start) asks first, shows the progress and starts the project only after they finish. Failures show on the card and in **Ver logs** (View logs). Rails (`db:migrate`), Phoenix (`ecto.migrate`) and Django migrations are not recognized yet. Databases configured only through `DATABASE_URL` or Spring, Django and .NET files are not recognized yet.
- **Formats not covered:** C/C++ with a Makefile still asks for the start command on Windows, and on macOS and Linux when the Makefile has no `run` target, has more than one `run` rule, uses `include`, or when the folder has more than one Makefile (`Makefile` and `GNUmakefile`, for example): the app does not guess which one is the program, and the card asks you to define the start command. CMake, Meson or Visual Studio projects with more than one executable, and Perl, Lua, R, Julia, Cabal or Leiningen projects with more than one possible entry (two `.pl` files, two executables in the `.cabal`) or with none also ask, and the card says why. The app installs CMake or Meson, but not the C/C++ compiler (on Windows, Visual Studio Build Tools with C++).
- **Containers:** the app does not install Docker or Podman. When one of them is installed and stopped, **Iniciar** (Start) asks first and turns the engine on (on Linux, only your user's service; the system service asks for a password and stays with you).
- **In-app update:** experimental until tested against a newer release. While updating, the app shows the download, verify, install, and restart steps and a download progress bar. While the app is closed for the update, a small **Atualizando o LocalCodePilot** ("updating LocalCodePilot") window follows the installation and the restart; closing it does not stop the update, and the outcome shows in the app when it reopens. Before downloading, it checks the project's signature on the release checksums and refuses the update if it does not match. If something fails, the notice says what happened in plain language, offers **Tentar de novo** (try again) or the official release page, and keeps the original error under **Detalhes técnicos** (technical details). If projects are running, the app asks before closing and ending their processes; **Agora não** (not now) keeps everything running. The notice also sums up what is new in one line ("O que há de novo: ..."), taken from the official release notes.

### For developers

LocalCodePilot is written in Rust: a desktop app (`egui`) and a CLI on the same core. To build it, you need [rustup](https://rustup.rs/).

```powershell
cargo run                                   # opens the desktop app
cargo run -p localcodepilot-cli -- scan     # lists projects from the CLI
cargo test --workspace                      # tests
cargo clippy --workspace --all-targets -- -D warnings
```

CI checks Rust dependency security advisories (RustSec) and licenses with `cargo deny check advisories licenses`, following the policy in [deny.toml](deny.toml). To reproduce locally, install `cargo-deny` and run that command from the repository root. If the check fails, see the CI log for the package and reason. It only reports issues already published in RustSec and does not guarantee the dependencies are free of flaws.

- **How the app decides** (classification, commands, ports, containers, environment setup, and updates): [docs/DETALHES-TECNICOS.md](docs/DETALHES-TECNICOS.md).
- **Containers and Compose name conflicts:** [docs/CONTAINERS.md](docs/CONTAINERS.md).
- **Databases:** [docs/DATABASES.md](docs/DATABASES.md).
- **Release history:** [CHANGELOG.md](CHANGELOG.md).
- **Contributing:** before a large change, open an issue. Every behavior change updates this README, the CHANGELOG, and the in-app Help.

## License

MIT. See [LICENSE](LICENSE).

## Code signing policy

See [CODE_SIGNING_POLICY.md](CODE_SIGNING_POLICY.md).
