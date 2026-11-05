[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Version,
    [string]$TargetDirectory = "target",
    [switch]$SkipBuild,
    # Also builds the installer (Inno Setup 6); fails when ISCC.exe is not available.
    [switch]$Installer
)

$ErrorActionPreference = "Stop"

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$distributionRoot = Join-Path $repositoryRoot "dist"
$packageName = "LocalCodePilot-$Version-windows-x64"
$stagingDirectory = Join-Path $distributionRoot $packageName
$archivePath = Join-Path $distributionRoot "$packageName.zip"
$standalonePath = Join-Path $distributionRoot "$packageName.exe"
# The installer must not start with "LocalCodePilot-": clients up to 0.4.0-alpha.8
# pick any "LocalCodePilot-...windows-x64...exe" asset as the update executable (#97).
$installerBaseName = "LocalCodePilotSetup-$Version-windows-x64"
$installerPath = Join-Path $distributionRoot "$installerBaseName.exe"
$resolvedTargetDirectory = if ([System.IO.Path]::IsPathRooted($TargetDirectory)) {
    [System.IO.Path]::GetFullPath($TargetDirectory)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $TargetDirectory))
}
$executablePath = Join-Path $resolvedTargetDirectory "release\localcodepilot-desktop.exe"

function Assert-ChildPath {
    param(
        [Parameter(Mandatory)] [string]$Path,
        [Parameter(Mandatory)] [string]$Parent
    )

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $fullParent = [System.IO.Path]::GetFullPath($Parent).TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if (-not $fullPath.StartsWith($fullParent, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Caminho de saída inesperado: $fullPath"
    }
}

Assert-ChildPath -Path $stagingDirectory -Parent $distributionRoot
Assert-ChildPath -Path $archivePath -Parent $distributionRoot
Assert-ChildPath -Path $standalonePath -Parent $distributionRoot
Assert-ChildPath -Path $installerPath -Parent $distributionRoot

if (-not $SkipBuild) {
    Push-Location $repositoryRoot
    try {
        cargo build --release --locked -p localcodepilot-desktop --target-dir $resolvedTargetDirectory
        if ($LASTEXITCODE -ne 0) {
            throw "A compilação release falhou com código $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
    throw "Executável release não encontrado em $executablePath"
}

New-Item -ItemType Directory -Path $distributionRoot -Force | Out-Null
if (Test-Path -LiteralPath $stagingDirectory) {
    Remove-Item -LiteralPath $stagingDirectory -Recurse -Force
}
if (Test-Path -LiteralPath $archivePath) {
    Remove-Item -LiteralPath $archivePath -Force
}
if (Test-Path -LiteralPath $standalonePath) {
    Remove-Item -LiteralPath $standalonePath -Force
}
if (Test-Path -LiteralPath $installerPath) {
    Remove-Item -LiteralPath $installerPath -Force
}

New-Item -ItemType Directory -Path $stagingDirectory | Out-Null
Copy-Item -LiteralPath $executablePath -Destination (Join-Path $stagingDirectory "LocalCodePilot.exe")
Copy-Item -LiteralPath (Join-Path $repositoryRoot "LICENSE") -Destination $stagingDirectory
Copy-Item -LiteralPath (Join-Path $repositoryRoot "docs\README-WINDOWS.txt") -Destination $stagingDirectory
Copy-Item -LiteralPath (Join-Path $repositoryRoot "docs\releases\v$Version.md") -Destination (Join-Path $stagingDirectory "RELEASE-NOTES.md")

Compress-Archive -LiteralPath $stagingDirectory -DestinationPath $archivePath -CompressionLevel Optimal
Copy-Item -LiteralPath $executablePath -Destination $standalonePath

$packaged = @($archivePath, $standalonePath)
if ($Installer) {
    $iscc = @(
        (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
        (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
        (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe")
    ) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } | Select-Object -First 1
    if (-not $iscc) {
        throw "Inno Setup 6 (ISCC.exe) não foi encontrado; o instalador não pode ser criado."
    }
    & $iscc /Qp `
        "/DAppVersion=$Version" `
        "/DSourceDir=$stagingDirectory" `
        "/DRepositoryRoot=$repositoryRoot" `
        "/DOutputDir=$distributionRoot" `
        "/DOutputBaseFilename=$installerBaseName" `
        (Join-Path $repositoryRoot "packaging\windows\LocalCodePilot.iss")
    if ($LASTEXITCODE -ne 0) {
        throw "O Inno Setup falhou com código $LASTEXITCODE."
    }
    if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
        throw "Instalador não encontrado em $installerPath"
    }
    $packaged += $installerPath
}

$checksumLines = $packaged | ForEach-Object {
    $hash = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash
    "$hash  $([System.IO.Path]::GetFileName($_))"
}
Set-Content -LiteralPath (Join-Path $distributionRoot "SHA256SUMS.txt") -Value $checksumLines -Encoding ascii

Write-Host "Pacote criado: $archivePath"
Write-Host "Executável para atualização criado: $standalonePath"
if ($Installer) {
    Write-Host "Instalador criado: $installerPath"
}
Write-Host "SHA-256:"
$checksumLines | ForEach-Object { Write-Host $_ }
