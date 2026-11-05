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
      sem-conexao        o feed não responde: nenhum aviso de atualização.

.EXAMPLE
    .\scripts\e2e-update-windows.ps1
    .\scripts\e2e-update-windows.ps1 -Cenario sucesso,rollback -Recompilar
#>
param(
    [ValidateSet('todos', 'sucesso', 'sha-errado', 'queda-no-meio', 'rollback', 'confirmacao-lenta', 'sem-conexao')]
    [string[]]$Cenario = @('todos'),
    [string]$Pasta = (Join-Path ([IO.Path]::GetTempPath()) 'lcp-e2e-update'),
    [switch]$Recompilar
)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'

$repositorio = Split-Path -Parent $PSScriptRoot
$versaoAntiga = '0.4.0-alpha.90'
$versaoNova = '0.4.0-alpha.91'
$todos = @('sucesso', 'sha-errado', 'queda-no-meio', 'rollback', 'confirmacao-lenta', 'sem-conexao')
if ($Cenario -contains 'todos') { $Cenario = $todos }

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$uia = [System.Windows.Automation.AutomationElement]

# --- Pasta descartável -------------------------------------------------------

$marcador = '.lcp-e2e-update'
$Pasta = [IO.Path]::GetFullPath($Pasta)
if ((Test-Path -LiteralPath $Pasta) -and (Get-ChildItem -LiteralPath $Pasta -Force | Select-Object -First 1) -and
    -not (Test-Path -LiteralPath (Join-Path $Pasta $marcador))) {
    throw "A pasta $Pasta já existe e não foi criada por este script. Escolha outra com -Pasta."
}
New-Item -ItemType Directory -Force $Pasta | Out-Null
Set-Content -LiteralPath (Join-Path $Pasta $marcador) -Value 'pasta descartável do teste de atualização' -Encoding UTF8
$binarios = New-Item -ItemType Directory -Force (Join-Path $Pasta 'bin')

function Stop-ProcessosDaPasta {
    Get-Process -ErrorAction SilentlyContinue |
        Where-Object { try { $_.Path -and $_.Path.StartsWith($Pasta, [StringComparison]::OrdinalIgnoreCase) } catch { $false } } |
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

$alvoCargo = (& cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $repositorio 'Cargo.toml') | ConvertFrom-Json).target_directory

# Com -Recompilar, cada binário é compilado uma vez por execução, não por cenário.
$compiladosNestaExecucao = @{}

function Test-Pronto([string]$Destino) {
    (Test-Path -LiteralPath $Destino) -and (-not $Recompilar -or $compiladosNestaExecucao.ContainsKey($Destino))
}

