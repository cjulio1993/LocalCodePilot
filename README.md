# LocalCodePilot

Aplicação nativa escrita integralmente em Rust. O domínio é independente das interfaces.

## Arquitetura

- `core`: projetos, runtimes, processos, portas e ambientes; não depende de UI ou SO.
- `platform`: integração nativa com Windows, Linux e macOS, incluindo varredura segura do filesystem.
- `runtime`: detecção de Rust, Node, PHP e Python.
- `services`: modelos de MySQL, PostgreSQL e Redis.
- `cli`: interface de terminal baseada no core.
- `desktop`: interface `egui`/`eframe` baseada no mesmo core.

## Executar

```powershell
cargo run -p localcodepilot-desktop
```

## Verificar e gerar uma versão otimizada

```powershell
cargo test
cargo build --release
```

## CLI

```powershell
cargo run -p localcodepilot-cli -- status
cargo run -p localcodepilot-cli -- scan
cargo run -p localcodepilot-cli -- inspect .
```
