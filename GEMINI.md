# GEMINI.md — AI Context & Architecture Guide for sane-rec

## Project Overview

**sane-rec** is a lightweight, cross-platform instant replay buffer and screen recorder designed for **Linux (Wayland / PipeWire)** and **Windows 10/11 (DXGI / GDI / WASAPI)**.
Its primary architectural philosophy is **CPU-First & Thermal Stability**:
1. **Zero SSD Wear:** Rolling replay buffer resides in physical RAM (`/dev/shm` on Linux, circular `VecDeque` on Windows). Clips are written to storage only on user request.
2. **Thermal & Driver Stability:** Prioritizes robust CPU encoding (`libx264` zerolatency in `Sane Safe` profile) to eliminate GPU driver hangs on legacy Linux GPUs (AMD Polaris) and 0% dGPU thermal load on Windows gaming laptops (e.g., HP Pavilion, ASUS TUF), while offering full hardware encoding (`NVENC`, `AMF`, `QuickSync`, `VA-API`).
3. **Smart Discord Compression (<10 MB):** Background dual-clip generation with dynamic bitrate & adaptive downscaling (`1080p` → `720p` → `480p`) plus automatic clipboard copying (`wl-copy` / `Set-Clipboard`).
4. **Material Design 3 Expressive UI:** Built natively with Slint 1.8, adhering strictly to M3 guidelines.

---

## Tech Stack & Dependencies

