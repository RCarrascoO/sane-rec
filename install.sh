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
ICON_PNG_DEST="${HOME}/.local/share/icons/hicolor/512x512/apps"
PIXMAPS_DEST="${HOME}/.local/share/pixmaps"

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

# 1. Crear directorios necesarios y limpiar instancias previas
mkdir -p "${BIN_DEST}" "${DESKTOP_DEST}" "${ICON_DEST}" "${ICON_PNG_DEST}" "${PIXMAPS_DEST}"
for sz in 256 128 64 48 32 16; do
    mkdir -p "${HOME}/.local/share/icons/hicolor/${sz}x${sz}/apps"
done

# Detener cualquier instancia anterior para evitar procesos huérfanos o sockets bloqueados
if command -v sane-rec >/dev/null 2>&1; then
    sane-rec stop >/dev/null 2>&1 || true
fi
pkill -x sane-rec 2>/dev/null || true
rm -f "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/sane-rec.sock" 2>/dev/null || true

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

# 3. Copiar archivo .desktop e iconos multiresolución
if [[ -f "${SCRIPT_DIR}/assets/sane-rec.desktop" ]]; then
    cp "${SCRIPT_DIR}/assets/sane-rec.desktop" "${DESKTOP_DEST}/sane-rec.desktop"
    echo "✔ Acceso directo de escritorio instalado en: ${DESKTOP_DEST}/sane-rec.desktop"
fi

# 3.1 Copiar perfiles de hardware a ~/.config/sane-rec/profiles/
PROFILES_DEST="${XDG_CONFIG_HOME:-${HOME}/.config}/sane-rec/profiles"
mkdir -p "${PROFILES_DEST}"
if [[ -d "${SCRIPT_DIR}/profiles" ]]; then
    cp "${SCRIPT_DIR}/profiles/"*.toml "${PROFILES_DEST}/" 2>/dev/null || true
    echo "✔ Perfiles de hardware actualizados en: ${PROFILES_DEST}"
fi

if [[ -f "${SCRIPT_DIR}/assets/sane-rec.svg" ]]; then
    cp "${SCRIPT_DIR}/assets/sane-rec.svg" "${ICON_DEST}/sane-rec.svg"
    cp "${SCRIPT_DIR}/assets/sane-rec.svg" "${PIXMAPS_DEST}/sane-rec.svg"
    if [[ -d "${HOME}/.local/share/icons/WhiteSur/apps/scalable" ]]; then
        cp "${SCRIPT_DIR}/assets/sane-rec.svg" "${HOME}/.local/share/icons/WhiteSur/apps/scalable/sane-rec.svg"
    fi
    echo "✔ Icono vectorial instalado en: ${ICON_DEST}/sane-rec.svg"
fi

if [[ -f "${SCRIPT_DIR}/assets/sane-rec.png" ]]; then
    cp "${SCRIPT_DIR}/assets/sane-rec.png" "${ICON_PNG_DEST}/sane-rec.png"
    cp "${SCRIPT_DIR}/assets/sane-rec.png" "${PIXMAPS_DEST}/sane-rec.png"
    
    # Generar resoluciones estándar para compatibilidad con todos los launchers
    if command -v python3 >/dev/null 2>&1; then
        python3 -c "
from PIL import Image
src = Image.open('${SCRIPT_DIR}/assets/sane-rec.png')
for s in [256, 128, 64, 48, 32, 16]:
    d = '${HOME}/.local/share/icons/hicolor/' + str(s) + 'x' + str(s) + '/apps'
    src.resize((s, s), Image.Resampling.LANCZOS).save(d + '/sane-rec.png')
" 2>/dev/null || true
    fi
    echo "✔ Iconos oficiales PNG multiresolución instalados"
fi

# 4. Actualizar bases de datos de escritorio y cachés de iconos
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${DESKTOP_DEST}" || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
    if [[ -d "${HOME}/.local/share/icons/WhiteSur" ]]; then
        gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/WhiteSur" >/dev/null 2>&1 || true
    fi
    if [[ -d "${HOME}/.local/share/icons/WhiteSur-dark" ]]; then
        gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/WhiteSur-dark" >/dev/null 2>&1 || true
    fi
fi

# 5. Si quickshell (sane-shell) está activo, reiniciarlo para invalidar su caché de texturas
if pgrep -x quickshell >/dev/null 2>&1; then
    pkill -x quickshell || true
    sleep 0.5
    quickshell -d -c sane-shell >/dev/null 2>&1 || true
fi

echo ""
echo "🎉 ¡Instalación completada con éxito!"
echo "Asegúrate de tener '${BIN_DEST}' en tu PATH. Puedes ejecutarlo ahora con 'sane-rec' o buscarlo en tu lanzador de aplicaciones (fuzzel / rofi / sane-shell / quickshell)."
