#!/usr/bin/env bash
set -e

# Script de instalación para AmoxliCode en sistemas Linux

echo "=== Instalador de AmoxliCode para Linux ==="

# Determinar modo de instalación (sistema vs usuario)
if [ "$(id -u)" -eq 0 ]; then
    BIN_DIR="/usr/local/bin"
    DESKTOP_DIR="/usr/share/applications"
    SHARE_DIR="/usr/local/share/amoxlicode"
    echo "[*] Modo: Instalación para todo el sistema (root)"
else
    BIN_DIR="$HOME/.local/bin"
    DESKTOP_DIR="$HOME/.local/share/applications"
    SHARE_DIR="$HOME/.local/share/amoxlicode"
    echo "[*] Modo: Instalación de usuario actual ($USER)"
fi

mkdir -p "$BIN_DIR"
mkdir -p "$DESKTOP_DIR"
mkdir -p "$SHARE_DIR"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 1. Instalar binario ejecutable
if [ -f "$SCRIPT_DIR/target/release/amoxlicode" ]; then
    SRC_BIN="$SCRIPT_DIR/target/release/amoxlicode"
elif [ -f "$SCRIPT_DIR/amoxlicode" ]; then
    SRC_BIN="$SCRIPT_DIR/amoxlicode"
else
    echo "[*] Compilando binario en modo release..."
    cargo build --release
    SRC_BIN="$SCRIPT_DIR/target/release/amoxlicode"
fi

echo "[*] Instalando ejecutable en $BIN_DIR/amoxlicode..."
install -m 755 "$SRC_BIN" "$BIN_DIR/amoxlicode"

# 2. Copiar recursos si existen
if [ -d "$SCRIPT_DIR/assets" ]; then
    echo "[*] Copiando recursos a $SHARE_DIR/assets..."
    cp -r "$SCRIPT_DIR/assets" "$SHARE_DIR/"
fi

# 3. Instalar acceso directo .desktop
if [ -f "$SCRIPT_DIR/amoxlicode.desktop" ]; then
    echo "[*] Registrando acceso directo en $DESKTOP_DIR/amoxlicode.desktop..."
    install -m 644 "$SCRIPT_DIR/amoxlicode.desktop" "$DESKTOP_DIR/amoxlicode.desktop"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
    fi
fi

echo ""
echo "=== ¡AmoxliCode instalado con éxito! ==="
echo "Puedes ejecutarlo desde tu terminal escribiendo: amoxlicode"
echo "O encontrarlo en tu menú de aplicaciones bajo 'AmoxliCode'."
