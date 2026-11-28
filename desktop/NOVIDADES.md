# Novidades do LocalCodePilot

O app mostra esta lista uma vez depois de atualizar, em "LocalCodePilot atualizado para {versão}", e de novo em Configurações > Atualizações > Ver novidades. Ela fica dentro do executável: aparece com ou sem internet.

**Ao mudar a versão do app**, acrescente a seção dela no topo:

- título `## {versão}`, exatamente como em `Cargo.toml`;
- de 1 a 3 linhas `- frase`, em texto simples (sem Markdown nem links), com até 120 caracteres cada;
- as mesmas novidades da seção "Novidades" das notas da release, sem os detalhes.

O CI falha se a versão do app não tiver a sua seção válida aqui.

## 1.0.0-alpha.1

- Encontre seus projetos em uma lista, com busca, estado e próxima ação.
- Abra um projeto para ver o que roda, banco de dados, logs e detalhes técnicos.
- Exporte um diagnóstico para pedir ajuda. Confira o arquivo antes de compartilhar.

## 0.4.0-alpha.10

- Banco de dados conectado de verdade: o app testa o login com os dados de acesso do próprio projeto.
- No Windows, Iniciar prepara PostgreSQL, MySQL e MariaDB para o projeto, com confirmação.
- A atualização usa o instalador oficial, e a janela "Atualizando o LocalCodePilot" acompanha o reinício.
