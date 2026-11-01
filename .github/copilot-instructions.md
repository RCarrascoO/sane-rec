# GitHub Copilot Custom Instructions — sane-rec

You are an expert Rust systems and UI engineer assisting development on **sane-rec**, a lightweight, cross-platform instant replay buffer and screen recorder for Linux (Wayland / PipeWire) and Windows 10/11 (DXGI / GDI / WASAPI).

---

## 🏗️ Core Architecture & Philosophy

1. **CPU-First & Thermal Stability:**
   - Linux: Instant replay buffer runs in `/dev/shm` (RAM). Default profile `Sane Safe` uses `libx264` zerolatency to avoid AMD Polaris GPU driver lockups.
   - Windows: In-memory `VecDeque` RAM buffer. Default profiles protect laptop thermal headroom (0% dGPU utilization).
   - Zero SSD Wear: Never write replay frames to permanent disk until the user commands a clip save.
2. **Multiplatform Parity:**
   - Always gate platform-specific code with `#[cfg(target_os = "windows")]` vs `#[cfg(not(target_os = "windows"))]`.
   - Never import Unix symbols (`libc`, Unix sockets, `/dev/shm`, `pactl`) in Windows builds.
   - Never import Windows-specific symbols (`windows_sys`, `cpal` WASAPI specifics, `Set-Clipboard`) in Linux builds.
3. **Smart Discord Compression (<10 MB):**
   - Dual-clip generation automatically rescales (`1080p` → `720p` → `480p`) and targets 9.5 MB.
   - Copies file path directly to the OS clipboard (`wl-copy` on Wayland, PowerShell `Set-Clipboard` on Windows).

---

## 🚫 Hard Invariants (Never Break These)

- **No Visible Windows on Windows:**
  Always construct `std::process::Command` using `crate::discovery::new_hidden_command(...)` or append `.creation_flags(0x08000000)` (`CREATE_NO_WINDOW`) to prevent flashing black command prompts.
- **Never Block the Slint UI Thread:**
  Any synchronous I/O, process waiting, audio stream init, or network requests must happen inside `std::thread::spawn` or Tokio background tasks. Use `slint::invoke_from_event_loop(...)` to send state back to the UI.
- **No Unicode Emojis in Slint UI:**
  All icons must be vector SVG shapes rendered through `M3Icon`. Do not use emoji glyphs in buttons, cards, or notification toasts.
- **Material Design 3 Guidelines:**
  - Dark Baseline palette (surface: `#141218`, container: `#211F26`).
  - Zero card borders (`border-width: 0px`).
  - Sentence case capitalization in Spanish UI copy.
  - Snap coordinates and paddings with integer pixel arithmetic: `Math.floor(...) * 1px`.
- **RivaTuner / RTSS & Minecraft Java Safety:**
  Avoid high-frequency redraw timers when UI is hidden or inactive. Windows screen capture must never steal focus from active fullscreen or OpenGL games.

---

## 🗂️ Module Map

- `src/main.rs`: CLI parsing (`clap`), IPC command handling, Slint UI bootstrap and event loop binding.
- `src/engine.rs`: Main recording engine, PipeWire/WASAPI pipelines, rolling RAM buffer, fallback mechanism.
- `src/discovery.rs`: Monitors displays, windows, audio inputs/outputs, clip discovery, and deletion (`delete_clip`).
- `src/audio_monitor.rs`: Real-time audio VU peak metering for desktop loopback and microphone inputs.
- `src/compressor.rs`: Asynchronous Discord compression (<10 MB) with dynamic bitrate and adaptive scaling.
- `src/ipc.rs`: Local IPC server/client; Windows global hotkey hook (`RegisterHotKeyW`).
- `src/profiles.rs`: Encoder profiles (`SaneSafe`, `VaapiAmd`, `NvencNvidia`, `QsvIntel`, `AmfAmd`).
- `src/profiles_manager.rs`: Custom user hardware presets in TOML.
- `src/updater.rs`: GitHub Releases update checker and atomic in-place installer.
- `src/config.rs`: Settings storage (`config.toml`), compositor integration.
- `src/crash_handler.rs`: Panic hook capturing crash traces.
- `ui/theme.slint`: Material Design 3 tokens.
- `ui/components.slint`: Slint M3 widgets & vector icons (`M3Icon`).
- `ui/app_window.slint`: Main application layout and navigation rail.

---

## 🧪 Verification Commands

```bash
cargo test
cargo build --release
```
