<#
.SYNOPSIS
    Testa a atualização pelo app entre duas versões, numa pasta descartável (Windows).

.DESCRIPTION
    Compila o mesmo commit como duas versões de teste (0.4.0-alpha.90 e
    0.4.0-alpha.91), com a feature update-test-feed, que nunca entra numa
    release. Depois roda cada cenário com um feed de releases local
    (desktop/examples/update_test_feed.rs), clica em "Atualizar agora" pela
    UI Automation (por PID, sem mouse) e confere o resultado sozinho.

    Tudo fica na pasta -Pasta: instalação, APPDATA, LOCALAPPDATA e TEMP
    isolados. O app instalado de verdade e as configurações reais não são
    tocados. O script só encerra processos que rodam de dentro dessa pasta.

    Cenários:
      sucesso            a versão nova abre e mostra que foi atualizada;
      sha-errado         o SHA-256 publicado não confere: nada é trocado;
      queda-no-meio      o servidor cai durante o download: nada é trocado;
      rollback           a versão nova fecha ao iniciar: a anterior volta;
      confirmacao-lenta  a versão nova demora mais que a espera do atualizador;
      sem-conexao        o feed não responde: nenhum aviso de atualização;
      assinatura-invalida  os checksums foram assinados com outra chave: nada é baixado nem trocado;
      processos-ativos   com um projeto rodando, pergunta antes de baixar; Agora não mantém tudo;
      processo-durante-download  projeto iniciado no meio do download: pergunta de novo antes de fechar.
      instalador-sucesso   instalado pelo instalador (Inno): atualiza rodando o instalador novo em silêncio;
      instalador-falha     o instalador novo não conclui: a versão anterior reabre com Tentar de novo e Detalhes técnicos;
      instalador-nao-abriu a versão instalada não abre: a anterior volta, e a lista de programas mostra a nova.

    Os cenários do instalador precisam do Inno Setup 6. O instalador de teste usa
    outro AppId e não cria atalhos; a entrada dele em Aplicativos instalados é
    removida no fim de cada cenário.

    As versões de teste confiam numa chave Ed25519 descartável, gerada na
    pasta do teste com o OpenSSL (o do Git for Windows serve), nunca na chave
    das releases.

.EXAMPLE
    .\scripts\e2e-update-windows.ps1
    .\scripts\e2e-update-windows.ps1 -Cenario sucesso,rollback -Recompilar
#>
param(
    [ValidateSet('todos', 'sucesso', 'sha-errado', 'queda-no-meio', 'rollback', 'confirmacao-lenta', 'sem-conexao', 'assinatura-invalida', 'processos-ativos', 'processo-durante-download', 'instalador-sucesso', 'instalador-falha', 'instalador-nao-abriu')]
    [string[]]$Cenario = @('todos'),
    [string]$Pasta = (Join-Path ([IO.Path]::GetTempPath()) 'lcp-e2e-update'),
    [switch]$Recompilar,
    # Pasta para salvar a janela de cada falha de download, com "Detalhes técnicos" aberto.
    [string]$Capturas = ''
)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'

$repositorio = Split-Path -Parent $PSScriptRoot
$versaoAntiga = '0.4.0-alpha.90'
$versaoNova = '0.4.0-alpha.91'
# AppId do instalador descartável: nunca o da instalação real.
$appIdDoTeste = '{7D1E2E57-0A11-4C0D-8E2E-5E7E57A1B2C3}'
$todos = @('sucesso', 'sha-errado', 'queda-no-meio', 'rollback', 'confirmacao-lenta', 'sem-conexao', 'assinatura-invalida', 'processos-ativos', 'processo-durante-download', 'instalador-sucesso', 'instalador-falha', 'instalador-nao-abriu')
if ($Cenario -contains 'todos') { $Cenario = $todos }

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
$uia = [System.Windows.Automation.AutomationElement]

# --- Pasta descartável -------------------------------------------------------

$marcador = '.lcp-e2e-update'
$Pasta = [IO.Path]::GetFullPath($Pasta)
if ($Pasta.TrimEnd('\') -eq ([IO.Path]::GetPathRoot($Pasta)).TrimEnd('\')) {
    throw "A pasta do teste não pode ser a raiz de um disco ($Pasta). Escolha uma subpasta com -Pasta."
}
if ((Test-Path -LiteralPath $Pasta) -and (Get-ChildItem -LiteralPath $Pasta -Force | Select-Object -First 1) -and
    -not (Test-Path -LiteralPath (Join-Path $Pasta $marcador))) {
    throw "A pasta $Pasta já existe e não foi criada por este script. Escolha outra com -Pasta."
}
New-Item -ItemType Directory -Force $Pasta | Out-Null
Set-Content -LiteralPath (Join-Path $Pasta $marcador) -Value 'pasta descartável do teste de atualização' -Encoding UTF8
$binarios = New-Item -ItemType Directory -Force (Join-Path $Pasta 'bin')

# Prefixo com o separador no fim: "C:\teste\e2e\" não casa com "C:\teste\e2e-outro\".
$prefixoDaPasta = $Pasta.TrimEnd('\') + '\'

function Test-DentroDaPasta([string]$Caminho) {
    $Caminho -and [IO.Path]::GetFullPath($Caminho).StartsWith($prefixoDaPasta, [StringComparison]::OrdinalIgnoreCase)
}

function Stop-ProcessosDaPasta {
    Get-Process -ErrorAction SilentlyContinue |
        Where-Object { try { Test-DentroDaPasta $_.Path } catch { $false } } |
        ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
}

# --- Compilação das versões de teste ----------------------------------------

function Invoke-Cargo([string[]]$Argumentos) {
    Push-Location $repositorio
    $preferencia = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & cargo @Argumentos
        if ($LASTEXITCODE -ne 0) { throw "cargo $($Argumentos -join ' ') falhou com código $LASTEXITCODE." }
    }
    finally {
        $ErrorActionPreference = $preferencia
        Pop-Location
    }
}

# --- Chaves de assinatura descartáveis ----------------------------------------

$comandoOpenssl = Get-Command openssl -ErrorAction SilentlyContinue
$openssl = if ($comandoOpenssl) { $comandoOpenssl.Source } else { Join-Path $env:ProgramFiles 'Git\usr\bin\openssl.exe' }
if (-not (Test-Path -LiteralPath $openssl)) { throw 'O OpenSSL não foi encontrado (instale o Git for Windows).' }
$chaves = New-Item -ItemType Directory -Force (Join-Path $Pasta 'chaves-de-teste')

function Get-Chave([string]$Nome) {
    $privada = Join-Path $chaves.FullName "$Nome.pem"
    if (-not (Test-Path -LiteralPath $privada)) {
        & $openssl genpkey -algorithm ed25519 -out $privada
        if ($LASTEXITCODE -ne 0) { throw "Não foi possível gerar a chave de teste $Nome." }
        # As versões já compiladas confiam na chave anterior.
        Get-ChildItem -LiteralPath $binarios -Filter '*.exe' | Where-Object Name -ne 'update_test_feed.exe' | Remove-Item -Force
    }
    $der = Join-Path $chaves.FullName "$Nome.pub.der"
    & $openssl pkey -in $privada -pubout -outform DER -out $der
    $bytes = [IO.File]::ReadAllBytes($der)
    [pscustomobject]@{ Privada = $privada; Publica = (($bytes[12..43] | ForEach-Object { $_.ToString('x2') }) -join '') }
}

function Set-Assinatura([string]$Checksums, [string]$Privada) {
    & $openssl pkeyutl -sign -inkey $Privada -rawin -in $Checksums -out "$Checksums.sig"
    if ($LASTEXITCODE -ne 0) { throw 'Não foi possível assinar os checksums de teste.' }
}

$chaveDoTeste = Get-Chave 'feed'
$outraChave = Get-Chave 'outra'

$alvoCargo = (& cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $repositorio 'Cargo.toml') | ConvertFrom-Json).target_directory