- **Language:** Rust (2021 Edition, Rust 1.75+)
- **UI Framework:** Slint 1.8 (`slint`, `slint-build`)
- **Async Runtime:** Tokio 1.40 (`tokio` with `full` features)
- **Serialization:** Serde 1.0, `serde_json`, `toml` 0.8
- **CLI & Commands:** Clap 4.5 (derive)
- **Notifications:** `notify-rust` 4.11
- **Platform-Specific:**
  - **Linux:** Unix Domain Sockets, `libc`, `wf-recorder`, `ffmpeg`, `pactl` (PipeWire/PulseAudio)
  - **Windows:** `cpal` 0.15 (WASAPI audio capture & loopback), `windows-sys` 0.59 (`Win32_Foundation`, `Win32_UI_WindowsAndMessaging`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_Graphics_Gdi`), `winresource` 0.1

---

## Architecture & Module Map

### 1. Core Rust Modules (`src/`)

- [`src/main.rs`](file:///mnt/Files/Proyectos/sane-rec/src/main.rs):
  - Application entry point, CLI argument parsing (`clap`), IPC command dispatching (`save`, `record`, `status`, `update`, `daemon`), Slint UI bootstrap and event loop setup.
  - Wires UI callbacks to backend worker threads (`invoke_from_event_loop`).
- [`src/engine.rs`](file:///mnt/Files/Proyectos/sane-rec/src/engine.rs):
  - Core recording and replay buffer engine (`ReplayEngine`).
  - **Linux:** Manages `wf-recorder` subprocesses with segmented ring-buffer in `/dev/shm/sane-rec-$UID/`. Manages PipeWire audio module links.
  - **Windows:** Launches silent `ffmpeg` capture (`gdigrab` on Desktop / screen capture) streaming into a circular in-memory `VecDeque` buffer. WASAPI audio loopback + microphone capture via `cpal` with real-time linear resampler to 48 kHz stereo piped over local TCP (127.0.0.1).
  - Health checks: 350ms health-probe on start with automatic fallback to `SaneSafe` (`libx264`) if hardware GPU encoder crashes.
  - Concat demuxer: Stitches segments into finalized `.mp4` or `.mkv` clips.
- [`src/discovery.rs`](file:///mnt/Files/Proyectos/sane-rec/src/discovery.rs):
  - Display and window enumeration:
    - Linux: Queries `wf-recorder -o \?` and compositor IPC (e.g. `niri msg outputs`, `swaymsg`).
    - Windows: Uses `EnumDisplayDevicesW` and `EnumWindows` with UTF-16 title decoding (`GetWindowTextW`).
  - Audio device enumeration:
    - Linux: Parses `pactl list sources` and `pactl list sinks`.
    - Windows: Uses `cpal::default_host().devices()`.
  - Clip discovery & management: Lists saved clips, extracts metadata, formats durations, and securely deletes clips and companion `_discord.mp4` files via `delete_clip`.
  - Process creation: Provides `new_hidden_command` for silent execution on Windows (`CREATE_NO_WINDOW = 0x08000000`).
- [`src/audio_monitor.rs`](file:///mnt/Files/Proyectos/sane-rec/src/audio_monitor.rs):
  - Real-time audio VU peak monitoring for desktop loopback and microphone inputs.
  - Computes RMS and Peak decibel levels and notifies UI asynchronously.
- [`src/compressor.rs`](file:///mnt/Files/Proyectos/sane-rec/src/compressor.rs):
  - Asynchronous Discord compression pipeline (`spawn_discord_compression`).
  - Calculates target bitrate for a target size (default 9.5 MB to fit non-Nitro 10 MB limit).
  - Applies adaptive resolution downscaling (`1080p` → `720p` → `480p`) when duration requires low bitrate.
  - Copies output path to clipboard: `wl-copy` on Wayland, PowerShell `Set-Clipboard` on Windows (using hidden command).
- [`src/ipc.rs`](file:///mnt/Files/Proyectos/sane-rec/src/ipc.rs):
  - Local IPC server and client protocol (`IpcCommand`, `IpcRequest`, `IpcResponse`).
  - Linux: Unix domain socket (`/run/user/$UID/sane-rec.sock`).
  - Windows: Named pipe (`\\.\pipe\sane-rec-$USERNAME`) or local dispatch.
  - Windows Global Hotkeys: Manages `RegisterHotKeyW` / `UnregisterHotKeyW` message loop in a dedicated background thread. Supports extended keys (`F1`–`F24`, `Home`, `End`, `PageUp`, `PageDown`, `Insert`, `Delete`, `Num0`–`Num9`, modifiers).
- [`src/profiles.rs`](file:///mnt/Files/Proyectos/sane-rec/src/profiles.rs):
  - Encoder profile definitions: `SaneSafe` (CPU libx264 zerolatency), `VaapiAmd`, `NvencNvidia`, `QsvIntel`, `AmfAmd`.
  - Pipeline builder functions constructing FFmpeg parameter flags.
- [`src/profiles_manager.rs`](file:///mnt/Files/Proyectos/sane-rec/src/profiles_manager.rs):
  - Custom hardware preset storage in TOML (`profiles/` subdirectory in user config).
  - Prevents path traversal vulnerabilities and seeds default hardware presets (e.g. `Pavilion Gaming - Celsius`).
- [`src/updater.rs`](file:///mnt/Files/Proyectos/sane-rec/src/updater.rs):
  - GitHub Releases API integration (`check_for_updates`, `perform_update`).
  - Non-blocking update checks with semantic version comparison (`v0.5.0` > `v0.4.1`).
  - In-place atomic binary replacement on Linux (`.tar.gz`).
  - Windows update handler: detects portable `.zip` vs installer `.exe`, runs silent install or hot-swaps `sane-rec.exe`.
  - Embedded changelog extractor (`CHANGELOG.md`) with markdown cleaner.
- [`src/config.rs`](file:///mnt/Files/Proyectos/sane-rec/src/config.rs):
  - Persistent user settings stored in `config.toml` (`directories::ProjectDirs`).
  - Wayland compositor shortcut injector (e.g. `niri` config binding modifier injection).
- [`src/crash_handler.rs`](file:///mnt/Files/Proyectos/sane-rec/src/crash_handler.rs):
  - Panic hook recording stack traces to crash report files in the configuration directory.

---

### 2. Slint UI Architecture (`ui/`)

- [`ui/theme.slint`](file:///mnt/Files/Proyectos/sane-rec/ui/theme.slint):
  - Material Design 3 Expressive theme tokens.
  - Dark baseline palette: Background `#141218`, Surface `#1D1B20`, Surface Container `#211F26`, Primary violet `#7C4DFF` / `#D0BCFF`.
  - Type tokens, spacing metrics, border radius, animations.
- [`ui/components.slint`](file:///mnt/Files/Proyectos/sane-rec/ui/components.slint):
  - Custom M3 components: `M3Button`, `M3Card`, `M3Toggle`, `M3Slider`, `M3Badge`, `M3TextField`, `M3Icon`.
  - `M3Icon`: Pure vector SVG path rendering. Never use raw unicode emoji glyphs in the UI.
- [`ui/app_window.slint`](file:///mnt/Files/Proyectos/sane-rec/ui/app_window.slint):
  - Root window `AppWindow` containing `NavigationRail`, Dashboard, Replays Gallery, Settings, Guide & Changelog, and collapsible Floating Compact Mode.

---

## Critical Architectural Invariants

When modifying or adding code, you MUST adhere to these rules:

1. **Never Spawn Visible Windows on Windows:**
   - Always execute external commands (`ffmpeg`, `ffprobe`, `curl`, `powershell`) using `discovery::new_hidden_command(...)` or explicitly call `.creation_flags(0x08000000)` (`CREATE_NO_WINDOW`).
2. **Never Block the Slint UI Event Loop:**
   - Any file I/O, subprocess execution, audio initialization, network request, or compression MUST be dispatched to `std::thread::spawn` or Tokio tasks.
   - Send results back to UI via `slint::invoke_from_event_loop(...)`.
3. **Prevent RTSS / RivaTuner & Game Hook Collisions:**
   - Do not run high-frequency GPU redraw timers when the window is hidden or minimized.
   - For screen capture on Windows, keep DWM desktop capture non-intrusive without stealing keyboard focus or triggering game pause states (e.g. Minecraft Java `ESC` screen).
4. **UI Styling Rules:**
   - Material Design 3 Dark Baseline palette only.
   - Zero card borders (`border-width: 0px`).
   - Sentence case for titles, labels, buttons, and toasts ("Guardar repetición", not "GUARDAR REPETICIÓN" or "Guardar Repetición").
   - Always use integer pixel arithmetic (`Math.floor(...) * 1px`) to prevent subpixel font/border blurriness in Slint.
   - Use vector SVG paths via `M3Icon`; never insert emoji glyphs into UI buttons or toasts.
5. **Cross-Platform Conditional Compilation:**
   - Keep platform boundaries clean: use `#[cfg(target_os = "windows")]` and `#[cfg(not(target_os = "windows"))]` (or `#[cfg(target_os = "linux")]`).
   - Never use Linux-only APIs (`libc`, Unix domain sockets, `/dev/shm`, `pactl`) inside Windows code paths and vice versa.

---

## Build, Test & Development Commands

```bash
# Run all unit and integration tests (39+ tests)
cargo test

# Build debug binary
cargo build

# Build optimized release binary
cargo build --release

# Install locally on Linux (copies binary, icon, and .desktop)
./install.sh

# Run sane-rec directly
cargo run --bin sane-rec

# Test CLI IPC commands
cargo run --bin sane-rec -- save
cargo run --bin sane-rec -- record
cargo run --bin sane-rec -- status
cargo run --bin sane-rec -- update
```
