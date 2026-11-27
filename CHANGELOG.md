# Historial de Cambios (Changelog) — sane-rec

## v0.8.3 - 2026-11-27
- Windows DXGI watchdog: raised video stall threshold from 4s to 10s and required 3 stalls in 60s (instead of 2) before falling back to `gdigrab`, preventing premature 40 FPS lock on Alt-Tab/loading screens.
- UI idle GPU temperature: throttled Slint UI timer from 50ms (20 Hz) to 250ms (4 Hz) to avoid waking the GPU unnecessarily.

## v0.8.2 - 2026-11-24
- **Fix Switch Replay (`M3Switch`):** Enlace bidireccional (`checked <=> ...`), eliminación del parpadeo de estado y apagado instantáneo desde la GUI (manteniendo anti-chatter de 1200 ms en hotkeys).
- **Resiliencia Windows & Telemetría:** Watchdog DXGI activo con ventana minimizada, cooldown de 6 s en `win_video_stale()` (evita caída a `gdigrab` 40 FPS en RTX 3060), aplicación de volumen en `reload_config` y `Nivel de Perfil` visible en `.benchmark.txt`.

## v0.8.1 - 2026-11-23
- **Fix crítico Windows (`NVIDIA`/`AMD`):** Restaurado pipeline `hwdownload,format=bgra` de `v0.7.6` en `DdaGrab` y `pop_front` seguro en `win_ram_buffer`.
- **Fix sincronización Replay/UI (`status_epoch`):** Eliminado falso positivo/desync de `Daemon replay`, rollback en fallo de toggle, debounce post-operación de 1200 ms, persistencia del listener IPC de la GUI tras `stop` y límite IPC a 64 KB.

## v0.8.0 - 2026-11-23
- **Estabilización Backend P0/P1:** Deadlock AB-BA en Linux, `/dev/shm` aislado por UID, sesión atómica en Windows, volumen en vivo sin reinicio de búfer, UI Slint no bloqueante e IPC acotado.
- **Perfiles `Low / Medium / High`:** Preset `Low` por defecto manteniendo compatibilidad `v0.7.6` y VA-API Zero B-Frames (`-b 0`) en Polaris.
- **Fix RTX 3060:** Prevención de `win_video_stale` sin downgrade prematuro a `gdigrab` e integración de telemetría Replay en `benchmark.rs`.
- **DXGI Zero-Copy:** Optimización con `scale_d3d11=format=nv12`, auto-detección y poda por lotes en `win_ram_buffer`.

## v0.7.6 - 2026-11-21
- **Discord Compressor:** Desacople de grabaciones continuas (`stop_continuous_recording`) y margen estricto `<10 MB` (`0.88`, `-maxrate 1.05x`, `-bufsize 1.5x`).
- **Resiliencia DXGI Replay (Windows):** Compuerta de vídeo real MPEG-TS (`PID 0x100` PUSI) en `save_clip()`, detección de stalls con auto-reinicio no bloqueante y fallback automático a `GdiGrab` tras 2 stalls en 60s.
- **Telemetría y Perfiles:** `-stats_period 5` en FFmpeg Windows para conservar errores DXGI en el anillo de 16 KB, y normalización de `video.codec` al arranque según perfil activo.

## v0.7.5 - 2026-11-20 (0.7.5)
- **Hotfix Crítico de Audio en Windows (`v0.7.4` regression):** eliminados `-use_wallclock_as_timestamps 1` y `first_pts=0` del pipeline de audio TCP/WASAPI que provocaban silencio total y caída de velocidad a `0.475x` en Replay, manteniendo `-af aresample=async=1000` para sincronía A/V suave.
- **Hot-Swap y Reconexión Automática WASAPI (`cpal`):** supervisión a 1 Hz en `start_windows_audio_feeder` y monitor VU (`audio_monitor.rs`) que detecta desconexiones (`err_fn`) y cambios del dispositivo predeterminado de Windows (USB/Bluetooth/parlantes), reconstruyendo los streams en caliente sin cortar la grabación ni el búfer de repetición.
- **Soporte 24-bit/64-bit en Audio Windows:** soporte añadido para formatos `I32` y `F64` en `WindowsAudioResampler`.