# Com -Recompilar, cada binário é compilado uma vez por execução, não por cenário.
$compiladosNestaExecucao = @{}

function Test-Contem([string]$Arquivo, [string]$Texto) {
    [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($Arquivo)).Contains($Texto)
}

function Test-Pronto([string]$Destino) {
    (Test-Path -LiteralPath $Destino) -and (-not $Recompilar -or $compiladosNestaExecucao.ContainsKey($Destino))
}

function Get-Variante([string]$Nome, [string]$Versao, [string]$Inicio) {
    $destino = Join-Path $binarios "$Nome.exe"
    # Versões compiladas antes do teste do instalador não conhecem o AppId de teste.
    if ((Test-Pronto $destino) -and (Test-Contem $destino $appIdDoTeste)) { return $destino }
    Write-Host "Compilando $Nome ($Versao$(if ($Inicio) { ", início: $Inicio" }))..."
    $env:LOCALCODEPILOT_TEST_VERSION = $Versao
    $env:LOCALCODEPILOT_TEST_SIGNING_KEY = $chaveDoTeste.Publica
    $env:LOCALCODEPILOT_TEST_INNO_APPID = $appIdDoTeste
    if ($Inicio) { $env:LOCALCODEPILOT_TEST_STARTUP = $Inicio } else { Remove-Item Env:LOCALCODEPILOT_TEST_STARTUP -ErrorAction SilentlyContinue }
    try {
        Invoke-Cargo @('build', '-p', 'localcodepilot-desktop', '--locked', '--features', 'update-test-feed')
    }
    finally {
        Remove-Item Env:LOCALCODEPILOT_TEST_VERSION, Env:LOCALCODEPILOT_TEST_STARTUP, Env:LOCALCODEPILOT_TEST_SIGNING_KEY, Env:LOCALCODEPILOT_TEST_INNO_APPID -ErrorAction SilentlyContinue
    }
    $compilado = Join-Path $alvoCargo 'debug\localcodepilot-desktop.exe'
    $texto = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($compilado))
    if (-not $texto.Contains($Versao) -or -not $texto.Contains('LOCALCODEPILOT_TEST_FEED') -or -not $texto.Contains($appIdDoTeste)) {
        throw "O executável compilado não é a versão de teste $Versao com o feed local."
    }
    Copy-Item -LiteralPath $compilado -Destination $destino -Force
    $compiladosNestaExecucao[$destino] = $true
    $destino
}

function Get-Servidor {
    $destino = Join-Path $binarios 'update_test_feed.exe'
    if (Test-Pronto $destino) { return $destino }
    Invoke-Cargo @('build', '-p', 'localcodepilot-desktop', '--locked', '--example', 'update_test_feed')
    Copy-Item -LiteralPath (Join-Path $alvoCargo 'debug\examples\update_test_feed.exe') -Destination $destino -Force
    $compiladosNestaExecucao[$destino] = $true
    $destino
}

# --- Interface pela UI Automation (por PID) ---------------------------------

function Get-Janela([int]$ProcessId) {
    $condicao = New-Object System.Windows.Automation.PropertyCondition($uia::ProcessIdProperty, $ProcessId)
    $uia::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $condicao)
}

function Get-TextosNaTela([int]$ProcessId) {
    try {
        $janela = Get-Janela $ProcessId
        if (-not $janela) { return @() }
        @($janela.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
            ForEach-Object { $_.Current.Name } | Where-Object { $_ } | Select-Object -Unique)
    }
    catch { @() }
}

function Find-Botao([int]$ProcessId, [string]$Nome) {
    try {
        $janela = Get-Janela $ProcessId
        if (-not $janela) { return $null }
        $condicao = New-Object System.Windows.Automation.PropertyCondition($uia::NameProperty, $Nome)
        $janela.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condicao)
    }
    catch { $null }
}

function Save-Janela([int]$ProcessId, [string]$Arquivo) {
    $janela = Get-Janela $ProcessId
    if (-not $janela) { return }
    if (-not ('E2eJanela' -as [type])) {
        Add-Type 'using System; using System.Runtime.InteropServices; public static class E2eJanela { [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); }'
    }
    [E2eJanela]::SetForegroundWindow([IntPtr]$janela.Current.NativeWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 600
    $r = $janela.Current.BoundingRectangle
    if ([double]::IsInfinity($r.X) -or $r.Width -le 0) { return }
    $imagem = New-Object System.Drawing.Bitmap ([int]$r.Width), ([int]$r.Height)
    $grafico = [System.Drawing.Graphics]::FromImage($imagem)
    $grafico.CopyFromScreen([int]$r.X, [int]$r.Y, 0, 0, $imagem.Size)
    $imagem.Save($Arquivo)
    $grafico.Dispose(); $imagem.Dispose()
}

function Invoke-Botao($Elemento) {
    if (-not $Elemento) { throw 'Botão não encontrado.' }
    $padroes = $Elemento.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }
    if ($padroes -contains 'InvokePatternIdentifiers.Pattern') {
        $Elemento.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    }
    else { $Elemento.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle() }
}

# Frase de abertura das notas de release do cenário de sucesso (vira o resumo do aviso).
$resumoDoTeste = '**Atualização de teste com resumo das novidades.**'

