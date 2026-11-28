# Changelog

Todas as mudanças relevantes do LocalCodePilot serão registradas neste arquivo.

O projeto usa [Versionamento Semântico](https://semver.org/lang/pt-BR/) a partir das versões públicas.

## [Não publicado]

### Adicionado

- **Barra lateral com 4 itens:** Projetos, Ambiente, Ajuda e Configurações. **Serviços** saiu: os processos e containers de cada projeto ficam em **O que roda**, na página do projeto; os bancos de dados e os containers que não são de um projeto ficam em **Ambiente**; a busca de Projetos acha serviço, porta e container. Uma aba de Serviços salva por uma versão anterior abre Projetos. Os avisos, a Ajuda e os documentos levam à página do projeto ou ao Ambiente. Na primeira camada ficam só os estados **Rodando**, **Parado**, **Iniciando** e **Com problema**: "Pronto", "Dependências ausentes", "Bloqueado", "sem acesso" e "Iniciar ambiente" saíram; um bloqueio do Compose aparece no aviso com o que fazer. **Parar em Containers** virou **Escolher o que parar**, e **Abrir Containers** (correção de banco sem motor pronto) virou **Abrir Ambiente**.
- **Busca única em Projetos:** "Buscar projeto, serviço, porta ou container" encontra o projeto pelo nome ou pela pasta e também pelo nome de um serviço, pela porta ou por um container da última verificação, e a linha diz o que encontrou ("Encontrado: serviço start · porta 3000"). Com um motor parado, a busca sem resultado diz que os containers dele não entram na busca, em vez de afirmar que não existem.
- **Ambiente em dois grupos:** **Controlado pelo LocalCodePilot** (Docker e Podman com o estado em palavras — Rodando, Parado, Não encontrado, Com problema —, **Iniciar** no próprio motor quando ele está instalado e parado, com a confirmação "Ligar o Docker?", e os bancos de dados dos projetos) e **Encontrado neste computador** (containers que não são de um projeto da lista ou cuja origem não foi confirmada, só com **Ver logs**, e as linguagens). O **Iniciar** de um projeto Compose na lista e na página usa o mesmo fluxo do ambiente, e um bloqueio do Compose aparece na linha como motivo, sem Iniciar.
- **Projetos é a tela inicial, em linhas:** uma lista única, como uma tabela, com Projeto (nome e tipo), Estado (com o motivo em até duas linhas), Endereço (abre o site quando o projeto roda) e, à direita, a ação do momento e **Detalhes**. Quando o projeto roda, a ação é **Parar**; parado, a próxima ação (**Iniciar**, **Instalar dependências**...); enquanto inicia, nenhuma outra ação, só **Parar** para uma compilação. Os filtros são **Com problema**, **Rodando** e **Parado**, sem contadores; clicar de novo no filtro mostra todos. A ordem não muda quando um projeto inicia ou para: favoritos primeiro, depois pelo nome. A **Visão geral** e a grade de cartões saíram: a lista mostra o mesmo, sem repetir os projetos. Clicar em qualquer parte da linha fora dos botões abre a página do projeto. Os botões da lista dizem o projeto ao leitor de tela ("Iniciar loja-web"). O aviso de atualização volta ao topo de todas as páginas.
- **Página do projeto:** clicar no nome do projeto, na linha ou em **Detalhes** abre uma página com tudo sobre ele, nesta ordem: **Resumo** (estado, motivo, a ação principal, Abrir site, Parar e Abrir no VS Code), **O que roda** (os processos e os containers do projeto, com porta, Ver logs e Detalhes de cada um), **Banco de dados**, **Logs** e **Detalhes técnicos**. **Projetos**, no topo, volta à lista. Os links "Ver" da Visão geral e da aba Bancos de dados abrem a página do projeto. O botão **Detalhes técnicos** dos cartões e da lista passou a se chamar **Detalhes** e abre a página. Ao parar um projeto cujo banco criado pelo app continua ligado, o aviso diz "Projeto parado. O banco de dados continua rodando.".
- **Serviços > Bancos de dados:** os bancos de todos os projetos, um por linha, com o estado, a porta e onde ficam (este computador, outro computador, Compose ou arquivo do projeto). Um servidor usado por vários projetos aparece uma vez, com os projetos que o usam. **Parar** só aparece para um servidor criado pelo app quando nenhum projeto que o usa está rodando; um banco em outro computador oferece **Testar conexão**; nada é oferecido para mudar um servidor que o app não criou.
- **Página Ambiente** ("Linguagens e motores neste computador"), na barra lateral: o estado do Docker e do Podman, com atalho para Serviços > Containers, onde fica **Iniciar** do motor, e as linguagens deste computador, que saíram de Configurações. **Configurações** passou a usar linhas, com título e explicação à esquerda e o controle à direita.
- **Visão geral nova:** primeiro **Com problema** (motivo e próxima ação), depois **Rodando agora** e **Projetos recentes**, com a contagem no título e **Ver todos**, que abre Projetos com o filtro certo. Os três contadores grandes do topo saíram. O aviso de atualização aparece no fim da Visão geral; nas outras páginas continua no topo.
- **Serviços > Containers em linhas:** os serviços de cada ambiente Compose seguem o padrão da aba Processos, com Serviço, Estado e Porta em colunas e **Abrir site** à direita (o endereço provável aparece ao passar o mouse). As ações em containers isolados continuam em **Outras ações**, e as proteções do Compose não mudaram.
- **Serviços > Processos em linhas:** cada serviço ocupa uma linha, com Serviço, Estado e Porta em colunas e as ações sempre no mesmo lugar, à direita (Abrir site, Iniciar ou Parar, Reiniciar, Ver logs, Detalhes). Uma falha mostra o motivo e a sugestão logo abaixo da linha. O comando, a pasta, o PID, o código de saída, a última execução e a saída ficam em **Detalhes**. Em janela estreita, as ações descem para uma segunda linha. O botão de repetir passou a se chamar **Tentar de novo**, como nos cartões.
- Barra lateral mais simples: **Processos** e **Containers** viraram as abas de **Serviços**, que reúne o que está rodando. O botão abre a última aba usada, e a página salva por versões anteriores abre a aba correspondente. Os links "Ver logs", "Abrir Containers" e os avisos levam à aba certa, com as mesmas proteções do Compose.
- Estados de página e confirmações com o mesmo padrão: carregando, vazio e erro mostram título, explicação e uma ação, com os mesmos textos de antes. Quando a busca de projetos falha, a Visão geral diz "Não foi possível concluir a varredura", com a causa e **Tentar de novo**, em vez de "Nenhum projeto encontrado". As confirmações "Limpar histórico de logs?" e "Confirmar dados do ambiente" (volumes do Compose) bloqueiam o resto da janela, começam com o foco em **Cancelar**, mantêm o Tab dentro delas e cancelam com Esc; o leitor de tela as anuncia como diálogo. Tamanhos de texto por papel ficam em `theme.rs`.
- Na primeira abertura depois de atualizar, **LocalCodePilot atualizado para {versão}** mostra uma vez até três novidades da versão, com **Ver notas completas** e **Fechar**. Não aparece numa instalação nova, ao abrir de novo a mesma versão nem quando uma atualização falha e a versão anterior volta. As novidades vêm dentro do app (`desktop/NOVIDADES.md`, conferido no CI a cada versão) e aparecem sem internet. **Ver novidades**, em Configurações > Atualizações, mostra de novo.
- **Exportar diagnóstico**, em **Detalhes técnicos**, salva um arquivo `.json` para suporte, só depois de você escolher onde (Cancelar não cria arquivo): estado, motivo, regra, o que foi encontrado, comandos, requisitos e bancos. Logs, `.env`, manifestos e saída do Compose nunca entram. Senhas, tokens, chaves, usuário e senha em endereços, servidores de outros computadores e caminhos pessoais são mascarados, e o que ainda parece sensível é omitido. Ao salvar, o aviso diz "Diagnóstico salvo. Confira o arquivo antes de compartilhá-lo.".
- **Detalhes técnicos** de cada projeto mostra "Por que este diagnóstico?": a regra que decidiu o estado e o motivo do cartão ("Regra: …") e o que o app encontrou ("Encontrado: …"). Um fato que o app não verificou aparece como "não verificado", sem causa presumida. A ordem das 18 regras, que decide quando duas condições valem ao mesmo tempo, está documentada em `docs/DETALHES-TECNICOS.md`. Um processo que parou sem causa conhecida agora diz "{processo} parou com erro. Veja os logs para encontrar a causa."; um manifesto ilegível diz "Corrija o arquivo indicado e atualize o projeto.".
- Projeto C/C++ sem compilador fica **Parado** com o motivo no cartão. No Windows, **Preparar ambiente** instala o Visual Studio Build Tools 2022 com a carga de C++ da Microsoft (pelo winget, com permissão de administrador) e verifica o compilador; no macOS, abre o instalador das ferramentas de linha de comando da Apple e espera a conclusão para verificar o compilador; no Linux, o cartão diz qual pacote instalar (`build-essential`, `base-devel`, `build-base` ou `gcc-c++`, conforme a distribuição), sem usar `sudo`. Uma falha de compilação ao **Iniciar** agora diz "A compilação falhou. O projeto não foi iniciado. Use Ver logs para encontrar o erro.".
- No macOS e no Linux, **Iniciar** roda projetos C/C++ com Makefile que declaram o alvo `run` (`make run`). A confirmação da primeira vez mostra as regras que ele executa (a compilação dos pré-requisitos e a regra `run`) e volta a aparecer sempre que o Makefile muda. Sem o alvo `run`, com mais de uma regra `run`, com `include` ou com mais de um Makefile na pasta, o app não adivinha qual é o programa e pede o comando de início. No Windows, que não tem o `make`, o Makefile continua pedindo o comando.
- No Windows, **Iniciar** compila e executa projetos C/C++ do Visual Studio: um `.vcxproj` executável, ou uma `.sln` com um único projeto executável. O app encontra o MSBuild do Visual Studio instalado (pelo `vswhere`, sem depender do PATH), compila em Debug numa pasta do app fora do projeto e executa o programa, com a mesma confirmação, o mesmo andamento ("Compilando o projeto…"), os logs e o Parar dos projetos CMake. Com mais de um programa, o cartão diz "Encontramos mais de um programa para iniciar neste projeto. Defina qual comando usar.". Sem programa identificável, diz "Reconhecemos C/C++, mas não encontramos um programa único para iniciar. Defina o comando de início.".
- MariaDB no macOS e MySQL/MariaDB no Linux ARM são preparados em container com Docker ou Podman, após confirmação. A imagem oficial é fixada por digest, a porta fica restrita a 127.0.0.1 e os dados permanecem num volume do motor de containers. O mesmo vale quando o sistema não atende ao pacote nativo (glibc antiga, macOS anterior ao 15, falta da libaio). A confirmação **Preparar {motor} para este projeto?** mostra o download e o espaço medidos (MySQL cerca de 250 MB e 1,1 GB; MariaDB cerca de 100 MB e 450 MB), e o usuário do projeto tem acesso só ao próprio banco. Onde há pacote nativo, nada muda.
- No macOS e no Linux, **Iniciar** prepara PostgreSQL local em container com Docker ou Podman, após confirmação. A imagem oficial (`postgres:19.0-trixie`) é fixada por digest e conferida depois do download, a porta fica restrita a 127.0.0.1 e os dados permanecem num volume do motor. A confirmação **Preparar PostgreSQL para este projeto?** mostra o download (cerca de 160 MB), o espaço (cerca de 550 MB), o banco e o usuário. O andamento aparece no cartão ("Baixando a imagem do PostgreSQL…", "Conferindo a imagem do PostgreSQL…", "Criando o container do PostgreSQL…", "Preparando o banco e o usuário…", "Testando a conexão do projeto…"), e o projeto só inicia depois do login. A senha inicial é trocada logo após o preparo pela senha guardada no cofre, e o usuário do projeto não tem poderes no servidor. Sem motor, o cartão diz "O PostgreSQL deste projeto não está rodando. Para prepará-lo, instale o Docker ou o Podman e abra Containers."; com o motor parado, Iniciar pede para ligá-lo e segue sozinho. No Windows, o PostgreSQL continua com o pacote nativo.
- **Iniciar** passa a preparar o banco de dados também no macOS e no Linux, como no Windows: MySQL 9.7 no macOS 15 ou mais novo (Apple Silicon e Intel) e no Linux x86_64, MariaDB 12.3 no Linux x86_64. O pacote é o oficial, com SHA-256 conferido contra a assinatura GPG da Oracle ou o `sha256sums.txt` assinado da MariaDB. A confirmação passa a dizer que "O servidor aceitará conexões só deste computador". O servidor e os dados ficam na pasta do app, e a senha de administração fica num arquivo que só o seu usuário lê. Onde ainda não há pacote verificável (PostgreSQL no macOS e no Linux, MariaDB no macOS, Linux ARM), o cartão diz "O {motor} deste projeto não está rodando. O LocalCodePilot ainda não consegue prepará-lo neste sistema.", e Detalhes técnicos traz o motivo. No Ubuntu 24.04 e mais novos, o app liga a `libaio` que o MySQL procura dentro da pasta do próprio pacote, sem mexer no sistema.
- **Iniciar** passa a compilar e executar projetos C/C++ com CMake ou Meson que declaram um único executável, sem pedir comando. A pasta de compilação fica fora do projeto, na pasta de cache do app, e a configuração só roda na primeira vez. A confirmação e Detalhes técnicos mostram as etapas (configurar, compilar) e o programa. Enquanto compila, o cartão mostra "Compilando o projeto…", com **Ver logs** e **Parar**, que interrompe a compilação. Se a compilação falhar, o programa não é executado e o cartão mostra "Não foi possível compilar o projeto. Veja os logs.", com **Ver logs** e **Tentar de novo**. Com mais de um executável, o app não escolhe e pede o comando. Se faltar o CMake ou o Meson, **Preparar ambiente** os instala pelo winget no Windows. O compilador C/C++ não é instalado pelo app.
- **Iniciar** passa a funcionar sem pedir comando em mais linguagens: Perl, Lua, R e Julia rodam o arquivo de entrada (`main.pl`, `main.lua`, `main.R`/`app.R`, `main.jl`… ou o único arquivo da linguagem na pasta), um `app.R` com Shiny abre como site na porta 3838, Haskell só com Cabal usa `cabal run` (com um executável no `.cabal`) e Clojure com Leiningen usa `lein run` (com `:main`). Na primeira vez, a confirmação mostra a pasta e o comando, como nas outras linguagens. Quando há mais de uma entrada possível, o app não escolhe sozinho e o cartão diz "Encontramos mais de uma forma de iniciar este projeto. Defina qual comando usar."; sem entrada, "Reconhecemos {linguagens}, mas não encontramos um arquivo ou alvo de entrada. Defina o comando de início.".
- **Iniciar** roda as migrations pendentes de projetos Laravel antes de iniciar, depois da confirmação **Rodar as migrations deste projeto?** ("O banco está pronto, mas há migrations pendentes. O LocalCodePilot vai aplicá-las antes de iniciar o projeto. Elas podem alterar a estrutura e os dados do banco."), com **Confirmar e iniciar** e **Agora não** (foco inicial; Escape tem o mesmo efeito). **Detalhes técnicos** mostra o comando exato (`php artisan migrate --force --no-interaction`), a pasta, o banco de destino e as migrations pendentes. Enquanto roda, o cartão mostra **Rodando as migrations do projeto...**; ao concluir, "Migrations concluídas. Iniciando o projeto..." e o projeto inicia. Se falhar, o projeto não inicia e fica **Com problema** com "Não foi possível concluir as migrations. O projeto não foi iniciado.", **Ver logs** (a saída do comando, com segredos mascarados) e **Tentar de novo**. A oferta só aparece quando o destino é comprovado (banco neste computador cujo login o app testou, ou SQLite dentro do projeto); nunca para banco em outro computador nem com `APP_ENV=production`. O app nunca roda `migrate:fresh`, `reset`, `rollback`, `db:wipe` nem seeds.
- **Iniciar** liga o Docker ou o Podman quando o motor está instalado e parado, sem abrir o terminal. A confirmação **Ligar o {motor}?** diz o que vai acontecer ("O LocalCodePilot vai ligar o {motor} neste computador e, quando ele estiver pronto, iniciar o projeto {projeto}."), com **Ligar e iniciar** e **Agora não** (foco inicial; Escape tem o mesmo efeito). Se a máquina padrão do Podman ainda não existe, a confirmação avisa que criá-la pode baixar arquivos e ocupar espaço. Enquanto liga, aparece **Iniciando o {motor}…**; quando o motor e o Compose respondem, o projeto segue pela confirmação de sempre, com as mesmas proteções do Compose. Se não der certo, o aviso diz "Não foi possível ligar o {motor}. O projeto não foi iniciado." com o motivo, e **Detalhes técnicos** mostra os comandos e o erro. Com os dois motores parados, vale o que o projeto já usou; sem histórico, a confirmação pergunta qual ligar. Em **Containers**, o aviso do motor parado passa a dizer "O {motor} está instalado, mas parado. Para iniciar este projeto, ligue o motor." com **Iniciar**, que liga só o motor. O app só oferece ligar quando o motor está comprovadamente parado e a CLI aponta para o alvo padrão deste computador; permissão negada, conexão remota ou outra máquina continuam com o motivo e os Detalhes técnicos. No Linux, só serviços do usuário são ligados.

### Corrigido

- Fechar o aplicativo no meio do primeiro preparo de um banco não deixa mais o servidor recém-criado rodando sem dono. O servidor é registrado assim que inicia, e, ao fechar, o LocalCodePilot tenta pará-lo normalmente, como os outros servidores e containers que preparou, sem nunca forçar e só depois de comprovar que é dele. Se um banco do app continuar rodando (porque não parou a tempo ou porque o app não fechou normalmente), a próxima abertura avisa "Um banco de dados do LocalCodePilot continuou rodando após o fechamento anterior. Ao fechar o aplicativo, tentaremos pará-lo normalmente; seus dados permanecerão guardados.", e ele é parado no fechamento seguinte.
- Os logs de um processo não param mais na primeira linha com acento escrita fora do UTF-8. Ferramentas do Windows como o MSBuild escrevem no código de página do console, e o resto da saída se perdia; agora essas linhas aparecem com os acentos certos.
- Na página **Linguagens**, Perl, Lua, R e Julia não aparecem mais como "ainda não inicia": o texto diz que o app inicia projetos com um único arquivo de entrada.
- As configurações (projetos favoritos, histórico, comandos personalizados, confirmações, pastas de busca e preferências) não se perdem mais em silêncio. Antes, se o arquivo ficasse ilegível, o app começava vazio e gravava por cima dele. Agora cada gravação troca o arquivo de uma vez e guarda a cópia anterior válida (`.bak`); um arquivo ilegível é guardado ao lado (`.ilegivel-<data>`) e nunca é sobrescrito. Com cópia válida, o app a restaura e avisa "Não foi possível ler seus projetos e configurações. Restauramos a última cópia salva."; sem cópia válida, avisa "Não foi possível carregar seus projetos e configurações. Guardamos os arquivos para recuperação. Alguns dados podem estar ausentes.", com **Abrir pasta**. O aviso continua em **Configurações > Recuperação de configurações** até **Entendi**, e **Detalhes técnicos** mostra a causa e os caminhos, nunca o conteúdo. São mantidos os 5 arquivos guardados mais recentes de cada tipo, e nada com menos de 30 dias é apagado. Um estado gravado por uma versão mais nova do app é copiado (`.versao-<n>`) antes de qualquer gravação.

### Alterado

- No Windows, uma cópia que não consegue se atualizar sozinha (por exemplo, o `.exe` avulso da release aberto direto da pasta Downloads) passa a levar ao instalador oficial em **Baixar atualização**, com a frase "Esta cópia não foi instalada. Baixe o instalador para atualizar e receber as próximas atualizações automaticamente.". Antes, o botão baixava o zip, e a cópia continuava sem se atualizar sozinha. A mesma frase aparece em **Configurações > Atualizações**.

### Segurança

- Atualizada a dependência `url` para corrigir o alerta RUSTSEC-2026-0592.

## [0.4.0-alpha.10] - 2026-11-21

### Alterado

- As confirmações do banco de dados (**Preparar banco de dados para este projeto?**, **Preparar SQL Server para este projeto?**, **Usar outra porta?**, **Corrigir conexão?** e **Criar banco novo?**) abrem com o foco em **Agora não**, como as outras confirmações do app; Tab leva à ação principal, e Escape tem o mesmo efeito de **Agora não**. Antes, abertas pelo teclado, o foco ficava no botão atrás da janela, e Escape não fechava a confirmação.
- Projetos com SQL Server passam a ter o login testado com os dados de acesso do próprio projeto, neste computador ou em outro (**Testar conexão** em **Detalhes técnicos**). No Windows, para um SQL Server neste computador que não está rodando, **Iniciar** abre a confirmação **Preparar SQL Server para este projeto?**: ela explica que a imagem oficial (cerca de 500 MB) será baixada e um container criado no Docker ou no Podman, que o SQL Server Developer é gratuito para desenvolvimento e teste nos termos da Microsoft e onde os dados ficam (um volume do motor de containers). **Preparar e iniciar** só fica disponível depois que você marca **Li e aceito os termos de licença do Microsoft SQL Server**; o app nunca aceita os termos por você. O container usa uma imagem fixada por digest, só escuta em 127.0.0.1, troca a senha inicial do administrador logo depois do preparo (a definitiva fica no Gerenciador de Credenciais do Windows) e cria o banco e o login do projeto com acesso só a esse banco. Sem Docker nem Podman prontos, o projeto fica **Com problema** com "Para preparar o SQL Server, prepare o Docker ou o Podman na página Containers." e a ação **Abrir Containers**. Ao fechar o aplicativo, o container é parado e a parada é conferida; se não for possível confirmar, o cartão diz isso e leva à página Containers. Se nenhuma senha de administração guardada entrar no SQL Server, **Detalhes técnicos** oferece **Criar SQL Server novo**, que cria outro container e outro volume e mantém o volume anterior com os dados.
- Quando o banco do projeto impede o início, o cartão oferece a correção que for segura, sempre com confirmação e com um novo teste do login do projeto antes de iniciar. **Usar outra porta**: quando outro programa ocupa a porta, muda só `DB_PORT` no `.env`, guarda uma cópia ao lado e prepara o banco do LocalCodePilot na porta nova; se o login falhar, o `.env` volta ao original, mas só se ainda for exatamente o que o app gravou (uma edição sua nunca é sobrescrita). **Corrigir conexão**: no servidor do LocalCodePilot, troca a senha do usuário do projeto pela do `.env`, e não aparece se outro projeto conhecido usa o mesmo usuário. **Criar banco novo**: depois de duas falhas com sinal de dados danificados no registro do servidor, guarda a pasta atual e cria outra; o cartão avisa "O projeto está conectado a um banco novo. Os dados anteriores foram guardados.". Quando o servidor do app não inicia por outro motivo, o cartão oferece **Tentar de novo**. Em **Detalhes técnicos**: **Parar o banco de dados** (só o servidor do app, e só quando nenhum projeto que o usa está rodando), **Testar conexão** para um banco fora deste computador e **Usar outra porta** quando um banco existente recusa o acesso.
- No Windows, **Iniciar** prepara e inicia o banco de dados do projeto quando ele é PostgreSQL, MySQL ou MariaDB neste computador e não está rodando. Na primeira vez, a confirmação **Preparar banco de dados para este projeto?** mostra o download, o espaço necessário depois do preparo, o banco e o usuário que serão criados e onde ficam os dados (o caminho completo fica em **Detalhes técnicos**); **Preparar e iniciar** baixa a versão fixada e conferida, cria um servidor só deste computador (senha de administração no Gerenciador de Credenciais do Windows), cria o banco e o usuário do projeto com permissão só nesse banco e testa o login do projeto. O cartão mostra as etapas sob **Iniciando** ("Baixando MariaDB: 42% (43 de 101 MB)", "Preparando o banco de dados…", "Iniciando o banco de dados…"), e os processos do projeto só iniciam depois que o banco aceita os dados de acesso. Nas vezes seguintes, o servidor do app é iniciado sem pergunta. Se faltar o Visual C++ (MySQL e PostgreSQL), se o nome do banco ou do usuário não puder ser criado, ou se outro programa usar a porta, nada é preparado e o projeto fica **Com problema**, com o motivo. Ao fechar o aplicativo, o LocalCodePilot tenta encerrar o servidor com o desligamento normal; se ele não parar a tempo, o app avisa na próxima abertura, depois de conferir que o servidor ainda é dele e continua rodando.
- O banco de dados de um projeto passa a ser testado com os dados de acesso do próprio projeto (banco, usuário e senha do `.env` ou do `wp-config.php`) quando roda neste computador, com PostgreSQL, MySQL ou MariaDB. O cartão só diz **Banco de dados conectado.** depois desse login; antes, uma porta aberta aparecia como "TCP acessível; login não verificado". O app diferencia o banco que não está rodando (o projeto fica **Parado**, com o motivo), o acesso recusado e o banco de dados que não existe (os dois ficam **Com problema**, com a orientação), e diz quando faltam dados de acesso que ele consiga ler. Para um banco do Compose, mostra "Container ativo; conexão não verificada.". **Detalhes técnicos** mostra onde está a configuração, o endereço testado, o usuário, o banco, o resultado e quando; a senha nunca aparece. A cada nova verificação dos projetos, o resultado anterior é descartado.

### Corrigido

- O preparo de um banco MySQL ou MariaDB pelo app não fica mais parado em **Preparando o banco de dados…**. Desde o suporte a SQL Server, o app passou a incluir dois motores de criptografia para TLS, e o login de administração no MySQL e no MariaDB falhava em silêncio, com o servidor já pronto. Agora o app escolhe um só motor antes de cada conexão, cada login de administração tem limite de 15 segundos, e uma tarefa do banco que termina sem resultado deixa o projeto **Com problema** com "Não foi possível concluir o preparo do banco de dados. Confira os detalhes e tente novamente." e a causa em **Detalhes técnicos**.
- Os cards de Projetos agora têm todos a mesma altura, com **Iniciar** e **Detalhes técnicos** alinhados na base, em qualquer número de colunas. Antes, cada card tinha a altura do próprio conteúdo, e a grade ficava irregular. O motivo do estado aparece em até 3 linhas; se for maior, termina em "…", e o texto completo fica ao passar o mouse, em **Detalhes técnicos** e para o leitor de tela. As tecnologias ficam numa linha, e as que não cabem viram "+N", com a lista completa nos mesmos lugares. Um card com **Detalhes técnicos** aberto cresce sem mudar os outros.
- Um teste automático abria `cmd.exe` sem janela oculta e o encerrava logo em seguida; sob carga, isso pode ter causado a janela "cmd.exe - Erro de aplicativo (0xc0000142)" vista uma vez durante os testes. O teste agora roda `ping` direto, oculto, como todos os programas que o app inicia. A verificação de linguagens do app, repetida 10 vezes numa instância isolada, não mostrou a janela.

### Documentação

- A Ajuda ganha a seção **Atualizar o LocalCodePilot**: a atualização pelo app é experimental, a assinatura dos checksums é verificada antes do download, as etapas **Baixar, Verificar, Instalar e Reiniciar**, a diferença entre a cópia instalada e a portátil, a confirmação quando há projetos rodando e o que acontece numa falha.
- A seção **Bancos de dados** da Ajuda passa a ter um item para cada situação: o preparo no Windows, onde ficam os dados, o servidor que você já tem, as correções, o Compose, o SQL Server, macOS e Linux e o banco em outro computador. O README (em português e em inglês) ganha a seção **Bancos de dados**, com uma tabela do que o app faz em cada caso. A limitação antiga ("ainda pedem esse passo manual") deu lugar aos limites reais: preparo só no Windows, SQL Server só em container e em computadores x64, migrations por conta do projeto e `DATABASE_URL` ainda não reconhecido. O `docs/DATABASES.md` foi reescrito com o comportamento atual.
- O README e a Ajuda (**Instalar no Windows com segurança**) explicam, passo a passo, como conferir o SHA-256 do instalador ou do zip no `SHA256SUMS.txt` da release antes de abrir o arquivo, e só então o que fazer no aviso do SmartScreen. Saiu a promessa de assinatura de código "a caminho": as versões atuais não têm assinatura de código.

- No Windows, a porta de outro programa não é mais atribuída a um projeto por engano. Para saber se o processo que escuta a porta foi iniciado pelo projeto, o app sobe pela árvore de processos. O Windows pode reaproveitar o número (PID) de um processo que já terminou, e esse número podia ser tomado como "pai" de um processo mais antigo. Agora um "pai" que começou depois do filho encerra a busca, e a porta conta como de outro programa.
- Avisos longos no canto da janela, como o resultado de uma atualização, quebram linha e aparecem inteiros. Antes, o começo da mensagem ficava fora da borda esquerda da janela.
- A consulta de versões das linguagens (ao abrir o app e em **Verificar de novo**) não deixa mais um `erl_crash.dump` de cerca de 3 MB na pasta de onde o LocalCodePilot foi aberto. Quando a máquina virtual do Erlang falhava ao responder `elixir --version`, ela gravava esse arquivo na pasta atual. Agora as consultas rodam na pasta temporária do sistema e pedem à VM do Erlang que não grave o despejo. Se a pasta temporária não estiver disponível, a versão aparece como indisponível, e nada roda na pasta de abertura.

### Alterado

- Enquanto o app está fechado para atualizar, uma janela pequena, **Atualizando o LocalCodePilot**, mostra o que está acontecendo. Antes, o app sumia por alguns segundos sem nenhum sinal, e dava para achar que tinha travado. A janela mostra as etapas **Baixar › Verificar › Instalar › Reiniciar**, **Instalando a atualização...** e depois **Reiniciando o LocalCodePilot...**, com "Você pode fechar esta janela. A atualização continuará." Se o app levar mais de 3 segundos para fechar, ela mostra **Aguardando o LocalCodePilot fechar...**. A janela não tem botões e nunca mostra sucesso nem falha: o resultado continua aparecendo no app que reabre. Fechar a janela não interrompe a atualização.
- O aviso de atualização e **Configurações > Atualizações** ganham a etapa **Instalar**. Antes de fechar, o app mostra **Instalando a atualização...** em vez de **Reiniciando o LocalCodePilot...**, então a sequência continua igual na janela do atualizador e nunca volta.
- Quem instalou pelo instalador passa a ser atualizado pelo próprio instalador oficial. **Atualizar agora** baixa `LocalCodePilotSetup-<versão>-windows-x64.exe`, confere o SHA-256 nos checksums assinados e o roda em silêncio na mesma pasta, com o app fechado. Assim, a lista de programas do Windows mostra a versão nova, e o desinstalador recebe os arquivos novos. A cópia portátil (zip) continua trocando só o executável. O app só usa o instalador quando o desinstalador está ao lado do executável e a entrada de desinstalação aponta para a mesma pasta; nos outros casos, segue como portátil.
- Se o instalador não concluir (bloqueado pelo antivírus ou com erro), o executável anterior é conferido pelo SHA-256 e, se mudou, restaurado. A versão anterior reabre com **A instalação não foi concluída**, a mensagem "Não foi possível instalar a versão {nova}. A versão {atual} foi reaberta e você pode continuar trabalhando...", **Tentar de novo**, **Ver página oficial da versão** e **Detalhes técnicos** (código de saída e o caminho do registro do instalador, guardado como `update-setup.log` nas configurações). O instalador não fecha outros programas: outra cópia aberta do app faz a instalação falhar em vez de ser encerrada.
- Se o instalador concluir, mas a versão nova não abrir, o executável e as configurações anteriores voltam. A mensagem avisa que a lista de programas do Windows ainda pode mostrar a versão nova.

- **Instalar dependências** e **Preparar ambiente** mantêm o mesmo verbo do clique até o resultado. Antes, os dois fluxos terminavam com os mesmos textos genéricos de "preparação". Agora, conforme a ação escolhida, o app mostra:
  - o botão de confirmação: **Instalar dependências** ou **Preparar ambiente** (antes, "Preparar");
  - a janela de progresso: "Instalar dependências · projeto" ou "Preparar ambiente · projeto";
  - o botão de cancelar: **Cancelar instalação de dependências** ou **Cancelar preparação do ambiente**;
  - os avisos de início ("Instalando dependências em segundo plano..."), cancelamento, sucesso ("Dependências instaladas. O projeto está pronto." ou "Ambiente preparado. O projeto está pronto."), pendência e falha.

  As etapas e os comandos executados não mudaram.

- Os erros de atualização aparecem em camadas. Antes, o app mostrava o erro técnico ("Atualização automática: ...") e o link **Baixar o ZIP manualmente**, inclusive quando o arquivo tinha sido recusado por segurança. Agora a primeira linha diz o que aconteceu em linguagem simples, e a segunda confirma que a versão atual continua funcionando:
  - sem conexão ou resposta inesperada ao verificar: **Não foi possível verificar atualizações**, com **Tentar de novo**;
  - queda durante o download: **O download foi interrompido**, com **Tentar de novo** e **Ver página oficial da versão**;
  - checksum, assinatura ou arquivo inválido: **Atualização recusada por segurança**. Não há **Tentar de novo** nem download direto; a ação é **Consultar página oficial da versão**, e o texto pede para não instalar manualmente aquela atualização;
  - problema neste computador (pasta sem permissão, disco): **Não foi possível preparar a atualização**, com **Tentar de novo**.

  O erro original continua em **Detalhes técnicos**, com **Copiar erro técnico**.

- **Atualizar agora** pergunta antes de encerrar projetos em execução. Antes, a atualização fechava o app e encerrava os processos dos projetos sem avisar. Agora, se houver projetos **Rodando** ou **Iniciando**, uma janela diz "Há projetos em execução.", explica que o LocalCodePilot precisará fechar e encerrar os processos que gerencia, lista os projetos com o estado de cada um e oferece:
  - **Agora não** (foco inicial; Escape tem o mesmo efeito): nada é baixado nem instalado, e os projetos continuam rodando;
  - **Atualizar e encerrar processos**: segue com a atualização.

  Se um projeto for iniciado durante o download, a mesma pergunta aparece antes de fechar. Nesse caso, **Agora não** descarta o arquivo baixado e a versão atual continua. Containers iniciados pelo Compose continuam rodando depois que o app fecha.

- O aviso de nova versão, e também **Configurações > Atualizações**, mostra o que mudou numa linha: "O que há de novo: ..." com a frase de abertura das notas da versão oficial (a frase em negrito logo depois do título, com até 140 caracteres, em texto puro). Sem essa frase, ou se ela for mais longa, a linha diz "Confira as novidades na página da versão.". **Ver novidades** continua abrindo a página oficial da versão.

## [0.4.0-alpha.9] - 2026-11-19

### Segurança

- A conexão HTTPS usada para consultar e baixar atualizações passa a usar `rustls-webpki` 0.103.17, que corrige o [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382): a versão anterior podia aceitar certificados emitidos por intermediários emSign que deixaram de ser confiáveis. Também foram atualizadas duas dependências retiradas pelos autores (`clipboard-win` 5.4.3, da área de transferência no Windows, e `async-executor` 1.14.1, só no Linux). O CI passa a verificar advisories de segurança e licenças das dependências com `cargo deny` a cada pull request.

- Os logs salvos no computador (`workspace-state.json`) têm prazo e máscara. Ao abrir, o app apaga os logs de serviços e de preparação com mais de 30 dias; nomes, comandos, estados e etapas continuam. **Limpar histórico**, em **Configurações**, apaga todos os logs salvos depois de confirmar, sem parar projetos. Antes de gravar, e em **Copiar logs** dos containers, valores de `NOME=valor` ou `NOME: valor` com `PASSWORD`, `PASSWD`, `TOKEN`, `SECRET` ou `KEY` no nome, de `Authorization: Bearer valor` e a senha de `esquema://usuario:senha@` viram `••••`; os logs salvos por versões anteriores são mascarados ao abrir. A saída ao vivo de **Ver logs** continua completa, para diagnóstico. A cópia de configurações feita antes de uma atualização não leva mais os logs. Antes, os logs ficavam salvos sem prazo, sem máscara, e eram copiados para `backups`.
- **Definir comando de início** avisa quando um argumento parece um segredo (por exemplo, `--token valor` ou `API_KEY=valor`): o comando ficaria salvo e apareceria nos detalhes do projeto. O aviso sugere uma variável de ambiente e oferece **Voltar e editar** ou **Salvar mesmo assim**. A Ajuda orienta a conferir logs e caminhos antes de colá-los num relato público.

- **Validar configuração** com Podman não mostra mais a configuração resolvida do Compose. O comando `compose config` imprime os valores de `environment` e do `.env`, como senhas e tokens, e eles apareciam na janela de execução e no **Copiar logs**. Agora a saída padrão desse comando é descartada; a janela mostra **Configuração válida** ou **Falhou**, com as mensagens de erro e o código de saída. Com Docker, o app já usava `--quiet`.

- Antes de executar pela primeira vez um comando definido pelo projeto, o app pede confirmação. A janela **Iniciar este projeto?** mostra a pasta, o comando (por exemplo, `npm run dev`) e o que ele executa (o script do `package.json` ou do `composer.json`), com o remoto Git em **Detalhes**. **Cancelar** (ou Escape) não executa nada. Depois de **Iniciar**, o mesmo comando volta a ser um clique. A confirmação reaparece se a pasta, o programa, os argumentos ou o texto do script mudarem. Projetos já usados antes desta versão pedem a confirmação uma vez. Comandos escritos pela própria pessoa em **Definir comando de início** não pedem. A janela de instalação, que já mostra os comandos, vale como confirmação e passa a se chamar **Instalar dependências deste projeto?** ou **Preparar este projeto?**. Antes, um clique em **Iniciar** num repositório de terceiros executava os scripts dele sem mostrar nada.

- A atualização pelo app passa a exigir a assinatura Ed25519 do `SHA256SUMS.txt`. Cada release publica `SHA256SUMS.txt.sig`, assinado no workflow de release com uma chave que só existe como secret do GitHub e num backup offline. O app confere essa assinatura com a chave pública embutida nele (`packaging/update-signing.pub.pem`) antes de baixar o executável. Se a assinatura faltar, for de outra chave ou não corresponder ao conteúdo, a atualização é recusada sem baixar nem trocar nada, e o download oficial continua disponível. Antes, o SHA-256 vinha da mesma release que o executável: quem conseguisse trocar os dois arquivos conseguiria publicar outro executável. O workflow de release falha se o secret faltar ou não corresponder à chave pública. As versões já instaladas até esta continuam atualizando sem conferir a assinatura.

### Alterado

- Projeto recém-clonado não aparece mais como **Com problema** (#101). Quando faltam só as dependências do projeto (`node_modules`, `vendor`) ou a linguagem ainda não está instalada, o cartão mostra **Parado**, o motivo (por exemplo, "Faltam dependências de Node.js para preparar este projeto.") e a ação **Instalar dependências** ou **Preparar ambiente**. **Com problema** fica para o que precisa de atenção: instalação ou execução que falhou, manifesto inválido, versão incompatível, extensões PHP ausentes e servidor de banco que não responde. O projeto continua sem **Iniciar** até as dependências serem instaladas. A confirmação se chama **Instalar dependências deste projeto?** quando só instala as dependências do projeto, e **Preparar este projeto?** quando também muda algo na máquina. A Ajuda e o README explicam a diferença. Na lista de **Projetos**, cada linha ganhou **Detalhes técnicos**, que abre o cartão do projeto com os detalhes; antes, a lista não tinha como chegar a eles, nem pelo teclado.
- A atualização pelo app mostra o andamento em etapas, no aviso e em **Configurações > Atualizações**: **Baixar › Verificar › Reiniciar**, com a etapa atual marcada por um símbolo e em negrito, sem depender só da cor. O download tem uma barra e "Baixando a atualização · 42% · 6,3 de 15,0 MB"; se o servidor não informa o tamanho, aparece só "Baixando a atualização · 6,3 MB baixados", sem barra nem percentual inventado. Depois vêm "Verificando a atualização..." e "Reiniciando o LocalCodePilot...", que fica na tela por pouco mais de um segundo antes de o app fechar. O aviso passa a dizer "Nova versão do LocalCodePilot disponível" e "Versão atual: … · Nova versão: …", e Configurações mostra as duas versões e, quando a atualização automática não está disponível, o botão **Baixar atualização**. Os leitores de tela ouvem a etapa ("Etapa 1 de 3: Baixar") e o andamento do download.

### Corrigido

- Documentação desatualizada depois da Alpha 8 (#102). O README (PT e EN) e as notas da Alpha 8 no repositório avisam que quem usa a Alpha 7 deve instalar a Alpha 8 pelo instalador ou pelo zip, porque o **Atualizar agora** da Alpha 7 não conclui essa atualização. `docs/DETALHES-TECNICOS.md` descreve a Alpha 8 publicada (instalador e ZIP), e não mais a Alpha 7, e separa o que só chega na próxima versão. `docs/README-MACOS.txt` deixa de citar o DMG da Alpha 6 e orienta a primeira abertura por Ajustes do Sistema > Privacidade e Segurança > "Abrir mesmo assim" no macOS 15 ou mais recente. A seção "Antes da alpha 6" de `docs/CONTAINERS.md` virou histórico e roteiro de validação.
- O executável do Windows mostrava a versão `0.4.0-alpha.5` nas Propriedades do arquivo, mesmo na alpha 8 (#99). Os campos de versão do arquivo e do produto agora vêm da versão do pacote no `Cargo.toml` a cada build (`0.4.0-alpha.8` e `0.4.0.8`). O CI do Windows e o workflow de release conferem a versão do executável gerado e falham se ela não corresponder.
- Projetos com o mesmo nome de pasta não se distinguiam (#100). Quando dois projetos têm o mesmo nome, os cartões e as listas da Visão geral, de Projetos, de Processos e de Containers mostram abaixo do nome o trecho da pasta que os diferencia, como "Pasta: acme\web" e "Pasta: beta\web"; os leitores de tela ouvem, por exemplo, "Iniciar web (acme\web)". A confirmação de qualquer ação de container mostra sempre, acima dos botões, o projeto, o caminho completo da pasta e o motor (Docker ou Podman) do comando que vai rodar, sem precisar abrir **Detalhes técnicos**. O bloqueio de conflito de nome do Compose não mudou.
- Aberto pelo instalador, o app examinava a pasta acima da sua instalação (`%LOCALAPPDATA%\Programs`) e mostrava partes de programas instalados, como o VS Code, o Docker Desktop e o Python, como projetos, com o botão **Iniciar** (#98). As pastas padrão agora vêm só do perfil do usuário, sem depender da pasta de onde o app foi aberto, e a busca não entra em pastas de programas instalados (`Program Files`, `ProgramData`, `%LOCALAPPDATA%\Programs`, a pasta do Windows e a do próprio app). Uma pasta dessas que a pessoa adicionar com **Adicionar pasta** continua sendo examinada. Quem usou **Adicionar pasta** ou removeu uma pasta na Alpha 8 ficou com essa pasta de programas salva na lista; ao abrir, o app a remove e mostra em **Projetos** o aviso "Removemos uma pasta de programas da busca de projetos.", com **Ver pastas examinadas** e **Fechar**. As outras pastas salvas continuam como estavam.

- O atualizador ficou mais seguro:
  - espera o app fechar de verdade antes de trocar o executável;
  - copia e verifica a nova versão na pasta do app antes da troca, sem deixar a pasta sem `LocalCodePilot.exe` durante a cópia;
  - guarda uma cópia das configurações e um registro da atualização. Se a atualização for interrompida, a próxima abertura conclui ou desfaz o que faltou e diz qual versão está em uso. Se a nova versão fechar ao iniciar, o executável e as configurações anteriores voltam.
- Uma nova versão que demora a abrir (por exemplo, por causa do antivírus) não aparece mais como falha, e o backup do executável não fica esquecido na pasta.
- A mensagem de falha só diz que a versão anterior foi restaurada quando isso realmente aconteceu, e é gravada antes de o app anterior reabrir.
- O resultado da atualização ao abrir o app ("LocalCodePilot atualizado para a versão…" ou o motivo da falha) fica 12 segundos na tela e não é mais substituído pelo aviso "Projetos e ambientes atualizados".

- A atualização pelo app falhava em conexões lentas: o download do executável (cerca de 15 MB) tinha um limite total de 90 segundos. Agora não há limite total; o download só falha se ficar 30 segundos sem receber dados ou se a conexão fechar antes do fim, e o arquivo parcial é apagado. Depois do download, o app mostra "Verificando a atualização...". Pastas temporárias de tentativas interrompidas, com mais de 24 horas, são removidas ao abrir o app.
- A atualização pelo app escolhia o instalador (`...-windows-x64-setup.exe`) em vez do executável de atualização e parava com "o nome do executável da release não corresponde à versão selecionada" (#97). O app agora escolhe só o arquivo de nome exato `LocalCodePilot-<versão>-windows-x64.exe`, e o instalador passa a se chamar `LocalCodePilotSetup-<versão>-windows-x64.exe`, um nome que as versões alpha 7 e alpha 8 já instaladas também ignoram. O recálculo do `SHA256SUMS.txt` depois da assinatura inclui o instalador com o nome novo.

## [0.4.0-alpha.8] - 2026-11-04

### Adicionado

- Instalador para Windows (`LocalCodePilot-<versão>-windows-x64-setup.exe`, feito com Inno Setup), publicado ao lado do zip. Ele instala por usuário em `%LOCALAPPDATA%\Programs\LocalCodePilot`, sem pedir administrador, o que mantém a atualização pelo aplicativo funcionando. Também cria o atalho no Menu Iniciar (o da área de trabalho é opcional) e registra o desinstalador. Cada release e cada PR que mexe no empacotamento testam a instalação e a desinstalação silenciosas no CI.
- Projetos Zig iniciam com `zig build run` quando o `build.zig` declara o passo `run` (como o `zig init` cria). Sem o Zig, **Preparar ambiente** o instala pelo winget.
- Projetos Haskell com `stack.yaml` e um executável iniciam com `stack run`. Sem o Stack, **Preparar ambiente** o instala pelo winget; **Instalar dependências** roda `stack setup` (baixa o compilador GHC do projeto) e `stack build`. Quando a pasta do usuário tem espaço (como `C:\Users\Julio Cesar`), onde o GHC não consegue rodar o próprio compilador C, o Stack guarda seus arquivos em `C:\ProgramData\LocalCodePilot\stack`. Projetos só com Cabal ainda pedem o comando.
- Projetos Clojure com `deps.edn` iniciam sem pedir comando, pelo alias de execução (`:run`, `:dev`, `:serve` ou `:server`, com `:main-opts`), com `bb clojure -M:<alias>`. Quando o computador não tem o que o projeto precisa, o cartão mostra **Preparar ambiente**, que instala o Babashka (o Clojure CLI embutido) pelo winget e o Java 17 ou mais novo, se faltar. Projetos Leiningen (`project.clj`) ainda pedem o comando.
- Projetos Scala com sbt iniciam sem pedir comando. No sbt 2, o app usa `sbt --server run`: sem essa opção, o `sbt` é um cliente que deixa um servidor rodando em segundo plano, que o app não conseguiria parar. Projetos com sbt 1.x seguem com `sbt run`. Aplicações Play usam a porta 9000.
- Pacotes Swift com alvo executável iniciam sem pedir comando, com `swift run` (servidores Vapor na porta 8080). No Windows, o Swift só compila no ambiente de desenvolvedor do Visual Studio: o app localiza o Visual Studio pelo `vswhere`, aplica o ambiente do `vcvars64` e o `SDKROOT` salvo pelo instalador só aos processos `swift`, sem que seja preciso abrir um prompt especial.
- Projetos Elixir iniciam sem pedir comando: Phoenix com `mix phx.server` (porta 4000) e aplicações com árvore de supervisão (`mod:` no `mix.exs`) com `mix run --no-halt`. **Instalar dependências** roda `mix deps.get` e, antes, instala o Hex e o Rebar quando faltam, sem a pergunta que travaria a instalação.
- Projetos Kotlin e Java com o plugin `application` do Gradle, como os servidores Ktor, iniciam sem pedir comando, com `gradlew run --no-daemon` (porta 8080 no Ktor). Projetos Gradle com o plugin Kotlin aparecem como Kotlin.
- Projetos Deno iniciam sem pedir comando: as tarefas contínuas do `deno.json` ou `deno.jsonc` (`deno task dev`) ou, sem tarefas, o arquivo de entrada (`main.ts`, `server.ts`) com `deno run --allow-net --allow-read --allow-env`.
- Projetos Ruby iniciam sem pedir comando: Rails com `ruby bin/rails server` (porta 3000), apps Rack com `bundle exec rackup` (9292), Sinatra com `ruby app.rb` (4567) e programas `main.rb`/`app.rb`. Com Gemfile, **Instalar dependências** roda `bundle install` dentro de `vendor/bundle` do próprio projeto, sem misturar com as gems de outros projetos.
- Projetos .NET (C#, F#, VB) iniciam sem pedir comando: `dotnet run --project` para apps web (ASP.NET, Blazor) e programas de console ou worker, inclusive os listados num `.sln` em subpastas. Bibliotecas e projetos de teste ficam de fora, e a porta vem do perfil do `launchSettings.json`.
- No aviso de conflito do Compose, quando os containers do outro projeto estão rodando, o botão **Parar containers do outro projeto** os para sem sair do app. A confirmação lista os containers, avisa que os volumes não serão removidos, e o app confere de novo, logo antes de executar, que eles continuam os mesmos e rodando. Antes, era preciso usar `docker compose -p <nome> stop` no terminal.
- **Definir comando de início**: um projeto sem comando detectado (de qualquer linguagem, como C/C++, Perl, Lua, R ou Julia) pode receber o comando que o inicia, com nome, programa, argumentos, pasta de execução e porta opcional. A janela mostra exatamente o que será executado, recusa recursos de terminal (`&&`, `|`, `;`, redirecionamentos, variáveis) e confere se o programa e a pasta existem. O comando passa a funcionar como um detectado, fica salvo, aparece no cartão como "Comando definido por você" e pode ser editado ou removido em Detalhes técnicos quando não está rodando.
- Proteção contra conflitos de nome Compose: o aplicativo identifica cada ambiente pelo motor, pelo nome que o provedor Compose resolve (`name:`, `COMPOSE_PROJECT_NAME` no ambiente ou no `.env`, nome da pasta) e pela pasta de origem dos containers. Quando dois projetos resolvem o mesmo nome (por exemplo, dois `docker/compose.yaml`), quando há containers desse nome criados a partir de outra pasta, quando há volumes sem container que comprove a origem ou quando não é possível confirmar, iniciar e parar o ambiente, e iniciar, parar e reiniciar seus containers, ficam bloqueados, com explicação e a configuração `name:` sugerida, com o botão **Copiar configuração sugerida**. A verificação é refeita logo antes de executar. Validar, consultar estado e logs continuam disponíveis.
- Volumes deixados sem containers (por exemplo, após `compose down`) podem ser confirmados como do projeto; a confirmação fica salva por motor e pasta e vale só para aqueles volumes (nome e data de criação).
- A CLI `containers` mostra o nome Compose efetivo de cada ambiente e os conflitos encontrados.
- Preparação guiada de Docker e Podman: quando um motor não está pronto, a aba Containers e a CLI `containers` explicam o que falta (WSL, instalação, motor parado, máquina Podman ou provedor Compose) e mostram comandos oficiais para o usuário copiar e rodar, links de documentação e condições de licença. O aplicativo não executa esses comandos.
- **Verificar motores e arquivos** relê o `PATH` do sistema enquanto um motor está ausente, encontrando um motor recém-instalado sem reiniciar o aplicativo.

### Alterado

- O README foi reescrito para o público geral: o que o app faz, como baixar e instalar (instalador ou zip, e o aviso do Windows), primeiros passos, uma tabela simples de linguagens e as limitações. O detalhamento técnico que estava nele foi para [docs/DETALHES-TECNICOS.md](docs/DETALHES-TECNICOS.md), sem cortes.
- Num projeto Laravel recém-clonado, **Instalar dependências** também cria o `.env` a partir do `.env.example`, gera a chave da aplicação e, quando o projeto usa SQLite, cria as tabelas com `php artisan migrate`; cada etapa aparece no plano antes de executar.
- A matriz de suporte separa **Inicia** e **Abre no navegador**. Rust, Node.js, PHP, Laravel, Python/Flask, Django, Spring Boot com Maven wrapper, site estático, Lumen, Spring Boot com Gradle wrapper, Go, Dart e Flutter (web e Windows) foram validados de ponta a ponta no Windows 11 e passaram para "Sim".
- A Visão geral mostra o que está rodando, o que precisa de atenção e o que está parado, com uma ação principal por projeto (Iniciar, Abrir site, Ver logs ou Detalhes). Os estados usam os mesmos nomes em todo o aplicativo: Rodando, Parado, Iniciando e Com problema.
- Projetos ganhou busca por nome e filtros por estado, com visualização em cartões ou lista; filtro e visualização ficam salvos.
- O selo do site descreve o resultado da verificação HTTP em linguagem simples (por exemplo, "Servidor respondeu; página não encontrada" para um 404) e mostra o código ao passar o mouse; sem verificação, não aparece selo. Em projetos com containers, o endereço aparece como "Endereço provável".
- Em projetos com containers, **Parar** abre a confirmação no próprio cartão quando o aplicativo sabe o que a parada alcança: um único ambiente rodando num único motor, liberado pelas checagens de Compose. A confirmação nomeia o projeto, lista os serviços que serão parados, os que já estão parados e os que continuarão rodando (serviços de perfil opcional e containers que não estão mais no arquivo, que `compose stop` não alcança), e o aviso final diz o que parou e o que continua rodando. Quando não dá para afirmar o que será parado, o botão diz **Parar em Containers** e abre o projeto em Containers com **Outras ações** aberto. Toda confirmação de container nomeia o projeto ou o container; comando e pasta ficam em **Detalhes técnicos**.
- Um `package.json` inválido mostra o que fazer: corrigir o arquivo para iniciar o projeto.
- O aplicativo e a CLI apresentam sites WordPress como projetos PHP; a detecção continua decidindo como o site inicia e onde está o banco.
- A Ajuda foi reescrita para quem está começando: primeiros passos, estados, o que fazer quando algo não funciona, sites e logs, containers e bancos, com os detalhes técnicos num bloco recolhido.
- O menu lateral pode ser recolhido para mostrar só os ícones, com o nome de cada página na dica; a escolha fica salva. O menu rola quando não cabe na janela, e seus itens e os filtros de Projetos funcionam pelo teclado (Tab e Enter) e informam o nome e a página atual a leitores de tela.
- Projetos ficou mais compacto: "Ver 1 pasta examinada" abre as pastas onde o aplicativo procura projetos, com "Adicionar pasta" e "Restaurar pastas padrão"; busca, filtros e a escolha entre cartões e lista ficam numa linha. Em janelas de 760×520, o nome, o estado e a ação principal do primeiro projeto aparecem sem rolar.
- Saíram da interface itens que não faziam nada ou não eram verdadeiros: a página Plugins, o sino de notificações e o rodapé "Todos os serviços online", que não verificava nenhum serviço. Containers ganhou um ícone próprio.
- Botões com ícone, a estrela de favoritos e os campos de busca passaram a ter nomes claros para leitores de tela.
- A Ajuda ficou interativa: o campo **Qual é a sua dúvida?** mostra só os trechos que respondem, ignorando acentos e maiúsculas e destacando o termo; atalhos levam direto a "O site não abre", "Porta ocupada", "Docker ou Podman parado" e aos estados; as seções abrem e fecham, e um aviso leva à Visão geral quando há projetos com problema.
- Avisos em duas camadas: o bloqueio de um ambiente Compose e um motor que não está pronto mostram primeiro uma frase e uma ação; os passos ficam em **Como resolver** (ou **Como instalar**, quando o motor não foi encontrado) e nomes, pastas, volumes e o estado do motor ficam em **Detalhes técnicos**. Um motor cuja consulta falha aparece como "encontrado, mas não foi possível acessá-lo", sem afirmar que está parado. Em Processos, o aviso de um container bloqueado tem o link **Como resolver**, que abre o aviso do ambiente em Containers.
- Linguagens: além de Rust, JavaScript/TypeScript, PHP, Python, Java, Go, Dart e Flutter, o aplicativo reconhece projetos .NET (C#, F#), Ruby, Kotlin, Elixir, Deno, C/C++, Swift, Scala, Clojure, Haskell, Perl, Lua, R, Julia e Zig, por manifests e arquivos de código, e também pela imagem dos serviços do Compose e pelo `FROM` do Dockerfile. Os detalhes técnicos do cartão dizem de onde cada linguagem veio ("nos arquivos do projeto" ou "na imagem do serviço api") e se ela está instalada. Para uma linguagem que o aplicativo ainda não inicia, o cartão diz isso em vez de "nenhum comando encontrado".
- Configurações ganhou **Linguagens neste computador**, no lugar da lista "Runtimes": cada linguagem aparece como "Instalado · versão", "Instalado · Versão indisponível" ou "Não encontrado nos locais verificados", com busca, os projetos que a usam e, em Detalhes técnicos, o programa, o caminho e os locais verificados (`PATH` e pastas padrão de instalação). O atalho do Python da Microsoft Store sem Python instalado conta como não encontrado.
- Containers mostra cada projeto com seus serviços e o estado de cada um (Rodando, Reiniciando, Pausado, Parado, Com problema ou Ainda não criado), juntando o que o arquivo Compose declara com o que o motor informa. Cada ambiente tem uma ação principal (**Iniciar** ou **Parar**) e **Ver logs**; validar, consultar estado e as ações por container ficam em **Outras ações**, e arquivos, imagem, portas, ID e comandos em **Detalhes técnicos**. Projetos bloqueados, com problema e rodando aparecem primeiro, e os parados começam recolhidos. **Buscar projeto ou serviço** filtra a lista. Containers que não pertencem com certeza a um projeto da lista ficam em **Outros containers**, com as mesmas ações. Os motores aparecem numa linha; o aviso de um motor que não está pronto abre quando o aplicativo depende dele. A escolha entre Docker e Podman só aparece quando os dois estão prontos. **Parar** usa o motor em que o ambiente já está rodando. Ações que só leem (validar, consultar estado e **Ver logs**) rodam sem pedir confirmação; as que mudam o ambiente continuam pedindo, com o comando e a pasta. O resultado de uma ação aparece no topo da página, que rola até ele. Quando só restam rodando serviços que a parada do ambiente não alcança, o cabeçalho oferece **Iniciar** e indica **Outras ações**.
- Processos mostra só os processos que rodam direto na máquina; os containers ficam em Containers, e Processos indica quantos estão rodando, com o link **Ver em Containers**.
- Nos cartões de projeto, **Detalhes** passou a se chamar **Detalhes técnicos** e não repete o motivo já mostrado no cartão.
- Visão geral, Projetos e Processos atualizam o estado dos containers a cada 30 segundos, consultando apenas a lista de containers, para manter cartões e diagnóstico de banco atualizados.
- URLs candidatas de containers reconhecem mais portas de servidores de desenvolvimento, como 3001, 4000, 4200, 5000 e 8025, e não repetem a mesma URL publicada em IPv4 e IPv6.

### Corrigido

- No Windows, um servidor que escuta em `0.0.0.0` (como o Deno) conseguia abrir a mesma porta que outro programa já usava em `127.0.0.1`, e o cartão mostrava "Site respondendo" com a resposta do outro programa. Agora o app detecta o outro programa na porta e mostra "Outro programa está usando a porta", com Tentar de novo.
- O aviso de linguagem ausente não mostra mais "latest" ou "stable" como se fossem versões: diz "Instale Clojure", "Instale Go" e assim por diante.
- Uma linguagem instalada numa pasta padrão, mas fora do PATH (como o Elixir e o Erlang instalados pelo winget), passa a iniciar e a instalar dependências: as pastas de instalação conhecidas entram no fim do PATH dos processos. A verificação antes de iniciar também usa o PATH atualizado, então um programa instalado depois de abrir o app é encontrado sem reiniciá-lo.
- As pastas `deps` e `_build` do Elixir não são mais examinadas como projetos; antes, os pacotes em `deps/` (como `deps/phoenix`, com `package.json`) apareciam como subprojetos Node.js. O TypeScript da pasta `assets/` do Phoenix não faz mais o projeto aparecer como TypeScript.
- O comando de projetos com Gradle Wrapper ou Maven Wrapper aparece com o caminho normal (`C:\...\gradlew.bat`), sem o prefixo interno `\\?\` do Windows.
- Depois de uma falha ao iniciar (porta ocupada, servidor que fechou antes de ficar pronto, programa que parou com erro), o projeto mostra **Tentar de novo** ao lado de Ver logs. Antes, o cartão só oferecia Ver logs, e não havia como iniciar de novo pelo próprio projeto depois de resolver a causa.
- **Abrir no VS Code** não abre mais o projeto em modo restrito. O app passava ao VS Code o caminho no formato interno do Windows, com o prefixo `\\?\`, que o VS Code trata como uma pasta diferente da que você marcou como confiável; agora passa o caminho normal.
- Em Containers, a escolha "Iniciar com: Podman" não volta mais para Docker na verificação automática a cada 15 segundos. Antes, o ambiente podia subir no Docker mesmo com o Podman escolhido. Com essa correção, Containers com Podman foram validados de ponta a ponta no Windows 11 (iniciar, endereço, logs e parar) e deixam de ser experimentais.
- Projetos sem framework não pedem mais um comando para iniciar: um arquivo Java com `main` roda com `java Main.java`, uma pasta de páginas PHP sem `index.php` abre com uma página que lista os arquivos, e páginas HTML sem `index.html` na raiz (como um jogo com `teste.html` e `source_code/index.html`) abrem como site estático. Antes, esses projetos mostravam "Reconhecemos Node, mas precisamos do comando para iniciar este projeto".
- A busca pela porta de um servidor só adota portas que respondem como servidor web. Antes, um projeto Spring Boot com Gradle ficava preso na porta do daemon do Gradle e mostrava "Site sem resposta".
- Spring Boot com Gradle inicia com `--no-daemon`: uma partida que falha, por exemplo com a porta ocupada, não deixa mais um processo Java do Gradle rodando em segundo plano.
- Num projeto Lumen recém-clonado, **Instalar dependências** também cria o `.env` a partir do `.env.example`.
- O Perl que acompanha o Git para Windows é encontrado em **Linguagens neste computador**.
- Go no layout `cmd/<nome>/main.go` passa a ter comando: um processo por programa, com `go run ./cmd/<nome>` na raiz do módulo. Antes, o cartão dizia que nenhum comando foi identificado.
- Dart encontra os programas criados por `dart create` (`bin/server.dart` e `bin/<pacote>.dart`). Antes, um servidor Dart com Dockerfile, como o do modelo oficial, só podia iniciar pelos containers.
- Um app Flutter para desktop não mostra mais **Abrir site** com o endereço do depurador; endereços do DevTools nunca são tratados como o site de nenhum projeto.
- Projetos em pastas com espaço no nome (como "Área de Trabalho") voltaram a iniciar comandos `.cmd` e `.bat`, como `npm`, `mvnw` e `gradlew`: o aplicativo passava ao `cmd.exe` o caminho no formato `\\?\`, que ele não aceita, e o comando rodava na pasta do Windows ou era cortado no espaço.
- Servidores que não escrevem o endereço no log (Express, Spring Boot, Go, servidores Rust próprios) passam a mostrar **Abrir site** e o selo do site: o aplicativo consulta no sistema a porta em que o processo do projeto, ou um processo filho, está ouvindo.
- Quando um servidor fecha porque a porta já está em uso, o cartão diz "Outro programa está usando a porta" em vez de "O processo fechou inesperadamente", em Node, Spring Boot, Flask, PHP, Rust, Go e Python. Se outro programa ocupa a porta durante a execução, o motivo também é esse, e não "parou de responder".
- No Windows, o dono de uma porta passa a ser lido da tabela TCP do sistema em vez do texto do `netstat`, que podia vir traduzido, com estados errados ou sem algumas linhas; um projeto que ouve na mesma porta que outro programa, em outro endereço, é reconhecido.
- Laravel 11 ou mais novo com SQLite não ficava mais preso em "Arquivo SQLite será preparado ao iniciar o ambiente" sem botão para iniciar: o arquivo `database/database.sqlite`, usado quando `DB_DATABASE` está comentado, é criado ao iniciar.
- A consulta de versão de cada linguagem tem tempo limite de 10 segundos; antes, um programa que não respondia prendia a verificação do ambiente.
- Um site PHP com o Compose numa subpasta (como `docker/`) inicia pelos containers quando o Compose serve o site (serviço com `build:` próprio ou imagem de PHP, Apache, Nginx ou Caddy), não só no WordPress. Um Compose só com o banco ou com phpMyAdmin mantém o servidor PHP local.
- Um `compose.yaml` salvo com BOM, comum em editores no Windows, não aparece mais como "YAML inválido".
- No Windows, quando o sistema informa qual programa usa a porta, um projeto só é marcado como Rodando, e só recebe o selo do site, se a porta for do processo dele ou de um processo filho. Se outro programa ocupar a porta, o motivo diz isso, e o log informa qual programa é.
- Quando um projeto falha ao iniciar ou para de funcionar, o cartão mostra o motivo (porta ocupada por outro programa, servidor que não respondeu a tempo, que fechou antes de ficar pronto ou que parou de responder, programa ou pasta não encontrados) em vez de "parou com erro". Processo, comando, código de saída e, quando o sistema informa, o programa que ocupa a porta ficam em **Detalhes técnicos**. O motivo some quando o projeto é iniciado de novo ou parado.
- Em Containers e em Processos, a verificação periódica dos motores não empurra mais a página para baixo enquanto roda; antes, os botões mudavam de lugar sob o cursor.
- Containers aparecem no projeto correto no Windows mesmo quando o Compose registrou a pasta com letra de unidade ou nomes em outra caixa, outro separador, prefixo `\\?\` ou caminho WSL `/mnt/<unidade>`.
- O banco em container é associado ao projeto quando o Compose está em uma subpasta e quando o `DB_HOST` usa o nome do container em vez do serviço.
- Containers Podman mostram nome, portas e URLs; antes os campos em lista retornados pelo Podman eram ignorados.
- Copiar texto do aplicativo para a área de transferência voltou a funcionar; o suporte a clipboard do `eframe` estava desativado no build.
- Com o motor parado, o Compose aparece como "será verificado quando o motor estiver ativo" em vez de ausente.
- Um site PHP com o arquivo Compose numa subpasta (como `docker/`) ou com um Compose que declara a imagem do site em outra pasta do projeto não oferece mais "Servidor PHP" (`php -S`): o cartão mostra **Iniciar com Containers**. Um Compose de outro serviço na mesma pasta pai continua sem afetar o site.
- `DB_HOST` com o nome de um serviço Compose do projeto, como `db`, aparece como banco em container, e não como servidor remoto. A CLI `databases` informa se o container está em execução, parado ou ainda não criado.
- Caminhos de projetos aparecem sem o prefixo `\\?\` na CLI e no aplicativo.
- A CLI aceita pastas relativas, como `.`, em `inspect`, `containers` e `databases`; antes, `databases .` não encontrava o container do banco em execução.

## [0.4.0-alpha.7] - 2026-10-22

### Adicionado

- Reconhecimento de WordPress e leitura segura do `DB_HOST` literal para diagnóstico MySQL.
- Listagem de containers reais Docker/Podman em Processos, com estado, portas, URLs candidatas, logs e ações de iniciar, parar ou reiniciar.
- Navegação de Projetos para Processos e Containers filtrada pelo projeto selecionado.

### Alterado

- Cartões de projeto destacam o próximo passo, mostram a resposta HTTP confirmada separadamente do estado dos serviços e agrupam opções secundárias em **Mais ações**.
- WordPress com Compose abre o fluxo de Containers em vez de iniciar um servidor PHP local sem acesso ao host do banco na rede Compose.
- README e Ajuda integrada descrevem os fluxos atuais, os limites do diagnóstico de bancos e a recuperação de falhas.

## [0.4.0-alpha.6] - 2026-10-21

### Adicionado

- Página **Ajuda** no aplicativo com tutorial de primeiros passos, explicação dos estados, orientação para bancos e containers e solução de problemas comuns.
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
