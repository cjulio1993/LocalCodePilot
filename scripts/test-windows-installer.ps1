[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Installer
)

# Installs silently into a temporary folder, checks the executable and the Start menu
# shortcut, then uninstalls and checks that everything was removed.

$ErrorActionPreference = "Stop"

$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$installDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "LocalCodePilot-installer-test-$PID"
$shortcut = Join-Path ([Environment]::GetFolderPath("Programs")) "LocalCodePilot.lnk"

$install = Start-Process -FilePath $installerPath -Wait -PassThru -ArgumentList @(
    "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CURRENTUSER", "/DIR=`"$installDirectory`""
)
if ($install.ExitCode -ne 0) {
    throw "A instalação silenciosa falhou com código $($install.ExitCode)."
}

$executable = Join-Path $installDirectory "LocalCodePilot.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "LocalCodePilot.exe não foi instalado em $installDirectory."
}
if (-not (Test-Path -LiteralPath $shortcut -PathType Leaf)) {
    throw "O atalho do Menu Iniciar não foi criado em $shortcut."
}
Write-Host "Instalado em $installDirectory, com atalho no Menu Iniciar."

$uninstaller = Join-Path $installDirectory "unins000.exe"
if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
    throw "O desinstalador não foi encontrado em $installDirectory."
}
$uninstall = Start-Process -FilePath $uninstaller -Wait -PassThru -ArgumentList @(
    "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"
)
if ($uninstall.ExitCode -ne 0) {
    throw "A desinstalação silenciosa falhou com código $($uninstall.ExitCode)."
}

# The uninstaller removes its own copy a moment after it exits.
$deadline = (Get-Date).AddSeconds(30)
while ((Test-Path -LiteralPath $executable) -and (Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 1
}
if (Test-Path -LiteralPath $executable) {
    throw "LocalCodePilot.exe continuou em $installDirectory depois da desinstalação."
}
if (Test-Path -LiteralPath $shortcut) {
    throw "O atalho do Menu Iniciar continuou depois da desinstalação."
}
Write-Host "Desinstalado: executável e atalho removidos."