## v0.7.4 - 2026-11-20 (0.7.4)
- **Sincronización de Códec con el Perfil**: Al seleccionar un perfil (UI o IPC `set_profile`) se ajusta automáticamente `video.codec` al encoder correspondiente (`libx264`, `h264_vaapi`/`h264_amf`, `h264_nvenc`, `h264_qsv`).
- **Intel QSV**: Pixel format `nv12` y baja latencia en el pipeline de codificación.
- **AMD AMF (Polaris)**: `lowlatency`, `nv12` y `header_insertion_mode gop` para evitar artefactos y recortes de GOP.
- **HEVC Replay**: Etiqueta `-tag:v hvc1` e inserción de cabeceras IDR para clips reproducibles en cualquier reproductor.
- **Sincronía A/V en Grabación Continua**: Corrección del drift entre NVENC y WASAPI en grabaciones largas.
- **Fallback No Destructivo en Windows**: El cambio automático a perfil seguro conserva la configuración previa y el clip en curso.
- **Telemetría Forense Real**: Métricas efectivas de pipeline en Replay y Grabación Continua.

## v0.7.3 - 2026-11-19 (0.7.3)
- **Estabilización VA-API (Linux)**: Vinculación explícita de nodo de renderizado `/dev/dri/renderD128` y framerate constante en `wf-recorder`.
- **Blindaje Multi-Shell contra Rebotes de Atajos**: Guarda defensiva de tiempo mínimo (1200ms) y debounce basado en finalización que previene cancelaciones prematuras en Niri, Hyprland y entornos Wayland/X11.
- **Smart Dual-Clip sin Duplicados**: Prevención de archivos clonados redundantes `_discord.mp4` cuando el clip original ya cumple con el tamaño objetivo, con límite estricto de 2 hilos CPU.
- **Telemetría y Benchmark Expandidos**: Detección de GPU, uso de VRAM DRM en Linux, comando ejecutado y eliminación de falsos positivos en clips cortos.

## v0.7.2 - 2026-11-18 (0.7.2)
- **Fixed (Windows App-Focus / Window Mode):**
  - **Eliminación del Zoom 800x450 en Modo Ventana / Splash Screens (Fortnite, Unreal Engine, EAC)**:
    - Clasificador y ranking de candidatos de ventana por superficie visible real en DWM (`width * height`), garantizando que la ventana principal de juego (1360x768 / 1080p) tenga prioridad sobre ventanas splash / launchers auxiliares de 800x450 con idéntico título.
    - Soporte directo de resolución por identificador nativo (`hwnd:0x...`, `HWND:0x...`, `0x...`) y desambiguación estricta de títulos puramente numéricos.
    - Filtrado robusto de ventanas de utilidad (`WS_EX_TOOLWINDOW`), minimizadas (`IsIconic`) y cloaked por el compositor DWM (`DWMWA_CLOAKED`).
  - **Deduplicación de Ventanas en la UI**:
    - Agrupación en `get_open_windows` por `(title, pid)` seleccionando el handle con mayor superficie de renderizado.
  - **Optimización de Telemetría de Benchmark y Pipeline de Recorte**:
    - Resolución de HWND en una única pasada en `target_details`, erradicando llamadas redundantes a `EnumWindows`.
    - Extracción de la aritmética euclidiana de recorte a `calculate_window_crop_rect` con clamping defensivo contra los bordes del monitor y suite de pruebas unitarias multiplataforma.

## v0.7.1 - 2026-11-17 (0.7.1)
- **Added:**
  - **Captura de Ventanas a 60 FPS con DXGI (DWM Bounds & Hardware Crop)**:
    - Superación del zoom y lag en modo ventana (App-Focus) mediante `DwmGetWindowAttribute` (`DWMWA_EXTENDED_FRAME_BOUNDS`) y detección automática del monitor contenedor con `MonitorFromWindow` / `GetMonitorInfoW`.
    - Captura por hardware DXGI (`ddagrab`) con filtro de recorte crop en GPU/SIMD (`-vf hwdownload,format=bgra,crop=w:h:x:y`), erradicando el descarte masivo de cuadros (1000+ dup frames de `gdigrab`).
    - Aislamiento de GDI exclusivamente como red de seguridad / fallback de emergencia.
  - **Suite de Benchmark y Telemetría Extendida (`.benchmark.txt`)**:
    - Registro de CLI exacta ejecutada, métricas de RAM física consumida por el búfer de repeticiones, topología y DPI de ventana, drift de sincronización A/V y detección automática de alertas de rendimiento.
