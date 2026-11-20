[CmdletBinding()]
param(
    # Folder with the packaged files (LocalCodePilot.exe, LICENSE, README-WINDOWS.txt,
    # RELEASE-NOTES.md), as left by package-windows.ps1 in dist\LocalCodePilot-<version>-windows-x64.
    [Parameter(Mandatory)] [string]$StagingDirectory,
    [Parameter(Mandatory)] [string]$Version
)

# Upgrade of an installed copy by the official installer, run the way the app's updater
# runs it (desktop/src/updates.rs, run_installer). Uses two installers built from the
# official script with a test AppId and no shortcuts, so a real installation on the
# machine is never read or touched:
#   1. the older test version installs into a folder with a space in its name;
#   2. the newer one upgrades it silently: the program list shows the new version, the
#      executable is the new one and the uninstaller stays;
#   3. with the executable in use, the installer fails (it does not close programs) and
#      rolls back: executable and program list unchanged;
#   4. the uninstaller removes the files and the entry.

$ErrorActionPreference = "Stop"

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$staging = (Resolve-Path -LiteralPath $StagingDirectory).Path
$testAppId = "{9C4D1A2B-5E6F-4A70-8B19-2C3D4E5F6A7B}"
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$($testAppId)_is1"
$work = Join-Path ([System.IO.Path]::GetTempPath()) "LocalCodePilot-upgrade-test-$PID"
$installDirectory = Join-Path $work "pasta com espaco"
# Same switches as the updater (desktop/src/updates.rs, run_installer).
$updaterSwitches = @(
    "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/NOCANCEL",
    "/NOCLOSEAPPLICATIONS", "/NORESTARTAPPLICATIONS"
)

$iscc = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source),
    (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
    (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
    (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe")
) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } | Select-Object -First 1
if (-not $iscc) {
    throw "Inno Setup 6 (ISCC.exe) não foi encontrado."
}

function New-TestInstaller([string]$Name, [string]$AppVersion, [string]$Content) {
    & $iscc /Qp "/DAppVersion=$AppVersion" "/DSourceDir=$Content" "/DRepositoryRoot=$repositoryRoot" `
        "/DOutputDir=$work" "/DOutputBaseFilename=$Name" "/DAppIdValue={$testAppId" `
        "/DAppDisplayName=LocalCodePilot (teste de upgrade)" "/DSemAtalhos" `
        (Join-Path $repositoryRoot "packaging\windows\LocalCodePilot.iss") | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "O Inno Setup falhou com código $LASTEXITCODE ao gerar $Name."
    }
    Join-Path $work "$Name.exe"
}

function Invoke-Installer([string]$Path, [string]$Log) {
    $arguments = $updaterSwitches + @("`"/DIR=$installDirectory`"", "`"/LOG=$Log`"")
    (Start-Process -FilePath $Path -Wait -PassThru -ArgumentList $arguments).ExitCode
}

function Get-RegisteredVersion {
    $entry = Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction SilentlyContinue
    if ($entry) { [string]$entry.DisplayVersion } else { "" }
}

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

if (Test-Path -LiteralPath $uninstallKey) {
    throw "A entrada do AppId de teste já existe; remova-a antes de rodar o teste."
}
New-Item -ItemType Directory -Force $work | Out-Null
$executable = Join-Path $installDirectory "LocalCodePilot.exe"
try {
    # The older version differs from the newer one by a few bytes at the end of the
    # executable, so the test can tell which file is installed.
    $olderContent = New-Item -ItemType Directory -Force (Join-Path $work "conteudo-antigo")
    Copy-Item -Path (Join-Path $staging "*") -Destination $olderContent.FullName -Force
    $olderExecutable = Join-Path $olderContent.FullName "LocalCodePilot.exe"
    [System.IO.File]::AppendAllText($olderExecutable, "versao-antiga-do-teste")
    $older = New-TestInstaller "setup-antigo" "0.0.1" $olderContent.FullName
    $newer = New-TestInstaller "setup-novo" $Version $staging
    $newerSha = Get-Sha256 (Join-Path $staging "LocalCodePilot.exe")

    $code = Invoke-Installer $older (Join-Path $work "1-instalar-antigo.log")
    if ($code -ne 0 -or (Get-RegisteredVersion) -ne "0.0.1") {
        throw "A versão antiga não foi instalada (código $code, versão registrada '$(Get-RegisteredVersion)')."
    }
    $olderSha = Get-Sha256 $executable
    Write-Host "Versão antiga instalada em $installDirectory."

    $code = Invoke-Installer $newer (Join-Path $work "2-atualizar.log")
    if ($code -ne 0) {
        throw "A atualização pelo instalador falhou com código $code."
    }
    if ((Get-RegisteredVersion) -ne $Version) {
        throw "A lista de programas mostra '$(Get-RegisteredVersion)' depois da atualização, e não $Version."
    }
    if ((Get-Sha256 $executable) -ne $newerSha) {
        throw "O executável instalado não é o da versão nova."
    }
    if (-not (Test-Path -LiteralPath (Join-Path $installDirectory "unins000.exe"))) {
        throw "O desinstalador sumiu depois da atualização."
    }
    $entries = @(Get-ChildItem "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall" |
        Where-Object { $_.PSChildName -eq "$($testAppId)_is1" })
    if ($entries.Count -ne 1) {
        throw "Esperava uma entrada de desinstalação; há $($entries.Count)."
    }
    Write-Host "Atualizado pelo instalador: lista de programas em $Version, executável novo, desinstalador mantido."

    # Another copy holding the executable: the installer must fail and roll back instead
    # of closing it. The file is kept open without sharing deletion, like a running image.
    $holder = [System.IO.File]::Open($executable, "Open", "Read", "Read")
    try {
        $code = Invoke-Installer $older (Join-Path $work "3-em-uso.log")
    }
    finally {
        $holder.Dispose()
    }
    if ($code -eq 0) {
        throw "O instalador terminou com sucesso com o executável em uso."
    }
    if ((Get-Sha256 $executable) -ne $newerSha -or (Get-RegisteredVersion) -ne $Version) {
        throw "Com o executável em uso, o instalador alterou a instalação (código $code)."
    }
    Write-Host "Executável em uso: o instalador falhou com código $code e nada mudou."
    if ($olderSha -eq $newerSha) {
        throw "As duas versões de teste têm o mesmo executável; o teste não distingue as versões."
    }

    $uninstall = Start-Process -FilePath (Join-Path $installDirectory "unins000.exe") -Wait -PassThru `
        -ArgumentList @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART")
    if ($uninstall.ExitCode -ne 0) {
        throw "A desinstalação falhou com código $($uninstall.ExitCode)."
    }
    $deadline = (Get-Date).AddSeconds(30)
    while (((Test-Path -LiteralPath $executable) -or (Test-Path -LiteralPath $uninstallKey)) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 1
    }
    if ((Test-Path -LiteralPath $executable) -or (Test-Path -LiteralPath $uninstallKey)) {
        throw "A desinstalação deixou o executável ou a entrada na lista de programas."
    }
    Write-Host "Desinstalado: arquivos e entrada removidos."
}
finally {
    # Never leaves the test entry behind, even when a check failed.
    $uninstaller = Join-Path $installDirectory "unins000.exe"
    if ((Test-Path -LiteralPath $uninstallKey) -and (Test-Path -LiteralPath $uninstaller)) {
        Start-Process -FilePath $uninstaller -Wait -ArgumentList @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART")
        Start-Sleep -Seconds 3
    }
    if (Test-Path -LiteralPath $uninstallKey) {
        Remove-Item -LiteralPath $uninstallKey -Recurse -Force
    }
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
