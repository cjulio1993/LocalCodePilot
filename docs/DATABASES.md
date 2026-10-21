# Diagnóstico de bancos de dados

O LocalCodePilot identifica `DB_CONNECTION`, `DB_HOST` e `DB_PORT` em arquivos `.env` e `.env.local` do projeto e, em repositórios Git, de seus módulos. São reconhecidos PostgreSQL (`pgsql`), MySQL, MariaDB, SQL Server (`sqlsrv`/`mssql`) e SQLite.

O desktop mostra o banco detectado e avisa quando uma porta local não responde. A CLI oferece o mesmo diagnóstico:

```text
localcodepilot databases <pasta>
```

Somente endereços de loopback (`127.0.0.1`, `localhost`, `::1`) são testados por TCP. Uma porta acessível não comprova que usuário, senha, banco e tabelas estejam corretos. Destinos remotos e SQLite não são testados por esse comando. Senhas, nomes de usuário e nomes de banco não aparecem na saída.

O diagnóstico separa servidor local acessível, servidor local indisponível, servidor remoto, arquivo SQLite e configuração incompleta. Para PostgreSQL, MySQL, MariaDB e SQL Server, o LocalCodePilot não instala, inicia nem configura o servidor. Quando um servidor local não responde, a CLI orienta a verificar instalação, serviço e porta. A conexão remota não é alterada.

Ao iniciar um projeto com `DB_CONNECTION=sqlite` e `DB_DATABASE` apontando para um arquivo dentro do projeto, o desktop cria a pasta e o banco quando faltam e verifica a integridade do arquivo. Arquivos fora do projeto e bancos em memória não são criados por esse fluxo. A preparação de SQLite não cria tabelas nem executa migrations.

Projetos que usam `DATABASE_URL`, configurações de Spring, Django ou .NET sem `DB_CONNECTION`, ou provedores de nuvem exigem detecção adicional. O teste TCP não verifica credenciais, existência do banco nem esquema. Esse diagnóstico será ampliado sem instalar ou modificar servidores de banco automaticamente.
