pub mod config;
pub mod discovery;
pub mod engine;
pub mod ipc;
pub mod profiles;

pub use config::Config;
pub use discovery::{
    get_audio_devices, get_connected_outputs, get_open_windows, get_recent_clips,
    run_interactive_slurp, AudioDeviceInfo, ClipInfo, OutputInfo, WindowInfo,
};
pub use engine::{EngineError, EngineStatus, ReplayEngine};
pub use ipc::{IpcCommand, IpcRequest, IpcResponse};
pub use profiles::EncoderProfile;