# O servidor do projeto descartável (node <script>), identificado pelo nome do script.
$scriptDoProjeto = 'lcp-e2e-servidor.js'
function Get-ProcessoDoProjeto {
    @(Get-CimInstance Win32_Process -Filter "Name='node.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -like "*$scriptDoProjeto*" })
}

# Inicia o projeto descartável pelo cartão e confirma o primeiro uso do comando.
function Start-ProjetoDeTeste([int]$ProcessId) {
    $janela = Get-Janela $ProcessId
    $porNome = New-Object System.Windows.Automation.PropertyCondition($uia::NameProperty, 'Iniciar')
    $iniciar = Wait-Ate { @($janela.FindAll([System.Windows.Automation.TreeScope]::Descendants, $porNome)) | Select-Object -First 1 } 30
    Invoke-Botao $iniciar
    Start-Sleep -Milliseconds 1500
    $confirmar = @($janela.FindAll([System.Windows.Automation.TreeScope]::Descendants, $porNome)) | Select-Object -Last 1
    if ($confirmar) { Invoke-Botao $confirmar }
    [bool](Wait-Ate { if (@(Get-ProcessoDoProjeto).Count -gt 0 -and ((Get-TextosNaTela $ProcessId) -contains 'Rodando')) { $true } } 30)
}

function Wait-Ate([scriptblock]$Condicao, [int]$Segundos) {
    $limite = (Get-Date).AddSeconds($Segundos)
    while ((Get-Date) -lt $limite) {
        $valor = & $Condicao
        if ($valor) { return $valor }
        Start-Sleep -Milliseconds 500
    }
    $null
}

# --- Um cenário --------------------------------------------------------------

function Invoke-Cenario([string]$Nome) {
    if ($Nome -like 'instalador-*') { return Invoke-CenarioInstalador $Nome }
    Stop-ProcessosDaPasta
    $raiz = Join-Path $Pasta $Nome
    if (Test-Path -LiteralPath $raiz) { Remove-Item -LiteralPath $raiz -Recurse -Force }
    $instalacao = New-Item -ItemType Directory -Force (Join-Path $raiz 'instalacao')
    $appdata = New-Item -ItemType Directory -Force (Join-Path $raiz 'appdata')
    $localappdata = New-Item -ItemType Directory -Force (Join-Path $raiz 'localappdata')
    $temp = New-Item -ItemType Directory -Force (Join-Path $raiz 'temp')
    $projetos = New-Item -ItemType Directory -Force (Join-Path $raiz 'projetos-vazios')
    $configuracoes = New-Item -ItemType Directory -Force (Join-Path $appdata 'LocalCodePilot')
    # A varredura olha só uma pasta vazia, nunca os projetos reais.
    Set-Content -LiteralPath (Join-Path $configuracoes 'scan-roots.txt') -Value $projetos.FullName -Encoding ascii
    $comProjeto = $Nome -in 'processos-ativos', 'processo-durante-download'
    if ($comProjeto) {
        # Projeto descartável, sem dependências: um servidor Node que fica rodando.
        $projeto = New-Item -ItemType Directory -Force (Join-Path $projetos.FullName 'api-exemplo')
        Set-Content -LiteralPath (Join-Path $projeto.FullName 'package.json') -Encoding ascii `
            -Value "{`"name`":`"api-exemplo`",`"version`":`"1.0.0`",`"private`":true,`"scripts`":{`"dev`":`"node $scriptDoProjeto`"}}"
        Set-Content -LiteralPath (Join-Path $projeto.FullName $scriptDoProjeto) -Encoding ascii `
            -Value 'require("http").createServer((q, r) => r.end("ok")).listen(0, () => console.log("pronto"));'
    }

    $antiga = Get-Variante 'antiga' $versaoAntiga ''
    $inicioDaNova = switch ($Nome) { 'rollback' { 'fail' } 'confirmacao-lenta' { 'slow' } default { '' } }
    $nova = Get-Variante "nova$(if ($inicioDaNova) { "-$inicioDaNova" })" $versaoNova $inicioDaNova
    $alvo = Join-Path $instalacao 'LocalCodePilot.exe'
    Copy-Item -LiteralPath $antiga -Destination $alvo
    $shaAntiga = (Get-FileHash -LiteralPath $antiga -Algorithm SHA256).Hash.ToLowerInvariant()
    $shaNova = (Get-FileHash -LiteralPath $nova -Algorithm SHA256).Hash.ToLowerInvariant()

    # Feed local: lista de releases, executável e SHA256SUMS.txt.
    $servidor = $null
    $feed = New-Item -ItemType Directory -Force (Join-Path $raiz 'feed')
    $tag = "v$versaoNova"
    $assets = New-Item -ItemType Directory -Force (Join-Path $feed.FullName "download\$tag")
    $nomeExe = "LocalCodePilot-$versaoNova-windows-x64.exe"
    Copy-Item -LiteralPath $nova -Destination (Join-Path $assets.FullName $nomeExe)
    $shaPublicado = if ($Nome -eq 'sha-errado') { 'ab' * 32 } else { $shaNova }
    $checksums = Join-Path $assets.FullName 'SHA256SUMS.txt'
    Set-Content -LiteralPath $checksums -Value "$shaPublicado  $nomeExe" -Encoding ascii
    Set-Assinatura $checksums $(if ($Nome -eq 'assinatura-invalida') { $outraChave.Privada } else { $chaveDoTeste.Privada })
    if ($Nome -eq 'sem-conexao') {
        $base = 'http://127.0.0.1:9'
    }
    else {
        $arquivoPorta = Join-Path $raiz 'porta.txt'
        $argumentos = @('--dir', "`"$($feed.FullName)`"", '--port', '0', '--port-file', "`"$arquivoPorta`"")
        if ($Nome -eq 'queda-no-meio') { $argumentos += @('--drop-after-bytes', '1048576') }
        # Download lento, para dar tempo de iniciar um projeto no meio dele.
        if ($Nome -eq 'processo-durante-download') { $argumentos += @('--bytes-per-second', '3000000') }
        $servidor = Start-Process -FilePath (Get-Servidor) -ArgumentList $argumentos -PassThru -WindowStyle Hidden `
            -RedirectStandardOutput (Join-Path $raiz 'servidor.log') -RedirectStandardError (Join-Path $raiz 'servidor-erros.log')
        if (-not (Wait-Ate { Test-Path -LiteralPath $arquivoPorta } 20)) { throw 'O feed local não começou a responder.' }
        $base = "http://127.0.0.1:$((Get-Content -LiteralPath $arquivoPorta -Raw).Trim())"
    }
    $download = "$base/download/$tag"
    $release = [ordered]@{
        tag_name = $tag; name = "LocalCodePilot $versaoNova (teste)"; html_url = "$base/releases/$tag"
        draft = $false; prerelease = $true
        # Notas com frase de abertura em negrito só no cenário de sucesso: os demais conferem
        # o texto alternativo (T002 CA-2).
        body = $(if ($Nome -eq 'sucesso') { "# LocalCodePilot $versaoNova`n`n$resumoDoTeste Versão de teste.`n`n## Novidades`n`n- **Outro destaque.** Não vira resumo." } else { $null })
        assets = @(
            [ordered]@{ name = "LocalCodePilot-$versaoNova-windows-x64.zip"; browser_download_url = "$download/LocalCodePilot-$versaoNova-windows-x64.zip" },
            [ordered]@{ name = $nomeExe; browser_download_url = "$download/$nomeExe" },
            [ordered]@{ name = 'SHA256SUMS.txt'; browser_download_url = "$download/SHA256SUMS.txt" }
        )
    }
    [IO.File]::WriteAllText((Join-Path $feed.FullName 'releases.json'), (ConvertTo-Json @($release) -Depth 5 -Compress))

    # Instância de teste isolada.
    $ambienteOriginal = @{}
    foreach ($nomeVariavel in 'APPDATA', 'LOCALAPPDATA', 'TEMP', 'TMP', 'LOCALCODEPILOT_TEST_FEED') {
        $ambienteOriginal[$nomeVariavel] = [Environment]::GetEnvironmentVariable($nomeVariavel)
    }
    $env:APPDATA = $appdata.FullName; $env:LOCALAPPDATA = $localappdata.FullName
    $env:TEMP = $temp.FullName; $env:TMP = $temp.FullName; $env:LOCALCODEPILOT_TEST_FEED = $base
    $app = $null
    $na_tela = ''
    $esperado = ''
    $tentarDeNovo = $false
    $atualizarAgora = $false
    $projetoRodando = $false
    $perguntaAntesDeBaixar = $false
    $perguntaSemProcessos = $false
    $listaComEstado = $false
    $focoNoAgoraNao = $false
    $agoraNaoManteve = $false
    $paginaOficial = $false
    $copiarErro = $false
    $novidadesNoAviso = $false
    $linhaDeNovidades = ''
    try {
        # Pasta de trabalho dentro do cenário: o que o app ou as ferramentas que ele consulta
        # gravarem na pasta atual (como um erl_crash.dump) não cai no repositório.
        $app = Start-Process -FilePath $alvo -WorkingDirectory $raiz -PassThru
        $botao = Wait-Ate { Find-Botao $app.Id 'Atualizar agora' } $(if ($Nome -eq 'sem-conexao') { 20 } else { 60 })
        if ($Nome -eq 'sem-conexao') {
            $esperado = '(nenhum aviso de atualização)'
            $na_tela = if ($botao) { 'Atualizar agora' } else { '' }
        }
        elseif (-not $botao) {
            $na_tela = (Get-TextosNaTela $app.Id) -join ' | '
            throw "O aviso de atualização não apareceu. Na tela: $na_tela"
        }
        else {
            $textosDoAviso = Get-TextosNaTela $app.Id
            $linhaDeNovidades = if ($Nome -eq 'sucesso') { "O que há de novo: $($resumoDoTeste.Trim('*'))" } else { 'Confira as novidades na página da versão.' }
            $novidadesNoAviso = $textosDoAviso -contains $linhaDeNovidades
            if ($Capturas -and $Nome -in 'sucesso', 'sha-errado') {
                New-Item -ItemType Directory -Force $Capturas | Out-Null
                Save-Janela $app.Id (Join-Path $Capturas "aviso-novidades-$Nome.png")
            }
            if ($Nome -eq 'processos-ativos') {
                # CA-5: com um projeto rodando, "Atualizar agora" pergunta antes de baixar.
                $projetoRodando = Start-ProjetoDeTeste $app.Id
                Invoke-Botao $botao
                $agoraNao = Wait-Ate { Find-Botao $app.Id 'Agora não' } 10
                $textosDaPergunta = Get-TextosNaTela $app.Id
                $perguntaAntesDeBaixar = [bool]$agoraNao -and ($textosDaPergunta -contains 'Há projetos em execução.') -and
                    ($textosDaPergunta -contains 'Atualizar e encerrar processos')
                $listaComEstado = ($textosDaPergunta -contains 'api-exemplo · dev') -and ($textosDaPergunta -contains 'Rodando')
                $focoNoAgoraNao = [bool]$agoraNao -and $agoraNao.Current.HasKeyboardFocus
                if ($Capturas) {
                    New-Item -ItemType Directory -Force $Capturas | Out-Null
                    Save-Janela $app.Id (Join-Path $Capturas 'confirmacao-processos-ativos.png')
                }
                if ($agoraNao) { Invoke-Botao $agoraNao }
                Start-Sleep -Seconds 3
                $agoraNaoManteve = (-not (Find-Botao $app.Id 'Agora não')) -and (@(Get-ProcessoDoProjeto).Count -gt 0) -and
                    -not ((Get-Content -LiteralPath (Join-Path $raiz 'servidor.log') -ErrorAction SilentlyContinue) -match [regex]::Escape($nomeExe))
                # Agora, sim: confirma e atualiza.
                Invoke-Botao (Wait-Ate { Find-Botao $app.Id 'Atualizar agora' } 10)
                $confirmar = Wait-Ate { Find-Botao $app.Id 'Atualizar e encerrar processos' } 10
                if (-not $confirmar) { throw "A confirmação não reapareceu. Na tela: $((Get-TextosNaTela $app.Id) -join ' | ')" }
                Invoke-Botao $confirmar
            }
            else {
                Invoke-Botao $botao
            }
            if ($Nome -eq 'processo-durante-download') {
                # Nada rodando no clique: baixa sem perguntar. Um projeto iniciado durante o
                # download não foi coberto por nenhuma escolha, então a pergunta vem no fim.
                $perguntaSemProcessos = [bool](Wait-Ate { Find-Botao $app.Id 'Agora não' } 2)
                $projetoRodando = Start-ProjetoDeTeste $app.Id
            }
            $esperado = switch ($Nome) {
                'sucesso' { "LocalCodePilot atualizado para a versão $versaoNova." }
                'processos-ativos' { "LocalCodePilot atualizado para a versão $versaoNova." }
                'processo-durante-download' { 'Há projetos em execução.' }
                'confirmacao-lenta' { "LocalCodePilot atualizado para a versão $versaoNova." }
                'rollback' { "A versão $versaoAntiga foi restaurada e continua em uso" }
                # Falhas aparecem em camadas (T002.07): o título simples fica na tela e o erro
                # técnico fica recolhido em "Detalhes técnicos".
                'sha-errado' { 'Atualização recusada por segurança' }
                'queda-no-meio' { 'O download foi interrompido' }
                'assinatura-invalida' { 'Atualização recusada por segurança' }
            }
            $falhaNoDownload = $Nome -in 'sha-errado', 'queda-no-meio', 'assinatura-invalida'
            $naMesmaJanela = $falhaNoDownload -or $Nome -eq 'processo-durante-download'
            $na_tela = Wait-Ate {
                $processos = if ($naMesmaJanela) { @($app) } else {
                    @(Get-Process -ErrorAction SilentlyContinue | Where-Object { try { $_.Path -eq $alvo -and $_.Id -ne $app.Id } catch { $false } })
                }
                foreach ($processo in $processos) {
                    $texto = (Get-TextosNaTela $processo.Id) | Where-Object {
                        if ($naMesmaJanela) { $_ -eq $esperado } else { $_.Contains($esperado) }
                    } | Select-Object -First 1
                    if ($texto) { return $texto }
                }
            } 150
            if ($Nome -eq 'processo-durante-download' -and $na_tela) {
                $textosDaPergunta = Get-TextosNaTela $app.Id
                $listaComEstado = ($textosDaPergunta -contains 'api-exemplo · dev') -and ($textosDaPergunta -contains 'Rodando')
                if ($Capturas) {
                    New-Item -ItemType Directory -Force $Capturas | Out-Null
                    Save-Janela $app.Id (Join-Path $Capturas 'confirmacao-processo-durante-download.png')
                }
                Invoke-Botao (Find-Botao $app.Id 'Agora não')
                Start-Sleep -Seconds 3
                $agoraNaoManteve = (-not (Find-Botao $app.Id 'Agora não')) -and (@(Get-ProcessoDoProjeto).Count -gt 0)
            }
            if ($falhaNoDownload -and $na_tela) {
                $textosDaFalha = Get-TextosNaTela $app.Id
                $tentarDeNovo = $textosDaFalha -contains 'Tentar de novo'
                $atualizarAgora = [bool](Find-Botao $app.Id 'Atualizar agora')
                $paginaOficial = ($textosDaFalha -contains 'Consultar página oficial da versão') -or ($textosDaFalha -contains 'Ver página oficial da versão')
                $detalhes = Find-Botao $app.Id 'Detalhes técnicos'
                if ($detalhes) {
                    $padroes = $detalhes.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }
                    try {
                        if ($padroes -contains 'InvokePatternIdentifiers.Pattern') {
                            $detalhes.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
                        }
                        elseif ($padroes -contains 'TogglePatternIdentifiers.Pattern') {
                            $detalhes.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle()
                        }
                        else { $detalhes.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Expand() }
                    }
                    catch {}
                }
                $copiarErro = [bool](Wait-Ate { Find-Botao $app.Id 'Copiar erro técnico' } 5)
                if ($Capturas) {
                    New-Item -ItemType Directory -Force $Capturas | Out-Null
                    Save-Janela $app.Id (Join-Path $Capturas "atualizacao-$Nome.png")
                }
            }
        }
        # O atualizador e a limpeza terminam antes de conferir os arquivos.
        Wait-Ate {
            -not (Get-Process -ErrorAction SilentlyContinue | Where-Object { try { $_.Path -like "$($temp.FullName)\*" } catch { $false } })
        } 60 | Out-Null
        Start-Sleep -Seconds 3

        $atualizacoes = Join-Path $temp.FullName 'LocalCodePilot\updates'
        $shaFinal = (Get-FileHash -LiteralPath $alvo -Algorithm SHA256).Hash.ToLowerInvariant()
        $resultado = [ordered]@{
            cenario = $Nome
            na_tela = $(if ($na_tela) { $na_tela } elseif ($Nome -eq 'sem-conexao') { $esperado } else { '(texto esperado não apareceu)' })
            exe_final = $(if ($shaFinal -eq $shaNova) { $versaoNova } elseif ($shaFinal -eq $shaAntiga) { $versaoAntiga } else { 'outro' })
            sobras_na_instalacao = @(Get-ChildItem -LiteralPath $instalacao.FullName -File | Where-Object Name -ne 'LocalCodePilot.exe' | ForEach-Object Name) -join ', '
            registro_da_atualizacao = Test-Path -LiteralPath (Join-Path $configuracoes.FullName 'update-state.json')
            copia_das_configuracoes = @(Get-ChildItem -LiteralPath (Join-Path $configuracoes.FullName 'backups') -Directory -ErrorAction SilentlyContinue | ForEach-Object Name) -join ', '
            preparos_em_temp = @(Get-ChildItem -LiteralPath $atualizacoes -Directory -ErrorAction SilentlyContinue).Count
            downloads_parciais = @(Get-ChildItem -LiteralPath $atualizacoes -Recurse -File -Filter '*.part' -ErrorAction SilentlyContinue).Count
            app_aberto = [bool](Get-Process -ErrorAction SilentlyContinue | Where-Object { try { $_.Path -eq $alvo } catch { $false } })
        }

        $exeEsperado = if ($Nome -in 'sucesso', 'confirmacao-lenta', 'processos-ativos') { $versaoNova } else { $versaoAntiga }
        $checagens = [ordered]@{
            'texto esperado na tela' = [bool]$na_tela
            "executável final é $exeEsperado" = $resultado.exe_final -eq $exeEsperado
            'nenhuma sobra ao lado do executável' = -not $resultado.sobras_na_instalacao
            'registro da atualização removido' = -not $resultado.registro_da_atualizacao
            'nenhum download parcial' = $resultado.downloads_parciais -eq 0
            'app aberto no fim' = $resultado.app_aberto
        }
        if ($Nome -eq 'sem-conexao') { $checagens['texto esperado na tela'] = -not $na_tela }
        if ($Nome -eq 'processos-ativos') {
            $checagens['projeto rodando antes de atualizar'] = [bool]$projetoRodando
            $checagens['pergunta antes de baixar (aviso e botões aprovados)'] = $perguntaAntesDeBaixar
            $checagens['lista o projeto com o estado'] = $listaComEstado
            $checagens['foco inicial em Agora não'] = $focoNoAgoraNao
            $checagens['Agora não: nada baixado e o projeto continua rodando'] = $agoraNaoManteve
            $checagens['processo do projeto encerrado depois de atualizar'] = @(Get-ProcessoDoProjeto).Count -eq 0
            $checagens['pasta de preparo removida'] = $resultado.preparos_em_temp -eq 0
        }
        if ($Nome -eq 'processo-durante-download') {
            $checagens['sem pergunta no clique (nada rodando)'] = -not $perguntaSemProcessos
            $checagens['projeto iniciado durante o download'] = [bool]$projetoRodando
            $checagens['lista o projeto com o estado'] = $listaComEstado
            $checagens['Agora não: nada instalado e o projeto continua rodando'] = $agoraNaoManteve
            $checagens['arquivo preparado descartado'] = $resultado.preparos_em_temp -eq 0
        }
        if ($Nome -in 'sucesso', 'confirmacao-lenta') {
            $checagens["cópia das configurações em backups\antes-de-$versaoNova"] = $resultado.copia_das_configuracoes -eq "antes-de-$versaoNova"
            $checagens['pasta de preparo removida'] = $resultado.preparos_em_temp -eq 0
        }
        if ($Nome -in 'sha-errado', 'queda-no-meio', 'assinatura-invalida') {
            $checagens['pasta de preparo removida'] = $resultado.preparos_em_temp -eq 0
            $checagens['link para a página oficial da versão'] = $paginaOficial
            $checagens['Detalhes técnicos com Copiar erro técnico'] = $copiarErro
            # Recusa por segurança não oferece "Tentar de novo" nem "Atualizar agora"; queda no download oferece.
            if ($Nome -eq 'queda-no-meio') { $checagens['oferece Tentar de novo'] = $tentarDeNovo }
            else { $checagens['sem Tentar de novo nem Atualizar agora na recusa por segurança'] = -not ($tentarDeNovo -or $atualizarAgora) }
        }
        if ($Nome -in 'sucesso', 'sha-errado') {
            # Com notas: o resumo; sem notas: o texto alternativo (T002 CA-2).
            $checagens["aviso mostra '$linhaDeNovidades'"] = $novidadesNoAviso
        }
        if ($Nome -eq 'assinatura-invalida') {
            $checagens['executável nem chegou a ser baixado'] = -not ((Get-Content -LiteralPath (Join-Path $raiz 'servidor.log') -ErrorAction SilentlyContinue) -match [regex]::Escape($nomeExe))
        }
        $resultado.checagens = $checagens
        $resultado.aprovado = -not ($checagens.Values -contains $false)
        [pscustomobject]$resultado
    }
    finally {
        Stop-ProcessosDaPasta
        Get-ProcessoDoProjeto | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
        foreach ($nomeVariavel in $ambienteOriginal.Keys) {
            [Environment]::SetEnvironmentVariable($nomeVariavel, $ambienteOriginal[$nomeVariavel])
        }
    }
}

# --- Cenários do instalador (Inno Setup) -------------------------------------

$iscc = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source),
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
    (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } | Select-Object -First 1
# Só a entrada do AppId de teste; a da instalação real nunca é lida nem tocada.
$chaveDoTesteNoRegistro = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$($appIdDoTeste)_is1"
$arquivosDoInstalador = @('LocalCodePilot.exe', 'LICENSE', 'README-WINDOWS.txt', 'RELEASE-NOTES.md', 'unins000.exe', 'unins000.dat')

# Gera um instalador de teste com o script oficial, outro AppId, outro nome e sem atalhos.
# -Falhar: o instalador recusa instalar no "Preparando para instalar" (como um bloqueio).
function Get-Instalador([string]$Nome, [string]$Executavel, [string]$Versao, [switch]$Falhar) {
    if (-not $iscc) { throw 'O Inno Setup 6 (ISCC.exe) não foi encontrado: winget install JRSoftware.InnoSetup.' }
    $saida = New-Item -ItemType Directory -Force (Join-Path $binarios 'instaladores')
    $conteudo = New-Item -ItemType Directory -Force (Join-Path $saida.FullName "$Nome-conteudo")
    Copy-Item -LiteralPath $Executavel -Destination (Join-Path $conteudo.FullName 'LocalCodePilot.exe') -Force
    Copy-Item -LiteralPath (Join-Path $repositorio 'LICENSE') -Destination $conteudo.FullName -Force
    Set-Content -LiteralPath (Join-Path $conteudo.FullName 'README-WINDOWS.txt') -Value 'teste' -Encoding ascii
    Set-Content -LiteralPath (Join-Path $conteudo.FullName 'RELEASE-NOTES.md') -Value "# $Versao (teste)" -Encoding ascii
    $script = Join-Path $repositorio 'packaging\windows\LocalCodePilot.iss'
    if ($Falhar) {
        $oficial = $script
        $script = Join-Path $saida.FullName "$Nome.iss"
        Set-Content -LiteralPath $script -Encoding UTF8 -Value @(
            "#include `"$oficial`"",
            '[Code]',
            'function PrepareToInstall(var NeedsRestart: Boolean): String;',
            'begin',
            "  Result := 'falha simulada pelo teste de atualização';",
            'end;'
        )
    }
    $preferencia = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $iscc /Qp "/DAppVersion=$Versao" "/DSourceDir=$($conteudo.FullName)" "/DRepositoryRoot=$repositorio" `
            "/DOutputDir=$($saida.FullName)" "/DOutputBaseFilename=$Nome" "/DAppIdValue={$appIdDoTeste" `
            '/DAppDisplayName=LocalCodePilot E2E (teste)' '/DSemAtalhos' $script | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "O Inno Setup falhou com código $LASTEXITCODE ao gerar $Nome." }
    }
    finally { $ErrorActionPreference = $preferencia }
    Join-Path $saida.FullName "$Nome.exe"
}

