use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use sane_rec::{Config, EngineStatus, ReplayEngine, ipc};

slint::include_modules!();

#[derive(Parser, Debug)]
#[command(
    name = "sane-rec",
    author = "RCarrascoO",
    version = "0.1.0",
    about = "Material Design 3 Instant Replay & Screen Recorder for Linux with CPU-first architecture"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Commands {
    /// Inicia el daemon headless en segundo plano escuchando el socket IPC
    Daemon,
    /// Guarda un replay instantáneo del búfer en memoria (para atajo de teclado en niri)
    Save,
    /// Controla la grabación continua (toggle, start, stop)
    Record {
        #[arg(default_value = "toggle")]
        action: RecordAction,
    },
    /// Consulta y muestra el estado actual del grabador y buffer de repetición
    Status,
    /// Detiene el daemon en ejecución
    Stop,
    /// Comprueba y actualiza sane-rec a la versión más reciente publicada en GitHub Releases
    Update,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordAction {
    Toggle,
    Start,
    Stop,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Instalar manejador global de pánicos y fallos críticos
    sane_rec::crash_handler::install_panic_hook();

    let cli = Cli::parse();

    match cli.command {
        None => {
            // Modo GUI interactivo con Slint
            run_gui()?;
        }
        Some(Commands::Daemon) => {
            // Daemon headless en segundo plano
            run_daemon()?;
        }
        Some(Commands::Save) => {
            match ipc::send_command_sync("save") {
                Ok(resp) => {
                    if resp.is_ok() {
                        println!(
                            "{}",
                            resp.message
                                .unwrap_or_else(|| "Replay guardado exitosamente".into())
                        );
                    } else {
                        eprintln!(
                            "Error: {}",
                            resp.message
                                .unwrap_or_else(|| "Fallo al guardar replay".into())
                        );
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("Error conectando con el daemon sane-rec: {}", e);
                    eprintln!(
                        "Asegúrate de que el daemon esté activo ejecutando 'sane-rec daemon' o abriendo la app."
                    );
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Record { action }) => {
            let action_str = match action {
                RecordAction::Toggle => "record_toggle",
                RecordAction::Start => "record_start",
                RecordAction::Stop => "record_stop",
            };
            match ipc::send_command_sync(action_str) {
                Ok(resp) => {
                    if resp.is_ok() {
                        println!(
                            "{}",
                            resp.message
                                .unwrap_or_else(|| "Operación completada".into())
                        );
                    } else {
                        eprintln!(
                            "Error: {}",
                            resp.message
                                .unwrap_or_else(|| "Error en grabación".into())
                        );
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("Error conectando con el daemon sane-rec: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Status) => {
            match ipc::send_command_sync("status") {
                Ok(resp) => {
                    if resp.is_ok() {
                        if let Some(data) = resp.data {
                            if let Ok(status) = serde_json::from_value::<EngineStatus>(data) {
                                println!("sane-rec daemon status:");
                                println!(
                                    "  Búfer de repetición: {}",
                                    if status.is_running {
                                        format!(
                                            "Activo ({}s / {}s)",
                                            status.current_buffer_secs, status.duration_secs
                                        )
                                    } else {
                                        "Detenido".to_string()
                                    }
                                );
                                println!("  Uso de RAM búfer:    ~{:.1} MB", status.ram_usage_mb);
                                println!(
                                    "  Grabación continua:  {}",
                                    if status.is_recording {
                                        "Grabando..."
                                    } else {
                                        "Inactiva"
                                    }
                                );
                                println!("  Perfil de encoder:   {}", status.active_profile);
                                println!("  Directorio salida:   {}", status.output_dir.display());
                            } else {
                                println!("{}", resp.message.unwrap_or_default());
                            }
                        } else {
                            println!("{}", resp.message.unwrap_or_default());
                        }
                    } else {
                        eprintln!("Error del daemon: {}", resp.message.unwrap_or_default());
                    }
                }
                Err(_) => {
                    println!(
                        "○ sane-rec daemon no se está ejecutando (socket {} no activo)",
                        ipc::socket_path().display()
                    );
                }
            }
        }
        Some(Commands::Stop) => {
            match ipc::send_command_sync("stop") {
                Ok(resp) => {
                    println!(
                        "{}",
                        resp.message.unwrap_or_else(|| "Daemon detenido".into())
                    );
                }
                Err(_) => {
                    println!("sane-rec daemon no se encuentra en ejecución.");
                }
            }
        }
        Some(Commands::Update) => {
            println!("🔍 Comprobando actualizaciones de sane-rec en GitHub Releases...");
            match sane_rec::updater::perform_update() {
                Ok(version) => {
                    println!("✨ sane-rec se ha actualizado correctamente a la versión {}.", version);
                }
                Err(e) => {
                    eprintln!("❌ No se pudo actualizar: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

/// Ejecuta el daemon headless de sane-rec con Tokio y escucha de sockets Unix IPC.
fn run_daemon() -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let config = Config::load().unwrap_or_default();
        let engine = Arc::new(ReplayEngine::new(config)?);

        // Inicia el búfer de repetición inmediatamente
        if let Err(e) = engine.start_replay() {
            eprintln!(
                "Aviso: no se pudo iniciar el búfer de repetición automáticamente: {}",
                e
            );
        } else {
            println!(
                "🎬 Búfer de repetición activo ({}s rodantes en RAM)",
                engine.config().replay.duration_secs
            );
        }

        let (tx, mut rx) = tokio::sync::mpsc::channel::<ipc::IpcCommand>(32);

        // Tarea del listener IPC
        let _listener_task = tokio::spawn(async move {
            if let Err(e) = ipc::start_listener(tx).await {
                eprintln!("Error en listener IPC: {}", e);
            }
        });

        println!(
            "● sane-rec daemon iniciado escuchando en {}",
            ipc::socket_path().display()
        );

        let engine_loop = engine.clone();
        #[cfg(unix)]
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

        loop {
            tokio::select! {
                Some(cmd) = rx.recv() => {
                    let should_stop = dispatch_ipc_command(&engine_loop, cmd).await;
                    if should_stop {
                        println!("Detención solicitada vía IPC. Cerrando daemon...");
                        break;
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    println!("\nSeñal SIGINT recibida, cerrando sane-rec daemon...");
                    let _ = engine_loop.stop_replay();
                    let _ = engine_loop.stop_continuous_recording();
                    break;
                }
                _ = async {
                    #[cfg(unix)]
                    {
                        sigterm.recv().await
                    }
                    #[cfg(not(unix))]
                    {
                        std::future::pending::<Option<()>>().await
                    }
                } => {
                    println!("\nSeñal SIGTERM recibida, cerrando sane-rec daemon...");
                    let _ = engine_loop.stop_replay();
                    let _ = engine_loop.stop_continuous_recording();
                    break;
                }
            }
        }

        let _ = std::fs::remove_file(ipc::socket_path());
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

/// Procesa las acciones IPC recibidas y envía la respuesta adecuada.
/// Retorna `true` si el comando solicitó detener el daemon.
async fn dispatch_ipc_command(engine: &Arc<ReplayEngine>, cmd: ipc::IpcCommand) -> bool {
    let action = cmd.action.clone();
    let mut should_exit = false;

    match action.as_str() {
        "save" => match engine.save_clip() {
            Ok(path) => {
                cmd.ok_with_data(
                    format!("Replay guardado en {}", path.display()),
                    serde_json::json!({ "path": path.to_string_lossy() }),
                );
            }
            Err(e) => cmd.error(format!("Error guardando replay: {}", e)),
        },
        "status" => {
            let status = engine.get_status();
            if let Ok(val) = serde_json::to_value(&status) {
                cmd.ok_with_data("Current status", val);
            } else {
                cmd.error("Failed to serialize engine status");
            }
        }
        "record" | "record_toggle" => match engine.toggle_continuous_recording() {
            Ok(recording) => {
                cmd.ok(if recording {
                    "Grabación continua iniciada"
                } else {
                    "Grabación continua detenida"
                });
            }
            Err(e) => cmd.error(format!("Error en grabación: {}", e)),
        },
        "record_start" => match engine.start_continuous_recording() {
            Ok(path) => cmd.ok(format!("Grabación iniciada en {}", path.display())),
            Err(e) => cmd.error(format!("Error iniciando grabación: {}", e)),
        },
        "record_stop" => match engine.stop_continuous_recording() {
            Ok(path) => cmd.ok(format!("Grabación guardada en {}", path.display())),
            Err(e) => cmd.error(format!("Error deteniendo grabación: {}", e)),
        },
        "toggle_replay" => match engine.toggle_replay() {
            Ok(running) => cmd.ok(if running {
                "Búfer de replay iniciado"
            } else {
                "Búfer de replay detenido"
            }),
            Err(e) => cmd.error(format!("Error en búfer: {}", e)),
        },
        "start_replay" => match engine.start_replay() {
            Ok(()) => cmd.ok("Búfer de replay iniciado"),
            Err(e) => cmd.error(format!("Error iniciando búfer: {}", e)),
        },
        "stop_replay" => match engine.stop_replay() {
            Ok(()) => cmd.ok("Búfer de replay detenido"),
            Err(e) => cmd.error(format!("Error deteniendo búfer: {}", e)),
        },
        "stop" => {
            let _ = engine.stop_replay();
            let _ = engine.stop_continuous_recording();
            cmd.ok("sane-rec daemon detenido exitosamente");
            should_exit = true;
        }
        action if action.starts_with("set_profile:") => {
            let prof = action.trim_start_matches("set_profile:").trim();
            let mut cfg = engine.config();
            cfg.profile.active_profile = prof.to_string();
            let _ = cfg.save();
            engine.update_config(cfg);
            cmd.ok(format!("Perfil cambiado a {}", prof));
        }
        action if action.starts_with("set_duration:") => {
            let dur_str = action.trim_start_matches("set_duration:").trim();
            if let Ok(dur) = dur_str.parse::<u32>() {
                engine.update_duration(dur);
                cmd.ok(format!("Duración del búfer actualizada a {}s", dur));
            } else {
                cmd.error("Duración inválida");
            }
        }
        action if action.starts_with("set_max_memory:") => {
            let ram_str = action.trim_start_matches("set_max_memory:").trim();
            if let Ok(ram) = ram_str.parse::<u32>() {
                engine.update_max_memory(ram);
                cmd.ok(format!("Límite de memoria RAM actualizado a {} MB", ram));
            } else {
                cmd.error("Límite de RAM inválido");
            }
        }
        other => {
            cmd.error(format!("Acción desconocida: {}", other));
        }
    }

    should_exit
}

/// Ejecuta la interfaz gráfica moderna Slint.
fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load().unwrap_or_default();
    let ui = AppWindow::new()?;

    // Inicializar propiedades de la ventana desde la configuración
    ui.set_active_profile(config.profile.active_profile.clone().into());
    ui.set_output_path(
        config
            .storage
            .output_dir
            .to_string_lossy()
            .to_string()
            .into(),
    );
    ui.set_replay_duration(config.replay.duration_secs as i32);
    ui.set_selected_format(config.storage.format.clone().into());
    ui.set_selected_screen(config.video.target.clone().into());
    ui.set_audio_system(config.audio.capture_desktop);
    ui.set_audio_mic(config.audio.capture_mic);
    ui.set_volume_system(config.audio.volume_system);
    ui.set_volume_mic(config.audio.volume_mic);
    ui.set_shortcut_save(config.shortcuts.save_replay.clone().into());
    ui.set_shortcut_record(config.shortcuts.toggle_record.clone().into());
    ui.set_custom_save_text(config.shortcuts.save_replay.clone().into());
    ui.set_custom_record_text(config.shortcuts.toggle_record.clone().into());
    ui.set_max_memory_mb(config.replay.max_memory_mb as i32);
    ui.set_custom_codec(config.video.codec.clone().into());
    ui.set_custom_preset(config.video.preset.clone().into());
    ui.set_custom_crf(config.video.crf as i32);

    // Inicializar perfiles de hardware por defecto (~/.config/sane-rec/profiles/)
    sane_rec::profiles_manager::init_default_profiles();
    update_profiles_model(&ui);

    // Cargar salidas de pantalla, ventanas activas y dispositivos de audio descubiertos
    update_screens_model(&ui);
    update_windows_model(&ui);
    update_audio_devices_model(&ui);
    update_gallery_model(&ui, &config.storage.output_dir);

    // Verificar si el daemon ya se encuentra activo
    let daemon_running = ipc::send_command_sync("status").is_ok();

    let local_engine = if !daemon_running {
        let engine = Arc::new(ReplayEngine::new(config.clone()).map_err(|e| e.to_string())?);

        // Si el daemon no está corriendo, iniciamos el listener IPC en segundo plano
        // para que atajos globales de Niri (`sane-rec save`) funcionen con la GUI abierta
        let engine_for_ipc = engine.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(_) => return,
            };

            rt.block_on(async move {
                let (tx, mut rx) = tokio::sync::mpsc::channel::<ipc::IpcCommand>(16);
                let _ = tokio::spawn(async move {
                    let _ = ipc::start_listener(tx).await;
                });

                while let Some(cmd) = rx.recv().await {
                    dispatch_ipc_command(&engine_for_ipc, cmd).await;
                }
            });
        });

        Some(engine)
    } else {
        None
    };

    // Callback save-clip
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        let output_dir = config.storage.output_dir.clone();
        ui.on_save_clip(move || {
            let res = if daemon_running {
                ipc::send_command_sync("save").map(|resp| {
                    resp.data
                        .and_then(|d| d.get("path").and_then(|p| p.as_str().map(PathBuf::from)))
                        .unwrap_or_else(|| PathBuf::from("clip.mp4"))
                })
            } else if let Some(ref eng) = engine_clone {
                eng.save_clip()
                    .map_err(|e| ipc::IpcError::Custom(e.to_string()))
            } else {
                Err(ipc::IpcError::Custom("No engine available".into()))
            };

            if let Ok(path) = res {
                if let Some(ui) = ui_weak.upgrade() {
                    if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                        ui.set_last_saved_clip(filename.into());
                        show_toast(&ui, "🎬", &format!("Replay guardado: {}", filename));
                    }
                    let out_dir = if let Some(ref eng) = engine_clone {
                        eng.config().storage.output_dir
                    } else {
                        path.parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| output_dir.clone())
                    };
                    update_gallery_model(&ui, &out_dir);
                }
            }
        });
    }

    // Callback toggle-replay
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_toggle_replay(move || {
            let is_running = if daemon_running {
                if let Ok(resp) = ipc::send_command_sync("toggle_replay") {
                    resp.message.map_or(false, |m| m.contains("iniciado"))
                } else {
                    false
                }
            } else if let Some(ref eng) = engine_clone {
                eng.toggle_replay().unwrap_or(false)
            } else {
                false
            };

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_is_running(is_running);
                ui.set_buffer_status(
                    if is_running {
                        "Búfer Activo".into()
                    } else {
                        "Detenido".into()
                    },
                );
                show_toast(
                    &ui,
                    "🧠",
                    if is_running {
                        "Búfer de repetición en RAM activado"
                    } else {
                        "Búfer de repetición detenido"
                    },
                );
            }
        });
    }

    // Callback select-profile
    {
        let engine_clone = local_engine.clone();
        ui.on_select_profile(move |prof_str| {
            let prof = prof_str.as_str();
            if daemon_running {
                let _ = ipc::send_command_sync(&format!("set_profile:{}", prof));
            } else if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.profile.active_profile = prof.to_string();
                let _ = cfg.save();
                eng.update_config(cfg);
            }
        });
    }

    // Callback select-output-dir
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_select_output_dir(move || {
            let output = std::process::Command::new("zenity")
                .arg("--file-selection")
                .arg("--directory")
                .arg("--title=Seleccionar carpeta de Replays")
                .output();

            let selected_path = match output {
                Ok(out) if out.status.success() => {
                    let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !path_str.is_empty() {
                        Some(PathBuf::from(path_str))
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(dir) = selected_path {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_output_path(dir.to_string_lossy().to_string().into());
                }
                if let Some(ref eng) = engine_clone {
                    let mut cfg = eng.config();
                    cfg.storage.output_dir = dir.clone();
                    let _ = cfg.save();
                    eng.update_config(cfg);
                }
            }
        });
    }

    // Callback toggle-recording (Grabación continua)
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        let output_dir_rec = config.storage.output_dir.clone();
        ui.on_toggle_recording(move || {
            let is_recording = if daemon_running {
                if let Ok(resp) = ipc::send_command_sync("record_toggle") {
                    resp.message.map_or(false, |m| m.contains("iniciada"))
                } else {
                    false
                }
            } else if let Some(ref eng) = engine_clone {
                eng.toggle_continuous_recording().unwrap_or(false)
            } else {
                false
            };

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_is_recording(is_recording);
                if is_recording {
                    show_toast(&ui, "🔴", "Grabación continua iniciada (Mod+F9)");
                } else {
                    show_toast(&ui, "⏹️", "Grabación continua finalizada y guardada");
                    update_gallery_model(&ui, &output_dir_rec);
                }
            }
        });
    }

    // Callbacks para la Galería y Reproducción
    {
        let output_dir = config.storage.output_dir.clone();
        ui.on_play_clip(move |path_str| {
            let p = PathBuf::from(path_str.as_str());
            let target = if p.is_absolute() && p.exists() {
                p
            } else {
                output_dir.join(path_str.as_str())
            };
            if std::process::Command::new("mpv").arg(&target).spawn().is_err() {
                let _ = std::process::Command::new("xdg-open").arg(target).spawn();
            }
        });
    }

    {
        let output_dir = config.storage.output_dir.clone();
        ui.on_open_clip_folder(move |path_str| {
            let p = PathBuf::from(path_str.as_str());
            let target = if p.is_absolute() && p.parent().is_some() {
                p.parent().unwrap().to_path_buf()
            } else {
                output_dir.clone()
            };
            if std::process::Command::new("cosmic-files").arg(&target).spawn().is_err() {
                let _ = std::process::Command::new("xdg-open").arg(target).spawn();
            }
        });
    }

    {
        let output_dir = config.storage.output_dir.clone();
        ui.on_open_output_folder(move || {
            if std::process::Command::new("cosmic-files").arg(&output_dir).spawn().is_err() {
                let _ = std::process::Command::new("xdg-open").arg(&output_dir).spawn();
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let output_dir = config.storage.output_dir.clone();
        ui.on_refresh_gallery(move || {
            if let Some(ui) = ui_weak.upgrade() {
                update_gallery_model(&ui, &output_dir);
            }
        });
    }

    // Callbacks para selección de Fuentes (Slurp y Ventanas)
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_select_area_slurp(move || {
            if let Some(geo) = sane_rec::run_interactive_slurp() {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_selected_screen(format!("area:{}", geo).into());
                    ui.set_selected_resolution(geo.clone().into());
                    ui.set_capture_mode(2);
                }
                if let Some(ref eng) = engine_clone {
                    let mut cfg = eng.config();
                    cfg.video.target = format!("area:{}", geo);
                    let _ = cfg.save();
                    eng.update_config(cfg);
                } else if let Ok(mut cfg) = Config::load() {
                    cfg.video.target = format!("area:{}", geo);
                    let _ = cfg.save();
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_refresh_windows(move || {
            if let Some(ui) = ui_weak.upgrade() {
                update_windows_model(&ui);
                let windows = sane_rec::get_open_windows();
                if let Some(focused) = windows.iter().find(|w| w.is_focused) {
                    ui.set_selected_window(focused.title.clone().into());
                }
            }
        });
    }

    {
        let engine_clone = local_engine.clone();
        ui.on_select_screen(move |screen_name, res| {
            let screen = screen_name.as_str();
            let resolution = res.as_str();
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.video.target = screen.to_string();
                cfg.video.scale = resolution.to_string();
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.video.target = screen.to_string();
                cfg.video.scale = resolution.to_string();
                let _ = cfg.save();
            }
        });
    }

    {
        let engine_clone = local_engine.clone();
        ui.on_select_window(move |_title, app_id| {
            let target = format!("window:{}", app_id.as_str());
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.video.target = target;
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.video.target = target;
                let _ = cfg.save();
            }
        });
    }

    {
        let engine_clone = local_engine.clone();
        ui.on_select_audio_sink(move |_sink_name| {
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.audio.capture_desktop = true;
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.audio.capture_desktop = true;
                let _ = cfg.save();
            }
        });
    }

    {
        let engine_clone = local_engine.clone();
        ui.on_select_audio_mic(move |mic_name| {
            let is_active = mic_name.as_str() != "Desactivado";
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.audio.capture_mic = is_active;
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.audio.capture_mic = is_active;
                let _ = cfg.save();
            }
        });
    }

    // Callbacks de persistencia y ajuste dinámico de volumen (desacoplados del hilo de render UI)
    {
        let engine_clone = local_engine.clone();
        ui.on_set_volume_system(move |vol| {
            let engine = engine_clone.clone();
            std::thread::spawn(move || {
                if let Some(ref eng) = engine {
                    eng.set_volume_system(vol);
                    let mut cfg = eng.config();
                    cfg.audio.volume_system = vol;
                    let _ = cfg.save();
                } else if let Ok(mut cfg) = Config::load() {
                    cfg.audio.volume_system = vol;
                    let _ = cfg.save();
                    ReplayEngine::apply_sink_input_volume("sane_rec_system", vol);
                }
            });
        });
    }

    {
        let engine_clone = local_engine.clone();
        ui.on_set_volume_mic(move |vol| {
            let engine = engine_clone.clone();
            std::thread::spawn(move || {
                if let Some(ref eng) = engine {
                    eng.set_volume_mic(vol);
                    let mut cfg = eng.config();
                    cfg.audio.volume_mic = vol;
                    let _ = cfg.save();
                } else if let Ok(mut cfg) = Config::load() {
                    cfg.audio.volume_mic = vol;
                    let _ = cfg.save();
                    ReplayEngine::apply_sink_input_volume("sane_rec_mic", vol);
                }
            });
        });
    }

    {
        let engine_clone = local_engine.clone();
        ui.on_toggle_audio_system(move |active| {
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.audio.capture_desktop = active;
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.audio.capture_desktop = active;
                let _ = cfg.save();
            }
        });
    }

    {
        let engine_clone = local_engine.clone();
        ui.on_toggle_audio_mic(move |active| {
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.audio.capture_mic = active;
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.audio.capture_mic = active;
                let _ = cfg.save();
            }
        });
    }

    // Callback select-replay-duration
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_select_replay_duration(move |dur| {
            let d = dur as u32;
            if daemon_running {
                let _ = ipc::send_command_sync(&format!("set_duration:{}", d));
            } else if let Some(ref eng) = engine_clone {
                eng.update_duration(d);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.replay.duration_secs = d;
                let _ = cfg.save();
            }

            if let Some(ui) = ui_weak.upgrade() {
                show_toast(&ui, "⏱️", &format!("Búfer de repetición fijado a {}s", d));
            }
        });
    }

    // Callback update-shortcuts
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_update_shortcuts(move |save_key, rec_key| {
            let save_str = save_key.as_str();
            let rec_str = rec_key.as_str();

            let _ = sane_rec::update_niri_shortcuts(save_str, rec_str);

            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.shortcuts.save_replay = save_str.to_string();
                cfg.shortcuts.toggle_record = rec_str.to_string();
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.shortcuts.save_replay = save_str.to_string();
                cfg.shortcuts.toggle_record = rec_str.to_string();
                let _ = cfg.save();
            }

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_shortcut_save(save_str.into());
                ui.set_shortcut_record(rec_str.into());
                ui.set_custom_save_text(save_str.into());
                ui.set_custom_record_text(rec_str.into());
                show_toast(&ui, "⌨️", &format!("Atajos actualizados en niri ({}, {})", save_str, rec_str));
            }
        });
    }

    // Callback set-custom-codec
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_set_custom_codec(move |codec| {
            let c = codec.as_str();
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.video.codec = c.to_string();
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.video.codec = c.to_string();
                let _ = cfg.save();
            }
            if let Some(ui) = ui_weak.upgrade() {
                show_toast(&ui, "⚙️", &format!("Códec personalizado: {}", c));
            }
        });
    }

    // Callback set-custom-preset
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_set_custom_preset(move |preset| {
            let p = preset.as_str();
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.video.preset = p.to_string();
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.video.preset = p.to_string();
                let _ = cfg.save();
            }
            if let Some(ui) = ui_weak.upgrade() {
                show_toast(&ui, "⚡", &format!("Preset CPU: {}", p));
            }
        });
    }

    // Callback set-custom-crf
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_set_custom_crf(move |crf| {
            let c = crf as u32;
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.video.crf = c;
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.video.crf = c;
                let _ = cfg.save();
            }
            if let Some(ui) = ui_weak.upgrade() {
                show_toast(&ui, "🎚️", &format!("Calidad CRF: {}", c));
            }
        });
    }

    // Callback set-custom-max-ram
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_set_custom_max_ram(move |max_ram| {
            let ram = max_ram as u32;
            if daemon_running {
                let _ = ipc::send_command_sync(&format!("set_max_memory:{}", ram));
            } else if let Some(ref eng) = engine_clone {
                eng.update_max_memory(ram);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.replay.max_memory_mb = ram;
                let _ = cfg.save();
            }
            if let Some(ui) = ui_weak.upgrade() {
                show_toast(&ui, "🧠", &format!("Límite búfer RAM: {} MB", ram));
            }
        });
    }

    // -------------------------------------------------------------------------
    // Callbacks del Gestor de Perfiles de Hardware (~/.config/sane-rec/profiles/)
    // -------------------------------------------------------------------------
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_load_hardware_profile(move |name| {
            let Some(ui) = ui_weak.upgrade() else { return; };
            match sane_rec::profiles_manager::load_profile(name.as_str()) {
                Ok(profile) => {
                    ui.set_custom_codec(profile.codec.clone().into());
                    ui.set_custom_preset(profile.preset.clone().into());
                    ui.set_custom_crf(profile.crf as i32);
                    ui.set_max_memory_mb(profile.max_memory_mb as i32);
                    ui.set_selected_fps(profile.fps as i32);
                    ui.set_selected_resolution(profile.resolution.clone().into());
                    ui.set_audio_track_mode(profile.audio_track_mode as i32);
                    ui.set_active_profile("custom".into());

                    if let Some(ref eng) = engine_clone {
                        let mut cfg = eng.config();
                        cfg.video.codec = profile.codec;
                        cfg.video.preset = profile.preset;
                        cfg.video.crf = profile.crf;
                        cfg.video.fps = profile.fps;
                        cfg.video.scale = profile.resolution;
                        cfg.replay.max_memory_mb = profile.max_memory_mb as u32;
                        cfg.profile.active_profile = "custom".to_string();
                        let _ = cfg.save();
                        eng.update_config(cfg);
                    } else if let Ok(mut cfg) = Config::load() {
                        cfg.video.codec = profile.codec;
                        cfg.video.preset = profile.preset;
                        cfg.video.crf = profile.crf;
                        cfg.video.fps = profile.fps;
                        cfg.video.scale = profile.resolution;
                        cfg.replay.max_memory_mb = profile.max_memory_mb as u32;
                        cfg.profile.active_profile = "custom".to_string();
                        let _ = cfg.save();
                    }
                    show_toast(&ui, "⚙️", &format!("Perfil '{}' cargado correctamente", profile.name));
                }
                Err(e) => {
                    show_toast(&ui, "❌", &format!("Error al cargar perfil: {}", e));
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_save_hardware_profile(move |name| {
            let Some(ui) = ui_weak.upgrade() else { return; };
            let profile_name = if name.trim().is_empty() {
                "Mi-Perfil-Personalizado".to_string()
            } else {
                name.trim().to_string()
            };
            let profile = sane_rec::profiles_manager::HardwareProfile {
                name: profile_name.clone(),
                description: format!("Perfil personalizado configurado para {}", profile_name),
                target_device: "Dispositivo actual".to_string(),
                codec: ui.get_custom_codec().to_string(),
                preset: ui.get_custom_preset().to_string(),
                crf: ui.get_custom_crf() as u32,
                fps: ui.get_selected_fps() as u32,
                resolution: ui.get_selected_resolution().to_string(),
                max_memory_mb: ui.get_max_memory_mb() as u64,
                audio_track_mode: ui.get_audio_track_mode() as u32,
            };

            match sane_rec::profiles_manager::save_profile(&profile) {
                Ok(_) => {
                    update_profiles_model(&ui);
                    show_toast(&ui, "💾", &format!("Perfil '{}' guardado", profile_name));
                }
                Err(e) => {
                    show_toast(&ui, "❌", &format!("Error al guardar perfil: {}", e));
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_export_hardware_profile(move |name| {
            let Some(ui) = ui_weak.upgrade() else { return; };
            let profile_res = sane_rec::profiles_manager::load_profile(name.as_str());
            let profile = match profile_res {
                Ok(p) => p,
                Err(_) => sane_rec::profiles_manager::HardwareProfile {
                    name: name.to_string(),
                    description: "Perfil exportado de sane-rec".to_string(),
                    target_device: "Dispositivo actual".to_string(),
                    codec: ui.get_custom_codec().to_string(),
                    preset: ui.get_custom_preset().to_string(),
                    crf: ui.get_custom_crf() as u32,
                    fps: ui.get_selected_fps() as u32,
                    resolution: ui.get_selected_resolution().to_string(),
                    max_memory_mb: ui.get_max_memory_mb() as u64,
                    audio_track_mode: ui.get_audio_track_mode() as u32,
                },
            };

            let default_filename = format!("{}.toml", name.trim().replace(' ', "_"));
            let output = std::process::Command::new("zenity")
                .args([
                    "--file-selection",
                    "--save",
                    "--confirm-overwrite",
                    "--title=Exportar Perfil de Hardware",
                    &format!("--filename={}", default_filename),
                ])
                .output();

            if let Ok(o) = output {
                if o.status.success() {
                    let path_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if !path_str.is_empty() {
                        let path = std::path::PathBuf::from(path_str);
                        match sane_rec::profiles_manager::export_profile(&profile, &path) {
                            Ok(_) => {
                                show_toast(&ui, "📤", &format!("Perfil exportado a {}", path.display()));
                            }
                            Err(e) => {
                                show_toast(&ui, "❌", &format!("Error al exportar: {}", e));
                            }
                        }
                    }
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_import_hardware_profile(move || {
            let Some(ui) = ui_weak.upgrade() else { return; };
            let output = std::process::Command::new("zenity")
                .args([
                    "--file-selection",
                    "--title=Seleccionar Perfil de Hardware (.toml)",
                    "--file-filter=Archivos TOML (*.toml) | *.toml",
                ])
                .output();

            if let Ok(o) = output {
                if o.status.success() {
                    let path_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if !path_str.is_empty() {
                        let path = std::path::PathBuf::from(path_str);
                        match sane_rec::profiles_manager::import_profile(&path) {
                            Ok(profile) => {
                                update_profiles_model(&ui);
                                ui.set_selected_profile_name(profile.name.clone().into());
                                ui.set_custom_codec(profile.codec.clone().into());
                                ui.set_custom_preset(profile.preset.clone().into());
                                ui.set_custom_crf(profile.crf as i32);
                                ui.set_max_memory_mb(profile.max_memory_mb as i32);
                                ui.set_selected_fps(profile.fps as i32);
                                ui.set_selected_resolution(profile.resolution.clone().into());
                                ui.set_audio_track_mode(profile.audio_track_mode as i32);
                                ui.set_active_profile("custom".into());

                                if let Some(ref eng) = engine_clone {
                                    let mut cfg = eng.config();
                                    cfg.video.codec = profile.codec;
                                    cfg.video.preset = profile.preset;
                                    cfg.video.crf = profile.crf;
                                    cfg.video.fps = profile.fps;
                                    cfg.video.scale = profile.resolution;
                                    cfg.replay.max_memory_mb = profile.max_memory_mb as u32;
                                    cfg.profile.active_profile = "custom".to_string();
                                    let _ = cfg.save();
                                    eng.update_config(cfg);
                                }
                                show_toast(&ui, "📥", &format!("Perfil '{}' importado exitosamente", profile.name));
                            }
                            Err(e) => {
                                show_toast(&ui, "❌", &format!("Error al importar perfil: {}", e));
                            }
                        }
                    }
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_refresh_hardware_profiles(move || {
            if let Some(ui) = ui_weak.upgrade() {
                update_profiles_model(&ui);
                show_toast(&ui, "🔄", "Lista de perfiles actualizada");
            }
        });
    }

    // -------------------------------------------------------------------------
    // Auto-actualizador: Verificación no bloqueante y 1-clic update
    // -------------------------------------------------------------------------
    {
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            if let Some(info) = sane_rec::updater::check_for_updates() {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_update_version(info.version.into());
                        ui.set_has_update(true);
                    }
                });
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_trigger_update(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_is_updating(true);
                show_toast(&ui, "🚀", "Descargando e instalando actualización...");
            }
            let ui_weak2 = ui_weak.clone();
            std::thread::spawn(move || {
                let res = sane_rec::updater::perform_update();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        ui.set_is_updating(false);
                        match res {
                            Ok(v) => {
                                ui.set_has_update(false);
                                show_toast(&ui, "✨", &format!("¡Actualizado a {}! Reinicia la app para aplicar cambios.", v));
                            }
                            Err(e) => {
                                show_toast(&ui, "❌", &format!("Error al actualizar: {}", e));
                            }
                        }
                    }
                });
            });
        });
    }

    // Inicia monitor de picos de audio en tiempo real para calibración visual (VU meter)
    let audio_monitor = std::sync::Arc::new(sane_rec::AudioPeakMonitor::new(None, None));

    // Temporizador Slint para actualizar métricas en vivo y medidores VU (50ms = 20 FPS)
    let _timer = {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        let audio_mon_clone = audio_monitor.clone();
        let timer = slint::Timer::default();
        let mut ticks: u32 = 0;
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                // Actualiza niveles visuales VU del sistema y micrófono a 20 FPS
                ui.set_system_volume_peak(audio_mon_clone.system_peak());
                ui.set_mic_volume_peak(audio_mon_clone.mic_peak());

                ticks += 1;
                // Cada 500ms consulta el estado del motor y actualiza contadores
                if ticks % 10 == 0 {
                    let status_opt = if daemon_running {
                        if let Ok(resp) = ipc::send_command_sync("status") {
                            resp.data
                                .and_then(|d| serde_json::from_value::<EngineStatus>(d).ok())
                        } else {
                            None
                        }
                    } else if let Some(ref eng) = engine_clone {
                        Some(eng.get_status())
                    } else {
                        None
                    };

                    if let Some(status) = status_opt {
                        ui.set_is_running(status.is_running);
                        ui.set_is_recording(status.is_recording);
                        ui.set_max_memory_mb(status.max_memory_mb as i32);
                        ui.set_ram_usage(format!("~{:.1} MB", status.ram_usage_mb).into());
                        let buf_text = if status.is_running {
                            format!(
                                "Búfer Activo ({}s / {}s)",
                                status.current_buffer_secs, status.duration_secs
                            )
                        } else {
                            "Detenido".to_string()
                        };
                        ui.set_buffer_status(buf_text.into());

                        // Actualización dinámica del contador de tiempo de grabación continua
                        if status.is_recording {
                            let s = status.recording_elapsed_secs;
                            let time_str = if s >= 3600 {
                                format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
                            } else {
                                format!("{:02}:{:02}", s / 60, s % 60)
                            };
                            ui.set_recording_time(time_str.into());
                        } else {
                            ui.set_recording_time("00:00".into());
                        }
                    }
                }
            },
        );
        timer
    };

    ui.run()?;
    Ok(())
}

/// Actualiza las pantallas disponibles leyendo las salidas Wayland con discovery.
fn update_screens_model(ui: &AppWindow) {
    let outputs = sane_rec::get_connected_outputs();
    if !outputs.is_empty() {
        let entries: Vec<OutputEntry> = outputs
            .into_iter()
            .map(|o| {
                let badge = if o.is_current { "Principal" } else { "Conectado" };
                let res = if o.width > 0 && o.height > 0 {
                    format!("{}x{}", o.width, o.height)
                } else {
                    "1360x768".to_string()
                };
                OutputEntry {
                    name: o.name.into(),
                    description: o.description.into(),
                    badge: badge.into(),
                    resolution: res.into(),
                }
            })
            .collect();
        ui.set_available_screens(std::rc::Rc::new(slint::VecModel::from(entries)).into());
    }
}

/// Actualiza la lista de ventanas activas detectadas en el compositor Wayland.
fn update_windows_model(ui: &AppWindow) {
    let windows = sane_rec::get_open_windows();
    if !windows.is_empty() {
        let entries: Vec<WindowEntry> = windows
            .into_iter()
            .map(|w| {
                let badge = if w.is_focused { "Enfocada" } else { "Ventana" };
                WindowEntry {
                    id: w.id as i32,
                    title: w.title.into(),
                    app_id: w.app_id.into(),
                    badge: badge.into(),
                }
            })
            .collect();
        ui.set_available_windows(std::rc::Rc::new(slint::VecModel::from(entries)).into());
    }
}

/// Actualiza las opciones de salida y micrófonos disponibles en PipeWire / PulseAudio.
fn update_audio_devices_model(ui: &AppWindow) {
    let (monitors, microphones) = sane_rec::get_audio_devices();
    if !monitors.is_empty() {
        let sink_entries: Vec<AudioDeviceEntry> = monitors
            .into_iter()
            .map(|m| {
                let badge = if m.is_default { "Por defecto" } else { "Salida" };
                AudioDeviceEntry {
                    name: m.name.into(),
                    description: m.description.into(),
                    badge: badge.into(),
                    is_default: m.is_default,
                }
            })
            .collect();
        ui.set_available_sinks(std::rc::Rc::new(slint::VecModel::from(sink_entries)).into());
    }

    let mut mic_entries: Vec<AudioDeviceEntry> = microphones
        .into_iter()
        .map(|m| {
            let badge = if m.is_default { "Por defecto" } else { "Entrada" };
            AudioDeviceEntry {
                name: m.name.into(),
                description: m.description.into(),
                badge: badge.into(),
                is_default: m.is_default,
            }
        })
        .collect();

    mic_entries.push(AudioDeviceEntry {
        name: "Desactivado".into(),
        description: "Desactivado (Solo audio de juego)".into(),
        badge: "Mudo".into(),
        is_default: false,
    });
    ui.set_available_mics(std::rc::Rc::new(slint::VecModel::from(mic_entries)).into());
}

/// Muestra un banner flotante Toast interactivo en la interfaz por 3.5 segundos.
fn show_toast(ui: &AppWindow, icon: &str, message: &str) {
    ui.set_toast_icon(icon.into());
    ui.set_toast_message(message.into());
    ui.set_toast_visible(true);
    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(3500), move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_toast_visible(false);
        }
    });
}

/// Obtiene la duración real del clip de video formateada como MM:SS usando ffprobe.
fn get_clip_duration_str(path: &std::path::Path) -> String {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Ok(secs) = s.trim().parse::<f64>() {
                let total_secs = secs.round() as u64;
                let mins = total_secs / 60;
                let rem_secs = total_secs % 60;
                return format!("{:02}:{:02}", mins, rem_secs);
            }
        }
    }
    "00:30".to_string()
}

