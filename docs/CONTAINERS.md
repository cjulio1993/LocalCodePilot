# Containers: execução controlada em validação

Esta entrega detecta Dockerfile, Containerfile, compose.yaml, compose.yml e os nomes legados docker-compose.yaml/yml. Projetos exclusivamente baseados nesses arquivos entram no catálogo como Containers; projetos híbridos preservam suas tecnologias nativas.

## Desktop

A aba Containers mostra os manifests dos projetos descobertos e oferece **Verificar motores e arquivos**. As consultas são executadas em segundo plano, com limite de quatro segundos por comando, sem consoles externos.

Docker e Podman são verificados independentemente:

- `--version`: disponibilidade da CLI;
- `info`: acesso ao motor;
- `compose version`: disponibilidade do plugin ou provedor Compose.

O aplicativo não instala ferramentas nem inicia VMs. O provedor Compose do Podman precisa estar instalado separadamente quando não estiver disponível.

### Proteção contra conflitos de nome Compose

O Docker e o Podman tratam todos os arquivos Compose com o mesmo **nome de projeto** como um único ambiente. Dois projetos com o Compose numa pasta `docker/` recebem o nome `docker`: o `up` de um recria os containers do outro e reaproveita os volumes de dados dele, e o `stop` de um para os containers do outro.

O aplicativo identifica cada ambiente por três informações:

- **motor** (Docker ou Podman);
- **nome efetivo**, perguntado ao próprio provedor com `compose --file … config` na pasta do manifesto, com os mesmos arquivos e variáveis usados no `up`. Assim valem `name:`, `COMPOSE_PROJECT_NAME` (ambiente ou `.env`) e o nome da pasta. Só o nome é lido da saída, que pode conter segredos;
- **pasta de origem**, gravada pelo Compose em cada container (`com.docker.compose.project.working_dir`).

| Situação | Consequência |
| --- | --- |
| Nome único e containers só desta pasta | Nenhuma restrição |
| Outro projeto do catálogo resolve o mesmo nome | Bloqueado: "Nome de ambiente compartilhado" |
| Containers com esse nome criados a partir de outra pasta (terminal ou outra ferramenta) | Bloqueado: "Ambiente em uso por outra pasta" |
| Volumes com esse nome e nenhum container que mostre a origem | Bloqueado até o usuário confirmar que os dados são do projeto |
| Nome não informado pelo provedor, motor sem resposta ou container sem pasta de origem | Bloqueado: "Não foi possível confirmar o ambiente" |

Quando bloqueado, **Iniciar ambiente** e **Parar serviços** ficam desativados, assim como iniciar, parar e reiniciar os containers desse ambiente em Processos: um ID exato não deixa claro o impacto num ambiente compartilhado. **Validar configuração**, **Consultar estado** e os logs continuam disponíveis. O cartão explica o motivo e sugere um `name:` único para copiar. O aplicativo não altera os arquivos do projeto e não usa `--project-name` automaticamente.

Logo antes de executar, depois da confirmação, a verificação é refeita com uma consulta nova ao motor; se algo mudou (por exemplo, containers criados pelo terminal), o comando é cancelado sem executar.

Ao adotar um nome novo, containers e volumes criados com o nome antigo continuam existindo e não são reaproveitados. Para pará-los, use o terminal na pasta de origem: `docker compose -p <nome-antigo> stop` (ou `podman compose`).

### Preparação guiada dos motores

Quando um motor não está pronto, o cartão dele mostra o que falta e os passos para resolver. O LocalCodePilot **não executa** esses passos: cada comando tem um botão **Copiar** para o usuário rodar no próprio terminal, e cada referência aponta para a documentação oficial.

| Situação | Orientação |
| --- | --- |
| CLI não encontrada | Requisitos e instalação: Docker Desktop (Windows/macOS) ou Docker Engine (Linux); Podman pelo `winget`, Homebrew ou gerenciador da distribuição. No Windows, `wsl --install` aparece primeiro quando `wsl --status` falha. |
| Motor parado | `docker desktop start` (ou `systemctl` no Linux); no Podman, `podman machine init` e/ou `podman machine start`, conforme `podman machine list`. |
| Motor pronto sem Compose | Docker: atualizar o Docker Desktop ou instalar o plugin; Podman: instalar um provedor, como o Docker Compose standalone. |
| Consulta sem resposta | Rodar `docker info` ou `podman info` para ver o erro completo. |

O Compose só é diagnosticado com o motor ativo: com a máquina do Podman parada, `podman compose version` também falha e não indica que o provedor esteja ausente.

A instalação do Docker Desktop mostra as condições de licença: é gratuito para uso pessoal, educação, open source não comercial e pequenas empresas; empresas maiores precisam de assinatura. O Podman é open source (Apache 2.0). No Linux, o app alerta que entrar no grupo `docker` equivale a privilégios de administrador.

As consultas adicionais são somente leitura: `wsl --status` no Windows e `podman machine list --format json` no Windows e macOS. Enquanto algum motor está ausente, **Verificar motores e arquivos** também relê o `PATH` do sistema, de modo que um motor instalado pelo terminal é encontrado sem reiniciar o aplicativo.

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

O comando CLI continua sendo somente diagnóstico e mostra os comandos propostos sem executá-los. Motores que não estão prontos recebem a mesma orientação da aba Containers. Um manifesto inválido é relatado e não produz comandos; a CLI retorna falha nesse caso. A execução controlada está disponível apenas no desktop.

## Limites atuais

- A seleção prioriza compose.yaml, compose.yml, docker-compose.yaml e docker-compose.yml, nessa ordem.
- Apenas o manifesto principal é analisado para montar a prévia; os overrides reconhecidos são incluídos nos comandos executados.
- `include`, `extends`, fragmentos herdados e interpolação não são resolvidos pela prévia local. Os avisos indicam quando ela não representa a configuração completa, e o Compose continua responsável pela resolução efetiva.
- Perfis são listados sem ativação automática.
- O usuário ainda pode iniciar diretamente; para projetos desconhecidos, execute **Validar configuração** primeiro.
- Um Dockerfile isolado oferece somente build, sem nomear a imagem nem oferecer `run`.
- A parada usa `stop`; não há remoção de containers, redes ou volumes.
- O app não instala Docker/Podman, não inicia Docker Desktop ou máquinas Podman e não aceita entrada interativa no comando; ele apenas orienta esses passos.
- A orientação não verifica virtualização no firmware nem espaço em disco; esses requisitos aparecem como texto e nos links oficiais.
- O CI valida os fluxos reais de comandos Docker e Podman (`config`, `up`, `ps`, `logs`, `stop` e `build`) com um projeto mínimo. No Windows 11, Docker Desktop passou pela interface em `config`, `up`, `ps`, logs, `stop`, build e cancelamento. Podman 6.1.4 passou pela interface em `config`, `up`, `ps`, logs e `stop`; o build passou pela CLI.

## Antes da alpha 6

Validar, em projetos descartáveis e com motores reais, os fluxos de configuração, download/build, `up`, `stop`, `ps`, logs, cancelamento, timeout e encerramento do aplicativo. Falhas encontradas nessa matriz devem ser corrigidas antes da publicação.

Referências: [modelo do Docker Compose](https://docs.docker.com/compose/intro/compose-application-model/), [configuração efetiva](https://docs.docker.com/reference/cli/docker/compose/config/) e [provedores do Podman Compose](https://docs.podman.io/en/latest/markdown/podman-compose.1.html).
