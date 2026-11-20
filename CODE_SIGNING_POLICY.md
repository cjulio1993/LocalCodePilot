# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).

> **Status:** LocalCodePilot has applied to the SignPath Foundation program. Until the first signed release, Windows binaries are not Authenticode-signed. Every release still publishes `SHA256SUMS.txt` with an Ed25519 signature (`SHA256SUMS.txt.sig`), which the app checks before installing an update.

## What is signed

Only binaries built from this repository by its automated release workflow (`.github/workflows/release-windows.yml`, GitHub Actions) are signed:

- `LocalCodePilot.exe`: the application (inside the portable zip and as the in-app update executable);
- `LocalCodePilotSetup-<version>-windows-x64.exe`: the Windows installer (per-user, with uninstaller).

No binary built on a developer machine is signed. Third-party binaries are never signed with this certificate.

## Team roles

- **Committers and reviewers:** [Júlio César (@cjulio1993)](https://github.com/cjulio1993). Changes from people who are not committers are reviewed in a pull request before merge.
- **Approvers:** [Júlio César (@cjulio1993)](https://github.com/cjulio1993). Every signing request is approved manually before a release is published.

All team members use multi-factor authentication for GitHub and for SignPath.

## Privacy policy

This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it.

In practice, LocalCodePilot:

- checks GitHub Releases for a newer version;
- downloads an update only when the user chooses **Atualizar agora** (Update now);
- opens links and local addresses only when the user clicks them;
- runs commands and installs language toolchains (with winget) only after the user confirms the plan shown on screen;
- sends no telemetry and no usage data.

## Política de assinatura de código (resumo em português)

A assinatura de código gratuita é fornecida pelo [SignPath.io](https://about.signpath.io), com certificado da [SignPath Foundation](https://signpath.org).

- **Situação:** o LocalCodePilot se candidatou ao programa. Até a primeira release assinada, os binários do Windows não têm assinatura Authenticode. Cada release continua publicando o `SHA256SUMS.txt` com assinatura Ed25519, conferida pelo app antes de instalar uma atualização.
- **O que é assinado:** só os binários gerados por este repositório no workflow automatizado de release: o `LocalCodePilot.exe` e o instalador `LocalCodePilotSetup-<versão>-windows-x64.exe`. Cada assinatura é aprovada manualmente pelo mantenedor.
- **Privacidade:** o programa não envia informações a outros sistemas, a não ser quando o usuário pede. Ele consulta as releases do GitHub para saber se há versão nova e não envia telemetria.
