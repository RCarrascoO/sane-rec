# Hoja de Ruta (Roadmap) — sane-rec (Post v0.7.2 / Actualizado v0.7.3)

Documento de planificación técnica para las próximas versiones de sane-rec.

```
                   HOJA DE RUTA SANE-REC (POST v0.7.2)
  ┌──────────────────────────────────────────────────────────────────┐
  │ 1. BACKEND & COMPRESSOR UPDATE (Base sólida, YAGNI & Eficiencia) │ [✅ COMPLETADO v0.7.3]
  │    • Limpieza de código muerto (GStreamer), YAGNI & optimización │
  │    • Smart Dual-Clip: Zero clones si peso <= target & -threads 2 │
  │    • Estabilización VA-API (renderD128) y blindaje multi-shell   │
  │    • Telemetría forense expandida (VRAM, GPU, CLI ejecutada)     │
  ├──────────────────────────────────────────────────────────────────┤
  │ 2. GRAPHICS UPDATE (Aceleración Hardware AMD & Nvidia en Windows)│ [✅ COMPLETADO v0.7.6]
  │    • AMD Radeon & Intel QSV: Mapeo AMF/QSV Zero B-Frames         │
  │    • Nvidia NVENC: Sincronía A/V WASAPI y normalización de códec │
  │    • HEVC Fix: Inyección VPS/SPS/PPS (hvc1) + hevc_mp4toannexb   │
  │    • Resiliencia DXGI: Compuerta MPEG-TS + Watchdog + GdiGrab    │
  ├──────────────────────────────────────────────────────────────────┤
  │ 3. OPTIMICE UPDATE (Tuning de Recursos en Windows)               │ [⏳ PRÓXIMA FASE]
  │    • Afinado fino del uso de CPU/GPU en captura DXGI             │
  │    • Reducción al mínimo de overhead durante gameplay            │
  ├──────────────────────────────────────────────────────────────────┤
  │ 4. FRONTEND & UX UPDATE (Salto Visual en Slint & System Tray)    │
  │    • Rediseño de tema principal, componentes y comunidad         │
  │    • System Tray (Windows & Linux): Cierre a 2do plano sin cortar│
  │      el búfer, icono en bandeja y menú contextual rápido         │
  └──────────────────────────────────────────────────────────────────┘
```

---

## 1. Backend & Compressor Update (Limpieza, YAGNI y Eficiencia) — ✅ COMPLETADA (v0.7.3)

Esta fase se consolidó y certificó en la versión **v0.7.3**, estableciendo una base sólida de rendimiento, limpieza de dependencias obsoletas y solución definitiva a los problemas de ciclo de vida, compresión y concurrencia:

