LocalCodePilot 0.3.0-alpha.1 para Windows x64
================================================

1. Extraia todos os arquivos do ZIP para uma pasta.
2. Execute LocalCodePilot.exe.
3. Adicione uma pasta de busca caso seus projetos não sejam encontrados automaticamente.

Esta é uma versão alpha experimental. Faça backup dos projetos importantes antes de
aceitar alterações de dependências ou migrações sugeridas pelo aplicativo.

O LocalCodePilot não inclui Node.js, PHP, Composer, Python, Rust ou bancos de dados.
Os runtimes usados por seus projetos devem estar instalados no computador.

Integridade do download
-----------------------

Compare o SHA-256 do ZIP com o arquivo SHA256SUMS.txt publicado na mesma release:

  (Get-FileHash .\LocalCodePilot-0.3.0-alpha.1-windows-x64.zip -Algorithm SHA256).Hash

Esta primeira alpha ainda não possui assinatura digital. Por isso, o Windows pode
exibir um aviso do SmartScreen.

Site: https://localcodepilot.com.br/
Problemas: https://github.com/cjulio1993/LocalCodePilot/issues
Licença: MIT
