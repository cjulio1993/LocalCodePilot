# Changelog

Todas as mudanças relevantes do LocalCodePilot serão registradas neste arquivo.

O projeto usa [Versionamento Semântico](https://semver.org/lang/pt-BR/) a partir das versões públicas.

## [Não publicado]

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

[Não publicado]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.4.0-alpha.3...HEAD
[0.4.0-alpha.1]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.3.0-alpha.1...v0.4.0-alpha.1
[0.3.0-alpha.1]: https://github.com/cjulio1993/LocalCodePilot/releases/tag/v0.3.0-alpha.1

[0.4.0-alpha.2]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.4.0-alpha.1...v0.4.0-alpha.2

[0.4.0-alpha.3]: https://github.com/cjulio1993/LocalCodePilot/compare/v0.4.0-alpha.2...v0.4.0-alpha.3
