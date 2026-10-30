#!/usr/bin/env bash
# ==============================================================================
# sane-rec installer — Instalador local para Wayland & Linux
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DEST="${HOME}/.local/bin"
DESKTOP_DEST="${HOME}/.local/share/applications"
ICON_DEST="${HOME}/.local/share/icons/hicolor/scalable/apps"

echo "=== Instalando sane-rec ==="

# 1. Crear directorios necesarios
mkdir -p "${BIN_DEST}" "${DESKTOP_DEST}" "${ICON_DEST}"

# 2. Copiar binario
if [[ -f "${SCRIPT_DIR}/sane-rec" ]]; then
    cp "${SCRIPT_DIR}/sane-rec" "${BIN_DEST}/sane-rec"
    chmod +x "${BIN_DEST}/sane-rec"
    echo "✔ Binario instalado en: ${BIN_DEST}/sane-rec"
elif [[ -f "${SCRIPT_DIR}/target/release/sane-rec" ]]; then
    cp "${SCRIPT_DIR}/target/release/sane-rec" "${BIN_DEST}/sane-rec"
    chmod +x "${BIN_DEST}/sane-rec"
    echo "✔ Binario compilado instalado en: ${BIN_DEST}/sane-rec"
else
    echo "❌ Error: No se encontró el binario 'sane-rec'. Ejecuta 'cargo build --release' primero."
    exit 1
fi

# 3. Copiar archivo .desktop e icono SVG
if [[ -f "${SCRIPT_DIR}/assets/sane-rec.desktop" ]]; then
    cp "${SCRIPT_DIR}/assets/sane-rec.desktop" "${DESKTOP_DEST}/sane-rec.desktop"
    echo "✔ Acceso directo de escritorio instalado en: ${DESKTOP_DEST}/sane-rec.desktop"
fi

if [[ -f "${SCRIPT_DIR}/assets/sane-rec.svg" ]]; then
    cp "${SCRIPT_DIR}/assets/sane-rec.svg" "${ICON_DEST}/sane-rec.svg"
    echo "✔ Icono vectorial instalado en: ${ICON_DEST}/sane-rec.svg"
fi

# 4. Actualizar bases de datos de escritorio si las herramientas existen
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${DESKTOP_DEST}" || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" || true
fi

echo ""
echo "🎉 ¡Instalación completada con éxito!"
echo "Asegúrate de tener '${BIN_DEST}' en tu PATH. Puedes ejecutarlo ahora con 'sane-rec' o buscarlo en tu lanzador de aplicaciones (fuzzel / rofi / wofi)."
