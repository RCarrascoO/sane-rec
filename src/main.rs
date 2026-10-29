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
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordAction {
    Toggle,
    Start,
    Stop,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
                    println!("\nSeñal recibida, cerrando sane-rec daemon...");
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
                    }
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

    // Temporizador Slint para actualizar métricas en vivo (cada 1 segundo)
    let _timer = {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(1000),
            move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

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
                }
            },
        );
        timer
    };

    ui.run()?;
    Ok(())
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
