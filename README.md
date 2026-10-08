# LocalCodePilot

<p align="center">
  <img src="assets/branding/logo.svg" alt="LocalCodePilot" width="600">
</p>

> Uma plataforma local para descobrir, executar e organizar projetos, processos e ferramentas em um só lugar.

> A local platform to discover, run, and organize projects, processes, and tools in one place.

**Status: alpha pública / public alpha.** O LocalCodePilot ainda está em desenvolvimento. Funcionalidades, interfaces e formatos internos podem mudar antes da primeira versão estável.

**Alpha para Windows:** a versão `0.4.0-alpha.1` é distribuída como ZIP portátil para Windows 10/11 x64 na página de [releases](https://github.com/cjulio1993/LocalCodePilot/releases). Extraia o arquivo e execute `LocalCodePilot.exe`. A versão ainda não possui assinatura digital; confira o SHA-256 publicado junto ao download.

O processo de assinatura Authenticode e os segredos opcionais do pipeline estão documentados em [docs/SIGNING-WINDOWS.md](docs/SIGNING-WINDOWS.md).

[Português](#português) · [English](#english)

---

## Português

### O que é o LocalCodePilot?

O LocalCodePilot pretende reunir os projetos, runtimes, processos, portas e serviços de desenvolvimento da máquina em um único lugar. Em vez de exigir que cada projeto seja cadastrado manualmente, a aplicação procura projetos automaticamente e identifica suas tecnologias por meio dos arquivos presentes em cada diretório.

O projeto oferece duas interfaces sobre o mesmo núcleo:

- Uma aplicação desktop nativa, construída com `egui` e `eframe`.
- Uma CLI para descoberta, inspeção e automação pelo terminal.

A interface não é o produto inteiro. As regras de domínio ficam em um core independente, que poderá ser reutilizado futuramente por desktop, CLI, daemon ou integrações remotas.

### Como funciona

Ao iniciar a aplicação, o LocalCodePilot:

1. Consulta locais comuns de projetos na máquina.
2. Percorre esses diretórios em segundo plano.
3. Ignora pastas de dependências, builds, metadados de arquivos compactados e ambientes virtuais.
4. Reconhece a raiz de repositórios Git, projetos com manifesto e pastas locais com arquivos de código.
5. Detecta os runtimes e comandos disponíveis e monta um catálogo compartilhado pelo desktop e pela CLI.

Diretórios como `.git`, `node_modules`, `target`, `vendor`, `dist`, `build`, `.venv` e `venv` não são examinados. A profundidade da busca também é limitada para evitar varreduras excessivas.

### O que já funciona

- Aplicação desktop nativa em Rust.
- Dashboard com projetos descobertos automaticamente.
- Varredura de diretórios executada fora da thread da interface.
- Detecção de Rust, Node.js, PHP, Python e Java.
- Reconhecimento experimental de Go, Dart, Flutter e sites estáticos.
- Reconhecimento de projetos locais sem Git ou manifesto por extensões de código.
- Identificação da raiz de repositórios e suporte inicial a monorepos.
- Busca de projetos no catálogo.
- Prevenção de caminhos duplicados.
- Configuração, adição, remoção e persistência das pastas de busca.
- Três projetos modificados mais recentemente na visão geral.
- Abertura da pasta de um projeto no Visual Studio Code.
- Informações básicas do sistema e uso de memória.
- Iniciais do usuário do computador geradas automaticamente na interface.
- Cartões de projeto responsivos ao espaço disponível na janela.
- CLI com comandos `status`, `scan` e `inspect`.
- Interface com ícones Phosphor.
- Estrutura modular baseada em Cargo workspace.

### Matriz de suporte atual

Reconhecer um projeto não significa que todo o fluxo de preparação, execução e abertura já esteja concluído. A matriz abaixo descreve o estado real da versão alpha:

| Ecossistema | Reconhecimento | Preparação assistida no Windows | Execução e abertura |
| --- | --- | --- | --- |
| Rust / Cargo | Sim | Não | Experimental |
| Node.js / JavaScript | Sim | Experimental | Experimental |
| PHP / Laravel | Sim | Experimental | Experimental com `php -S` ou Artisan, conforme o projeto |
| Python / Django | Sim | Experimental | Experimental |
| Java / Spring Boot | Sim | Experimental | Experimental |
| Go | Experimental | Implementação inicial não validada | Ainda não suportada de ponta a ponta |
| Dart | Experimental | Implementação inicial não validada | Ainda não suportada de ponta a ponta |
| Flutter | Experimental | Não | Ainda não suportada de ponta a ponta |
| Site estático / JavaScript sem framework | Experimental | Requer um servidor local disponível | Experimental |

Go, Dart e Flutter aparecem no diagnóstico porque seus arquivos e manifests já podem ser reconhecidos. Isso ainda não deve ser interpretado como suporte funcional para iniciar e abrir esses aplicativos.

### Classificação e execução

- Manifests e arquivos característicos do framework têm prioridade sobre extensões de arquivos isoladas.
- Um projeto PHP com HTML ou arquivos públicos continua sendo classificado como PHP e deve usar um servidor PHP, não o servidor de site estático.
- Um site estático só é criado a partir da raiz que contém o documento de entrada e quando não existe um backend ou framework com maior prioridade.
- Pastas internas como `public`, `web`, `dist` e `build` não devem virar projetos independentes quando pertencem a outra aplicação.
- Serviços web exibem uma URL quando ela é confirmada; aplicações de terminal devem permanecer acompanhadas pelos logs.
- Portas de sites estáticos são derivadas do caminho do projeto para reduzir conflitos e impedir que a URL de outro projeto seja reutilizada.
- Aplicações relacionadas que vivem em repositórios ou diretórios separados ainda são tratadas separadamente. A identificação automática de microsserviços pertencentes ao mesmo ambiente ainda está em desenvolvimento.

### Preparação assistida do ambiente (experimental)

- O diagnóstico separa runtime ausente, versão incompatível, gerenciador de pacotes ausente, dependências do projeto e extensões PHP requeridas.
- Antes de executar uma instalação, a interface apresenta um plano com comando, origem do pacote, versão pretendida, necessidade de administrador e possíveis alterações no `PATH`.
- A execução só começa depois da confirmação do usuário e mantém logs e resultado para consulta.
- No Windows, existem provedores iniciais para instalar Node.js, Python, PHP e Java com `winget`. Os provedores de Go e Dart ainda não foram validados de ponta a ponta.
- O fluxo de PHP pode preparar o Composer por meio do instalador oficial verificado e diagnosticar extensões requeridas pelo projeto.
- O fluxo de Python pode criar um ambiente virtual local antes de instalar as dependências.
- npm, pnpm, Yarn, Bun, Composer e ferramentas de dependência Python são escolhidos de acordo com manifests, lockfiles e disponibilidade local.
- A compatibilidade de versão segue a restrição declarada pelo projeto; uma versão numericamente maior não é aceita automaticamente quando o intervalo não permite isso.
- Por exemplo, Python 3.13.5 é anterior ao requisito 3.14. Já PHP 8.5 só atende a um projeto PHP 8.4 quando a restrição do projeto também aceita a série 8.5.
- Depois da instalação, o ambiente é verificado novamente em vez de ser considerado pronto apenas porque o comando terminou.

### Experimental e em testes

- Inicialização e interrupção do ambiente completo de cada projeto com um clique.
- Verificação de runtimes e dependências antes de iniciar um ambiente.
- Instalação das dependências ausentes com um clique, usando o gerenciador detectado.
- Logs em tempo real da instalação, com diagnóstico e sugestão de correção quando ela falha.
- Migração assistida de versões legadas do Laravel como fluxo de recuperação, com simulação isolada, confirmação e backup dos manifests.
- Seleção automática entre npm, pnpm, Yarn e Bun por manifesto ou lockfile.
- Ação para abrir a aplicação quando uma URL local é identificada nos logs.
- Projetos e serviços ativos priorizados na tela de processos, com visualizador de logs em formato de terminal.
- Reinício de processos, limpeza da saída e abertura de URLs locais identificadas nos logs.
- Detecção da porta esperada e seleção automática da próxima porta livre para Laravel, Django, PHP e servidores Node.js compatíveis.
- Exibição da porta ajustada na interface e nos logs do processo; comandos que não permitem uma troca segura continuam bloqueados.
- Confirmação de que a porta ficou disponível antes de marcar o serviço como executando.
- Monitoramento da porta durante a execução, com timeout, detecção de queda e sugestões baseadas nos logs.
- Persistência local de favoritos, projetos recentes, portas, URLs, resultados e dos 200 logs mais recentes por serviço.
- Ação **Retomar ambiente** após reabrir o aplicativo, sem reiniciar processos automaticamente.
- Verificação diária de novas releases, com suporte ao canal alpha e acesso ao download oficial.
- Identificação do processo conflitante no Windows, com confirmação antes de encerrá-lo.
- Detecção de comandos do Cargo, npm, Composer, PHP, Laravel, Django, Python e Java, além do reconhecimento inicial de Go, Dart e Flutter.
- Inicialização e interrupção manual de processos pela interface.
- Exibição de estado, PID e saída básica dos processos.
- Execução de comandos na pasta correta de módulos em monorepos.
- Encerramento dos processos gerenciados ao fechar o aplicativo.

O gerenciamento de processos ainda está em fase inicial. Antes de usá-lo em projetos importantes, confira o comando e a pasta de execução apresentados na interface. O tratamento de árvores de processos fora do Windows ainda não está finalizado.

Nos cartões de projeto e na tela **Processos**, o LocalCodePilot identifica comandos a partir dos arquivos do projeto e mostra se o ambiente está pronto, sem runtime ou sem dependências. Nos ecossistemas já integrados, **Instalar dependências** executa em segundo plano o gerenciador detectado. Antes da confirmação, o usuário pode conferir o comando, a origem do pacote, a versão, a necessidade de administrador e possíveis mudanças no `PATH`. **Iniciar ambiente** sobe os serviços classificados como executáveis, e **Abrir aplicação** só deve ser oferecido quando existir uma URL local válida. Como esta área ainda é experimental, confira sempre o comando e a pasta de execução mostrados na interface.

### Limitações conhecidas

- Go, Dart e Flutter são reconhecidos, mas ainda não possuem fluxo confiável de execução e abertura de ponta a ponta.
- A associação automática entre frontend, API, worker e outros microsserviços de um mesmo produto ainda não foi implementada.
- A abertura automática no navegador depende de uma URL confirmada e ainda precisa de mais testes de regressão.
- A classificação de PHP, JavaScript e sites estáticos está sendo ampliada com casos reais; projetos ambíguos devem ter o comando conferido antes da execução.
- Instalação de runtime, alterações no `PATH` e comandos que exigem administrador dependem de confirmação explícita do usuário.

### Em desenvolvimento

- Detecção de frameworks e metadados mais detalhados.
- Modelo de ambiente composto para agrupar frontend, API, workers e microsserviços relacionados.
- Suporte de execução e abertura de ponta a ponta para Go, Dart e Flutter.
- Mais fixtures e testes de regressão para a classificação de PHP, JavaScript e sites estáticos.
- Verificações HTTP de saúde e tempo de resposta dos serviços.
- Inicialização e controle de serviços como MySQL, PostgreSQL e Redis.
- Aperfeiçoamento do histórico de instalações e migrações assistidas.
- Atualização automática do executável após a adoção de assinatura digital e instalador seguro.
- Monitoramento contínuo de alterações no filesystem.
- Assistente para criar novos projetos.
- Melhorias de acessibilidade e experiência de uso.
- Integração opcional com Docker e Podman sem substituir instalações locais.
- Instalador e validação completa no macOS após a estabilização do fluxo local e da integração com contêineres.

### Arquitetura

```text
LocalCodePilot
├── core/       # domínio: projetos, runtimes, processos, portas e ambientes
├── platform/   # filesystem e integrações específicas do sistema operacional
├── runtime/    # detecção de runtimes e tecnologias
├── services/   # modelos e futuras integrações com serviços locais
├── cli/        # interface de linha de comando
└── desktop/    # interface gráfica nativa
```

O fluxo de dependências mantém o domínio independente:

```text
Desktop ─┐
CLI ─────┼──> Core
Daemon ──┤      ↑
Cloud ───┘      ├── Runtime
                ├── Platform
                └── Services
```

O `core` não depende de interface gráfica nem de APIs específicas do sistema operacional. `platform`, `runtime` e `services` implementam capacidades consumidas pelas interfaces externas.

### Pré-requisitos

- Rust e Cargo instalados por meio do [rustup](https://rustup.rs/).
- Toolchain compatível com Rust 2024.
- Dependências nativas exigidas pelo `eframe` na plataforma utilizada.

Confira a instalação:

```powershell
rustc --version
cargo --version
```

### Executar o desktop

Na raiz do repositório:

```powershell
cargo run
```

O desktop é o membro padrão do workspace. A forma explícita equivalente é:

```powershell
cargo run -p localcodepilot-desktop
```

### Usar a CLI

Mostrar informações do ambiente:

```powershell
cargo run -p localcodepilot-cli -- status
```

Procurar projetos automaticamente:

```powershell
cargo run -p localcodepilot-cli -- scan
```

Inspecionar um diretório específico:

```powershell
cargo run -p localcodepilot-cli -- inspect .
```

### Desenvolvimento e qualidade

```powershell
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Para verificar alterações automaticamente sem reiniciar a janela desktop:

```powershell
cargo watch -x "check -p localcodepilot-desktop"
```

### Build de produção

```powershell
cargo build --release --workspace
```

No Windows, os executáveis são gerados em `target\release\`.

### Contribuição

O projeto ainda está definindo suas APIs e seus fluxos principais. Antes de implementar uma funcionalidade grande, abra uma issue ou descreva claramente a proposta para que ela possa ser alinhada à separação entre domínio, plataforma e interfaces.

Ao enviar alterações, execute a formatação, os testes e o Clippy apresentados acima.

---

## English

**Windows alpha:** version `0.4.0-alpha.1` is distributed as a portable ZIP for Windows 10/11 x64 on the [releases page](https://github.com/cjulio1993/LocalCodePilot/releases). Extract it and run `LocalCodePilot.exe`. This alpha is not digitally signed yet; verify the SHA-256 published with the download.

The Authenticode signing process and optional pipeline secrets are documented in [docs/SIGNING-WINDOWS.md](docs/SIGNING-WINDOWS.md).

### What is LocalCodePilot?

LocalCodePilot aims to bring the machine's development projects, runtimes, processes, ports, and services together in one place. Instead of requiring every project to be registered manually, the application automatically searches for projects and identifies their technologies from the files found in each directory.

The project provides two interfaces powered by the same core:

- A native desktop application built with `egui` and `eframe`.
- A CLI for discovery, inspection, and terminal automation.

The user interface is not the whole product. Domain rules live in an independent core that may later power the desktop, CLI, a daemon, or remote integrations.

### How it works

When the application starts, LocalCodePilot:

1. Resolves common project locations on the machine.
2. Scans those directories in the background.
3. Skips dependency, build, archive-metadata, and virtual-environment directories.
4. Recognizes Git repository roots, manifest-based projects, and local folders containing source files.
5. Detects available runtimes and commands and builds a catalog shared by the desktop and CLI.

Directories such as `.git`, `node_modules`, `target`, `vendor`, `dist`, `build`, `.venv`, and `venv` are not scanned. Search depth is also limited to prevent unnecessarily broad filesystem scans.

### What already works

- Native Rust desktop application.
- Dashboard populated through automatic project discovery.
- Directory scanning outside the UI thread.
- Detection for Rust, Node.js, PHP, Python, and Java.
- Experimental recognition of Go, Dart, Flutter, and static sites.
- Local-project detection without Git or manifests based on source-file extensions.
- Repository-root identification and initial monorepo support.
- Project catalog search.
- Duplicate-path prevention.
- Configurable, removable, and persistent project discovery locations.
- Three most recently modified projects on the overview page.
- Opening project folders in Visual Studio Code.
- Basic system and memory information.
- Computer-user initials generated automatically in the interface.
- Project cards that adapt to the available window space.
- CLI commands for `status`, `scan`, and `inspect`.
- Phosphor icons in the desktop interface.
- Modular Cargo workspace architecture.

### Current support matrix

Recognizing a project does not mean that its preparation, execution, and browser-opening workflow is complete. This matrix describes the actual alpha status:

| Ecosystem | Recognition | Assisted Windows setup | Execution and opening |
| --- | --- | --- | --- |
| Rust / Cargo | Yes | No | Experimental |
| Node.js / JavaScript | Yes | Experimental | Experimental |
| PHP / Laravel | Yes | Experimental | Experimental with `php -S` or Artisan, depending on the project |
| Python / Django | Yes | Experimental | Experimental |
| Java / Spring Boot | Yes | Experimental | Experimental |
| Go | Experimental | Initial implementation, not validated | Not yet supported end to end |
| Dart | Experimental | Initial implementation, not validated | Not yet supported end to end |
| Flutter | Experimental | No | Not yet supported end to end |
| Static site / frameworkless JavaScript | Experimental | Requires an available local server | Experimental |

Go, Dart, and Flutter appear in diagnostics because their files and manifests can already be recognized. This must not be interpreted as functional support for starting and opening those applications.

### Classification and execution

- Framework manifests and characteristic files take precedence over isolated file extensions.
- A PHP project containing HTML or public files remains a PHP project and should use a PHP server, not the static-site server.
- A static site is created only from the root containing its entry document and when no higher-priority backend or framework is present.
- Internal directories such as `public`, `web`, `dist`, and `build` should not become independent projects when they belong to another application.
- Web services expose a URL after it is confirmed; terminal applications should remain observable through their logs.
- Static-site ports are derived from the project path to reduce conflicts and prevent another project's URL from being reused.
- Related applications stored in separate repositories or directories are still handled independently. Automatic grouping of microservices into one environment is still under development.

### Assisted environment setup (experimental)

- Diagnostics distinguish a missing runtime, an incompatible version, a missing package manager, project dependencies, and required PHP extensions.
- Before an installation runs, the interface presents a plan containing the command, package source, target version, administrator requirement, and possible `PATH` changes.
- Execution starts only after user confirmation, and its logs and result remain available for inspection.
- On Windows, initial providers can install Node.js, Python, PHP, and Java through `winget`. The Go and Dart providers have not been validated end to end.
- The PHP flow can prepare Composer through its verified official installer and diagnose extensions required by the project.
- The Python flow can create a local virtual environment before installing dependencies.
- npm, pnpm, Yarn, Bun, Composer, and Python dependency tools are selected from manifests, lockfiles, and local availability.
- Version compatibility follows the constraint declared by the project; a numerically newer version is not automatically accepted when the allowed range excludes it.
- For example, Python 3.13.5 is older than a 3.14 requirement. PHP 8.5 only satisfies a PHP 8.4 project when that project's constraint also allows the 8.5 series.
- After installation, the environment is checked again instead of being considered ready merely because the command exited.

### Experimental and under testing

- One-click startup and shutdown for each project's complete environment.
- Runtime and dependency checks before an environment starts.
- One-click installation of missing dependencies with the detected package manager.
- Live installation logs with diagnostics and a suggested correction when installation fails.
- Assisted migration for legacy Laravel versions as a recovery flow, with an isolated preview, confirmation, and manifest backups.
- Automatic selection among npm, pnpm, Yarn, and Bun from manifests and lockfiles.
- An action to open the application when a local URL is found in its logs.
- Active projects and services prioritized on the processes page, with a terminal-style log viewer.
- Process restart, output clearing, and opening local URLs found in logs.
- Expected-port detection and automatic selection of the next free port for compatible Laravel, Django, PHP, and Node.js servers.
- The adjusted port is shown in the interface and process logs; commands that cannot be safely changed remain blocked.
- A service remains in the starting state until its port becomes available.
- Port monitoring during execution, with startup timeout, service-loss detection, and log-based suggestions.
- Local persistence for favorites, recent projects, ports, URLs, results, and the latest 200 log lines per service.
- A **Resume environment** action after reopening the app, without restarting processes automatically.
- Daily release checks with alpha-channel support and access to the official download.
- Conflicting-process identification on Windows, with confirmation before termination.
- Command detection for Cargo, npm, Composer, PHP, Laravel, Django, Python, and Java, plus initial Go, Dart, and Flutter recognition.
- Manual process start and stop controls in the desktop interface.
- Process state, PID, and basic output display.
- Commands launched from the correct module directory in monorepos.
- Managed-process termination when the application closes.

Process management is still at an early stage. Before using it with important projects, verify the command and working directory shown in the interface. Process-tree handling outside Windows is not finished yet.

On project cards and the **Processos** page, LocalCodePilot identifies commands from project files and reports whether the environment is ready, missing a runtime, or missing dependencies. For integrated ecosystems, **Instalar dependências** runs the detected package manager in the background. Before confirmation, users should be able to inspect the command, package source, version, administrator requirement, and possible `PATH` changes. **Iniciar ambiente** starts services classified as executable, and **Abrir aplicação** should only be offered when a valid local URL exists. Because this area is still experimental, always verify the displayed command and working directory.

### Known limitations

- Go, Dart, and Flutter are recognized but do not yet have a reliable end-to-end execution and opening workflow.
- Automatic association among a product's frontend, API, workers, and other microservices is not implemented yet.
- Automatic browser opening depends on a confirmed URL and needs broader regression testing.
- PHP, JavaScript, and static-site classification is being expanded with real projects; verify the command before running ambiguous projects.
- Runtime installation, `PATH` changes, and administrator-level commands require explicit user confirmation.

### Work in progress

- Framework detection and richer project metadata.
- A composed-environment model for grouping related frontends, APIs, workers, and microservices.
- End-to-end execution and opening support for Go, Dart, and Flutter.
- More fixtures and regression tests for PHP, JavaScript, and static-site classification.
- HTTP health checks and service response-time monitoring.
- Starting and controlling services such as MySQL, PostgreSQL, and Redis.
- Improvements to installation and assisted-migration history.
- Automatic executable updates after code signing and a secure installer are available.
- Continuous filesystem change monitoring.
- New-project creation assistant.
- Accessibility and user-experience improvements.
- Optional Docker and Podman integration without replacing local installations.
- A macOS installer and complete validation after the local workflow and container integration are stabilized.

### Architecture

```text
LocalCodePilot
├── core/       # domain: projects, runtimes, processes, ports, environments
├── platform/   # filesystem and operating-system integrations
├── runtime/    # runtime and technology detection
├── services/   # models and future local-service integrations
├── cli/        # command-line interface
└── desktop/    # native graphical interface
```

Dependencies point inward and keep the domain independent:

```text
Desktop ─┐
CLI ─────┼──> Core
Daemon ──┤      ↑
Cloud ───┘      ├── Runtime
                ├── Platform
                └── Services
```

The `core` does not depend on the graphical interface or operating-system APIs. `platform`, `runtime`, and `services` provide capabilities consumed by the outer interfaces.

### Requirements

- Rust and Cargo installed through [rustup](https://rustup.rs/).
- A toolchain compatible with Rust 2024.
- Native dependencies required by `eframe` on the target platform.

Verify the installation:

```powershell
rustc --version
cargo --version
```

### Run the desktop application

From the repository root:

```powershell
cargo run
```

The desktop is the workspace's default member. The explicit equivalent is:

```powershell
cargo run -p localcodepilot-desktop
```

### Use the CLI

Display local environment information:

```powershell
cargo run -p localcodepilot-cli -- status
```

Automatically discover projects:

```powershell
cargo run -p localcodepilot-cli -- scan
```

Inspect a specific directory:

```powershell
cargo run -p localcodepilot-cli -- inspect .
```

### Development and quality checks

```powershell
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

To check changes automatically without restarting the desktop window:

```powershell
cargo watch -x "check -p localcodepilot-desktop"
```

### Production build

```powershell
cargo build --release --workspace
```

On Windows, binaries are generated under `target\release\`.

### Contributing

The project is still defining its APIs and main workflows. Before implementing a large feature, open an issue or clearly describe the proposal so it can be aligned with the separation between domain, platform, and interfaces.

Before submitting changes, run the formatting, testing, and Clippy commands shown above.

---

## License

This project is licensed under the terms of the [MIT License](LICENSE).