- **Fixed:**
  - **Corrección de Mapeo y Detección de Monitores Dobles**:
    - Resolución de la inversión entre pantallas horizontales y verticales correlacionando `szDevice` / `DeviceName` y priorizando el monitor principal en el índice de captura.
  - **Escalado Dinámico del Búfer Circular en RAM (120s / 180s Replays)**:
    - Eliminación del corte prematuro a los 86s mediante cálculo adaptativo de `max_bytes` según la duración solicitada (hasta 1.5 GB de RAM para 120s @ 60 FPS).

## v0.7.0 - 2026-11-16 (0.7.0)
- **Added:**
  - **Fase 2 de Arquitectura de Grabación en Windows (60 FPS con DXGI Desktop Duplication)**:
    - Modernización del pipeline de captura mediante Desktop Duplication API (`ddagrab` / DXGI / D3D11), alcanzando 60 FPS estables sin cuellos de botella de software.
    - Descarga GPU a CPU optimizada con `-vf "hwdownload,format=bgra,..."` para alimentar `libx264` con apenas ~3.1% de ancho de banda PCIe.
    - Fallback dinámico automático a `gdigrab` y perfil seguro `SaneSafe` CPU ante fallas de compatibilidad D3D11 o RDP.
    - Soporte robusto de captura de ventanas específicas manteniendo composición DWM.
    - Hilos de codificación dinámicos para CPU según núcleos del procesador (`std::thread::available_parallelism()`).
  - **Modo Benchmark y Suite de Telemetría Activable (`.benchmark.txt`)**:
    - Botón de acción rápida en el Dashboard ("Benchmark: ON/OFF") y switch en Ajustes para auditar grabaciones y repeticiones.
    - Generación automática de informe forense `<video>.benchmark.txt` con FPS reales, pérdidas de cuadros (`drop`), cuadros duplicados (`dup`), velocidad del codificador (`speed`), estado de pistas A/V y veredicto de rendimiento ("PASS / EXCELENTE").
    - Notificación nativa de escritorio y telemetría de encoder capturada en tiempo real.

## v0.6.4 - 2026-11-16 (0.6.4)
- **Fixed (Windows):**
  - **Audio/Video Lag a 15 FPS eliminado**: el audio feeder de Windows dormía obligatoriamente ≥ 1 ms (2-4 ms reales en el scheduler de Win32) incluso cuando el adelanto era mínimo (< 20 ms), estrangulando la frecuencia de envío. Ahora usa `std::thread::yield_now()` cuando el adelanto es < 960 frames (< 20 ms) y solo duerme cuando hay un adelanto real (≥ 960 frames), sin el `max(1.0)` forzado.
  - **`hevc_amf` grababa solo audio**: `-bsf:v dump_extra` es exclusivo de H.264 y corrompe/elimina el video en MPEG-TS cuando el códec es HEVC/H.265 (`hevc_amf`, `libx265`). Ahora se aplica condicionalmente solo para codecs H.264.
  - **`hevc_amf` perfil Custom**: cambia `-usage ultralowlatency` a `-usage lowlatency` (evita fallo en drivers Adrenalin) y usa `-pix_fmt nv12` en lugar de `yuv420p` para mayor compatibilidad AMF HEVC.
  - **Stderr de grabación continua capturado**: el proceso FFmpeg en `start_record` ahora usa `Stdio::piped()` en lugar de `Stdio::null()` para stderr, con un hilo lector que almacena los últimos 16 KB en un ring buffer para crash reports.
  - **Zoom/Recorte en captura de ventana con DPI scaling**: `get_window_capture_rect` ahora escala las coordenadas lógicas a píxeles físicos usando `GetDpiForWindow` cuando DPI ≠ 96 (125%, 150%, etc.), corrigiendo el zoom centrado que afectaba a monitores HiDPI.

