# sane-rec — Guía de Arquitectura, Instalación y Operación en Windows

Este documento detalla la arquitectura de ejecución, el empaquetado nativo y la guía técnica para la ejecución de **sane-rec** en entornos Microsoft Windows, con énfasis en optimización térmica de hardware crítico y despliegues optimizados.

---

## 1. Requisitos del Sistema y Compatibilidad

- **Sistemas Operativos Soportados:**
  - **Windows 10 (Versión 1903 o superior, 64-bit)**: Totalmente compatible con builds 19041, 19042, 19044 y **Windows 10 IoT Enterprise LTSC 2021 (Build 19045)** / distribuciones optimizadas (como KernelOS / ReviOS / AtlasOS).
  - **Windows 11 (todas las versiones, 64-bit)**: 21H2, 22H2, 23H2 y 24H2.
- **Procesador (CPU):** Procesador de 64 bits con soporte AVX/AVX2 (Intel Core i3/i5/i7/i9 4ª gen en adelante o AMD Ryzen).
- **Memoria RAM:** Mínimo 4 GB (se recomiendan 8 GB o 16 GB para búfer de replay circular holgado en RAM).
- **Tarjeta Gráfica:** Cualquier GPU con soporte DirectX 11 Feature Level 11_0 o superior (Intel HD/UHD Graphics, AMD Radeon, NVIDIA GeForce).
- **Subsistema de Audio:** Dispositivo compatible con WASAPI (Windows Audio Session API).

---

## 2. Arquitectura de Captura Nativa en Windows

`sane-rec` implementa en Windows un motor de captura optimizado para garantizar latencia cero y mínimo consumo de recursos del sistema:

1. **Captura de Pantalla:**
   - **Motor Primario (`ddagrab`):** Utiliza la API nativa **DirectX 11 Desktop Duplication API (DXGI)** mediante el filtro `ddagrab` de FFmpeg / GStreamer. Permite capturar el frame buffer del monitor directamente en memoria de video sin pasar por la CPU ni realizar costosos bloqueos de memoria.
   - **Fallback de Compatibilidad (`gdigrab`):** En entornos virtuales, sesiones RDP o hardware donde Desktop Duplication no esté expuesto, el motor conmuta automáticamente a `gdigrab` (GDI API) garantizando compatibilidad total.

2. **Captura y Mezcla de Audio:**
   - **Audio del Sistema (Loopback):** Captura mediante **WASAPI Loopback**, interceptando las muestras PCM de audio directamente de los altavoces/auriculares sin requerir software de cable virtual (VAC).
   - **Micrófono:** Captura simultánea vía WASAPI Input, procesada en pistas separadas o multiplexada en AAC estéreo con control de volumen independiente.

3. **Búfer de Instant Replay 100% en Memoria RAM:**
   - A diferencia de herramientas convencionales (como NVIDIA ShadowPlay o Radeon ReLive) que escriben constantemente segmentos temporales en el disco de almacenamiento degradando la vida útil del SSD, `sane-rec` almacena los últimos 30 a 120 segundos de video codificado en un **búfer circular en RAM**.
   - Cero lecturas/escrituras en SSD durante la sesión de juego.
   - La persistencia a disco (`.mp4`) se realiza únicamente al presionar el atajo de guardado de replay (por defecto `Mod+F10`), ensamblando los segmentos en milisegundos mediante concatenación directa de stream (`-c copy`).

---

## 3. Perfil Térmico Especial: "Pavilion Gaming - Celsius"

### Especificaciones de Hardware Objetivo
- **Equipo:** HP Pavilion Gaming 15-dk0xxx (Laptop gaming).
- **CPU:** Intel Core i5-9300H (4 núcleos / 8 hilos, TDP 45W).
- **dGPU:** NVIDIA GeForce GTX 1050 3GB GDDR5 (Turing/Pascal mobile, encapsulado térmico compartido).
- **iGPU:** Intel UHD Graphics 630.
- **RAM:** 16 GB DDR4 Dual-Channel.

