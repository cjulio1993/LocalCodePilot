[CmdletBinding()]
param(
    [string]$Version = "0.3.0-alpha.1",
    [string]$TargetDirectory = "target",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$distributionRoot = Join-Path $repositoryRoot "dist"
$packageName = "LocalCodePilot-$Version-windows-x64"
$stagingDirectory = Join-Path $distributionRoot $packageName
$archivePath = Join-Path $distributionRoot "$packageName.zip"
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

New-Item -ItemType Directory -Path $stagingDirectory | Out-Null
Copy-Item -LiteralPath $executablePath -Destination (Join-Path $stagingDirectory "LocalCodePilot.exe")
Copy-Item -LiteralPath (Join-Path $repositoryRoot "LICENSE") -Destination $stagingDirectory
Copy-Item -LiteralPath (Join-Path $repositoryRoot "docs\README-WINDOWS.txt") -Destination $stagingDirectory
Copy-Item -LiteralPath (Join-Path $repositoryRoot "docs\releases\v$Version.md") -Destination (Join-Path $stagingDirectory "RELEASE-NOTES.md")

Compress-Archive -LiteralPath $stagingDirectory -DestinationPath $archivePath -CompressionLevel Optimal

$archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash
$checksumLine = "$archiveHash  $([System.IO.Path]::GetFileName($archivePath))"
Set-Content -LiteralPath (Join-Path $distributionRoot "SHA256SUMS.txt") -Value $checksumLine -Encoding ascii

Write-Host "Pacote criado: $archivePath"
Write-Host "SHA-256: $archiveHash"
