# sane-rec

Lightweight Material Design 3 Instant Replay & Screen Recorder for Linux with CPU-first architecture.

*Grabador de pantalla y repetición instantánea ligero con Material Design 3 para Linux y arquitectura orientada a CPU.*

---

## English

### Overview
`sane-rec` is a minimalist, efficient desktop screen recorder and instant replay tool designed specifically for Linux (Wayland & X11). Built in Rust with Slint and GStreamer, it prioritizes predictable CPU usage, minimal system overhead, and a clean Material Design 3 interface.

### Features
- **Instant Replay:** Continuous rolling buffer in memory to save the last N minutes of gameplay or desktop activity on demand.
- **Screen Recording:** Standard recording to modern video formats with configurable codecs and bitrates.
- **CPU-First Architecture:** Optimized pipelines via GStreamer and PipeWire for low latency and minimal resource consumption.
- **Modern UI:** Responsive, native interface powered by Slint and Material Design 3 guidelines.

### Requirements
- Linux (Wayland with PipeWire recommended, or X11)
- Rust 1.75+
- GStreamer 1.20+ with base/good/bad plugins:
  - `gstreamer`
  - `gst-plugins-base`
  - `gst-plugins-good`
  - `gst-plugins-bad`
  - `pipewire`

### Building
```bash
cargo build --release
```

### Running
```bash
cargo run --release
```

---

## Español

### Descripción
`sane-rec` es una herramienta minimalista y eficiente de grabación de pantalla y repetición instantánea diseñada específicamente para Linux (Wayland y X11). Desarrollado en Rust con Slint y GStreamer, prioriza un consumo predecible y bajo de CPU, sobrecarga mínima del sistema y una interfaz limpia inspirada en Material Design 3.

### Características
- **Repetición Instantánea (Instant Replay):** Buffer circular en memoria para guardar los últimos N minutos de partida o actividad en el escritorio bajo demanda.
- **Grabación de Pantalla:** Grabación tradicional a formatos de vídeo modernos con códecs y tasas de bits configurables.
- **Arquitectura orientada a CPU:** Pipelines optimizados mediante GStreamer y PipeWire para baja latencia y mínimo impacto en el rendimiento.
- **Interfaz Moderna:** UI nativa y fluida construida con Slint y principios de Material Design 3.

### Requisitos
- Linux (Wayland con PipeWire recomendado, o X11)
- Rust 1.75+
- GStreamer 1.20+ con plugins base/good/bad:
  - `gstreamer`
  - `gst-plugins-base`
  - `gst-plugins-good`
  - `gst-plugins-bad`
  - `pipewire`

### Compilación
```bash
cargo build --release
```

### Ejecución
```bash
cargo run --release
```

---

## Licencia / License
MIT OR Apache-2.0
