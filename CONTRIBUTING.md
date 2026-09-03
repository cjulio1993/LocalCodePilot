# Contribuindo / Contributing

Obrigado por considerar uma contribuição ao LocalCodePilot. O projeto ainda está em construção; APIs, arquitetura e escopo podem mudar antes da primeira versão estável.

Thank you for considering a contribution to LocalCodePilot. The project is still under construction; APIs, architecture, and scope may change before the first stable release.

## Antes de começar / Before you start

- Procure uma issue existente antes de abrir outra.
- Discuta mudanças grandes, novas dependências e alterações arquiteturais em uma issue antes de implementá-las.
- Mantenha o `core` independente de UI e APIs específicas do sistema operacional.
- Não inclua credenciais, tokens, arquivos `.env` ou dados pessoais.

---

- Search for an existing issue before creating a new one.
- Discuss large changes, new dependencies, and architectural changes in an issue before implementing them.
- Keep `core` independent from UI and operating-system-specific APIs.
- Never include credentials, tokens, `.env` files, or personal data.

## Fluxo / Workflow

1. Faça um fork do repositório.
2. Crie uma branch a partir de `main`, como `feat/project-details` ou `fix/scanner-permissions`.
3. Faça alterações pequenas e focadas.
4. Adicione ou atualize testes quando o comportamento mudar.
5. Execute todas as verificações locais.
6. Abra um pull request explicando motivação, implementação e forma de teste.

---

1. Fork the repository.
2. Create a branch from `main`, such as `feat/project-details` or `fix/scanner-permissions`.
3. Keep changes small and focused.
4. Add or update tests whenever behavior changes.
5. Run every local check.
6. Open a pull request explaining its motivation, implementation, and test procedure.

## Verificações obrigatórias / Required checks

```powershell
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Commits

Use mensagens objetivas, preferencialmente no formato Conventional Commits:

```text
feat: adicionar detalhes do projeto
fix: ignorar diretórios sem permissão
docs: explicar descoberta automática
refactor: separar estado da página de projetos
test: cobrir detecção de runtimes
```

Use clear commit messages, preferably following Conventional Commits.

## Pull requests

Um pull request pode ser recusado ou solicitar mudanças quando estiver fora do escopo, quebrar a separação arquitetural, não possuir testes suficientes ou aumentar a manutenção sem benefício proporcional.

A pull request may be declined or require changes when it is out of scope, breaks architectural boundaries, lacks sufficient tests, or increases maintenance without proportional benefit.

## Governança / Governance

O LocalCodePilot é atualmente mantido por Julio Cesar (`@cjulio1993`). Contribuições são bem-vindas, mas abrir uma issue ou pull request não garante sua incorporação. Decisões finais sobre roadmap, arquitetura, releases e manutenção pertencem ao mantenedor.

LocalCodePilot is currently maintained by Julio Cesar (`@cjulio1993`). Contributions are welcome, but opening an issue or pull request does not guarantee acceptance. Final decisions regarding roadmap, architecture, releases, and maintenance belong to the maintainer.

