LocalCodePilot alpha para Windows x64
====================================

Instalador (recomendado): execute LocalCodePilot-<versão>-windows-x64-setup.exe.
Ele instala o app em %LOCALAPPDATA%\Programs\LocalCodePilot, sem pedir administrador,
cria o atalho no Menu Iniciar e aparece em "Aplicativos instalados" para desinstalar.

ZIP (portátil):
1. Extraia todos os arquivos do ZIP para uma pasta.
2. Execute LocalCodePilot.exe.

Depois, adicione uma pasta de busca caso seus projetos não sejam encontrados automaticamente.

Atualizações futuras
--------------------

Quando uma nova versão estiver disponível, use "Atualizar agora" no aviso do
aplicativo ou em Configurações > Atualizações. O LocalCodePilot baixa o executável,
confere o SHA-256, reinicia e remove os arquivos temporários. Se a pasta não permitir
escrita, o aplicativo mantém o download manual do ZIP como alternativa.

Esta é uma versão alpha experimental. Faça backup dos projetos importantes antes de
aceitar alterações de dependências ou migrações sugeridas pelo aplicativo.

O LocalCodePilot não inclui as linguagens nem os bancos de dados. Quando uma linguagem
não está instalada, o cartão do projeto oferece "Preparar ambiente" para instalá-la
pelo winget, depois da sua confirmação.

Integridade do download
-----------------------

Compare o SHA-256 do arquivo baixado com o SHA256SUMS.txt publicado na mesma release:

  Get-ChildItem .\LocalCodePilot-*-windows-x64* | Get-FileHash -Algorithm SHA256

Enquanto os arquivos não tiverem assinatura digital, o Windows pode exibir
"O Windows protegeu o computador": clique em "Mais informações" e depois em
"Executar assim mesmo".

Site: https://localcodepilot.com.br/
Problemas: https://github.com/cjulio1993/LocalCodePilot/issues
Licença: MIT
