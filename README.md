# LocalCodePilot

> Um gerenciador local de projetos e ambientes de desenvolvimento, nativo e escrito em Rust.

> A native, Rust-based manager for local development projects and environments.

**Status: em construção / work in progress.** O LocalCodePilot ainda está em desenvolvimento inicial. Funcionalidades, interfaces e formatos internos podem mudar antes da primeira versão estável.

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
3. Ignora pastas de dependências, builds, controle de versão e ambientes virtuais.
4. Reconhece projetos por arquivos como `Cargo.toml`, `package.json`, `composer.json`, `pyproject.toml` e `requirements.txt`.
5. Detecta os runtimes encontrados e monta um catálogo compartilhado pelo desktop e pela CLI.

Diretórios como `.git`, `node_modules`, `target`, `vendor`, `dist`, `build`, `.venv` e `venv` não são examinados. A profundidade da busca também é limitada para evitar varreduras excessivas.

### O que já funciona

- Aplicação desktop nativa em Rust.
- Dashboard com projetos descobertos automaticamente.
- Varredura de diretórios executada fora da thread da interface.
- Detecção inicial de Rust, Node.js, PHP e Python.
- Busca de projetos no catálogo.
- Prevenção de caminhos duplicados.
- Informações básicas do sistema e uso de memória.
- CLI com comandos `status`, `scan` e `inspect`.
- Interface com ícones Phosphor.
- Estrutura modular baseada em Cargo workspace.

### Em desenvolvimento

- Página completa de gerenciamento de projetos.
- Detecção de frameworks e metadados mais detalhados.
- Gerenciamento real de processos e portas.
- Inicialização e controle de serviços como MySQL, PostgreSQL e Redis.
- Configuração dos diretórios usados pela descoberta.
- Persistência do catálogo e das preferências.
- Monitoramento contínuo de alterações no filesystem.
- Assistente para criar novos projetos.
- Melhorias de acessibilidade, responsividade e experiência de uso.

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
3. Skips dependency, build, version-control, and virtual-environment directories.
4. Recognizes projects through files such as `Cargo.toml`, `package.json`, `composer.json`, `pyproject.toml`, and `requirements.txt`.
5. Detects the available runtimes and builds a catalog shared by the desktop and CLI.

Directories such as `.git`, `node_modules`, `target`, `vendor`, `dist`, `build`, `.venv`, and `venv` are not scanned. Search depth is also limited to prevent unnecessarily broad filesystem scans.

### What already works

- Native Rust desktop application.
- Dashboard populated through automatic project discovery.
- Directory scanning outside the UI thread.
- Initial detection for Rust, Node.js, PHP, and Python.
- Project catalog search.
- Duplicate-path prevention.
- Basic system and memory information.
- CLI commands for `status`, `scan`, and `inspect`.
- Phosphor icons in the desktop interface.
- Modular Cargo workspace architecture.

### Work in progress

- Complete project-management page.
- Framework detection and richer project metadata.
- Actual process and port management.
- Starting and controlling services such as MySQL, PostgreSQL, and Redis.
- Configurable project discovery locations.
- Catalog and preference persistence.
- Continuous filesystem change monitoring.
- New-project creation assistant.
- Accessibility, responsive layout, and user-experience improvements.

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
