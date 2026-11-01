pub mod audio_monitor;
pub mod compressor;
pub mod config;
pub mod crash_handler;
pub mod discovery;
pub mod engine;
pub mod ipc;
pub mod profiles;
pub mod profiles_manager;
pub mod updater;

pub use audio_monitor::AudioPeakMonitor;
pub use compressor::{
    compress_to_target_size, get_discord_clip_path, get_video_duration_secs,
    spawn_discord_compression,
};
pub use config::{update_niri_shortcuts, Config, ShortcutsConfig};
pub use discovery::{
    delete_clip, detect_gpu_info, get_audio_devices, get_connected_outputs, get_open_windows,
    get_recent_clips, new_hidden_command, resolve_binary, run_interactive_slurp, AudioDeviceInfo,
    ClipInfo, OutputInfo, WindowInfo,
};
pub use engine::{
    get_default_audio_sink, get_default_audio_source, is_source_available, EngineError,
    EngineStatus, ReplayEngine,
};
pub use ipc::{
    parse_windows_hotkey, reload_windows_hotkeys, IpcCommand, IpcRequest, IpcResponse,
};
pub use profiles::EncoderProfile;
pub use profiles_manager::HardwareProfile;
pub use updater::{
    check_for_updates, clean_changelog_markdown, cleanup_old_executable, get_embedded_changelog,
    perform_update, ReleaseInfo,
};
