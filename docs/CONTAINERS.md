# Containers: execução controlada em validação

Esta entrega detecta Dockerfile, Containerfile, compose.yaml, compose.yml e os nomes legados docker-compose.yaml/yml. Projetos exclusivamente baseados nesses arquivos entram no catálogo como Containers; projetos híbridos preservam suas tecnologias nativas.

## Desktop

A aba Containers mostra os manifests dos projetos descobertos e oferece **Verificar motores e arquivos**. As consultas são executadas em segundo plano, com limite de quatro segundos por comando, sem consoles externos.

Docker e Podman são verificados independentemente:

- `--version`: disponibilidade da CLI;
- `info`: acesso ao motor;
- `compose version`: disponibilidade do plugin ou provedor Compose.

O aplicativo não instala ferramentas nem inicia VMs. O provedor Compose do Podman precisa estar instalado separadamente quando não estiver disponível.

Serviços, imagens, build, portas declaradas e perfis aparecem numa prévia local do YAML. Variáveis de ambiente, segredos e comandos internos dos serviços não são exibidos. As portas da prévia são declarações; **Consultar estado** usa a saída efetiva de `compose ps --all` no Docker ou `compose ps` no Podman para mostrar o estado e as portas publicadas pelo motor.

Quando o motor e o provedor Compose selecionados estão disponíveis, cada projeto oferece estas ações:

- **Validar configuração** executa `compose config --quiet` no Docker e `compose config` no Podman para manter compatibilidade com versões anteriores do `podman-compose`;
- **Iniciar ambiente** executa `compose up --detach` e pode baixar ou construir imagens;
- **Parar serviços** executa `compose stop`, sem remover volumes;
- **Consultar estado** executa `compose ps --all` no Docker e `compose ps` no Podman;
- **Consultar logs** lê as 200 linhas mais recentes com `compose logs --tail 200`;
- um Dockerfile isolado oferece **Construir imagem**, sem inventar nome, porta ou comando `run`.

Antes de executar, o app mostra o comando completo, a pasta e o efeito esperado. A ação só começa após confirmação. A saída fica numa área rolável dentro do app, códigos de saída diferentes de zero são relatados como falha e o usuário pode cancelar o comando. Cada execução tem limite de 30 minutos e o processo de comando é encerrado se o app for fechado normalmente.

Arquivos `compose.override.*` reconhecidos são passados ao provedor depois do manifesto principal. A prévia local continua mostrando somente as declarações do arquivo principal; o próprio Compose resolve a configuração efetiva durante a validação e a execução.

## CLI

```powershell
cargo run -p localcodepilot-cli -- containers "C:\caminho\do projeto"
```

O comando CLI continua sendo somente diagnóstico e mostra os comandos propostos sem executá-los. Um manifesto inválido é relatado e não produz comandos; a CLI retorna falha nesse caso. A execução controlada está disponível apenas no desktop.

## Limites atuais

- A seleção prioriza compose.yaml, compose.yml, docker-compose.yaml e docker-compose.yml, nessa ordem.
- Apenas o manifesto principal é analisado para montar a prévia; os overrides reconhecidos são incluídos nos comandos executados.
- `include`, `extends`, fragmentos herdados e interpolação não são resolvidos pela prévia local. Os avisos indicam quando ela não representa a configuração completa, e o Compose continua responsável pela resolução efetiva.
- Perfis são listados sem ativação automática.
- O usuário ainda pode iniciar diretamente; para projetos desconhecidos, execute **Validar configuração** primeiro.
- Um Dockerfile isolado oferece somente build, sem nomear a imagem nem oferecer `run`.
- A parada usa `stop`; não há remoção de containers, redes ou volumes.
- O app não instala Docker/Podman, não inicia Docker Desktop ou máquinas Podman e não aceita entrada interativa no comando.
- O CI valida os fluxos reais de comandos Docker e Podman (`config`, `up`, `ps`, `logs`, `stop` e `build`) com um projeto mínimo. No Windows 11, Docker Desktop passou pela interface em `config`, `up`, `ps`, logs, `stop`, build e cancelamento. Podman 6.1.4 passou pela interface em `config`, `up`, `ps`, logs e `stop`; o build passou pela CLI.

## Antes da alpha 6

Validar, em projetos descartáveis e com motores reais, os fluxos de configuração, download/build, `up`, `stop`, `ps`, logs, cancelamento, timeout e encerramento do aplicativo. Falhas encontradas nessa matriz devem ser corrigidas antes da publicação.

Referências: [modelo do Docker Compose](https://docs.docker.com/compose/intro/compose-application-model/), [configuração efetiva](https://docs.docker.com/reference/cli/docker/compose/config/) e [provedores do Podman Compose](https://docs.podman.io/en/latest/markdown/podman-compose.1.html).