function Get-Variante([string]$Nome, [string]$Versao, [string]$Inicio) {
    $destino = Join-Path $binarios "$Nome.exe"
    if (Test-Pronto $destino) { return $destino }
    Write-Host "Compilando $Nome ($Versao$(if ($Inicio) { ", início: $Inicio" }))..."
    $env:LOCALCODEPILOT_TEST_VERSION = $Versao
    if ($Inicio) { $env:LOCALCODEPILOT_TEST_STARTUP = $Inicio } else { Remove-Item Env:LOCALCODEPILOT_TEST_STARTUP -ErrorAction SilentlyContinue }
    try {
        Invoke-Cargo @('build', '-p', 'localcodepilot-desktop', '--locked', '--features', 'update-test-feed')
    }
    finally {
        Remove-Item Env:LOCALCODEPILOT_TEST_VERSION, Env:LOCALCODEPILOT_TEST_STARTUP -ErrorAction SilentlyContinue
    }
    $compilado = Join-Path $alvoCargo 'debug\localcodepilot-desktop.exe'
    $texto = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($compilado))
    if (-not $texto.Contains($Versao) -or -not $texto.Contains('LOCALCODEPILOT_TEST_FEED')) {
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
    Set-Content -LiteralPath (Join-Path $assets.FullName 'SHA256SUMS.txt') -Value "$shaPublicado  $nomeExe" -Encoding ascii
    if ($Nome -eq 'sem-conexao') {
        $base = 'http://127.0.0.1:9'
    }
    else {
        $arquivoPorta = Join-Path $raiz 'porta.txt'
        $argumentos = @('--dir', "`"$($feed.FullName)`"", '--port', '0', '--port-file', "`"$arquivoPorta`"")
        if ($Nome -eq 'queda-no-meio') { $argumentos += @('--drop-after-bytes', '1048576') }
        $servidor = Start-Process -FilePath (Get-Servidor) -ArgumentList $argumentos -PassThru -WindowStyle Hidden `
            -RedirectStandardOutput (Join-Path $raiz 'servidor.log') -RedirectStandardError (Join-Path $raiz 'servidor-erros.log')
        if (-not (Wait-Ate { Test-Path -LiteralPath $arquivoPorta } 20)) { throw 'O feed local não começou a responder.' }
        $base = "http://127.0.0.1:$((Get-Content -LiteralPath $arquivoPorta -Raw).Trim())"
    }
    $download = "$base/download/$tag"
    $release = [ordered]@{
        tag_name = $tag; name = "LocalCodePilot $versaoNova (teste)"; html_url = "$base/releases/$tag"
        draft = $false; prerelease = $true
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
    try {
        $app = Start-Process -FilePath $alvo -PassThru
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
            $botao.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
            $esperado = switch ($Nome) {
                'sucesso' { "LocalCodePilot atualizado para a versão $versaoNova." }
                'confirmacao-lenta' { "LocalCodePilot atualizado para a versão $versaoNova." }
                'rollback' { "A versão $versaoAntiga foi restaurada e continua em uso" }
                'sha-errado' { 'o SHA-256 do executável baixado não corresponde ao publicado' }
                'queda-no-meio' { 'download' }
            }
            $falhaNoDownload = $Nome -in 'sha-errado', 'queda-no-meio'
            $na_tela = Wait-Ate {
                $processos = if ($falhaNoDownload) { @($app) } else {
                    @(Get-Process -ErrorAction SilentlyContinue | Where-Object { try { $_.Path -eq $alvo -and $_.Id -ne $app.Id } catch { $false } })
                }
                foreach ($processo in $processos) {
                    $texto = (Get-TextosNaTela $processo.Id) | Where-Object {
                        $_.Contains($esperado) -and (-not $falhaNoDownload -or $_.StartsWith('Atualização automática:'))
                    } | Select-Object -First 1
                    if ($texto) { return $texto }
                }
            } 150
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

        $exeEsperado = if ($Nome -in 'sucesso', 'confirmacao-lenta') { $versaoNova } else { $versaoAntiga }
        $checagens = [ordered]@{
            'texto esperado na tela' = [bool]$na_tela
            "executável final é $exeEsperado" = $resultado.exe_final -eq $exeEsperado
            'nenhuma sobra ao lado do executável' = -not $resultado.sobras_na_instalacao
            'registro da atualização removido' = -not $resultado.registro_da_atualizacao
            'nenhum download parcial' = $resultado.downloads_parciais -eq 0
            'app aberto no fim' = $resultado.app_aberto
        }
        if ($Nome -eq 'sem-conexao') { $checagens['texto esperado na tela'] = -not $na_tela }
        if ($Nome -in 'sucesso', 'confirmacao-lenta') {
            $checagens["cópia das configurações em backups\antes-de-$versaoNova"] = $resultado.copia_das_configuracoes -eq "antes-de-$versaoNova"
            $checagens['pasta de preparo removida'] = $resultado.preparos_em_temp -eq 0
        }
        if ($Nome -in 'sha-errado', 'queda-no-meio') { $checagens['pasta de preparo removida'] = $resultado.preparos_em_temp -eq 0 }
        $resultado.checagens = $checagens
        $resultado.aprovado = -not ($checagens.Values -contains $false)
        [pscustomobject]$resultado
    }
    finally {
        Stop-ProcessosDaPasta
        foreach ($nomeVariavel in $ambienteOriginal.Keys) {
            [Environment]::SetEnvironmentVariable($nomeVariavel, $ambienteOriginal[$nomeVariavel])
        }
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
