# Hoja de Ruta (Roadmap) — sane-rec (Post v0.7.2)

Documento de planificación técnica para las próximas versiones de sane-rec.

```
                   HOJA DE RUTA SANE-REC (POST v0.7.2)
  ┌──────────────────────────────────────────────────────────────────┐
  │ 1. BACKEND & COMPRESSOR UPDATE (Base sólida, YAGNI & Eficiencia) │
  │    • Limpieza de código muerto, unificación de lógica redundante │
  │    • Optimización del compresor Discord (bypass si peso <= target│
  │      y perfiles de transcodificación de bajo impacto en CPU)     │
  ├──────────────────────────────────────────────────────────────────┤
  │ 2. GRAPHICS UPDATE (Aceleración Hardware AMD & Nvidia en Windows)│
  │    • AMD Radeon: Port del perfil VA-API Zero B-Frames (RX 570)   │
  │    • Nvidia NVENC: Corrección de desfase A/V (WASAPI vs DXGI)    │
  │    • HEVC Fix: Solución al cuadro congelado en repeticiones      │
  │      (inyección forzada de VPS/SPS/PPS y hevc_mp4toannexb)       │
  ├──────────────────────────────────────────────────────────────────┤
  │ 3. OPTIMICE UPDATE (Tuning de Recursos en Windows)               │
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

## 1. Backend & Compressor Update (Limpieza, YAGNI y Eficiencia)
- **Eliminación de Sobreingeniería y Código Redundante:**
  - Auditar módulos del motor para unificar helpers de video y audio que hoy duplican lógica entre Linux y Windows.
  - Eliminar estructuras o funciones heredadas de versiones tempranas que ya no se usan tras la estabilización de `ddagrab`.
- **Discord Compressor Inteligente:**
  - **Bypass por Umbral:** Si el clip original ya pesa menos o igual que el límite solicitado (ej. 8 MB o 25 MB), omitir por completo la recompresión y reutilizar el archivo existente sin consumir CPU ni demorar al usuario.
  - **Refactorización de Carga:** Limitar hilos (`-threads 2` o `-threads 4`) y emplear presets de transcodificación ligeros para evitar caídas de FPS o stuttering en juegos mientras se procesa la compresión en segundo plano.

---

## 2. Graphics Update (Aceleración por Hardware AMD & Nvidia en Windows)

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
