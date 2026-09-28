[Setup]
; Información básica del programa
AppName=AmoxliCode
AppVersion=0.1.0
AppPublisher=Diego
AppPublisherURL=https://github.com/diegomatias0303/amoxtlicode
DefaultDirName={autopf}\AmoxliCode
DefaultGroupName=AmoxliCode
DisableProgramGroupPage=yes
; Carpeta donde se guardará el instalador generado
OutputDir=Output
OutputBaseFilename=Instalar_AmoxliCode
Compression=lzma
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; Copiar el ejecutable principal
Source: "target\release\amoxlicode.exe"; DestDir: "{app}"; Flags: ignoreversion
; Copiar la carpeta assets completa con videos/fuentes
Source: "assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
; Crear acceso directo en el menú inicio
Name: "{group}\AmoxliCode"; Filename: "{app}\amoxlicode.exe"
; Crear acceso directo en el escritorio (si el usuario marca la casilla)
Name: "{autodesktop}\AmoxliCode"; Filename: "{app}\amoxlicode.exe"; Tasks: desktopicon

[Registry]
; Agregar "Abrir con AmoxliCode" al clic derecho de Windows para cualquier archivo (*)
Root: HKCU; Subkey: "Software\Classes\*\shell\AmoxliCode"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\*\shell\AmoxliCode"; ValueType: string; ValueName: ""; ValueData: "Abrir con AmoxliCode"
Root: HKCU; Subkey: "Software\Classes\*\shell\AmoxliCode"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\amoxlicode.exe"""
Root: HKCU; Subkey: "Software\Classes\*\shell\AmoxliCode\command"; ValueType: string; ValueName: ""; ValueData: """{app}\amoxlicode.exe"" ""%1"""

[Run]
; Dar la opción de ejecutar el programa al terminar la instalación
Filename: "{app}\amoxlicode.exe"; Description: "{cm:LaunchProgram,AmoxliCode}"; Flags: nowait postinstall skipifsilent
