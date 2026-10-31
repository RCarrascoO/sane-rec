#!/usr/bin/env bash
# ==============================================================================
# sane-rec installer — Instalador universal para Wayland & Linux
# ==============================================================================
set -euo pipefail

SCRIPT_SOURCE="${BASH_SOURCE[0]:-}"
if [[ -n "${SCRIPT_SOURCE}" && -f "${SCRIPT_SOURCE}" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${SCRIPT_SOURCE}")" && pwd)"
else
    SCRIPT_DIR=""
fi

BIN_DEST="${HOME}/.local/bin"
DESKTOP_DEST="${HOME}/.local/share/applications"
ICON_DEST="${HOME}/.local/share/icons/hicolor/scalable/apps"

echo "=== Instalando sane-rec ==="

# Si se ejecuta directamente vía `curl ... | bash` o fuera de la carpeta del paquete:
if [[ -z "${SCRIPT_DIR}" || (! -f "${SCRIPT_DIR}/sane-rec" && ! -f "${SCRIPT_DIR}/target/release/sane-rec") ]]; then
    echo "⬇ Descargando el último release oficial desde GitHub..."
    TMP_DIR="$(mktemp -d /tmp/sane-rec-install.XXXXXX)"
    trap 'rm -rf "${TMP_DIR}"' EXIT

    TARBALL_URL=$(curl -s "https://api.github.com/repos/RCarrascoO/sane-rec/releases/latest" \
        | grep -o '"browser_download_url": *"[^"]*linux-x86_64\.tar\.gz"' \
        | head -n1 \
        | cut -d '"' -f 4)

    if [[ -z "${TARBALL_URL}" ]]; then
        echo "❌ Error: No se pudo localizar el paquete de descarga en GitHub Releases."
        exit 1
    fi

    echo "📦 Obteniendo paquete: ${TARBALL_URL}"
    curl -sSL "${TARBALL_URL}" -o "${TMP_DIR}/release.tar.gz"
    tar -xzf "${TMP_DIR}/release.tar.gz" -C "${TMP_DIR}"

    EXTRACTED_DIR=$(find "${TMP_DIR}" -mindepth 1 -maxdepth 1 -type d | head -n1)
    SCRIPT_DIR="${EXTRACTED_DIR}"
fi

# 1. Crear directorios necesarios
mkdir -p "${BIN_DEST}" "${DESKTOP_DEST}" "${ICON_DEST}"

# 2. Copiar binario
rm -f "${BIN_DEST}/sane-rec"
if [[ -f "${SCRIPT_DIR}/sane-rec" ]]; then
    cp "${SCRIPT_DIR}/sane-rec" "${BIN_DEST}/sane-rec"
    chmod +x "${BIN_DEST}/sane-rec"
    echo "✔ Binario instalado en: ${BIN_DEST}/sane-rec"
elif [[ -f "${SCRIPT_DIR}/target/release/sane-rec" ]]; then
    cp "${SCRIPT_DIR}/target/release/sane-rec" "${BIN_DEST}/sane-rec"
    chmod +x "${BIN_DEST}/sane-rec"
    echo "✔ Binario compilado instalado en: ${BIN_DEST}/sane-rec"
else
    echo "❌ Error: No se encontró el binario 'sane-rec'."
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
