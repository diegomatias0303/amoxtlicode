$exePath = (Get-Item ".\amoxlicode.exe").FullName

if (-Not (Test-Path $exePath)) {
    Write-Host "Error: No se encontró amoxlicode.exe en esta carpeta." -ForegroundColor Red
    Write-Host "Por favor ejecuta este script en la misma carpeta donde está el ejecutable." -ForegroundColor Yellow
    Pause
    Exit
}

Write-Host "Agregando AmoxliCode al menú contextual de Windows..." -ForegroundColor Cyan

# Crear las llaves en el registro (Para el usuario actual)
$registryPath = "HKCU:\Software\Classes\*\shell\AmoxliCode"
$commandPath = "$registryPath\command"

if (-Not (Test-Path $registryPath)) {
    New-Item -Path $registryPath -Force | Out-Null
}
if (-Not (Test-Path $commandPath)) {
    New-Item -Path $commandPath -Force | Out-Null
}

# Configurar icono y nombre
Set-ItemProperty -Path $registryPath -Name "(default)" -Value "Abrir con AmoxliCode"
Set-ItemProperty -Path $registryPath -Name "Icon" -Value ""$exePath""

# Configurar comando
Set-ItemProperty -Path $commandPath -Name "(default)" -Value ""$exePath" "%1""

Write-Host "¡Listo! Ahora puedes hacer clic derecho en cualquier archivo y elegir 'Abrir con AmoxliCode'." -ForegroundColor Green
Pause
