#!/usr/bin/env bash
set -e

# Script de instalación para AmoxliCode v1.0.0 en sistemas Linux

echo "=== Instalador de AmoxliCode v1.0.0 para Linux ==="

if [ "$(id -u)" -eq 0 ]; then
    BIN_DIR="/usr/local/bin"
    DESKTOP_DIR="/usr/share/applications"
    SHARE_DIR="/usr/local/share/amoxlicode"
    ICON_DIR="/usr/share/icons/hicolor/256x256/apps"
    echo "[*] Modo: Instalación para todo el sistema (root)"
else
    BIN_DIR="$HOME/.local/bin"
    DESKTOP_DIR="$HOME/.local/share/applications"
    SHARE_DIR="$HOME/.local/share/amoxlicode"
    ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"
    echo "[*] Modo: Instalación de usuario actual ($USER)"
fi

mkdir -p "$BIN_DIR" "$DESKTOP_DIR" "$SHARE_DIR" "$ICON_DIR"

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

# 2. Copiar recursos e ícono oficial
if [ -d "$SCRIPT_DIR/assets" ]; then
    echo "[*] Copiando recursos a $SHARE_DIR/assets..."
    mkdir -p "$SHARE_DIR/assets"
    cp -r "$SCRIPT_DIR/assets/"* "$SHARE_DIR/assets/"
    if [ -f "$SCRIPT_DIR/assets/icon.png" ]; then
        install -m 644 "$SCRIPT_DIR/assets/icon.png" "$ICON_DIR/amoxlicode.png"
    fi
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
echo "=== ¡AmoxliCode v1.0.0 instalado con éxito! ==="
echo "Puedes ejecutarlo desde tu terminal escribiendo: amoxlicode"
echo "O encontrarlo en tu menú de aplicaciones bajo 'AmoxliCode'."
