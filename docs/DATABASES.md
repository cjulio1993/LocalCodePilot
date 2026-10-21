# Diagnóstico de bancos de dados

O LocalCodePilot identifica `DB_CONNECTION`, `DB_HOST` e `DB_PORT` em arquivos `.env` e `.env.local` do projeto e, em repositórios Git, de seus módulos. São reconhecidos PostgreSQL (`pgsql`), MySQL, MariaDB, SQL Server (`sqlsrv`/`mssql`) e SQLite.

O desktop mostra o banco detectado e avisa quando uma porta local não responde. A CLI oferece o mesmo diagnóstico:

```text
localcodepilot databases <pasta>
```

Somente endereços de loopback (`127.0.0.1`, `localhost`, `::1`) são testados por TCP. Uma porta acessível não comprova que usuário, senha, banco e tabelas estejam corretos. Destinos remotos e SQLite não são testados por esse comando. Senhas, nomes de usuário e nomes de banco não aparecem na saída.

O diagnóstico ainda não instala nem inicia servidores de banco. Projetos que usam `DATABASE_URL`, configurações de Spring, Django ou .NET sem `DB_CONNECTION`, ou provedores de nuvem exigem detecção adicional. A preparação automática será selecionada pelo banco detectado em cada projeto: PostgreSQL, MySQL, MariaDB ou SQL Server terão provedores próprios; SQLite terá preparação de arquivo e esquema, sem serviço. Cada provedor deverá verificar a origem do instalador, proteger credenciais e preservar instalações existentes. O suporte de instalação será validado separadamente por banco e sistema operacional antes de ser anunciado como completo.
