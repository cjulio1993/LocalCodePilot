LocalCodePilot para macOS
========================

Requisitos
----------
- macOS 12 ou mais recente.
- Mac Intel ou Apple Silicon; o aplicativo é universal.

Instalação
----------
1. Abra o arquivo LocalCodePilot-<versão>-macos-universal.dmg.
2. Arraste LocalCodePilot para a pasta Applications mostrada na janela.
3. Abra LocalCodePilot pela pasta Aplicativos.

Os artefatos de validação usam assinatura ad hoc. Se o macOS bloquear a primeira
abertura, clique com o botão direito no aplicativo, escolha Abrir e confirme.
O DMG da Alpha 6 é um pacote experimental para validação em Macs reais. Ele não
possui notarização da Apple e ainda não representa suporte estável ao macOS.

Atualizações
------------
O aplicativo avisa quando existe uma nova versão e abre o DMG correto para macOS.
A substituição automática do aplicativo continua disponível somente no Windows.

Containers
----------
Docker Desktop ou Podman e um provedor Compose precisam estar instalados e em
execução. O LocalCodePilot não instala nem inicia o motor.

Roteiro de validação
---------------------
1. Informe o modelo do Mac, processador Intel/Apple Silicon e versão do macOS.
2. Confira o SHA-256 do DMG, monte o arquivo e arraste o app para Applications.
3. Registre se a primeira abertura foi direta ou exigiu botão direito > Abrir.
4. Confirme que a janela aparece, permite navegar e encontra os projetos esperados.
5. Adicione e remova uma pasta de busca descartável.
6. Feche e abra o app novamente; confirme que projetos e preferências persistem.
7. Se houver um projeto seguro para teste, inicie e pare o ambiente e confira os logs.
8. Envie a descrição do resultado e capturas de tela sem senhas ou dados privados.

Dados locais
------------
As configurações ficam em:
~/Library/Application Support/LocalCodePilot
