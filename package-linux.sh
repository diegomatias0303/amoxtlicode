#!/usr/bin/env bash
set -e

# Script de empaquetado para distribución en Linux

echo "=== Empaquetando AmoxliCode para Linux ==="

echo "[*] Compilando en modo Release..."
cargo build --release

RELEASE_DIR="AmoxliCode_Linux_Portable"
ARCHIVE_NAME="AmoxliCode_Linux_x86_64.tar.gz"

echo "[*] Preparando carpeta de empaquetado ($RELEASE_DIR)..."
rm -rf "$RELEASE_DIR"
mkdir -p "$RELEASE_DIR"

echo "[*] Copiando binarios y archivos de instalacion..."
cp target/release/amoxlicode "$RELEASE_DIR/"
cp install.sh "$RELEASE_DIR/"
chmod +x "$RELEASE_DIR/amoxlicode" "$RELEASE_DIR/install.sh"

if [ -f "amoxlicode.desktop" ]; then
    cp amoxlicode.desktop "$RELEASE_DIR/"
fi

if [ -f "README.md" ]; then
    cp README.md "$RELEASE_DIR/"
fi

if [ -d "assets" ]; then
    echo "[*] Copiando recursos (assets)..."
    cp -r assets "$RELEASE_DIR/"
fi

echo "[*] Creando archivo comprimido $ARCHIVE_NAME..."
rm -f "$ARCHIVE_NAME"
tar -czf "$ARCHIVE_NAME" "$RELEASE_DIR"

echo "[*] Limpiando carpeta temporal..."
rm -rf "$RELEASE_DIR"

echo ""
echo "=== ¡Empaquetado completado! ==="
echo "Tu archivo para distribución en Linux es: $ARCHIVE_NAME"
