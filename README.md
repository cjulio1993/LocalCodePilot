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

Tudo roda no seu computador. O app não envia seus projetos para lugar nenhum, e nada é instalado sem você ver o plano e confirmar.

### Baixar e instalar

**Windows 10/11 (64 bits).** Na página de [releases](https://github.com/cjulio1993/LocalCodePilot/releases):

- **Instalador (`LocalCodePilotSetup-<versão>-windows-x64.exe`; até a alpha 8, `...-windows-x64-setup.exe`), recomendado.** Instala o app e cria o atalho no Menu Iniciar. Quando você usa **Atualizar agora**, o app roda o instalador da nova versão em segundo plano, e a lista de programas do Windows passa a mostrar a versão certa. O outro `.exe` da release (`LocalCodePilot-<versão>-windows-x64.exe`) é o arquivo que a cópia portátil usa para se atualizar.
- **Zip (portátil).** Extraia e execute `LocalCodePilot.exe`, sem instalar.

**Se você usa a Alpha 8:** o app avisa a nova versão; use **Atualizar agora**. Se a atualização não concluir, baixe o instalador ou o zip na [página de releases](https://github.com/cjulio1993/LocalCodePilot/releases).

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
| Zig | `zig build run` | **instala o Zig** se faltar |
| Sites HTML e projetos sem framework | um servidor local (ou `java Main.java`, ou o servidor do PHP) | — |
| Containers (Docker, Podman, Compose) | `compose up`, escolhendo o motor | não instala o Docker/Podman; mostra o passo a passo |
| Outras linguagens (C/C++, Perl, Lua, R, Julia…) | o comando que você definir uma vez | — |

Quando uma linguagem não está instalada, **Preparar ambiente** usa o `winget` do Windows. Nas linhas em negrito, a instalação foi validada de ponta a ponta. Para Python, Java, PHP, Node.js, Go e Dart, a instalação pelo app ainda está em validação.

### Limitações desta alpha

- **Plataforma:** a validação de ponta a ponta foi feita no Windows 11. No macOS o pacote é experimental, e no Linux há só testes automáticos.
- **Assinatura:** os downloads ainda não têm assinatura digital (veja [Baixar e instalar](#baixar-e-instalar)).
- **Bancos de dados:** projetos que precisam de banco (Rails com `db:prepare`, Phoenix com Ecto) ainda pedem esse passo manual. O erro aparece em **Ver logs**.
- **Formatos não cobertos:** Leiningen (`project.clj`) e Haskell só com Cabal ainda pedem o comando de início.
- **Containers:** o app não instala o Docker nem o Podman.
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

Everything runs on your computer. The app does not send your projects anywhere, and nothing is installed before you see the plan and confirm it.

### Download and install

**Windows 10/11 (64-bit).** On the [releases](https://github.com/cjulio1993/LocalCodePilot/releases) page:

- **Installer (`LocalCodePilotSetup-<version>-windows-x64.exe`; up to alpha 8, `...-windows-x64-setup.exe`), recommended.** Installs the app and adds a Start menu shortcut. When you use **Update now**, the app runs the new version's installer in the background, so the Windows program list shows the right version. The other `.exe` on the release (`LocalCodePilot-<version>-windows-x64.exe`) is the file the portable copy uses to update itself.
- **Zip (portable).** Extract it and run `LocalCodePilot.exe`, no install needed.

**If you use Alpha 8:** the app announces the new version; use **Update now**. If the update does not complete, download the installer or zip from the [releases page](https://github.com/cjulio1993/LocalCodePilot/releases).

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
| Zig | `zig build run` | **installs Zig** if missing |
| HTML sites and projects without a framework | a local server (or `java Main.java`, or PHP's server) | — |
| Containers (Docker, Podman, Compose) | `compose up`, with your choice of engine | does not install Docker/Podman; shows the steps |
| Other languages (C/C++, Perl, Lua, R, Julia…) | the command you define once | — |

When a language is not installed, **Prepare environment** uses Windows' `winget`. Installation was validated end to end for the rows in bold. For Python, Java, PHP, Node.js, Go, and Dart, installation by the app is still being validated.

### Limitations in this alpha

- **Platform:** end-to-end validation was done on Windows 11. The macOS package is experimental, and Linux has automated tests only.
- **Signing:** downloads are not digitally signed yet (see [Download and install](#download-and-install)).
- **Databases:** projects that need a database (Rails with `db:prepare`, Phoenix with Ecto) still need that manual step. The error shows in **View logs**.
- **Formats not covered:** Leiningen (`project.clj`) and Cabal-only Haskell projects still ask for the start command.
- **Containers:** the app does not install Docker or Podman.
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