/// Actualiza el modelo de clips de la galería leyendo el directorio de salida en disco.
fn update_gallery_model(ui: &AppWindow, output_dir: &std::path::Path) {
    let clips = sane_rec::get_recent_clips(output_dir);
    if clips.is_empty() {
        ui.set_gallery_clips(std::rc::Rc::new(slint::VecModel::default()).into());
        ui.set_recent_clips(std::rc::Rc::new(slint::VecModel::default()).into());
    } else {
        let entries: Vec<ClipEntry> = clips
            .into_iter()
            .map(|c| {
                let date_str = c
                    .modified
                    .map(|t| {
                        let dt: chrono::DateTime<chrono::Local> = t.into();
                        dt.format("%d %b %H:%M").to_string()
                    })
                    .unwrap_or_else(|| "Reciente".into());

                let duration_text = get_clip_duration_str(&c.path);

                ClipEntry {
                    filename: c.name.into(),
                    file_size: format!("{:.1} MB", c.size_mb).into(),
                    date_text: date_str.into(),
                    duration_text: duration_text.into(),
                    filepath: c.path.to_string_lossy().to_string().into(),
                }
            })
            .collect();
        let recent_entries: Vec<ClipEntry> = entries.iter().take(3).cloned().collect();
        ui.set_gallery_clips(std::rc::Rc::new(slint::VecModel::from(entries)).into());
        ui.set_recent_clips(std::rc::Rc::new(slint::VecModel::from(recent_entries)).into());
    }
}