# Desinstala a cópia de teste (só se estiver dentro da pasta do teste) e remove a entrada dela.
function Remove-InstalacaoDeTeste {
    $chave = Get-ItemProperty -LiteralPath $chaveDoTesteNoRegistro -ErrorAction SilentlyContinue
    if (-not $chave) { return }
    $local = [string]$chave.InstallLocation
    $desinstalador = if ($local) { Join-Path $local 'unins000.exe' } else { '' }
    if ($local -and (Test-DentroDaPasta $local) -and (Test-Path -LiteralPath $desinstalador)) {
        Start-Process -FilePath $desinstalador -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait
        Wait-Ate { -not (Test-Path -LiteralPath $chaveDoTesteNoRegistro) } 30 | Out-Null
    }
    if (Test-Path -LiteralPath $chaveDoTesteNoRegistro) { Remove-Item -LiteralPath $chaveDoTesteNoRegistro -Recurse -Force }
}

function Get-VersaoRegistrada {
    $chave = Get-ItemProperty -LiteralPath $chaveDoTesteNoRegistro -ErrorAction SilentlyContinue
    if ($chave -and $chave.PSObject.Properties['DisplayVersion']) { [string]$chave.DisplayVersion } else { '' }
}

function Invoke-CenarioInstalador([string]$Nome) {
    Stop-ProcessosDaPasta
    Remove-InstalacaoDeTeste
    $raiz = Join-Path $Pasta $Nome
    if (Test-Path -LiteralPath $raiz) { Remove-Item -LiteralPath $raiz -Recurse -Force }
    # Pasta com espaço: o atualizador passa /DIR entre aspas ao instalador.
    $instalacao = Join-Path $raiz 'instalacao com espaco'
    $appdata = New-Item -ItemType Directory -Force (Join-Path $raiz 'appdata')
    $localappdata = New-Item -ItemType Directory -Force (Join-Path $raiz 'localappdata')
    $temp = New-Item -ItemType Directory -Force (Join-Path $raiz 'temp')
    $projetos = New-Item -ItemType Directory -Force (Join-Path $raiz 'projetos-vazios')
    $configuracoes = New-Item -ItemType Directory -Force (Join-Path $appdata 'LocalCodePilot')
    Set-Content -LiteralPath (Join-Path $configuracoes 'scan-roots.txt') -Value $projetos.FullName -Encoding ascii

    $antiga = Get-Variante 'antiga' $versaoAntiga ''
    $inicioDaNova = if ($Nome -eq 'instalador-nao-abriu') { 'fail' } else { '' }
    $nova = Get-Variante "nova$(if ($inicioDaNova) { "-$inicioDaNova" })" $versaoNova $inicioDaNova
    $setupAntigo = Get-Instalador 'setup-antigo' $antiga $versaoAntiga
    $sufixo = switch ($Nome) { 'instalador-falha' { '-falha' } 'instalador-nao-abriu' { '-nao-abre' } default { '' } }
    $setupNovo = Get-Instalador "setup-novo$sufixo" $nova $versaoNova -Falhar:($Nome -eq 'instalador-falha')
    $shaAntiga = (Get-FileHash -LiteralPath $antiga -Algorithm SHA256).Hash.ToLowerInvariant()
    $shaNova = (Get-FileHash -LiteralPath $nova -Algorithm SHA256).Hash.ToLowerInvariant()

    $servidor = $null
    $app = $null
    $ambienteOriginal = @{}
    foreach ($nomeVariavel in 'APPDATA', 'LOCALAPPDATA', 'TEMP', 'TMP', 'LOCALCODEPILOT_TEST_FEED') {
        $ambienteOriginal[$nomeVariavel] = [Environment]::GetEnvironmentVariable($nomeVariavel)
    }
    try {
        # A versão antiga instalada pelo instalador, como na máquina de quem baixou o setup.
        $instalouAntiga = Start-Process -FilePath $setupAntigo -PassThru -Wait -ArgumentList @(
            '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', "`"/DIR=$instalacao`"", "`"/LOG=$(Join-Path $raiz 'instalacao-antiga.log')`"")
        if ($instalouAntiga.ExitCode -ne 0) { throw "O instalador antigo falhou com código $($instalouAntiga.ExitCode)." }
        $alvo = Join-Path $instalacao 'LocalCodePilot.exe'
        if ((Get-VersaoRegistrada) -ne $versaoAntiga) { throw 'A instalação antiga não registrou a versão esperada.' }

        # Feed local com o executável, o instalador novo e os checksums dos dois.
        $feed = New-Item -ItemType Directory -Force (Join-Path $raiz 'feed')
        $tag = "v$versaoNova"
        $assets = New-Item -ItemType Directory -Force (Join-Path $feed.FullName "download\$tag")
        $nomeExe = "LocalCodePilot-$versaoNova-windows-x64.exe"
        $nomeSetup = "LocalCodePilotSetup-$versaoNova-windows-x64.exe"
        Copy-Item -LiteralPath $nova -Destination (Join-Path $assets.FullName $nomeExe)
        Copy-Item -LiteralPath $setupNovo -Destination (Join-Path $assets.FullName $nomeSetup)
        $shaSetup = (Get-FileHash -LiteralPath $setupNovo -Algorithm SHA256).Hash.ToLowerInvariant()
        $checksums = Join-Path $assets.FullName 'SHA256SUMS.txt'
        Set-Content -LiteralPath $checksums -Value @("$shaNova  $nomeExe", "$shaSetup  $nomeSetup") -Encoding ascii
        Set-Assinatura $checksums $chaveDoTeste.Privada
        $arquivoPorta = Join-Path $raiz 'porta.txt'
        $servidor = Start-Process -FilePath (Get-Servidor) -PassThru -WindowStyle Hidden `
            -ArgumentList @('--dir', "`"$($feed.FullName)`"", '--port', '0', '--port-file', "`"$arquivoPorta`"") `
            -RedirectStandardOutput (Join-Path $raiz 'servidor.log') -RedirectStandardError (Join-Path $raiz 'servidor-erros.log')
        if (-not (Wait-Ate { Test-Path -LiteralPath $arquivoPorta } 20)) { throw 'O feed local não começou a responder.' }
        $base = "http://127.0.0.1:$((Get-Content -LiteralPath $arquivoPorta -Raw).Trim())"
        $download = "$base/download/$tag"
        $release = [ordered]@{
            tag_name = $tag; name = "LocalCodePilot $versaoNova (teste)"; html_url = "$base/releases/$tag"
            draft = $false; prerelease = $true; body = $null
            assets = @(
                [ordered]@{ name = "LocalCodePilot-$versaoNova-windows-x64.zip"; browser_download_url = "$download/LocalCodePilot-$versaoNova-windows-x64.zip" },
                [ordered]@{ name = $nomeSetup; browser_download_url = "$download/$nomeSetup" },
                [ordered]@{ name = $nomeExe; browser_download_url = "$download/$nomeExe" },
                [ordered]@{ name = 'SHA256SUMS.txt'; browser_download_url = "$download/SHA256SUMS.txt" }
            )
        }
        [IO.File]::WriteAllText((Join-Path $feed.FullName 'releases.json'), (ConvertTo-Json @($release) -Depth 5 -Compress))

        $env:APPDATA = $appdata.FullName; $env:LOCALAPPDATA = $localappdata.FullName
        $env:TEMP = $temp.FullName; $env:TMP = $temp.FullName; $env:LOCALCODEPILOT_TEST_FEED = $base
        $app = Start-Process -FilePath $alvo -WorkingDirectory $raiz -PassThru
        $botao = Wait-Ate { Find-Botao $app.Id 'Atualizar agora' } 60
        if (-not $botao) { throw "O aviso de atualização não apareceu. Na tela: $((Get-TextosNaTela $app.Id) -join ' | ')" }
        Invoke-Botao $botao

        $esperado = switch ($Nome) {
            'instalador-sucesso' { "LocalCodePilot atualizado para a versão $versaoNova." }
            'instalador-falha' { 'A instalação não foi concluída' }
            'instalador-nao-abriu' { "A versão $versaoNova não abriu. Restauramos o aplicativo e as configurações da versão $versaoAntiga; você pode continuar trabalhando. A lista de programas do Windows ainda pode mostrar a versão $versaoNova. Tente novamente mais tarde ou baixe o instalador na página oficial." }
        }
        $reaberto = $null
        $na_tela = Wait-Ate {
            foreach ($processo in @(Get-Process -ErrorAction SilentlyContinue | Where-Object { try { $_.Path -eq $alvo -and $_.Id -ne $app.Id } catch { $false } })) {
                $texto = (Get-TextosNaTela $processo.Id) | Where-Object {
                    if ($Nome -eq 'instalador-falha') { $_ -eq $esperado } else { $_.Contains($esperado) }
                } | Select-Object -First 1
                if ($texto) { $script:reabertoId = $processo.Id; return $texto }
            }
        } 180
        if ($na_tela) { $reaberto = $script:reabertoId }

        $mensagemAprovada = $false; $tentarDeNovo = $false; $paginaOficial = $false; $copiarErro = $false; $codigoNoDetalhe = $false
        if ($Nome -eq 'instalador-falha' -and $reaberto) {
            $textos = Get-TextosNaTela $reaberto
            $mensagemAprovada = $textos -contains "Não foi possível instalar a versão $versaoNova. A versão $versaoAntiga foi reaberta e você pode continuar trabalhando. Tente novamente mais tarde ou baixe o instalador na página oficial."
            $tentarDeNovo = $textos -contains 'Tentar de novo'
            $paginaOficial = $textos -contains 'Ver página oficial da versão'
            $detalhes = Find-Botao $reaberto 'Detalhes técnicos'
            if ($detalhes) { try { Invoke-Botao $detalhes } catch {} }
            $copiarErro = [bool](Wait-Ate { Find-Botao $reaberto 'Copiar erro técnico' } 5)
            $codigoNoDetalhe = [bool]((Get-TextosNaTela $reaberto) | Where-Object { $_ -like 'o instalador terminou com o código *registro do instalador:*update-setup.log' })
        }
        if ($Capturas -and $reaberto) {
            New-Item -ItemType Directory -Force $Capturas | Out-Null
            Save-Janela $reaberto (Join-Path $Capturas "atualizacao-$Nome.png")
        }

        Wait-Ate {
            -not (Get-Process -ErrorAction SilentlyContinue | Where-Object { try { $_.Path -like "$($temp.FullName)\*" } catch { $false } })
        } 60 | Out-Null
        Start-Sleep -Seconds 3

        $atualizacoes = Join-Path $temp.FullName 'LocalCodePilot\updates'
        $shaFinal = (Get-FileHash -LiteralPath $alvo -Algorithm SHA256).Hash.ToLowerInvariant()
        $resultado = [ordered]@{
            cenario = $Nome
            na_tela = $(if ($na_tela) { $na_tela } else { '(texto esperado não apareceu)' })
            exe_final = $(if ($shaFinal -eq $shaNova) { $versaoNova } elseif ($shaFinal -eq $shaAntiga) { $versaoAntiga } else { 'outro' })
            versao_na_lista_de_programas = Get-VersaoRegistrada
            sobras_na_instalacao = @(Get-ChildItem -LiteralPath $instalacao -File | Where-Object { $_.Name -notin $arquivosDoInstalador } | ForEach-Object Name) -join ', '
            desinstalador = Test-Path -LiteralPath (Join-Path $instalacao 'unins000.exe')
            registro_da_atualizacao = Test-Path -LiteralPath (Join-Path $configuracoes.FullName 'update-state.json')
            preparos_em_temp = @(Get-ChildItem -LiteralPath $atualizacoes -Directory -ErrorAction SilentlyContinue).Count
            app_aberto = [bool](Get-Process -ErrorAction SilentlyContinue | Where-Object { try { $_.Path -eq $alvo } catch { $false } })
        }
        $exeEsperado = if ($Nome -eq 'instalador-sucesso') { $versaoNova } else { $versaoAntiga }
        # Quando a versão nova não abre, só o executável volta: a lista de programas continua na nova (texto aprovado).
        $listaEsperada = if ($Nome -eq 'instalador-falha') { $versaoAntiga } else { $versaoNova }
        $checagens = [ordered]@{
            'texto esperado na tela' = [bool]$na_tela
            "executável final é $exeEsperado" = $resultado.exe_final -eq $exeEsperado
            "lista de programas mostra $listaEsperada" = $resultado.versao_na_lista_de_programas -eq $listaEsperada
            'desinstalador continua na pasta' = $resultado.desinstalador
            'nenhuma sobra ao lado do executável' = -not $resultado.sobras_na_instalacao
            'registro da atualização removido' = -not $resultado.registro_da_atualizacao
            'app aberto no fim' = $resultado.app_aberto
        }
        if ($Nome -eq 'instalador-sucesso') {
            $checagens['instalador novo baixado (não o executável)'] = (Get-Content -LiteralPath (Join-Path $raiz 'servidor.log') -ErrorAction SilentlyContinue) -match [regex]::Escape($nomeSetup)
            $checagens['pasta de preparo removida'] = $resultado.preparos_em_temp -eq 0
        }
        if ($Nome -eq 'instalador-falha') {
            $checagens['mensagem aprovada'] = $mensagemAprovada
            $checagens['oferece Tentar de novo'] = $tentarDeNovo
            $checagens['link para a página oficial da versão'] = $paginaOficial
            $checagens['Detalhes técnicos com Copiar erro técnico'] = $copiarErro
            $checagens['código de saída e registro do instalador nos detalhes'] = $codigoNoDetalhe
            $checagens['registro do instalador guardado com as configurações'] = Test-Path -LiteralPath (Join-Path $configuracoes.FullName 'update-setup.log')
        }
        $resultado.checagens = $checagens
        $resultado.aprovado = -not ($checagens.Values -contains $false)
        [pscustomobject]$resultado
    }
    finally {
        Stop-ProcessosDaPasta
        if ($servidor) { Stop-Process -Id $servidor.Id -Force -ErrorAction SilentlyContinue }
        foreach ($nomeVariavel in $ambienteOriginal.Keys) {
            [Environment]::SetEnvironmentVariable($nomeVariavel, $ambienteOriginal[$nomeVariavel])
        }
        Remove-InstalacaoDeTeste
    }
}

# --- Execução ----------------------------------------------------------------

$resultados = @()
foreach ($nome in $Cenario) {
    Write-Host ''
    Write-Host "=== Cenário: $nome ==="
    try {
        $resultado = Invoke-Cenario $nome
    }
    catch {
        $resultado = [pscustomobject]@{ cenario = $nome; aprovado = $false; erro = $_.Exception.Message }
    }
    $resultados += $resultado
    $resultado | Format-List | Out-String | Write-Host
    if ($resultado.PSObject.Properties['checagens']) {
        foreach ($item in $resultado.checagens.GetEnumerator()) {
            Write-Host ("  [{0}] {1}" -f $(if ($item.Value) { 'ok' } else { 'FALHOU' }), $item.Key)
        }
    }
}

Write-Host ''
Write-Host '=== Resumo ==='
foreach ($resultado in $resultados) {
    Write-Host ("  {0,-18} {1}" -f $resultado.cenario, $(if ($resultado.aprovado) { 'aprovado' } else { 'FALHOU' }))
}
Write-Host "Pasta do teste: $Pasta"
if ($resultados | Where-Object { -not $_.aprovado }) { exit 1 }
