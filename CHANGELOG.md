# Changelog

Todas as mudanças relevantes do LocalCodePilot serão registradas neste arquivo.

O projeto usa [Versionamento Semântico](https://semver.org/lang/pt-BR/) a partir das versões públicas.

## [Não publicado]

## [0.4.0-alpha.6] - 2026-10-21

### Adicionado

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
