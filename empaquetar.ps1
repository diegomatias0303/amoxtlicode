Write-Host "Construyendo AmoxliCode en modo Release..." -ForegroundColor Cyan
cargo build --release

Write-Host "Preparando carpeta de empaquetado..." -ForegroundColor Cyan
$releaseDir = "AmoxliCode_Portable"
if (Test-Path $releaseDir) {
    Remove-Item -Recurse -Force $releaseDir
}
New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null

Write-Host "Copiando ejecutable y scripts..." -ForegroundColor Cyan
Copy-Item "target\release\amoxlicode.exe" -Destination $releaseDir
Copy-Item "Add-ContextMenu.ps1" -Destination $releaseDir

Write-Host "Copiando recursos (assets)..." -ForegroundColor Cyan
if (Test-Path "assets") {
    Copy-Item "assets" -Destination $releaseDir -Recurse
}

Write-Host "Comprimiendo en formato ZIP..." -ForegroundColor Cyan
$zipFile = "AmoxliCode_Release.zip"
if (Test-Path $zipFile) {
    Remove-Item -Force $zipFile
}
Compress-Archive -Path "$releaseDir\*" -DestinationPath $zipFile

Write-Host "Limpiando..." -ForegroundColor Cyan
Remove-Item -Recurse -Force $releaseDir

Write-Host "¡Listo! Tu paquete está en $zipFile" -ForegroundColor Green
