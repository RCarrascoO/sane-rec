# CLAUDE.md — Instructions for Claude Code & AI Assistants

This file documents conventions, architecture, build instructions, and invariants for `sane-rec`.

---

## 🛠️ Build & Test Commands

```bash
# Run all unit and integration tests
cargo test

# Build debug binary
cargo build

# Build optimized release binary
cargo build --release

# Local installation script (Linux)
./install.sh

# Run CLI commands directly
cargo run --bin sane-rec -- status
cargo run --bin sane-rec -- save
cargo run --bin sane-rec -- record
cargo run --bin sane-rec -- update
```

---

## 🎯 Architecture & Tech Stack

- **Language:** Rust 2021 (Rust 1.75+)
- **UI Framework:** Slint 1.8 (`slint-build`, `ui/*.slint`)
- **Async Runtime:** Tokio 1.40 (`tokio`)
- **Platform Matrix:**
  - **Linux (Wayland):** `wf-recorder`, `ffmpeg`, PipeWire/PulseAudio (`pactl`), `/dev/shm` shared RAM buffer, Unix Domain Sockets.
  - **Windows 10/11:** `ffmpeg` (`gdigrab`), `cpal` 0.15 (WASAPI loopback & mic capture with linear 48 kHz stereo resampler), `windows-sys` 0.59 (`RegisterHotKeyW`, `EnumWindows`, `EnumDisplayDevicesW`), anonymous in-memory `VecDeque` RAM buffer.

---

## 📁 Module Map

| File | Purpose |
| :--- | :--- |
| `src/main.rs` | App entry point, CLI parser (`clap`), IPC command dispatcher, Slint UI event loop binding. |
| `src/engine.rs` | Core recording engine (`ReplayEngine`), ring buffer management, PipeWire/WASAPI audio pipeline, FFmpeg concatenation demuxer, health-probe auto-fallback. |
| `src/discovery.rs` | Display outputs, open windows, audio devices, saved clips listing & deletion (`delete_clip`), `new_hidden_command` helper. |
| `src/audio_monitor.rs` | Asynchronous real-time audio VU peak/RMS meter emitting live levels to Slint UI. |
| `src/compressor.rs` | Smart Discord compression (<10 MB), adaptive downscaling (`1080p` → `720p` → `480p`), clipboard copy (`wl-copy` / PowerShell `Set-Clipboard`). |
| `src/ipc.rs` | IPC server/client protocols; Windows global hotkey loop (`RegisterHotKeyW`) and key parser. |
| `src/profiles.rs` | Encoder profiles (`SaneSafe` CPU, `VaapiAmd`, `NvencNvidia`, `QsvIntel`, `AmfAmd`) and FFmpeg CLI builder. |
| `src/profiles_manager.rs`| Hardware preset manager (TOML files in `profiles/`), security path traversal protection, laptop presets. |
| `src/updater.rs` | GitHub Releases update checker, atomic updater (Linux `.tar.gz`, Windows `.zip` portable & `.exe` installer), changelog viewer. |
| `src/config.rs` | Persistent app configuration (`config.toml`), Niri Wayland shortcut injector. |
| `src/crash_handler.rs` | Panic hooks and crash report logging to disk. |
| `ui/theme.slint` | Material Design 3 Expressive tokens, Dark Baseline palette (`#141218`). |
| `ui/components.slint` | Reusable M3 widgets: `M3Button`, `M3Card`, `M3Toggle`, `M3Slider`, `M3Icon` (pure vector SVG paths). |
| `ui/app_window.slint` | Main window, NavigationRail, Dashboard, Replays, Settings, Guide & Changelog, Floating Compact Mode. |

---

## ⚠️ Critical Architectural Invariants

1. **Silent Windows Processes:**
   - External processes (`ffmpeg`, `ffprobe`, `curl`, `powershell`) on Windows **must** be created with `discovery::new_hidden_command(...)` or `.creation_flags(0x08000000)` (`CREATE_NO_WINDOW`). No console windows may ever pop up.
2. **Non-blocking UI Thread:**
   - Slint event loop must never be blocked with I/O, subprocesses, network calls, or audio initialization.
   - Run heavy tasks via `std::thread::spawn` or Tokio and update the UI with `slint::invoke_from_event_loop(...)`.
3. **Gaming & RTSS Compatibility:**
   - Do not run continuous high-frequency redraw timers in Slint when the window is hidden/minimized.
   - Screen capture in Windows must not steal keyboard focus or trigger game pause menus (e.g. Minecraft Java `ESC`).
4. **UI Design Standards:**
   - Material Design 3 Dark Baseline palette only.
   - Zero card borders (`border-width: 0px`).
   - Sentence case for all Spanish strings ("Guardar repetición", not "GUARDAR REPETICIÓN").
   - Integer pixel alignment using `Math.floor(...) * 1px`.
   - Never use emoji unicode characters in buttons or toasts; use `M3Icon` vector SVG paths.
5. **Cross-Platform Separation:**
   - Guard platform code strictly with `#[cfg(target_os = "windows")]` and `#[cfg(not(target_os = "windows"))]`.
