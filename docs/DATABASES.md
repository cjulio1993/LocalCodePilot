# Bancos de dados

O LocalCodePilot identifica `DB_CONNECTION`, `DB_HOST`, `DB_PORT`, `DB_DATABASE`, `DB_USERNAME` e `DB_PASSWORD` em arquivos `.env` e `.env.local` do projeto (e, em repositórios Git, de seus módulos), além das constantes `DB_*` do `wp-config.php`. São reconhecidos PostgreSQL (`pgsql`), MySQL, MariaDB, SQL Server (`sqlsrv`/`mssql`) e SQLite.

## Teste de conexão

Para PostgreSQL, MySQL, MariaDB e SQL Server, o desktop testa o login com o banco, o usuário e a senha do próprio projeto. O cartão só mostra **Banco de dados conectado.** depois que o banco aceita esse login; uma porta aberta ou um container ativo não bastam. O teste diferencia:

- banco que não está rodando: o projeto fica **Parado**, com o motivo;
- acesso recusado e banco de dados que não existe: o projeto fica **Com problema**, com a orientação;
- dados de acesso que o app não consegue ler: o cartão diz o que falta.

O teste roda em Windows, macOS e Linux. Ele é automático para bancos neste computador (`127.0.0.1`, `localhost`, `::1`). Para um banco em outro computador, ele só roda quando você escolhe **Testar conexão** em **Detalhes técnicos**. A cada nova verificação dos projetos, o resultado anterior é descartado.

**Detalhes técnicos** mostra onde está a configuração, o endereço testado, o usuário, o banco, o resultado e quando ele foi obtido. A senha nunca aparece no cartão, em Detalhes, nos logs salvos nem no texto copiado.

## Servidor preparado pelo LocalCodePilot

Quando o projeto usa PostgreSQL, MySQL ou MariaDB neste computador e nada responde na porta configurada, **Iniciar** prepara um servidor do LocalCodePilot para o mesmo motor que o projeto pede (MySQL nunca é trocado por MariaDB, nem o contrário).

- **Primeira vez.** A confirmação **Preparar banco de dados para este projeto?** mostra o download, o espaço necessário depois do preparo, o banco e o usuário que serão criados e onde ficam os dados. Nada é baixado antes de **Preparar e iniciar**.
- **Pacote.** Cada motor tem uma versão fixada no catálogo do app, baixada por HTTPS e conferida por SHA-256 antes de ser usada. Se o arquivo não conferir, ele é recusado; **Tentar de novo** baixa e confere outra cópia. Não há opção de ignorar a falha.
- **Servidor.** Escuta só em `127.0.0.1`. A senha de administração fica no Gerenciador de Credenciais do Windows (no macOS e no Linux, num arquivo que só o seu usuário lê, com permissão `0600`), nunca em linha de comando. O banco e o usuário do projeto são criados com permissão só nesse banco.
- **Ordem.** O cartão mostra as etapas em **Iniciando** (download, preparo, início do banco). Os processos do projeto só iniciam depois que o banco aceita os dados de acesso do projeto.
- **Vezes seguintes.** O servidor do app é iniciado sem pergunta.
- **Ao fechar o app.** O LocalCodePilot tenta encerrar o servidor com o desligamento normal. Se ele não parar a tempo, o app avisa na próxima abertura, depois de conferir que o servidor ainda é dele e continua rodando.
- **Dados.** Ficam em `%LOCALAPPDATA%\LocalCodePilot\bancos` (Windows), `~/Library/Application Support/LocalCodePilot/bancos` (macOS) ou `~/.local/share/localcodepilot/bancos` (Linux) e permanecem entre as sessões.

No Windows, o PostgreSQL e o MySQL precisam do Microsoft Visual C++ Redistributable. Sem ele, nada é preparado, e a confirmação orienta a instalação.

### PostgreSQL no macOS e no Linux