- **Eliminación de Código Muerto y Sobreingeniería (Doctrina Ponytail):**
  - **Poda de GStreamer:** Eliminadas más de 183 líneas de perfiles y pipelines muertos de GStreamer en [`src/profiles.rs`](file:///mnt/Files/Proyectos/sane-rec/src/profiles.rs) que habían quedado en desuso tras la consolidación de `wf-recorder` (Linux) y `ffmpeg` directo (Windows).
  - Reducción de superficie de mantenimiento y código más predecible.

- **Discord Compressor Inteligente y Eficiente ([`src/compressor.rs`](file:///mnt/Files/Proyectos/sane-rec/src/compressor.rs)):**
  - **Zero Clones Redundantes (Smart Bypass):** Verificación de tamaño previo (`metadata().len() <= target_bytes`). Si el clip original ya pesa menos o igual al tamaño objetivo configurado (ej. 15 MB), no se genera ningún archivo clon `_discord.mp4` redundante ni se duplica espacio en disco.
  - **Límite Estricto de Hilos CPU (`-threads 2`):** La transcodificación secundaria limita a 2 hilos el uso de CPU para eliminar caídas de FPS o microstuttering durante gameplay intenso en segundo plano.
  - **Manejo No Intrusivo de Avisos:** En caso de omisión o interrupción menor de compresión, se eliminaron las alertas críticas falsas (banners rojos), sustituyéndolas por avisos informativos discretos en stderr.

- **Estabilización de VA-API y Ciclo de Vida en Linux ([`src/engine.rs`](file:///mnt/Files/Proyectos/sane-rec/src/engine.rs)):**
  - **Nodo DRM Explícito (`-d /dev/dri/renderD128`):** Vinculación directa al nodo de renderizado por hardware, eliminando cuelgues del encoder en GPUs AMD Radeon (Polaris / RX 570) y sistemas multi-GPU.
  - **Framerate Constante (`-r <fps>`):** Inyección de tasa de cuadros fija en `wf-recorder` para que el codificador no quede suspendido esperando eventos de daño de pantalla (damage events) del compositor Wayland.
  - **Fallback no Destructivo:** En caso de falla de hardware, la degradación a `SaneSafe` (CPU) opera temporalmente en memoria para la sesión activa sin sobreescribir ni corromper de forma permanente el archivo `config.toml` del usuario.

- **Blindaje Multi-Shell contra Rebotes de Atajos (Anti-Chatter / Anti-Race Condition):**
  - **Diagnóstico y Solución de la Carrera Niri + Evdev:** Se resolvió la colisión donde Niri y el listener global del kernel competían al presionar `F9`, despachando dos eventos en microsegundos y cancelando grabaciones a los 0.01 segundos (archivo de 48 bytes).
  - **Guarda de Tiempo Mínimo (1200 ms):** Inyección de guarda de ejecución mínima de 1.2 segundos en `toggle_continuous_recording()` y `toggle_replay()`.
  - **Debounce de Finalización:** Se elevó el debounce IPC a 1200 ms calculado sobre la hora en que el proceso termina de inicializarse, aislando el comportamiento contra colisiones en Niri, Hyprland, Sway, KDE y GNOME.

- **Telemetría Forense y Benchmark Expansivo ([`src/benchmark.rs`](file:///mnt/Files/Proyectos/sane-rec/src/benchmark.rs)):**
  - Detección precisa de hardware gráfico (GPU y memoria VRAM leída desde sysfs en Linux).
  - Registro de la línea de comandos exacta ejecutada por el motor.
  - Eliminación del falso positivo de clip abortado en pruebas rápidas (<0.5s), reconociendo ahora clips válidos de corta duración con veredicto en verde.

---

## 2. Graphics Update (Aceleración por Hardware AMD, Intel & Nvidia en Windows) — ✅ COMPLETADA (v0.7.4 – v0.7.6)

Esta fase se consolidó entre **v0.7.4** y **v0.7.6**, unificando la aceleración por GPU dedicada en Windows, la sincronía de audio WASAPI y la resiliencia del búfer DXGI:

- **v0.7.4:** Mapeo de encoders hardware en Windows (`h264_amf`, `h264_nvenc`, `h264_qsv`), soporte HEVC `hvc1` con `hevc_mp4toannexb`, sincronía A/V con `aresample` y fallback no destructivo.
- **v0.7.5:** Hotfix de audio WASAPI (eliminación de `-use_wallclock_as_timestamps 1` y `first_pts=0`) + hot-swap y reconexión automática de dispositivos de audio a 1 Hz (`WinCapture` y `audio_monitor.rs`).
- **v0.7.6:** Compuerta de vídeo real MPEG-TS (`PID 0x100` PUSI) en `save_clip()`, watchdog con auto-reinicio no bloqueante y fallback automático a `GdiGrab` tras 2 stalls DXGI en 60s, `-stats_period 5` para conservar errores en telemetría, desacople de Discord Compressor en grabaciones continuas (`<10 MB` estricto) y normalización de códec al arranque.

Esta actualización unifica el soporte y optimización de aceleración por GPU dedicada en Windows, garantizando paridad de rendimiento tanto en arquitecturas AMD Radeon como en Nvidia GeForce.

### 2.1. AMD Radeon (Portación de Perfil VA-API Zero B-Frames a Windows)
- **Investigación y Paridad AMF / D3D11VA:**
  - Trasladar los flags de bajísima latencia y cero b-frames (`b_frames=0`) de VA-API Linux a los encoders AMF (`h264_amf` / `hevc_amf`) en Windows.
  - Diseñar el perfil optimizado para tarjetas AMD Radeon Polaris (como la RX 570) para lograr el mismo rendimiento fluido y bajo uso de CPU que se obtiene en Linux con `vaapi`.

### 2.2. Nvidia GeForce (NVENC Pipeline & Sincronía A/V)
- **Corrección del Desfase de Audio en Grabación Continua (`h264_nvenc`):**
  - **Diagnóstico empírico (Benchmark 18-11 19:20):** La codificación por GPU alcanza 60 FPS clavados con 0 pérdidas de cuadros, pero se percibe un retraso progresivo del audio respecto al video en gameplay intenso (Fortnite).
  - **Causa raíz:** La latencia ultrabaja del codificador NVENC (`-tune ull`, `-zerolatency 1`) avanza a mayor velocidad que el búfer TCP/WASAPI loopback.
  - **Solución técnica:** Alinear el reloj de captura de audio y video mediante compensación de drift en el filtro de remuestreo (`aresample=async=1000:min_hard_comp=0.100000:first_pts=0`) o estampación de timestamps con `-use_wallclock_as_timestamps 1`.
  - **Mapeo de Perfiles:** Corregir el selector en la UI para que la opción `nvidia-nvenc` configure directamente el encoder nativo `h264_nvenc` en lugar del fallback por software `libx264`.

### 2.3. Corrección Definitiva del Modo HEVC (`hevc_nvenc` / `hevc_amf`)
- **Solución al "Cuadro Congelado con Audio" en Repeticiones (Replay Ring Buffer):**
  - **Diagnóstico empírico (Benchmark 18-11 19:29 e imagen adjunta):** El búfer en RAM retiene los paquetes MPEG-TS y FFprobe detecta la pista de video HEVC a 60 FPS, pero al reproducir el archivo `.mp4` el reproductor muestra una imagen estática fija de la UI con solo audio sonando de fondo.
  - **Causa raíz:** A diferencia de H.264 (que usa `-bsf:v dump_extra`), en HEVC dentro de contenedores MPEG-TS faltan las cabeceras de secuencia in-band repetitivas (VPS, SPS, PPS). Al cortar microsegmentos en RAM, el remuxer `-c:v copy` hacia MP4 carece de los conjuntos de parámetros en cada IDR, provocando que los decodificadores de video (Windows Media Player, Movies & TV, MPV) no puedan inicializar la decodificación de los frames siguientes y se congelen en el primer frame.
  - **Solución técnica:**
    1. Forzar en el encoder HEVC de Windows la repetición periódica de cabeceras completas (`-forced-idr 1` en `hevc_nvenc`, y parámetros de IDR periódicos en `hevc_amf`).
    2. Incorporar el bitstream filter `-bsf:v hevc_mp4toannexb` (o `-bsf:v extract_extradata`) en la extracción y ensamblado del clip en `save_clip()`.

---

## 3. Optimice Update (Optimización de Rendimiento en Windows)
- **Afinado Fino de Captura DXGI:**
  - Optimizar el loop de lectura de paquetes MPEG-TS y la gestión de memoria en RAM para minimizar cualquier consumo residual de CPU y GPU.
  - Asegurar la menor latencia posible durante sesiones prolongadas de repetición instantánea (120s / 180s).
  - Reducción al mínimo de la contención de hilos durante gameplay competitivo.
- **Escalado y conversión Zero-Copy en GPU (`scale_d3d11` / D3D11 → NV12):** Evitar la descarga de frames `1080p BGRA` a RAM para superar el techo de ~36 FPS en iGPUs (Intel UHD 630 en laptops Optimus).
- **Mapeo explícito de `device_idx` / `output_idx` DXGI:** Selección explícita de adaptador DXGI en sistemas multi-GPU / Optimus para evitar pérdida de captura al alternar entre iGPU y dGPU.

---

## 4. Frontend & UX Update (Renovación Visual en Slint & System Tray Multiplataforma)

### 4.1. Renovación Visual en Slint
- **Modernización de la Interfaz:**
  - Rediseñar el tema visual principal, paletas de colores, contraste y tipografía.
  - Refinar el selector de fuentes, botones, switches y vistas de galería.
  - Compatibilidad pulida con temas personalizados de la comunidad y coherencia estética en Windows (DWM) y Linux (Wayland).

### 4.2. System Tray Multiplataforma (Cierre a Segundo Plano)
- **Comportamiento "Cerrar a la Bandeja" (Close-to-Tray):**
  - Interceptar el evento de cierre de ventana (`close-requested` de Slint) para que al pulsar el botón de cerrar la ventana principal se oculte (`hide()`) en lugar de matar el proceso.
  - El búfer de repetición instantánea en RAM física y la escucha de hotkeys globales (F9 / F10) continúan operando de forma transparente en segundo plano sin interrumpir la grabación del juego.
- **Implementación Multiplataforma Nativa:**
  - **Linux (Wayland & X11):** Integración con el estándar `StatusNotifierItem` (SNI sobre D-Bus `org.kde.StatusNotifierItem`), compatible de forma nativa con Niri (waybar / quickshell), Hyprland, KDE Plasma y GNOME (vía AppIndicator).
  - **Windows (Win32):** Integración mediante `Shell_NotifyIconW` con icono de aplicación `.ico` en el área de notificación (bandeja del sistema de la barra de tareas de Windows 10 y 11).
- **Menú Contextual e Interacción Rápida:**
  - **Clic izquierdo:** Restaurar y traer al frente la ventana principal de `sane-rec`.
  - **Clic derecho (Menú contextual):**
    - *Mostrar sane-rec*
    - *Guardar repetición (F10)*
    - *Iniciar / Detener grabación (F9)*
    - *Separador*
    - *Estado del búfer (ej. "Búfer: 45s activo")*
    - *Salir definitivamente* (cierre total y ordenado del proceso y liberación de recursos).