/// Actualiza el modelo de perfiles de hardware disponibles en la interfaz.
fn update_profiles_model(ui: &AppWindow) {
    let profiles = sane_rec::profiles_manager::list_profiles();
    let slint_profiles: Vec<slint::SharedString> = profiles.into_iter().map(|s| s.into()).collect();
    ui.set_available_profiles_list(std::rc::Rc::new(slint::VecModel::from(slint_profiles)).into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_instantiation_and_properties() {
        let ui = AppWindow::new().unwrap();
        assert_eq!(ui.get_active_profile(), "sane-safe");
        assert_eq!(ui.get_is_running(), false);
        assert_eq!(ui.get_replay_duration(), 30);
        assert_eq!(ui.get_selected_format(), "mp4");
        assert_eq!(ui.get_output_path(), "/mnt/Files/Videos/Replays");

        ui.set_is_running(true);
        assert_eq!(ui.get_is_running(), true);

        ui.set_active_profile("amd-vaapi".into());
        assert_eq!(ui.get_active_profile(), "amd-vaapi");

        // Verificación de la navegación multi-sección
        assert_eq!(ui.get_active_view(), 0);
        ui.set_active_view(1);
        assert_eq!(ui.get_active_view(), 1);
        ui.set_active_view(2);
        assert_eq!(ui.get_active_view(), 2);
        ui.set_active_view(3);
        assert_eq!(ui.get_active_view(), 3);
        ui.set_active_view(4);
        assert_eq!(ui.get_active_view(), 4);

        // Propiedades de fuentes, audio y grabación
        assert_eq!(ui.get_capture_mode(), 0);
        ui.set_capture_mode(1);
        assert_eq!(ui.get_capture_mode(), 1);

        assert_eq!(ui.get_selected_fps(), 60);
        ui.set_selected_fps(30);
        assert_eq!(ui.get_selected_fps(), 30);

        assert_eq!(ui.get_audio_track_mode(), 0);
        ui.set_audio_track_mode(1);
        assert_eq!(ui.get_audio_track_mode(), 1);

        assert_eq!(ui.get_is_recording(), false);
        ui.set_is_recording(true);
        assert_eq!(ui.get_is_recording(), true);

        assert_eq!(ui.get_shortcut_save(), "Mod+F10");
        assert_eq!(ui.get_shortcut_record(), "Mod+F9");
        ui.set_shortcut_save("Ctrl+F12".into());
        assert_eq!(ui.get_shortcut_save(), "Ctrl+F12");
        ui.set_shortcut_save("Mod+F10".into());

        // Verificación de sincronización de galería vacía (elimina mock data)
        use slint::Model;
        let empty_dir = std::env::temp_dir().join(format!("sane_rec_empty_gal_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&empty_dir);
        update_gallery_model(&ui, &empty_dir);
        assert_eq!(ui.get_gallery_clips().row_count(), 0);
        assert_eq!(ui.get_recent_clips().row_count(), 0);
        assert_eq!(ui.get_system_volume_peak(), 0.0);
        assert_eq!(ui.get_recording_time(), "00:00");
        assert_eq!(ui.get_toast_visible(), false);
        assert_eq!(ui.get_shortcut_save(), "Mod+F10");
        assert_eq!(ui.get_shortcut_record(), "Mod+F9");
        assert_eq!(ui.get_max_memory_mb(), 150);
        assert_eq!(ui.get_custom_codec(), "libx264");
        assert_eq!(ui.get_custom_preset(), "ultrafast");
        assert_eq!(ui.get_custom_crf(), 23);
        let _ = std::fs::remove_dir_all(&empty_dir);
    }

    #[test]
    fn test_cli_parsing() {
        // Por defecto: sin argumentos
        let cli_default = Cli::try_parse_from(["sane-rec"]).unwrap();
        assert_eq!(cli_default.command, None);

        // Subcomando daemon
        let cli_daemon = Cli::try_parse_from(["sane-rec", "daemon"]).unwrap();
        assert_eq!(cli_daemon.command, Some(Commands::Daemon));

        // Subcomando save
        let cli_save = Cli::try_parse_from(["sane-rec", "save"]).unwrap();
        assert_eq!(cli_save.command, Some(Commands::Save));

        // Subcomando record (toggle por defecto)
        let cli_record = Cli::try_parse_from(["sane-rec", "record"]).unwrap();
        assert_eq!(
            cli_record.command,
            Some(Commands::Record {
                action: RecordAction::Toggle
            })
        );

        // Subcomando record start
        let cli_record_start = Cli::try_parse_from(["sane-rec", "record", "start"]).unwrap();
        assert_eq!(
            cli_record_start.command,
            Some(Commands::Record {
                action: RecordAction::Start
            })
        );

        // Subcomando record stop
        let cli_record_stop = Cli::try_parse_from(["sane-rec", "record", "stop"]).unwrap();
        assert_eq!(
            cli_record_stop.command,
            Some(Commands::Record {
                action: RecordAction::Stop
            })
        );

        // Subcomando status
        let cli_status = Cli::try_parse_from(["sane-rec", "status"]).unwrap();
        assert_eq!(cli_status.command, Some(Commands::Status));

        // Subcomando stop
        let cli_stop = Cli::try_parse_from(["sane-rec", "stop"]).unwrap();
        assert_eq!(cli_stop.command, Some(Commands::Stop));
    }
}
