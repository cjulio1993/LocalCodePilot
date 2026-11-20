<#
.SYNOPSIS
    Confere os pacotes de banco fixados em runtime/src/database-packages.json contra as
    fontes independentes de cada um.

.DESCRIPTION
    - PostgreSQL: o SHA-256, a versão e a URL têm de ser os do manifesto do Scoop
      (ScoopInstaller/Main, bucket/postgresql.json) no commit citado em "provenance".
    - MariaDB: o SHA-256 tem de estar no sha256sums.txt da mesma pasta da release.
    - MySQL: o tamanho anunciado pelo servidor tem de ser o fixado; com -Completo, o
      zip é baixado e a assinatura GPG da MySQL Release Engineering é conferida.
    Com -Completo, os três zips são baixados e o SHA-256 de cada um é recalculado.

.EXAMPLE
    ./scripts/verify-database-packages.ps1
    ./scripts/verify-database-packages.ps1 -Completo
#>
param([switch]$Completo)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'
$repositorio = Split-Path -Parent $PSScriptRoot
$catalogo = Get-Content -Raw -Encoding UTF8 (Join-Path $repositorio 'runtime/src/database-packages.json') | ConvertFrom-Json
# Chave da MySQL Release Engineering para 9.5.0 e posteriores (manual do MySQL, "Signature Checking Using GnuPG").
$chaveMysql = 'BCA43417C3B485DD128EC6D4B7B3B788A8D3785C'
$falhas = New-Object System.Collections.Generic.List[string]

# No Windows PowerShell 5.1, `curl` é um apelido de Invoke-WebRequest.
$curl = if ($env:OS -eq 'Windows_NT') { 'curl.exe' } else { 'curl' }

function Get-Texto([string]$Url) { (Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 60).Content }

foreach ($pacote in $catalogo.packages) {
    $nome = "$($pacote.engine) $($pacote.version)"
    if ($pacote.url -notlike "https://*" -or -not $pacote.url.EndsWith($pacote.file_name)) { $falhas.Add("${nome}: URL inválida") }
    switch ($pacote.engine) {
        'postgresql' {
            if ($pacote.provenance -notmatch 'commit ([0-9a-f]{40})') { $falhas.Add("${nome}: provenance sem o commit do Scoop"); break }
            $manifesto = Get-Texto "https://raw.githubusercontent.com/ScoopInstaller/Main/$($Matches[1])/bucket/postgresql.json" | ConvertFrom-Json
            $x64 = $manifesto.architecture.'64bit'
            if ($manifesto.version -ne $pacote.version) { $falhas.Add("${nome}: Scoop tem a versão $($manifesto.version)") }
            if ($x64.url -ne $pacote.url) { $falhas.Add("${nome}: Scoop aponta para $($x64.url)") }
            if ($x64.hash -ne $pacote.sha256) { $falhas.Add("${nome}: Scoop tem o SHA-256 $($x64.hash)") }
        }
        'mariadb' {
            $pasta = $pacote.url.Substring(0, $pacote.url.LastIndexOf('/'))
            $somas = Get-Texto "$pasta/sha256sums.txt"
            if ($somas -notmatch "(?m)^$($pacote.sha256)\s+\S*$([regex]::Escape($pacote.file_name))\s*$") { $falhas.Add("${nome}: SHA-256 ausente do sha256sums.txt") }
        }
        'mysql' {
            $cabecalho = Invoke-WebRequest -Uri $pacote.url -Method Head -UseBasicParsing -TimeoutSec 60
            $tamanho = [int64]([string]$cabecalho.Headers['Content-Length'])
            if ($tamanho -ne $pacote.size_bytes) { $falhas.Add("${nome}: o servidor anuncia $tamanho bytes") }
        }
        default { $falhas.Add("${nome}: motor desconhecido") }
    }
    if ($Completo) {
        $pasta = Join-Path ([IO.Path]::GetTempPath()) 'lcp-database-packages'
        New-Item -ItemType Directory -Force $pasta | Out-Null
        $arquivo = Join-Path $pasta $pacote.file_name
        & $curl -sSfL --retry 3 -o $arquivo $pacote.url
        if ($LASTEXITCODE -ne 0) { $falhas.Add("${nome}: download falhou"); continue }
        $hash = (Get-FileHash -LiteralPath $arquivo -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne $pacote.sha256) { $falhas.Add("${nome}: o arquivo baixado tem SHA-256 $hash") }
        if ((Get-Item -LiteralPath $arquivo).Length -ne $pacote.size_bytes) { $falhas.Add("${nome}: tamanho diferente do fixado") }
        if ($pacote.engine -eq 'mysql') {
            $gnupg = Join-Path $pasta 'gnupg'
            New-Item -ItemType Directory -Force $gnupg | Out-Null
            $env:GNUPGHOME = $gnupg
            & $curl -sSfL -o "$arquivo.asc" "$($pacote.url).asc"
            & $curl -sSfL -o (Join-Path $pasta 'mysql-key.asc') "https://keyserver.ubuntu.com/pks/lookup?op=get&search=0x$chaveMysql"
            & gpg --batch --import (Join-Path $pasta 'mysql-key.asc') 2>$null
            $saida = & gpg --batch --status-fd 1 --verify "$arquivo.asc" $arquivo 2>$null
            if (-not ($saida -match "VALIDSIG $chaveMysql")) { $falhas.Add("${nome}: assinatura GPG não confere com a chave $chaveMysql") }
        }
        Remove-Item -LiteralPath $arquivo -Force
    }
    Write-Host "conferido: $nome"
}

if ($falhas.Count) {
    $falhas | ForEach-Object { Write-Host "FALHA: $_" }
    exit 1
}
Write-Host 'Todos os pacotes de banco conferem com as fontes independentes.'
