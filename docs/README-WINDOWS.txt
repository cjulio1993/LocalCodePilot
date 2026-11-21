LocalCodePilot alpha para Windows x64
====================================

Instalador (recomendado): execute LocalCodePilotSetup-<versão>-windows-x64.exe.
Ele instala o app em %LOCALAPPDATA%\Programs\LocalCodePilot, sem pedir administrador,
cria o atalho no Menu Iniciar e aparece em "Aplicativos instalados" para desinstalar.

ZIP (portátil):
1. Extraia todos os arquivos do ZIP para uma pasta.
2. Execute LocalCodePilot.exe.

Depois, adicione uma pasta de busca caso seus projetos não sejam encontrados automaticamente.

Atualizações futuras
--------------------

Quando uma nova versão estiver disponível, use "Atualizar agora" no aviso do
aplicativo ou em Configurações > Atualizações. O LocalCodePilot confere a assinatura
dos checksums e o SHA-256 do download e mostra as etapas Baixar, Verificar, Instalar e
Reiniciar. Na cópia instalada pelo instalador, ele roda o instalador oficial da nova
versão em silêncio; na cópia portátil (ZIP), troca o executável. Se houver projetos em
execução, ele pergunta antes de encerrá-los. Se algo falhar, o aviso informa o resultado
e a próxima ação. A versão anterior é reaberta quando sua recuperação for confirmada.

Esta é uma versão alpha experimental. Faça backup dos projetos importantes antes de
aceitar alterações de dependências ou migrações sugeridas pelo aplicativo.

O LocalCodePilot não inclui as linguagens. Quando uma linguagem não está instalada,
o cartão do projeto oferece "Preparar ambiente" para instalá-la pelo winget, depois
da sua confirmação.

Bancos de dados: quando o projeto usa PostgreSQL, MySQL ou MariaDB neste computador e
o banco não está rodando, "Iniciar" pode preparar um servidor do LocalCodePilot, depois
da sua confirmação (os dados ficam em %LOCALAPPDATA%\LocalCodePilot\bancos). Para SQL
Server, com o Docker ou o Podman prontos, ele pode criar um container, depois que você
aceitar os termos de licença da Microsoft. Veja a seção Bancos de dados no README.

Integridade do download
-----------------------

Compare o SHA-256 do arquivo baixado com o SHA256SUMS.txt publicado na mesma release:

  Get-ChildItem .\LocalCodePilot-*-windows-x64* | Get-FileHash -Algorithm SHA256

O SHA256SUMS.txt é assinado com a chave do projeto (SHA256SUMS.txt.sig). Para conferir
a assinatura, use a chave pública packaging/update-signing.pub.pem do repositório:

  openssl pkeyutl -verify -pubin -inkey update-signing.pub.pem -rawin -in SHA256SUMS.txt -sigfile SHA256SUMS.txt.sig

Enquanto os arquivos não tiverem assinatura digital, o Windows pode exibir
"O Windows protegeu o computador": clique em "Mais informações" e depois em
"Executar assim mesmo".

Site: https://localcodepilot.com.br/
Problemas: https://github.com/cjulio1993/LocalCodePilot/issues
Licença: MIT