### Problemática Térmica y Causa Raíz de BSODs
En las laptops HP Pavilion Gaming 15, la CPU y la dGPU NVIDIA comparten un sistema de disipación unificado (heatpipes compartidos con radiador dual). Cuando la GTX 1050 opera bajo carga intensiva en juegos y simultáneamente se activa el codificador por hardware `NVENC`, la temperatura del silicio y de las fases de alimentación (VRM / VRAM) supera con facilidad los **75°C–85°C**, alcanzando límites térmicos de protección que inducen caídas de voltaje (*thermal throttling*, cuelgues del controlador `nvlddmkm.sys` y **BSODs** / pantallazos azules).

### Solución Técnica Implementada en `Pavilion-Gaming-Celsius.toml`
El perfil térmico **Pavilion Gaming - Celsius** elimina de raíz la sobrecarga en la dGPU:
- **0% Carga en dGPU NVIDIA (Zero dGPU Load):** Se deshabilita por completo el codificador `NVENC`. La GTX 1050 queda 100% libre para el renderizado del juego sin estrés térmico adicional, manteniéndose por debajo de los **60°C–65°C**.
- **Codificación x264 Ultra-Ligera:** Codificación H.264 por CPU (`libx264` / `x264enc`) configurada con preset `ultrafast`, `crf = 25` y **limitación estricta a 2 hilos (`threads = 2`)** para evitar saturar los núcleos de la CPU.
- **Resolución y Cuadros:** 720p (1280x720) a 30 FPS. Proporciona una captura nítida y fluida para clips de jugadas y comunicación en Discord sin impacto perceptible en los FPS del juego.
- **Búfer en RAM de 350 MB:** Aprovecha los 16 GB de RAM del sistema para mantener 30 a 60 segundos de replay sin tocar VRAM ni SSD.

| Parámetro | Valor | Justificación Técnica |
|---|---|---|
| **Codec** | `libx264` (CPU) | Desconecta `NVENC`, previniendo sobrecalentamiento del encapsulado GTX 1050 |
| **Preset** | `ultrafast` | Algoritmos de compensación de movimiento de bajo cómputo |
| **CRF** | `25` | Balance óptimo entre compresión y nulo consumo de ciclos de reloj |
| **FPS** | `30` | Reduce a la mitad la tasa de cuadros respecto a 60 FPS (-50% uso CPU/térmico) |
| **Resolución** | `1280x720` | Escala a 720p reduciendo el ancho de banda de memoria |
| **Hilos (Threads)** | `2` | Restringe el codificador a 2 hilos lógicos del i5-9300H |
| **Búfer RAM** | `350 MB` | Almacenamiento circular en memoria principal sin desgaste de hardware |

---

## 4. Compilación Local y Generación del Instalador en Windows

### Requisitos Previos en Windows
1. **Rust Toolchain:**
   Instalar Rustup desde [https://rustup.rs](https://rustup.rs):
   ```cmd
   rustup default stable-x86_64-pc-windows-msvc
   ```
2. **Visual Studio C++ Build Tools:**
   Instalar la carga de trabajo "Desarrollo para el escritorio con C++" de Visual Studio 2022 o Build Tools.
3. **FFmpeg Essentials Build:**
   Descargar el release oficial estático de FFmpeg (por ejemplo, desde `gyan.dev` o `BtbN/FFmpeg-Builds`) y copiar `ffmpeg.exe` y `ffprobe.exe` en la carpeta `packaging\windows\`.
4. **Inno Setup 6:**
   Descargar e instalar Inno Setup 6 desde [https://jrsoftware.org/isdl.php](https://jrsoftware.org/isdl.php).

### Pasos de Compilación

1. **Compilar el binario optimizado:**
   ```cmd
   cargo build --release
   ```
   El ejecutable se generará en `target\release\sane-rec.exe`.

2. **Generar el Instalador con Inno Setup:**
   Ejecutar el compilador de Inno Setup apuntando al archivo `.iss`:
   ```cmd
   "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" packaging\windows\installer.iss
   ```
   El instalador autocontenido `sane-rec-setup-x64.exe` se creará en el directorio `packaging\windows\`.

3. **Modo Portable (.ZIP):**
   Para desplegar `sane-rec` sin necesidad de instalación:
   - Crear una carpeta `sane-rec-portable\`.
   - Copiar `target\release\sane-rec.exe`, `ffmpeg.exe`, `ffprobe.exe`, las carpetas `profiles\` y `assets\`.
   - Comprimir en formato `.zip`.