A EDB não publica checksum do PostgreSQL para macOS nem binários atuais para Linux. Por isso, no macOS e no Linux, **Iniciar** prepara o PostgreSQL local em container, com o Docker ou o Podman, usando a imagem oficial do PostgreSQL (`docker.io/library/postgres:19.0-trixie`) fixada por digest. Depois do download, o app confere que a cópia do motor tem esse digest; se não tiver, nada é criado. A confirmação **Preparar PostgreSQL para este projeto?** mostra o download estimado (cerca de 160 MB), o espaço em disco (cerca de 550 MB), o banco e o usuário. A porta é publicada só em `127.0.0.1`, e os dados ficam num volume do motor (`localcodepilot-pg-<identificador>`), que permanece depois de fechar o app. A senha inicial do administrador passa por um arquivo temporário só do seu usuário e é trocada logo após o preparo pela senha guardada no cofre. O usuário do projeto é dono só do próprio banco, sem poderes no servidor. O andamento aparece no cartão: baixando a imagem, conferindo a imagem, criando o container, preparando o banco e o usuário, testando a conexão do projeto. Sem Docker nem Podman instalados, o cartão diz "O PostgreSQL deste projeto não está rodando. Para prepará-lo, instale o Docker ou o Podman e ligue-o em Ambiente." e oferece **Abrir Ambiente**. Com o motor instalado e parado, o app pede para ligá-lo e segue sozinho depois. Ao fechar o app, o container é parado, e a parada é confirmada pelo motor.

### Onde o preparo existe

| Motor | Windows (x64) | macOS | Linux |
|---|---|---|---|
| PostgreSQL 19 | sim (pacote nativo) | em container, com Docker ou Podman | em container, com Docker ou Podman |
| MySQL 9.7 | sim | macOS 15 ou mais novo (Apple Silicon e Intel); antes do 15, em container | x86_64 com glibc 2.28 ou mais nova; ARM, em container |
| MariaDB 12.3 | sim | em container, com Docker ou Podman | x86_64 com glibc 2.28 ou mais nova; ARM, em container |

Cada pacote é o oficial do projeto, com SHA-256 conferido contra uma fonte independente (assinatura GPG da Oracle para o MySQL, `sha256sums.txt` assinado para o MariaDB). MariaDB no macOS e MySQL/MariaDB no Linux ARM são preparados em container com Docker ou Podman, após confirmação. A imagem oficial é fixada por digest, a porta fica restrita a 127.0.0.1 e os dados permanecem num volume do motor de containers. As imagens são `mysql:9.7.4-oraclelinux9` (download de cerca de 250 MB, cerca de 1,1 GB em disco) e `mariadb:12.3.3-noble` (cerca de 100 MB, cerca de 450 MB em disco); a imagem oficial do MariaDB ainda não tem a 12.3.4. O mesmo vale quando o sistema não atende ao pacote nativo (glibc antiga, macOS anterior ao 15, falta da libaio). O usuário do projeto tem acesso só ao próprio banco; por isso, um banco que não existe aparece como acesso recusado, que é como o MySQL e o MariaDB respondem. O app nunca troca o motor pedido por outro.

No Linux, o MySQL precisa da biblioteca `libaio` do sistema (pacote `libaio1` ou `libaio1t64`, presente na maioria das instalações). No Ubuntu 24.04 e mais novos, que só têm `libaio.so.1t64`, o app cria o link que o MySQL procura dentro da pasta do próprio pacote, sem mexer no sistema. Sem a biblioteca, ou com Alpine (sem glibc), o cartão diz que o preparo não está disponível e por quê.

## SQL Server em container (Windows)

Com o Docker ou o Podman prontos, **Iniciar** num projeto com SQL Server neste computador abre **Preparar SQL Server para este projeto?**:

- a imagem oficial do SQL Server 2022 Developer (gratuito para desenvolvimento e teste, nos termos da Microsoft) tem cerca de 500 MB para baixar e ocupa cerca de 1,6 GB depois do preparo;
- **Preparar e iniciar** só fica disponível depois que você marca **Li e aceito os termos de licença do Microsoft SQL Server**. O app nunca aceita os termos por você;
- a imagem é fixada por digest, o container só escuta em `127.0.0.1`, e os dados ficam num volume do motor de containers;
- a senha inicial do administrador é trocada logo depois do preparo, e a definitiva fica no Gerenciador de Credenciais do Windows;
- o banco e o login do projeto são criados com acesso só a esse banco.

Sem Docker nem Podman prontos, o projeto fica **Com problema**, e a ação **Abrir Ambiente** leva à página onde o motor é ligado. O app não instala o Docker nem o Podman, não instala o SQL Server direto no Windows, e a imagem funciona só em computadores x64.

Ao fechar o app, o container é parado, e a parada é conferida. Se nenhuma senha de administração guardada entrar no SQL Server, **Detalhes técnicos** oferece **Criar SQL Server novo**: ele cria outro container e outro volume e mantém o volume anterior com os dados.

## Servidor que você já tem

Se um banco já responde na porta do projeto e não é um servidor do LocalCodePilot, o app só testa o login. Ele não cria banco nem usuário, não altera o servidor e não oferece **Parar** para ele. Quando a identificação de um processo é incerta (por exemplo, depois de reabrir o app ou com um PID reaproveitado pelo Windows), ele é tratado como servidor alheio.

## Correções guiadas

Quando o banco impede o início, o cartão oferece a correção que for segura, sempre com confirmação e com um novo teste do login antes de iniciar:

- **Usar outra porta:** quando outro programa ocupa a porta, muda só `DB_PORT` no `.env`, guarda uma cópia ao lado e prepara o banco do LocalCodePilot na porta nova. A confirmação avisa que isso pode fazer o projeto usar outro banco de dados. Se o login falhar, o `.env` volta ao original, mas só se ainda for exatamente o que o app gravou.
- **Corrigir conexão:** no servidor do LocalCodePilot, troca a senha do usuário do projeto pela do `.env`. Não aparece se outro projeto conhecido usa o mesmo usuário.
- **Criar banco novo:** depois de duas falhas com sinal de dados danificados, guarda a pasta atual e cria outra. O cartão avisa que o projeto está conectado a um banco novo e que os dados anteriores foram guardados. A pasta anterior nunca é apagada.

## Compose

Quando o host (por exemplo, `db` no `DB_HOST` do `wp-config.php`) é o nome de um serviço declarado num arquivo Compose do projeto, inclusive numa subpasta como `docker/`, o banco é tratado como **serviço do Compose**. Ele só existe na rede dos containers, é iniciado em **O que roda**, na página do projeto, com as proteções de conflito de nomes de sempre, e o cartão mostra "Container ativo; conexão não verificada.". O LocalCodePilot não prepara outro servidor para ele.

## Banco em outro computador

Hosts remotos ou de rede não recebem preparo nem mudança. O teste de login só roda quando você escolhe **Testar conexão** em **Detalhes técnicos**.

## SQLite

Ao iniciar um projeto com `DB_CONNECTION=sqlite` e `DB_DATABASE` apontando para um arquivo dentro do projeto, o desktop cria a pasta e o banco quando faltam e verifica a integridade do arquivo. Arquivos fora do projeto e bancos em memória não são criados por esse fluxo. A preparação de SQLite não cria tabelas nem executa migrations.

## CLI

```text
localcodepilot databases <pasta>
```

A CLI mostra o banco detectado e testa só a porta TCP de endereços de loopback. Ela não testa o login e não prepara servidores. Senhas, nomes de usuário e nomes de banco não aparecem na saída.

## Limites

- Projetos que usam só `DATABASE_URL`, configurações de Spring, Django ou .NET sem `DB_CONNECTION`, ou provedores de nuvem ainda não são reconhecidos.
- O app não cria tabelas nem executa migrations.
- MongoDB e Redis não são cobertos.
