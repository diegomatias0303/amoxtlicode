$exeCandidate = Join-Path (Get-Location) "amoxlicode.exe"

if (-Not (Test-Path $exeCandidate)) {
    Write-Host "Error: No se encontró amoxlicode.exe en esta carpeta." -ForegroundColor Red
    Write-Host "Por favor ejecuta este script en la misma carpeta donde está el ejecutable." -ForegroundColor Yellow
    Pause
    Exit 1
}

$exePath = (Resolve-Path $exeCandidate).Path

Write-Host "Agregando AmoxliCode al menú contextual de Windows..." -ForegroundColor Cyan

$registryPath = "HKCU:\Software\Classes\*\shell\AmoxliCode"
$commandPath = "$registryPath\command"

if (-Not (Test-Path $registryPath)) {
    New-Item -Path $registryPath -Force | Out-Null
}
if (-Not (Test-Path $commandPath)) {
    New-Item -Path $commandPath -Force | Out-Null
}

Set-ItemProperty -Path $registryPath -Name "(default)" -Value "Abrir con AmoxliCode"
Set-ItemProperty -Path $registryPath -Name "Icon" -Value "`"$exePath`",0"
Set-ItemProperty -Path $commandPath -Name "(default)" -Value "`"$exePath`" `"%1`""

Write-Host "¡Listo! Ahora puedes hacer clic derecho en cualquier archivo y elegir 'Abrir con AmoxliCode'." -ForegroundColor Green
Pause