## v0.6.3 - 2026-11-16 (0.6.3)
- **Fixed:**
  - **Corrección Definitiva del Descarte de Cuadros y Stuttering de 12+ Segundos en Windows**:
    - Implementación de temporizador de alta precisión Win32 (`timeBeginPeriod(1)` de `winmm.dll`) con RAII estricto en el worker thread de audio feeder, reduciendo la granularidad de los sleeps del scheduler de Windows de 15.6 ms a 1 ms exacto.
    - Regulación estricta de ritmo de audio a 48 kHz basada en tiempo real (`elapsed` / `expected_frames`) que previene ráfagas de audio adelantadas hacia el socket TCP de FFmpeg y evita la saturación de la cola de entrelazado de video (`interleave_packet`).
    - Eliminación de la contención de cerraduras (lock contention) en `WindowsAudioResampler` mediante acumulación local por bloque (`staging`), reduciendo los bloqueos de Mutex de 48.000 por segundo a un único bloqueo por ráfaga WASAPI.
    - Purga activa de retraso de buffers acumulados por encima de 100 ms para mantener sincronización audiovisual perfecta en tiempo real.
    - Corrección de `-probesize` en `gdigrab` de `32K` a `32M`, garantizando análisis íntegro de fotogramas sin fallas de inicialización de códec.
  - **Remux Limpio de Clips MP4 en RAM sin Transcodificación**:
    - Sustitución de `-fflags +igndts` por `-avoid_negative_ts make_zero -fflags +genpts` en `save_clip`, erradicando el error `non monotonically increasing dts` y permitiendo remux ultrarrápido por copia directa (`-c:v copy`).
  - **Desacoplamiento No Bloqueante de Guardado en la Interfaz Slint**:
    - Ejecución asíncrona de `save_clip` desde `ui.on_save_clip` en hilo secundario con notificación por `invoke_from_event_loop`, eliminando congelamientos de la interfaz gráfica y el estado "(No responde)" de Windows al guardar repeticiones.
    - Extensión del timeout de lectura en IPC síncrono para la acción `save` de 5 s a 30 s para prevenir errores falsos de tiempo de espera al escribir clips voluminosos a disco.
  - **Sincronización de Versión en Componentes Slint y Tests**:
    - Sincronización de propiedades `app-version` en `app_window.slint` y `components.slint` con `Cargo.toml` (`v0.6.3`), asegurando el paso del 100% de la suite de pruebas.

## v0.6.2 - 2026-11-13 (0.6.2)
- **Fixed:**
  - **Eliminación Definitiva del Stuttering y Descarte de Cuadros a los 12-13s en Windows**:
    - Feeder de audio TCP sincronizado estrictamente al reloj de pared (`expected_frames`) con despacho inmediato (catch-up mode), eliminando la deriva acumulativa del scheduler Win32 y la subalimentación del pipeline de audio.
    - Purgado preventivo de muestras acumuladas en colas WASAPI durante la inicialización de FFmpeg.
    - Expansión de la cola de entrada de vídeo `-thread_queue_size` a `2048` fotogramas en FFmpeg (`gdigrab`), garantizando un margen de retención holgado ante variaciones temporales.
    - Aplicación estricta de `-r <fps>` con `-fps_mode cfr` en los perfiles de codificación de Windows para asegurar fluidez perfecta y constante a 60 FPS.
  - **Ampliación de Memoria Predeterminada del Replay Buffer**: incremento del búfer base en RAM de `150 MB` a `500 MB` en `Config::default()` para soportar repeticiones instantáneas en 1080p60 a altas tasas de bits sin descarte prematuro de segmentos.
  - **Normalización de Rutas en Concat Demuxer**: conversión de separadores de ruta `\` a `/` en la lista temporal de segmentos para evitar errores de escape de caracteres en FFmpeg sobre Windows.
  - **Protección contra Cuelgues DWM en Ventanas Minimizadas**: detección de ventanas minimizadas con `IsIconic` en `get_window_capture_rect` para evitar coordenadas de recorte espurias `(-32000, -32000)`.
  - **Compatibilidad de Portapapeles con PowerShell 5.1**: cambio de parámetro en comando `Set-Clipboard` de `-Path` a `-Value` para compatibilidad transparente en todas las versiones de Windows 10/11 sin requerir PowerShell Core.

## v0.6.1 - 2026-11-10 (0.6.1)
- **Added:**
  - **Tema Fortnite Edition** (`#091128` Lobby oscuro, amarillo vibrante `#F7E600` con borde superior blanco de `2px`, cian `#00D2FF` y cortes angulares `4px`), con migración transparente desde configuraciones previas `liquid-glass`.
  - **Soporte Nativo de Botones Laterales del Mouse (`Mouse4` / `Mouse5` / `MouseMiddle`)**:
    - Captura global en Windows mediante muestreo a `20 ms` con `GetAsyncKeyState` (compatible con juegos en pantalla completa exclusiva).
    - Listener global nativo `evdev` en Linux leyendo `/dev/input/event*` (`BTN_SIDE`, `BTN_EXTRA`, `BTN_MIDDLE` y teclado) con soporte de modificadores cruzados teclado+ratón.
    - Chips dedicados por sistema operativo, captura interactiva de botones de ratón en las cajas de atajos de la UI y protección anti doble disparo IPC (`400 ms`).
  - **Auto-inyección KDL en Niri y Soporte en Caliente para Hyprland**: inserción automática dentro de `binds { ... }` en `~/.config/niri/config.d/70-binds.kdl` (`MouseBack` / `MouseForward`) y registro/desregistro dinámico en Hyprland vía `hyprctl keyword bind` (`mouse:275` / `mouse:276`).
  - **Insignia de Versión en la Barra Lateral**: indicador dinámico `app-version` en el Navigation Rail y anclaje fijo del cuadro de estado del backend al borde inferior de la ventana.
