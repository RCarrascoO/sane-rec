# Historial de Cambios (Changelog) — sane-rec

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
