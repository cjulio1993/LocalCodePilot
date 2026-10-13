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
A alpha 6 só será publicada depois da validação desse fluxo num Mac real.

Atualizações
------------
O aplicativo avisa quando existe uma nova versão e abre o DMG correto para macOS.
A substituição automática do aplicativo continua disponível somente no Windows.

Containers
----------
Docker Desktop ou Podman e um provedor Compose precisam estar instalados e em
execução. O LocalCodePilot não instala nem inicia o motor.

Dados locais
------------
As configurações ficam em:
~/Library/Application Support/LocalCodePilot