- **Fixed:**
  - **Texturas Tileables 1:1 en Tema Minecraft**: texturas `64x64` pixel-art (`mc_dirt_bg.png`, `mc_stone_card.png`, `mc_cobble_btn.png`, `mc_emerald_btn.png`) repetidas en mosaico (`horizontal-tiling`/`vertical-tiling: repeat` + `image-rendering: pixelated`) sin estiramientos ni deformaciones, más padding adicional en la tarjeta superior del panel.
  - **Garbage Collector Circular en `/dev/shm` para Linux**: limpieza previa de instancias huérfanas de `wf-recorder` / segmentos residuales e hilo `sane-rec-shm-gc` cada `2s` manteniendo `duration_secs + 8` segmentos.
  - **Pipeline Zero-Lag y Captura Stretched en Windows**: reducción de `-probesize` en `gdigrab` a `32K` (`-analyzeduration 0 -fflags +nobuffer+flush_packets`), ajustes de baja latencia en `h264_amf`, `h264_nvenc` y `libx264`, reemplazo de `GetWindowRect` por `GetClientRect` + `ClientToScreen` con truncamiento a dimensiones pares (ej. `1024x664` sin bordes DWM), y sustitución de fallbacks fijos por `GetSystemMetrics`.

## v0.6.0 - 2026-11-07 (0.6.0)
- **Added:**
  - Motor de Temas Reactivo en vivo (`Sane Dark`, `Sane Light`, `OLED Pitch Black`, `Minecraft Edition` con texturas pixel-art y bisel 3D, `Liquid Glass iOS` con refracción especular y orbes de luz) + soporte de Temas de la Comunidad en `.toml` (`~/.config/sane-rec/themes/` y `%APPDATA%\sane-rec\themes\`).
  - Arquitectura ergonómica de interfaz con submenús desplegables (`M3CollapsibleCard`).
  - Arranque instantáneo (`<100ms`) con descubrimiento de hardware, pantallas, audio y cálculo de duraciones de galería (`ffprobe`) asíncronos en segundo plano con caché en memoria.
  - Sincronización caliente no bloqueante entre GUI y Daemon (`reload_config`) y limpieza garantizada de procesos de captura (`Drop` anti-zombies).
- **Fixed:**
  - Desacople total del compresor de Discord de las grabaciones continuas normales (`Record`), limitándolo exclusivamente a clips del Replay Buffer con modo Dual Clip o compresión manual.
  - Reparación del Daemon e IPC con timeouts estrictos de lectura/escritura/conexión en Windows y Linux.
  - Eliminación del bloqueo del Message Loop Win32 (`WM_HOTKEY`) mediante despacho asíncrono con debounce de 350ms y prevención de doble registro de atajos entre Daemon y GUI.
  - Reparación del selector de carpetas en Windows con hilos STA desacoplados y candado atómico anti-reentrada.
  - Eliminación de escritura intermedia en `%TEMP%` al guardar clips en Windows (streaming directo a `ffmpeg stdin` desde RAM alineado al primer paquete IDR/PUSI de 188 bytes).
  - Cambio de segmentos del Replay Buffer en Linux de MP4 a MPEG-TS (`.ts`) con cabeceras PAT/PMT en cada fotograma, eliminando definitivamente el fallo `moov atom not found`.
  - Eliminación del `DAR 16:9` forzado en el filtro de escalado de Windows que deformaba monitores 16:10, 4:3, 5:4 y 21:9.
  - Eliminación de rutas y perfiles hardcodeados del autor y filtrado de perfiles de hardware por sistema operativo.
  - Corrección de cortes visuales en monitores compactos mediante `ScrollView` en la lista de perfiles y `Flickable` en el Navigation Rail y filas de presets.

## [0.5.7] - 2026-11-04 (v0.5.7)
- **Estirado Automático 16:9 para Resoluciones Competitivas (`setdar=16/9`)**: Soporte nativo para resoluciones estiradas (Stretched Res) 4:3 o 5:4 (ej. `1440x1080`, `1280x960`, `1080x1080`, `1600x1080`) utilizadas competitivamente en Fortnite, Counter-Strike 2 y Valorant. El filtro de video aplica `setdar='if(gte(iw,ih),16/9,9/16)'` asegurando que el clip MP4 se guarde y reproduzca a pantalla completa en 16:9 tal como el jugador lo ve en su monitor físico, eliminando barras negras laterales y evitando clips comprimidos en 4:3.
- **Captura y Codificación CFR a 60 FPS Constantes sin Límite de Hilos**: Eliminada la restricción hardcodeada de 2 hilos en `libx264` que estrangulaba la CPU y provocaba descarte de hasta el 70% de los cuadros durante partidas exigentes. Implementación de `-vsync cfr` y escalado ultraligero `flags=fast_bilinear` sin sobrecarga por frame en el hilo de captura de Windows (`gdigrab`), garantizando clips perfectamente fluidos a 60 FPS constantes.
- **Selector de FPS en Vivo y Selección de Pantalla Nativa al 100%**: Conexión bidireccional en tiempo real del selector de 30/60 FPS en la interfaz gráfica que persiste la configuración y reinicia el búfer de repetición al instante. Al seleccionar monitores en la lista de Fuentes se preserva la resolución nativa al 100% (`scale = "none"`), evitando filtros de reescalado innecesarios.

## [0.5.6] - 2026-11-03 (v0.5.6)
- **Alineación de Paquetes MPEG-TS de 188 Bytes, `dump_extra` SPS/PPS y Guardado Atómico**: El hilo lector del búfer en RAM alinea estrictamente cada lectura al byte de sincronización `0x47` y a múltiplos de 188 bytes, eliminando la desincronización de contenedor al podar la cola. En el stream MPEG-TS se inyectan `-bsf:v dump_extra` y `-mpegts_flags +resend_headers+pat_pmt_at_frames` para que todos los keyframes lleven cabeceras SPS/PPS/VPS. El guardado de clips ahora utiliza un archivo staging `.ts` seguro con resincronización de audio AAC (`aresample=async=1:first_pts=0 -avoid_negative_ts make_zero`) y fallback automático a recodificación ultrarrápida si el remux directo detecta paquetes corruptos o cambios dinámicos de resolución.
- **Compatibilidad Universal de Reproducción de Video en Windows y Discord**: Resuelto de raíz el error `Error al guardar MP4 desde el buffer MPEG-TS en RAM` y el problema de clips con solo audio y sin video. Reproductores nativos como Windows Media Player, Películas y TV, Discord embebido y navegadores reproducen instantáneamente el video y audio sincronizados desde el segundo `0.00`.
- **Captura DWM por Geometría de Ventana para Juegos DirectX 11/12 y Resoluciones Estiradas**: Selección de ventanas en Windows migrada a captura a través del compositor DWM (`-i desktop` con `-offset_x/y` y `-video_size` exactos calculados dinámicamente mediante `get_window_capture_rect`). Esto permite capturar juegos DirectX 11, DirectX 12, Vulkan y OpenGL (Fortnite, Valorant, Counter-Strike 2, Minecraft, etc.) sin pantalla negra. En configuraciones de un solo monitor no se fija `-video_size`, adaptándose en vivo a cambios de modo como resoluciones estiradas (`1440x1080`, `1280x960`), junto con el filtro `scale=trunc(iw/2)*2:trunc(ih/2)*2` para garantizar dimensiones pares.
- **Optimización Zero-Lag en Windows con `ultrafast` + `zerolatency` + `0 B-Frames`**: Perfil `SaneSafe` optimizado con `-preset ultrafast -tune zerolatency -bf 0`, reduciendo más del 45% del uso de CPU en partidas exigentes y eliminando la latencia de codificación. Perfil `NvidiaHardware` ajustado a `p2` de ultra-baja latencia con 0 B-frames y demuxer de `gdigrab` acelerado con `-probesize 32M -analyzeduration 0 -fflags nobuffer`.

## [0.5.5] - 2026-11-02 (v0.5.5)
- **Eliminación del Monitor Hardcodeado `DVI-D-1` y Auto-Detección Inteligente en Wayland**: Eliminada por completo la referencia fija a `DVI-D-1`. El motor ahora auto-detecta dinámicamente el monitor enfocado o activo en el primer arranque para cualquier compositor Wayland (`wf-recorder`), seleccionando la salida correcta (`eDP-1`, `DP-1`, `HDMI-A-1`) y su resolución nativa. Si la configuración hace referencia a un monitor que ya no está conectado, recurre fluidamente al monitor activo sin fallar.
- **Soporte Nativo para Hyprland (`hyprctl monitors -j`)**: Integración directa con el comando IPC de Hyprland para consultar el array completo de pantallas conectadas, resoluciones nativas, geometría y tasas de refresco con detección precisa del monitor con foco antes del fallback a `wf-recorder -L`.
- **Ruta de Guardado Estándar Multi-Usuario Portable**: Cuando `/mnt/Files/Videos/Replays` no existe en el sistema, sane-rec conmuta automáticamente a la ruta estándar de videos del usuario (`~/Videos/SaneRec`) usando directivas XDG y `UserDirs`, previniendo errores de permisos o rutas inexistentes en nuevos equipos.

## v0.5.4 - 2026-11-01
- **Sincronización Automática de Atajos Globales al Inicio (`niri` / `RegisterHotKey`)**: Sincronización y registro inmediato de los atajos configurados (`shortcuts.save_replay` y `shortcuts.toggle_record`) al iniciar tanto en modo daemon (`sane-rec daemon`) como en la interfaz gráfica, eliminando la necesidad de alternar manualmente entre chips de atajo para activarlos.
- **Auto-Recuperación y Reintento Resiliente en Windows**: Verificación del código de retorno en `RegisterHotKey` con bucle de auto-reintento periódico cada 500ms (`WM_TIMER`) para registrar automáticamente las teclas globales si ocurre una colisión temporal durante el arranque rápido, auto-actualización o cierre del proceso previo.

## v0.5.3 - 2026-11-01
- **Captura Individual Multi-Monitor en Windows (`gdigrab` offset & geometry)**: Selección exacta del monitor elegido (`\\.\DISPLAY1`, `\\.\DISPLAY2`, etc.) leyendo coordenadas `dmPosition.x/y` y resolución `dmPelsWidth/Height` vía Win32 `EnumDisplaySettingsExW`, evitando grabar ambas pantallas combinadas.
- **Fix de Aceleración GPU AMD AMF (`h264_amf` / `hevc_amf`) en Windows y Soporte HEVC GPU**: Eliminados los flags incompatibles `-bf 0` y `-usage ultralowlatency` en el perfil personalizado de Windows que causaban que `h264_amf` fallara al iniciar y cayera silenciosamente a CPU (`libx264`). Añadido soporte nativo para `hevc_amf`, `hevc_vaapi` y `hevc_nvenc` (H.265 acelerado por GPU).
- **Preservación de Monitor y Etiquetas Nativas Windows**: El fallback de seguridad conserva el monitor elegido y la interfaz adapta las etiquetas (`Windows Display`, `RAM Ring`, `Hardware GPU`) según el sistema operativo.

## v0.5.2
- **Aceleración por Hardware AMD Polaris VA-API Zero B-Frames (`-b 0`)**: Grabación fluida por GPU (`h264_vaapi`) a 60 FPS con solo ~3% de uso de CPU y 0 cuelgues del kernel (`ring vce0 timeout`) en tarjetas AMD Radeon RX 470/480/570/580 (Polaris 10/20 / VCE 3.4) y RDNA en Linux Wayland.
- **Perfil Predefinido `Polaris-VAAPI-ZeroBFrames`**: Nuevo perfil optimizado listo para usar en el gestor de perfiles de equipo.
- **Parámetros Adaptativos de Códec y Calidad en Tiempo Real**: Controles dinámicos de velocidad y calidad adaptados al códec seleccionado (Modo VCE Directo en VA-API, NVIDIA NVENC p1–p4, AMD AMF speed/balanced/quality, Intel QSV TU y VP9 Realtime cpu-used). Mapeo automático de calidad QP en GPU vs CRF en CPU.
- **Rejilla 2x2 Responsiva para Códecs por GPU**: Reorganización de la fila de códecs acelerados con prioridad a `h264_vaapi` y prevención de desbordes en ventanas estrechas.
- **División en 2 Filas del Límite de Memoria RAM**: Adaptación limpia de los chips de memoria en ventanas verticales o compactas.
- **Prevención de Desbordes en Chips de Perfiles**: Ajuste de ancho mínimo y elisión automática de texto en chips de la interfaz para tarjetas y ventanas reducidas.

## v0.5.1
- **Diseño Responsivo Compacto**: Adaptación fluida para anchos reducidos (~600–720px) con botones de acción compactos e iconos limpios en tarjetas de clips, evitando recortes en los botones 'Carpeta' y 'Borrar'.
- **Prevención de Desbordes Horizontales**: Ajuste y elisión en títulos y descripciones largas en perfiles de hardware ('Intel Hardware QuickSync') y en el gestor de perfiles de equipo.
- **Scroll Preciso**: Corrección del cálculo de altura en párrafos multilínea dentro de `ScrollView`, garantizando desplazamiento fluido hasta el final del contenido en la Guía y registro de novedades.
- **Renderizado Nítido con Skia**: Activación del backend Skia en Linux Wayland para textos e iconos con nitidez y antialiasing de alta definición.
- **Cabeceras Adaptables**: Elisión y botones compactos en las cabeceras de la Galería y el Panel de Control.

## v0.5.0
- **Auto-Actualizador Nativo en Windows y Linux**: Soporte completo de actualización en 1 clic tanto en paquetes `.zip` portables de Windows (reemplazo en caliente de `sane-rec.exe`) como instaladores `.exe` (`/SILENT`) y tarballs `.tar.gz` de Linux.
- **Visor de Novedades Integrado**: Visualización de notas de la versión (`CHANGELOG.md`) directamente en la pestaña Sistema.
- **Captura sin pérdida de foco en Minecraft Java / OpenGL (Windows)**: Se desactivaron los toasts WinRT invasivos al iniciar grabación o guardar clip y se optimizó `gdigrab` hacia el compositor DWM (`desktop`) para evitar la apertura automática del menú de pausa (`ESC`).
- **Verificación de Salud y Fallback Automático en Windows**: Comprobación de arranque de `ffmpeg` (350ms) en el búfer de repetición y grabación continua con caída automática a `SaneSafe` (`libx264` CPU) si el codificador por hardware GPU falla.
- **Corrección de Atajos Globales en Windows**: Eliminación de registros fantasma `F9`/`F10` que bloqueaban el cambio de teclas, soporte extendido de teclas (`F13`–`F24`, `Insert`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `Num0`–`Num9`) y prevención de colisiones.
- **Gestión de Clips Recientes**: Soporte para eliminar clips `.mp4` y `.mkv` de forma segura desde la interfaz y detección del título real de ventanas en Windows.

## v0.4.1
- **Estabilidad de Audio WASAPI en Windows**: Resampler lineal a 48 kHz estéreo con mezcla en tiempo real de loopback del sistema y micrófono vía stream TCP local.
- **Ocultación de Ventanas de Consola**: Ejecución silenciosa (`CREATE_NO_WINDOW`) en todos los subprocesos de `ffmpeg`, `ffprobe` y `curl` en Windows.
- **Instalador Inno Setup 6 y Portable ZIP**: Empaquetado automatizado en GitHub Actions con `ffmpeg.exe` estático incluido.

## v0.4.0
- **Soporte Multiplataforma (Windows y Linux)**: Portabilidad completa del motor de grabación, búfer circular en RAM física para Windows y descubrimiento de pantallas (`EnumDisplayDevicesW`), ventanas (`EnumWindows`) y dispositivos WASAPI (`cpal`).
- **Compresor Automático para Discord (Doble Clip)**: Generación en segundo plano de copias optimizadas (`_discord.mp4`) ajustadas al límite de tamaño configurado.
- **Monitor de Picos de Audio en Tiempo Real**: Medidores VU en vivo para el audio del escritorio y micrófono.

## v0.3.0
- **Interfaz Material Design 3 en Slint**: Rediseño integral con barra de navegación lateral, tarjetas de estado y selector visual de fuentes de captura.
- **Mezclador Dual PipeWire / PulseAudio**: Aislamiento de nodos virtuales (`sane_rec_mix`) con control independiente de ganancia sin alterar el volumen físico del micrófono ni interferir con EasyEffects.
- **Perfiles de Hardware y CPU (`SaneSafe`)**: Selección inteligente de codificadores (`libx264`, `VA-API`, `NVENC`, `AMF`, `QSV`) y poda estricta del búfer en `/dev/shm`.
