$ErrorActionPreference = "Stop"

Write-Host "=== Empaquetando AmoxliCode v1.0.0 (Windows x64) ===" -ForegroundColor Cyan
Write-Host "[1/5] Compilando AmoxliCode en modo Release..." -ForegroundColor Cyan
cargo build --release

# Localizar las DLLs de FFmpeg requeridas por el binario
$ffmpegSearchDirs = @(
    $env:FFMPEG_BIN_DIR,
    $(if ($env:FFMPEG_DIR) { Join-Path $env:FFMPEG_DIR "bin" }),
    "C:\vcpkg\installed\x64-windows\bin"
) + ($env:PATH -split ';') | Where-Object { $_ -and (Test-Path $_) } | Select-Object -Unique

$ffmpegBinDir = $null
foreach ($dir in $ffmpegSearchDirs) {
    if (Test-Path (Join-Path $dir "avcodec-*.dll")) {
        $ffmpegBinDir = $dir
        break
    }
}

$dllPatterns = @("avcodec-*.dll", "avdevice-*.dll", "avfilter-*.dll", "avformat-*.dll", "avutil-*.dll", "swresample-*.dll", "swscale-*.dll")

Write-Host "[2/5] Preparando carpetas de distribución y DLLs..." -ForegroundColor Cyan
$distDllDir = "dist_dlls"
if (Test-Path $distDllDir) { Remove-Item -Recurse -Force $distDllDir }
New-Item -ItemType Directory -Force -Path $distDllDir | Out-Null

if ($ffmpegBinDir) {
    Write-Host "      FFmpeg encontrado en: $ffmpegBinDir" -ForegroundColor Gray
    foreach ($pat in $dllPatterns) {
        Get-ChildItem -Path $ffmpegBinDir -Filter $pat -ErrorAction SilentlyContinue | ForEach-Object {
            Copy-Item $_.FullName -Destination $distDllDir -Force
        }
    }
} else {
    Write-Warning "No se encontraron las DLLs de FFmpeg en PATH/vcpkg. El paquete podría requerir FFmpeg en el equipo destino."
}

$releaseDir = "AmoxliCode_v1.0.0_Portable"
if (Test-Path $releaseDir) { Remove-Item -Recurse -Force $releaseDir }
New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null

Write-Host "[3/5] Copiando ejecutable, DLLs y scripts..." -ForegroundColor Cyan
Copy-Item "target\release\amoxlicode.exe" -Destination $releaseDir
Copy-Item "Add-ContextMenu.ps1" -Destination $releaseDir
Copy-Item "README.md" -Destination $releaseDir
Copy-Item "$distDllDir\*.dll" -Destination $releaseDir -ErrorAction SilentlyContinue

Write-Host "[4/5] Copiando recursos (assets)..." -ForegroundColor Cyan
if (Test-Path "assets") {
    $assetsOut = Join-Path $releaseDir "assets"
    New-Item -ItemType Directory -Force -Path $assetsOut | Out-Null
    Get-ChildItem "assets" | Where-Object { $_.Extension -ne ".rc" } | ForEach-Object {
        Copy-Item $_.FullName -Destination $assetsOut -Recurse
    }
}

Write-Host "[5/5] Comprimiendo en formato ZIP..." -ForegroundColor Cyan
$zipFile = "AmoxliCode_v1.0.0_Windows_x64.zip"
if (Test-Path $zipFile) { Remove-Item -Force $zipFile }
Compress-Archive -Path "$releaseDir\*" -DestinationPath $zipFile

Remove-Item -Recurse -Force $releaseDir

Write-Host ""
Write-Host "¡Listo! Paquete portable generado en: $zipFile" -ForegroundColor Green
Write-Host "Las DLLs también quedaron listas en 'dist_dlls\' para compilar installer.iss con Inno Setup." -ForegroundColor Green
