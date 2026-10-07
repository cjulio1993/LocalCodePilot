# Changelog

Todas as mudanças relevantes do LocalCodePilot serão registradas neste arquivo.

O projeto usa [Versionamento Semântico](https://semver.org/lang/pt-BR/) a partir das versões públicas.

## [Não publicado]

### Adicionado

- Detecção da porta esperada para Laravel, Django, PHP e servidores Node.js comuns.
- Verificação de conflitos de porta antes de iniciar um serviço ou ambiente.
- Seleção automática da próxima porta livre para comandos compatíveis, com ajuste seguro dos argumentos de inicialização.
- Exibição da porta escolhida na interface e registro da troca nos logs do processo.
- Estado de inicialização mantido até a porta do serviço ficar disponível.
- Monitoramento periódico da porta, com detecção de timeout, encerramento inesperado e perda do serviço.
- Verificação de listeners IPv4 e IPv6 para evitar falsos alertas em servidores que usam `::1`, como o Vite no Windows.
- Sugestões de correção nos cartões e no terminal a partir dos erros encontrados nos logs.
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

[0.3.0-alpha.1]: https://github.com/cjulio1993/LocalCodePilot/releases/tag/v0.3.0-alpha.1
