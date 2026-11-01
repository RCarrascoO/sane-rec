# Historial de Cambios (Changelog) — sane-rec

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
