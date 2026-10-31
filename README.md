# sane-rec 🎬

[![Release](https://img.shields.io/github/v/release/RCarrascoO/sane-rec?color=7c4dff&logo=github)](https://github.com/RCarrascoO/sane-rec/releases)
[![License](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Linux%20(Wayland%20%2F%20PipeWire)-informational)](https://github.com/RCarrascoO/sane-rec)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

**sane-rec** es un grabador de pantalla y búfer de repetición instantánea (*Instant Replay / Shadowplay*) para Linux, con una arquitectura optimizada para CPU (cero riesgo de cuelgues en GPUs como AMD Polaris) y una interfaz gráfica moderna basada en **Material Design 3 (M3 Expressive)** construida con [Slint](https://slint.dev/).

---

## ✨ Características Principales

- ⚡ **Repetición Instantánea (Instant Replay):** Búfer rodante en memoria RAM (`/dev/shm`) configurable (15s, 30s, 45s, 60s, 90s, 120s) para guardar tus mejores momentos con un solo atajo de teclado o clic, sin escribir en disco hasta que tú lo decidas.
- 🔴 **Grabación Continua:** Grabación directa a disco a 60 FPS o 30 FPS en formatos `.mp4` o `.mkv` con códecs configurables.
- 🛡️ **Perfil Seguro `Sane Safe (CPU x264)`:** Codificación por CPU ultra rápida con `libx264` en modo zerolatency. Diseñado específicamente para evitar el conocido bloqueo/congelamiento del driver de GPU AMD Polaris (RX 570/580) en Linux Wayland.
- 🎛️ **Perfiles de Hardware & Gestor de Equipos:** Presets para AMD (VA-API), NVIDIA (NVENC) e Intel (QuickSync), además de un gestor de perfiles exportables/importables (`~/.config/sane-rec/profiles/`) para adaptar configuraciones a portátiles o PCs específicas.
- 🔊 **Mezclador de Audio PipeWire:** Control de volumen y calibrador visual VU en tiempo real para el audio del juego y micrófono, con opción de mezclar en una pista única (ideal para Discord) o guardar pistas de audio separadas (para edición en DaVinci Resolve / Kdenlive).
- ⌨️ **Atajos de Teclado Globales Personalizables:** Compatible con compositores Wayland (niri, Sway, Hyprland). Incluye presets como `Alt+F10` y un campo para escribir cualquier combinación personalizada propia.
- 🚀 **Auto-Actualizador Integrado:** Verificación no bloqueante con GitHub Releases, alerta visual de 1 clic en la cabecera y comando `sane-rec update` en terminal.

---

## 📦 Instalación

### Requisitos del Sistema
Para funcionar en cualquier distribución Linux (Wayland):
- `wf-recorder` (captura por dmabuf/PipeWire)
- `ffmpeg` (multiplexado y concatenación ultra rápida)
- `pipewire` y herramientas de audio (`pactl` / `pulseaudio-utils`)

**En Arch Linux / CachyOS:**
```bash
sudo pacman -S wf-recorder ffmpeg pipewire-pulse
```

**En Ubuntu / Debian:**
```bash
sudo apt install wf-recorder ffmpeg pulseaudio-utils
```

**En Fedora:**
```bash
sudo dnf install wf-recorder ffmpeg pulseaudio-utils
```

---

### Opción 1: Instalación Rápida desde Release (Recomendada)
Descarga la versión precompilada más reciente desde [GitHub Releases](https://github.com/RCarrascoO/sane-rec/releases):

```bash
# 1. Descarga el paquete tar.gz de la última versión
curl -sSL https://github.com/RCarrascoO/sane-rec/releases/latest/download/sane-rec-v0.1.0-linux-x86_64.tar.gz | tar -xz

# 2. Ingresa a la carpeta extraída e instala
cd sane-rec-v0.1.0-linux-x86_64
./install.sh
```

El script `install.sh` instala:
- El binario en `~/.local/bin/sane-rec`
- El acceso directo en `~/.local/share/applications/sane-rec.desktop`
- El icono vectorial en `~/.local/share/icons/hicolor/scalable/apps/sane-rec.svg`

*(Asegúrate de que `~/.local/bin` esté en tu `$PATH`).*

---

### Opción 2: Compilación desde el Código Fuente
Requiere Rust 1.75+ y librerías de desarrollo:

```bash
# Clonar repositorio
git clone https://github.com/RCarrascoO/sane-rec.git
cd sane-rec

# Compilar en modo Release optimizado
cargo build --release

# Instalar binario, icono y acceso directo de escritorio
./install.sh
```

---

## 🚀 Uso y Atajos de Teclado

### Modo Gráfico (GUI)
Abre tu lanzador de aplicaciones (fuzzel, rofi, wofi o menú de tu escritorio) y busca **sane-rec**, o ejecuta en tu terminal:
```bash
sane-rec
```

### Atajos Globales en Wayland (Niri / Sway / Hyprland)
Por defecto:
- **Guardar Replay Instantáneo:** `Mod + F10` (o `Alt + F10` configurable en Ajustes)
- **Alternar Grabación Continua:** `Mod + F9` (o `Alt + F9` configurable en Ajustes)

Los atajos se pueden personalizar libremente desde la pestaña **Ajustes** en la app.

### Comandos de Terminal (CLI)
`sane-rec` incluye una interfaz CLI completa que se comunica vía socket IPC Unix con la app o daemon en segundo plano:

```bash
sane-rec save               # Guarda un replay instantáneo desde la terminal o atajo
sane-rec record             # Alterna la grabación continua (toggle)
sane-rec record start       # Inicia la grabación continua
sane-rec record stop        # Detiene la grabación continua
sane-rec status             # Consulta el estado del búfer y de la grabación
sane-rec update             # Comprueba y actualiza a la última versión de GitHub
sane-rec daemon             # Inicia en modo headless en segundo plano
```

---

## 🔄 Auto-Actualización

Cuando haya una nueva versión publicada en GitHub:
1. La aplicación mostrará un banner superior con el botón interactivo **[Actualizar ahora]**.
2. También puedes actualizar en cualquier momento desde la consola ejecutando:
   ```bash
   sane-rec update
   ```

---

## 📄 Licencia
Este proyecto está bajo licencia dual **MIT** o **Apache-2.0**.
