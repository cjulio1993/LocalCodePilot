; Instalador do LocalCodePilot para Windows (Inno Setup 6).
;
; Instala por usuário em %LOCALAPPDATA%\Programs\LocalCodePilot, sem pedir administrador: a
; pasta continua gravável, o que mantém a atualização pelo aplicativo funcionando.
; O script de pacote passa AppVersion, SourceDir, RepositoryRoot, OutputDir e OutputBaseFilename.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
; O AppId e o nome exibido só mudam no instalador descartável do teste de atualização
; (scripts/e2e-update-windows.ps1), para ele nunca tocar na instalação real.
#ifndef AppIdValue
  #define AppIdValue "{{8B3C2F1E-6D4A-4C8E-9A51-3F2B7C1D9E40}"
#endif
#ifndef AppDisplayName
  #define AppDisplayName "LocalCodePilot"
#endif

[Setup]
; Identidade fixa: atualizações e desinstalação reconhecem a mesma instalação.
AppId={#AppIdValue}
AppName=LocalCodePilot
AppVersion={#AppVersion}
AppVerName=LocalCodePilot {#AppVersion}
AppPublisher=LocalCodePilot
AppPublisherURL=https://github.com/cjulio1993/LocalCodePilot
AppSupportURL=https://github.com/cjulio1993/LocalCodePilot/issues
AppUpdatesURL=https://github.com/cjulio1993/LocalCodePilot/releases
DefaultDirName={localappdata}\Programs\LocalCodePilot
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
SetupIconFile={#RepositoryRoot}\assets\branding\localcodepilot.ico
UninstallDisplayIcon={app}\LocalCodePilot.exe
UninstallDisplayName={#AppDisplayName}
LicenseFile={#RepositoryRoot}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\LocalCodePilot.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README-WINDOWS.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\RELEASE-NOTES.md"; DestDir: "{app}"; Flags: ignoreversion

; O instalador do teste de atualização não cria atalhos no Menu Iniciar de quem roda o teste.
#ifndef SemAtalhos
[Icons]
Name: "{autoprograms}\{#AppDisplayName}"; Filename: "{app}\LocalCodePilot.exe"
Name: "{autodesktop}\{#AppDisplayName}"; Filename: "{app}\LocalCodePilot.exe"; Tasks: desktopicon
#endif

[Run]
Filename: "{app}\LocalCodePilot.exe"; Description: "{cm:LaunchProgram,LocalCodePilot}"; Flags: nowait postinstall skipifsilent
