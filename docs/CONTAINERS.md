# Containers: diagnóstico e prévia inicial

Esta entrega detecta Dockerfile, Containerfile, compose.yaml, compose.yml e os nomes legados docker-compose.yaml/yml. Projetos exclusivamente baseados nesses arquivos entram no catálogo como Containers; projetos híbridos preservam suas tecnologias nativas.

## Desktop

A aba Containers mostra os manifests dos projetos descobertos e oferece Verificar motores e arquivos. As consultas são executadas em segundo plano, com limite de quatro segundos por comando, sem consoles externos.

Docker e Podman são verificados independentemente:

- `--version`: disponibilidade da CLI;
- `info`: acesso ao motor;
- `compose version`: disponibilidade do plugin ou provedor Compose.

O aplicativo não instala ferramentas, inicia VMs, baixa imagens ou executa ambientes durante o diagnóstico. O provedor Compose do Podman precisa estar instalado separadamente quando não estiver disponível.

Serviços, imagens, build, portas declaradas e perfis aparecem numa prévia local do YAML. Variáveis de ambiente, segredos e comandos internos dos serviços não são exibidos. As portas são declarações, não URLs ou listeners confirmados.

## CLI

```powershell
cargo run -p localcodepilot-cli -- containers "C:\caminho\do projeto"
```

O comando mostra diagnóstico e comandos propostos, sem executá-los. Um manifesto inválido é relatado e não produz comandos; a CLI retorna falha nesse caso.

## Limites da prévia

- A seleção prioriza compose.yaml, compose.yml, docker-compose.yaml e docker-compose.yml, nessa ordem.
- Apenas o arquivo indicado é analisado; os comandos propostos usam explicitamente esse arquivo.
- Overrides, include, extends, fragmentos herdados e interpolação não são resolvidos nesta etapa. Os avisos indicam quando uma prévia não representa a configuração completa.
- Perfis são listados sem ativação automática.
- O manifesto efetivo deve ser validado com Compose antes de execução.
- Um Dockerfile isolado oferece prévia de build, sem inventar imagem, porta ou comando run.
- A parada proposta usa stop; não há remoção de volumes.

## Próxima entrega

Confirmação do plano de execução, integração de up/stop/logs/ps, estado de containers, portas confirmadas, cancelamento e recuperação. O fluxo precisa ser validado com motores reais antes de ser anunciado como execução integrada.

Referências: [modelo do Docker Compose](https://docs.docker.com/compose/intro/compose-application-model/), [configuração efetiva](https://docs.docker.com/reference/cli/docker/compose/config/) e [provedores do Podman Compose](https://docs.podman.io/en/latest/markdown/podman-compose.1.html).
