LocalCodePilot alpha para Windows x64
====================================

1. Extraia todos os arquivos do ZIP para uma pasta.
2. Execute LocalCodePilot.exe.
3. Adicione uma pasta de busca caso seus projetos não sejam encontrados automaticamente.

Atualizações futuras
--------------------

Quando uma nova versão estiver disponível, use "Atualizar agora" no aviso do
aplicativo ou em Configurações > Atualizações. O LocalCodePilot baixa o executável,
confere o SHA-256, reinicia e remove os arquivos temporários. Se a pasta não permitir
escrita, o aplicativo mantém o download manual do ZIP como alternativa.

Esta é uma versão alpha experimental. Faça backup dos projetos importantes antes de
aceitar alterações de dependências ou migrações sugeridas pelo aplicativo.

O LocalCodePilot não inclui Node.js, PHP, Composer, Python, Rust ou bancos de dados.
Os runtimes usados por seus projetos devem estar instalados no computador.

Integridade do download
-----------------------

Compare o SHA-256 do ZIP com o arquivo SHA256SUMS.txt publicado na mesma release:

  Get-ChildItem .\LocalCodePilot-*-windows-x64.zip | Get-FileHash -Algorithm SHA256

Se o executável não tiver assinatura digital, o Windows pode exibir um aviso
do SmartScreen.

Site: https://localcodepilot.com.br/
Problemas: https://github.com/cjulio1993/LocalCodePilot/issues
Licença: MIT
