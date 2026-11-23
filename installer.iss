[Setup]
; Información básica del programa
AppName=AmoxliCode
AppVersion=1.0.0
AppVerName=AmoxliCode 1.0.0
AppPublisher=Diego
AppPublisherURL=https://github.com/diegomatias0303/amoxtlicode
DefaultDirName={autopf}\AmoxliCode
DefaultGroupName=AmoxliCode
DisableProgramGroupPage=yes
SetupIconFile=assets\icon.ico
UninstallDisplayIcon={app}\amoxlicode.exe
; Carpeta donde se guardará el instalador generado
OutputDir=Output
OutputBaseFilename=Instalar_AmoxliCode_v1.0.0
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; Ejecutable principal
Source: "target\release\amoxlicode.exe"; DestDir: "{app}"; Flags: ignoreversion
; Bibliotecas dinámicas de FFmpeg (generadas por empaquetar.ps1 en dist_dlls\)
Source: "dist_dlls\*.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
; Recursos (video de fondo e ícono)
Source: "assets\background.mp4"; DestDir: "{app}\assets"; Flags: ignoreversion skipifsourcedoesntexist
Source: "assets\icon.png"; DestDir: "{app}\assets"; Flags: ignoreversion skipifsourcedoesntexist
Source: "assets\icon.ico"; DestDir: "{app}\assets"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\AmoxliCode"; Filename: "{app}\amoxlicode.exe"; IconFilename: "{app}\assets\icon.ico"
Name: "{autodesktop}\AmoxliCode"; Filename: "{app}\amoxlicode.exe"; IconFilename: "{app}\assets\icon.ico"; Tasks: desktopicon

[Registry]
; Agregar "Abrir con AmoxliCode" al clic derecho de Windows para cualquier archivo (*)
Root: HKCU; Subkey: "Software\Classes\*\shell\AmoxliCode"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\*\shell\AmoxliCode"; ValueType: string; ValueName: ""; ValueData: "Abrir con AmoxliCode"
Root: HKCU; Subkey: "Software\Classes\*\shell\AmoxliCode"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\amoxlicode.exe"",0"
Root: HKCU; Subkey: "Software\Classes\*\shell\AmoxliCode\command"; ValueType: string; ValueName: ""; ValueData: """{app}\amoxlicode.exe"" ""%1"""

[Run]
Filename: "{app}\amoxlicode.exe"; Description: "{cm:LaunchProgram,AmoxliCode}"; Flags: nowait postinstall skipifsilent
