#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use sane_rec::{Config, EngineStatus, ReplayEngine, ipc};

slint::include_modules!();

#[derive(Parser, Debug)]
#[command(
    name = "sane-rec",
    author = "RCarrascoO",
    version,
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
    // Limpiar binario antiguo (.exe.old) si venimos de una auto-actualización en Windows
    sane_rec::cleanup_old_executable();

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
        let engine = Arc::new(ReplayEngine::new(config.clone())?);

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

        let _ = sane_rec::update_niri_shortcuts(
            &config.shortcuts.save_replay,
            &config.shortcuts.toggle_record,
        );
        sane_rec::ipc::reload_windows_hotkeys(
            &config.shortcuts.save_replay,
            &config.shortcuts.toggle_record,
        );

        // En Windows, escuchar atajos globales F9 y F10
        #[cfg(target_os = "windows")]
        sane_rec::ipc::spawn_global_hotkeys_listener();

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
        "reload_config" => {
            if let Ok(cfg) = Config::load() {
                engine.update_config(cfg);
                cmd.ok("Configuración recargada exitosamente");
            } else {
                cmd.error("Error al leer archivo de configuración");
            }
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
    #[allow(unused_mut)]
    let mut config = Config::load().unwrap_or_default();

    #[cfg(target_os = "windows")]
    {
        let out_str = config.storage.output_dir.to_string_lossy();
        if out_str.starts_with("/mnt/") || out_str.starts_with("/home/") || !config.storage.output_dir.is_absolute() {
            config.storage.output_dir = sane_rec::config::default_output_dir();
            let _ = config.save();
        }
    }

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
    ui.set_dual_clip(config.storage.dual_clip);
    ui.set_discord_target_mb(config.storage.discord_target_mb as i32);
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
    ui.set_changelog_text(
        sane_rec::clean_changelog_markdown(sane_rec::get_embedded_changelog()).into(),
    );

    // Configuración adaptativa de textos de Backend y Driver en la UI
    #[cfg(target_os = "windows")]
    {
        ui.set_backend_title("Windows DXGI".into());
        ui.set_backend_subtitle("WASAPI • Activo".into());
    }
    #[cfg(not(target_os = "windows"))]
    {
        ui.set_backend_title("Wayland / niri".into());
        ui.set_backend_subtitle(format!("PipeWire • {} FPS", config.video.fps).into());
    }

    // Inicializar perfiles de hardware por defecto (~/.config/sane-rec/profiles/)
    sane_rec::profiles_manager::init_default_profiles();
    update_profiles_model(&ui);

    let _ = sane_rec::update_niri_shortcuts(
        &config.shortcuts.save_replay,
        &config.shortcuts.toggle_record,
    );
    sane_rec::ipc::reload_windows_hotkeys(
        &config.shortcuts.save_replay,
        &config.shortcuts.toggle_record,
    );

    // En Windows, activar escucha global de teclas F9 y F10
    #[cfg(target_os = "windows")]
    sane_rec::ipc::spawn_global_hotkeys_listener();

    // Cargar salidas de pantalla, ventanas activas y dispositivos de audio descubiertos
    update_screens_model(&ui);
    update_windows_model(&ui);
    update_audio_devices_model(&ui);
    update_gallery_model(&ui, &config.storage.output_dir);

    // Candado atómico para evitar que múltiples clics abran diálogos superpuestos o bloqueen la UI
    let dialog_open_guard = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Verificar si el daemon ya se encuentra activo
    let daemon_running = ipc::send_command_sync("status").is_ok();

    let local_engine = if !daemon_running {
        let engine = Arc::new(ReplayEngine::new(config.clone()).map_err(|e| e.to_string())?);

        // Si el daemon no está corriendo, iniciamos el listener IPC en segundo plano
        // para que atajos globales de Niri o Windows (`sane-rec save` / `record_toggle`) funcionen con la GUI abierta
        let engine_for_ipc = engine.clone();
        let ui_weak_ipc = ui.as_weak();
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
                    let action = cmd.action.clone();
                    let was_running = engine_for_ipc.is_running();
                    dispatch_ipc_command(&engine_for_ipc, cmd).await;

                    // Sincronizar estado visual e informar en la GUI cuando se usa un atajo global
                    let status = engine_for_ipc.get_status();
                    let out_dir = engine_for_ipc.config().storage.output_dir;
                    let ui_weak_clone = ui_weak_ipc.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_clone.upgrade() {
                            ui.set_is_running(status.is_running);
                            ui.set_is_recording(status.is_recording);
                            if action == "save" {
                                if was_running {
                                    update_gallery_model(&ui, &out_dir);
                                    show_toast(&ui, "save", "Replay guardado mediante atajo global");
                                } else {
                                    show_toast(
                                        &ui,
                                        "warning",
                                        "Activa el 'Daemon replay' primero para guardar clips",
                                    );
                                }
                            } else if action == "record" || action == "record_toggle" {
                                if status.is_recording {
                                    show_toast(&ui, "record", "Grabación continua iniciada");
                                } else {
                                    update_gallery_model(&ui, &out_dir);
                                    show_toast(&ui, "stop", "Grabación continua guardada");
                                }
                            }
                        }
                    });
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

            match res {
                Ok(path) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                            ui.set_last_saved_clip(filename.into());
                            show_toast(&ui, "save", &format!("Replay guardado: {}", filename));
                        }
                        let out_dir = resolve_active_output_dir(&engine_clone, &output_dir);
                        update_gallery_model(&ui, &out_dir);
                    }
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        show_toast(&ui, "warning", &format!("No se pudo guardar clip: {}", e));
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
                show_toast(
                    &ui,
                    "memory",
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
                let was_running = eng.is_running();
                let mut cfg = eng.config();
                cfg.profile.active_profile = prof.to_string();
                let _ = cfg.save();
                eng.update_config(cfg);
                if was_running {
                    let _ = eng.stop_replay();
                    let _ = eng.start_replay();
                }
            }
        });
    }

    // Callback select-output-dir (Asíncrono en hilo separado + TopMost en Windows para no bloquear ni abrirse detrás)
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        let guard = dialog_open_guard.clone();
        ui.on_select_output_dir(move || {
            if guard
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_err()
            {
                return;
            }

            let ui_weak_bg = ui_weak.clone();
            let engine_bg = engine_clone.clone();
            let guard_bg = guard.clone();

            std::thread::spawn(move || {
                #[cfg(target_os = "windows")]
                let selected_path: Option<PathBuf> = {
                    use std::os::windows::process::CommandExt;
                    let ps_script = r#"
                    Add-Type -AssemblyName System.Windows.Forms
                    $owner = New-Object System.Windows.Forms.Form
                    $owner.TopMost = $true
                    $owner.StartPosition = [System.Windows.Forms.FormStartPosition]::CenterScreen
                    $owner.Width = 0
                    $owner.Height = 0
                    $owner.ShowInTaskbar = $false
                    $owner.Opacity = 0
                    $owner.Show()
                    $owner.Activate()
                    $dlg = New-Object System.Windows.Forms.FolderBrowserDialog
                    $dlg.Description = 'Seleccionar carpeta de Replays'
                    $dlg.ShowNewFolderButton = $true
                    if ($dlg.ShowDialog($owner) -eq [System.Windows.Forms.DialogResult]::OK) {
                        Write-Output $dlg.SelectedPath
                    }
                    $owner.Close()
                    "#;
                    let output = std::process::Command::new("powershell")
                        .args(["-NoProfile", "-Sta", "-WindowStyle", "Hidden", "-Command", ps_script])
                        .creation_flags(0x08000000)
                        .output();
                    match output {
                        Ok(out) if out.status.success() => {
                            let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                            if !path_str.is_empty() {
                                Some(PathBuf::from(path_str))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                };

                #[cfg(not(target_os = "windows"))]
                let selected_path: Option<PathBuf> = {
                    let output = std::process::Command::new("zenity")
                        .arg("--file-selection")
                        .arg("--directory")
                        .arg("--title=Seleccionar carpeta de Replays")
                        .output();
                    match output {
                        Ok(out) if out.status.success() => {
                            let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                            if !path_str.is_empty() {
                                Some(PathBuf::from(path_str))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                };

                guard_bg.store(false, std::sync::atomic::Ordering::SeqCst);

                if let Some(dir) = selected_path {
                    if let Some(ref eng) = engine_bg {
                        let mut cfg = eng.config();
                        cfg.storage.output_dir = dir.clone();
                        let _ = cfg.save();
                        eng.update_config(cfg);
                    } else if let Ok(mut cfg) = Config::load() {
                        cfg.storage.output_dir = dir.clone();
                        let _ = cfg.save();
                    }
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_bg.upgrade() {
                            ui.set_output_path(dir.to_string_lossy().to_string().into());
                            update_gallery_model(&ui, &dir);
                            show_toast(&ui, "folder", &format!("Carpeta seleccionada: {}", dir.display()));
                        }
                    });
                }
            });
        });
    }

    // Callback change-output-dir (Edición manual o tecla Enter en la ruta)
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_change_output_dir(move |path_str| {
            let trimmed = path_str.trim();
            if trimmed.is_empty() {
                return;
            }
            let dir = PathBuf::from(trimmed);
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.storage.output_dir = dir.clone();
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.storage.output_dir = dir.clone();
                let _ = cfg.save();
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_output_path(dir.to_string_lossy().to_string().into());
                update_gallery_model(&ui, &dir);
                show_toast(&ui, "folder", &format!("Carpeta de salida: {}", dir.display()));
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
                let out_dir = resolve_active_output_dir(&engine_clone, &output_dir_rec);
                if is_recording {
                    show_toast(&ui, "record", &format!("Grabación continua iniciada ({})", ui.get_shortcut_record()));
                } else {
                    show_toast(&ui, "stop", "Grabación continua finalizada y guardada");
                    update_gallery_model(&ui, &out_dir);
                }
            }
        });
    }

    // Callbacks para la Galería, Reproducción y Eliminación de Clips
    {
        let output_dir = config.storage.output_dir.clone();
        let engine_clone = local_engine.clone();
        ui.on_play_clip(move |path_str| {
            let current_dir = resolve_active_output_dir(&engine_clone, &output_dir);
            let p = PathBuf::from(path_str.as_str());
            let target = if p.is_absolute() && p.exists() {
                p
            } else {
                current_dir.join(path_str.as_str())
            };

            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                let path_arg = target.to_string_lossy().to_string();
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "", &path_arg])
                    .creation_flags(0x08000000)
                    .spawn();
            }

            #[cfg(not(target_os = "windows"))]
            {
                if std::process::Command::new("mpv").arg(&target).spawn().is_err() {
                    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
                }
            }
        });
    }

    {
        let output_dir = config.storage.output_dir.clone();
        let engine_clone = local_engine.clone();
        ui.on_open_clip_folder(move |path_str| {
            let current_dir = resolve_active_output_dir(&engine_clone, &output_dir);
            let p = PathBuf::from(path_str.as_str());
            let target_file = if p.is_absolute() && p.exists() {
                p.clone()
            } else {
                current_dir.join(path_str.as_str())
            };
            let target_dir = target_file
                .parent()
                .map(|parent| parent.to_path_buf())
                .unwrap_or(current_dir);

            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                if target_file.exists() {
                    let path_arg = target_file.to_string_lossy().to_string();
                    let _ = std::process::Command::new("explorer.exe")
                        .args(["/select,", &path_arg])
                        .creation_flags(0x08000000)
                        .spawn();
                } else {
                    let dir_arg = target_dir.to_string_lossy().to_string();
                    let _ = std::process::Command::new("explorer.exe")
                        .arg(&dir_arg)
                        .creation_flags(0x08000000)
                        .spawn();
                }
            }

            #[cfg(not(target_os = "windows"))]
            {
                if std::process::Command::new("cosmic-files").arg(&target_dir).spawn().is_err() {
                    let _ = std::process::Command::new("xdg-open").arg(target_dir).spawn();
                }
            }
        });
    }

    {
        let output_dir = config.storage.output_dir.clone();
        let engine_clone = local_engine.clone();
        ui.on_open_output_folder(move || {
            let current_dir = resolve_active_output_dir(&engine_clone, &output_dir);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                let path_arg = current_dir.to_string_lossy().to_string();
                let _ = std::process::Command::new("explorer.exe")
                    .arg(&path_arg)
                    .creation_flags(0x08000000)
                    .spawn();
            }

            #[cfg(not(target_os = "windows"))]
            {
                if std::process::Command::new("cosmic-files").arg(&current_dir).spawn().is_err() {
                    let _ = std::process::Command::new("xdg-open").arg(&current_dir).spawn();
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let output_dir = config.storage.output_dir.clone();
        let engine_clone = local_engine.clone();
        ui.on_refresh_gallery(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let current_dir = resolve_active_output_dir(&engine_clone, &output_dir);
                update_gallery_model(&ui, &current_dir);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let output_dir = config.storage.output_dir.clone();
        let engine_clone = local_engine.clone();
        ui.on_delete_clip(move |path_str| {
            let current_dir = resolve_active_output_dir(&engine_clone, &output_dir);
            let p = PathBuf::from(path_str.as_str());
            let target = if p.is_absolute() && p.exists() {
                p
            } else {
                current_dir.join(path_str.as_str())
            };

            match sane_rec::delete_clip(&target) {
                Ok(()) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        update_gallery_model(&ui, &current_dir);
                        show_toast(&ui, "delete", "Clip eliminado correctamente");
                    }
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        show_toast(&ui, "warning", &format!("No se pudo eliminar: {}", e));
                    }
                }
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
                    let was_running = eng.is_running();
                    let mut cfg = eng.config();
                    cfg.video.target = format!("area:{}", geo);
                    let _ = cfg.save();
                    eng.update_config(cfg);
                    if was_running {
                        let _ = eng.stop_replay();
                        let _ = eng.start_replay();
                    }
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
                let was_running = eng.is_running();
                let mut cfg = eng.config();
                cfg.video.target = screen.to_string();
                cfg.video.scale = resolution.to_string();
                let _ = cfg.save();
                eng.update_config(cfg);
                if was_running {
                    let _ = eng.stop_replay();
                    let _ = eng.start_replay();
                }
            } else if let Ok(mut cfg) = Config::load() {
                cfg.video.target = screen.to_string();
                cfg.video.scale = resolution.to_string();
                let _ = cfg.save();
            }
        });
    }

    {
        let engine_clone = local_engine.clone();
        ui.on_select_window(move |title, app_id| {
            #[cfg(target_os = "windows")]
            let chosen = if !title.is_empty() {
                title.as_str()
            } else {
                app_id.as_str()
            };
            #[cfg(not(target_os = "windows"))]
            let chosen = if !app_id.is_empty() {
                app_id.as_str()
            } else {
                title.as_str()
            };
            let target = format!("window:{}", chosen);
            if let Some(ref eng) = engine_clone {
                let was_running = eng.is_running();
                let mut cfg = eng.config();
                cfg.video.target = target;
                let _ = cfg.save();
                eng.update_config(cfg);
                if was_running {
                    let _ = eng.stop_replay();
                    let _ = eng.start_replay();
                }
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
                show_toast(&ui, "memory", &format!("Búfer de repetición fijado a {}s", d));
            }
        });
    }

    // Callback update-shortcuts
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_update_shortcuts(move |save_key, rec_key| {
            let save_str = save_key.as_str().trim();
            let rec_str = rec_key.as_str().trim();

            if !save_str.is_empty() && save_str.eq_ignore_ascii_case(rec_str) {
                if let Some(ui) = ui_weak.upgrade() {
                    show_toast(
                        &ui,
                        "warning",
                        "El atajo de Guardar Replay y Grabar no pueden ser la misma tecla",
                    );
                }
                return;
            }

            let _ = sane_rec::update_niri_shortcuts(save_str, rec_str);

            #[cfg(target_os = "windows")]
            sane_rec::ipc::reload_windows_hotkeys(save_str, rec_str);

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

            #[cfg(target_os = "windows")]
            let toast_msg = format!("Atajos globales actualizados ({}, {})", save_str, rec_str);
            #[cfg(not(target_os = "windows"))]
            let toast_msg = format!("Atajos actualizados en niri ({}, {})", save_str, rec_str);

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_shortcut_save(save_str.into());
                ui.set_shortcut_record(rec_str.into());
                ui.set_custom_save_text(save_str.into());
                ui.set_custom_record_text(rec_str.into());
                show_toast(&ui, "keyboard", &toast_msg);
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
                show_toast(&ui, "settings", &format!("Códec personalizado: {}", c));
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
                show_toast(&ui, "bolt", &format!("Preset CPU: {}", p));
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
                show_toast(&ui, "tune", &format!("Calidad CRF: {}", c));
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
                show_toast(&ui, "memory", &format!("Límite búfer RAM: {} MB", ram));
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
                        let was_running = eng.is_running();
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
                        if was_running {
                            let _ = eng.stop_replay();
                            let _ = eng.start_replay();
                        }
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
                    show_toast(&ui, "settings", &format!("Perfil '{}' cargado correctamente", profile.name));
                }
                Err(e) => {
                    show_toast(&ui, "warning", &format!("Error al cargar perfil: {}", e));
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_load_system_profile(move |name| {
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
                        let was_running = eng.is_running();
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
                        if was_running {
                            let _ = eng.stop_replay();
                            let _ = eng.start_replay();
                        }
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
                    show_toast(&ui, "settings", &format!("Perfil '{}' cargado correctamente", profile.name));
                }
                Err(e) => {
                    show_toast(&ui, "warning", &format!("Error al cargar perfil: {}", e));
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
                    show_toast(&ui, "save", &format!("Perfil '{}' guardado", profile_name));
                }
                Err(e) => {
                    show_toast(&ui, "warning", &format!("Error al guardar perfil: {}", e));
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let guard = dialog_open_guard.clone();
        ui.on_export_hardware_profile(move |name| {
            if guard
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_err()
            {
                return;
            }
            let Some(ui) = ui_weak.upgrade() else {
                guard.store(false, std::sync::atomic::Ordering::SeqCst);
                return;
            };
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
            let ui_weak_bg = ui_weak.clone();
            let guard_bg = guard.clone();

            std::thread::spawn(move || {
                #[cfg(target_os = "windows")]
                let output_path: Option<std::path::PathBuf> = {
                    use std::os::windows::process::CommandExt;
                    let ps_script = format!(
                        r#"
                        Add-Type -AssemblyName System.Windows.Forms
                        $owner = New-Object System.Windows.Forms.Form
                        $owner.TopMost = $true
                        $owner.Width = 0
                        $owner.Height = 0
                        $owner.ShowInTaskbar = $false
                        $owner.Opacity = 0
                        $owner.Show()
                        $owner.Activate()
                        $dlg = New-Object System.Windows.Forms.SaveFileDialog
                        $dlg.Title = 'Exportar Perfil de Hardware'
                        $dlg.Filter = 'Archivos TOML (*.toml)|*.toml'
                        $dlg.FileName = '{}'
                        if ($dlg.ShowDialog($owner) -eq [System.Windows.Forms.DialogResult]::OK) {{
                            Write-Output $dlg.FileName
                        }}
                        $owner.Close()
                        "#,
                        default_filename
                    );
                    let output = std::process::Command::new("powershell")
                        .args(["-NoProfile", "-Sta", "-WindowStyle", "Hidden", "-Command", &ps_script])
                        .creation_flags(0x08000000)
                        .output();
                    match output {
                        Ok(o) if o.status.success() => {
                            let path_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
                            if !path_str.is_empty() {
                                Some(std::path::PathBuf::from(path_str))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                };

                #[cfg(not(target_os = "windows"))]
                let output_path: Option<std::path::PathBuf> = {
                    let output = std::process::Command::new("zenity")
                        .args([
                            "--file-selection",
                            "--save",
                            "--confirm-overwrite",
                            "--title=Exportar Perfil de Hardware",
                            &format!("--filename={}", default_filename),
                        ])
                        .output();
                    match output {
                        Ok(o) if o.status.success() => {
                            let path_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
                            if !path_str.is_empty() {
                                Some(std::path::PathBuf::from(path_str))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                };

                guard_bg.store(false, std::sync::atomic::Ordering::SeqCst);

                if let Some(path) = output_path {
                    let res = sane_rec::profiles_manager::export_profile(&profile, &path);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_bg.upgrade() {
                            match res {
                                Ok(_) => {
                                    show_toast(&ui, "save", &format!("Perfil exportado a {}", path.display()));
                                }
                                Err(e) => {
                                    show_toast(&ui, "warning", &format!("Error al exportar: {}", e));
                                }
                            }
                        }
                    });
                }
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        let guard = dialog_open_guard.clone();
        ui.on_import_hardware_profile(move || {
            if guard
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_err()
            {
                return;
            }
            let ui_weak_bg = ui_weak.clone();
            let engine_bg = engine_clone.clone();
            let guard_bg = guard.clone();

            std::thread::spawn(move || {
                #[cfg(target_os = "windows")]
                let input_path: Option<std::path::PathBuf> = {
                    use std::os::windows::process::CommandExt;
                    let ps_script = r#"
                    Add-Type -AssemblyName System.Windows.Forms
                    $owner = New-Object System.Windows.Forms.Form
                    $owner.TopMost = $true
                    $owner.Width = 0
                    $owner.Height = 0
                    $owner.ShowInTaskbar = $false
                    $owner.Opacity = 0
                    $owner.Show()
                    $owner.Activate()
                    $dlg = New-Object System.Windows.Forms.OpenFileDialog
                    $dlg.Title = 'Seleccionar Perfil de Hardware (.toml)'
                    $dlg.Filter = 'Archivos TOML (*.toml)|*.toml'
                    if ($dlg.ShowDialog($owner) -eq [System.Windows.Forms.DialogResult]::OK) {
                        Write-Output $dlg.FileName
                    }
                    $owner.Close()
                    "#;
                    let output = std::process::Command::new("powershell")
                        .args(["-NoProfile", "-Sta", "-WindowStyle", "Hidden", "-Command", ps_script])
                        .creation_flags(0x08000000)
                        .output();
                    match output {
                        Ok(o) if o.status.success() => {
                            let path_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
                            if !path_str.is_empty() {
                                Some(std::path::PathBuf::from(path_str))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                };

                #[cfg(not(target_os = "windows"))]
                let input_path: Option<std::path::PathBuf> = {
                    let output = std::process::Command::new("zenity")
                        .args([
                            "--file-selection",
                            "--title=Seleccionar Perfil de Hardware (.toml)",
                            "--file-filter=Archivos TOML (*.toml) | *.toml",
                        ])
                        .output();
                    match output {
                        Ok(o) if o.status.success() => {
                            let path_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
                            if !path_str.is_empty() {
                                Some(std::path::PathBuf::from(path_str))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                };

                guard_bg.store(false, std::sync::atomic::Ordering::SeqCst);

                if let Some(path) = input_path {
                    let import_res = sane_rec::profiles_manager::import_profile(&path);
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(ui) = ui_weak_bg.upgrade() else { return; };
                        match import_res {
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

                                if let Some(ref eng) = engine_bg {
                                    let was_running = eng.is_running();
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
                                    if was_running {
                                        let _ = eng.stop_replay();
                                        let _ = eng.start_replay();
                                    }
                                }
                                show_toast(&ui, "check", &format!("Perfil '{}' importado exitosamente", profile.name));
                            }
                            Err(e) => {
                                show_toast(&ui, "warning", &format!("Error al importar perfil: {}", e));
                            }
                        }
                    });
                }
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_refresh_hardware_profiles(move || {
            if let Some(ui) = ui_weak.upgrade() {
                update_profiles_model(&ui);
                show_toast(&ui, "refresh", "Lista de perfiles actualizada");
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
                let cleaned_notes = sane_rec::clean_changelog_markdown(&info.body);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_update_version(info.version.into());
                        ui.set_update_release_notes(cleaned_notes.into());
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
                show_toast(&ui, "rocket", "Descargando e instalando actualización...");
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
                                show_toast(&ui, "check", &format!("¡Actualizado a {}! Reinicia la app para aplicar cambios.", v));
                            }
                            Err(e) => {
                                show_toast(&ui, "warning", &format!("Error al actualizar: {}", e));
                            }
                        }
                    }
                });
            });
        });
    }

    // -------------------------------------------------------------------------
    // Modo Doble Clip & Compresor Discord
    // -------------------------------------------------------------------------
    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_toggle_dual_clip(move |enabled| {
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.storage.dual_clip = enabled;
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.storage.dual_clip = enabled;
                let _ = cfg.save();
            }
            let _ = ipc::send_command_sync("reload_config");

            if let Some(ui) = ui_weak.upgrade() {
                let msg = if enabled {
                    "Modo Doble Clip activado (copia Discord automática)"
                } else {
                    "Modo Doble Clip desactivado"
                };
                show_toast(&ui, "chat", msg);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        ui.on_set_discord_target_mb(move |target_mb| {
            let mb = target_mb.max(1) as u32;
            if let Some(ref eng) = engine_clone {
                let mut cfg = eng.config();
                cfg.storage.discord_target_mb = mb;
                let _ = cfg.save();
                eng.update_config(cfg);
            } else if let Ok(mut cfg) = Config::load() {
                cfg.storage.discord_target_mb = mb;
                let _ = cfg.save();
            }
            let _ = ipc::send_command_sync("reload_config");

            if let Some(ui) = ui_weak.upgrade() {
                show_toast(&ui, "chat", &format!("Límite para Discord fijado a {} MB", mb));
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        let out_dir_clone = config.storage.output_dir.clone();
        ui.on_compress_clip_for_discord(move |path_str| {
            let path = PathBuf::from(path_str.as_str());
            let target_mb = Config::load()
                .map(|c| c.storage.discord_target_mb)
                .unwrap_or_else(|_| {
                    if let Some(ref eng) = engine_clone {
                        eng.config().storage.discord_target_mb
                    } else {
                        10
                    }
                });

            if let Some(ui) = ui_weak.upgrade() {
                show_toast(&ui, "chat", "Generando versión optimizada para Discord...");
            }

            let ui_weak_bg = ui_weak.clone();
            let target_dir = resolve_active_output_dir(&engine_clone, &out_dir_clone);

            std::thread::spawn(move || {
                let out_path = sane_rec::get_discord_clip_path(&path);
                match sane_rec::compress_to_target_size(&path, &out_path, target_mb as f64) {
                    Ok(out) => {
                        let size_mb = out
                            .metadata()
                            .map(|m| (m.len() as f64) / (1024.0 * 1024.0))
                            .unwrap_or(0.0);
                        #[allow(unused_variables)]
                        let filename = out
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("clip_discord.mp4");

                        #[cfg(not(target_os = "windows"))]
                        {
                            let _ = notify_rust::Notification::new()
                                .appname("sane-rec")
                                .summary("Versión Discord Lista")
                                .body(&format!("Guardado: {}\nPeso: {:.1} MB", filename, size_mb))
                                .icon("video-x-generic")
                                .show();
                        }

                        sane_rec::engine::play_feedback_sound("message");
                        copy_file_to_clipboard(&out);

                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak_bg.upgrade() {
                                update_gallery_model(&ui, &target_dir);
                                show_toast(&ui, "chat", &format!("Clip Discord listo: {:.1} MB (Copiado)", size_mb));
                            }
                        });
                    }
                    Err(e) => {
                        sane_rec::crash_handler::write_crash_report("Error al comprimir clip para Discord", &e);
                    }
                }
            });
        });
    }

    // Inicia monitor de picos de audio en tiempo real para calibración visual (VU meter)
    let audio_monitor = std::sync::Arc::new(sane_rec::AudioPeakMonitor::new(None, None));

    // Temporizador Slint para actualizar métricas en vivo y medidores VU.
    // Solo redibuja picos de audio cuando el usuario está en la pestaña Audio (active_view == 2)
    // y el valor cambió > 0.008, evitando que MSI Afterburner / RivaTuner (RTSS) enganche a sane-rec como juego 3D activo.
    let _timer = {
        let ui_weak = ui.as_weak();
        let engine_clone = local_engine.clone();
        let audio_mon_clone = audio_monitor.clone();
        let timer = slint::Timer::default();
        let mut ticks: u32 = 0;
        let mut last_sys_peak: f32 = -1.0;
        let mut last_mic_peak: f32 = -1.0;
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                // Solo actualiza niveles visuales VU si la pestaña de Audio (vista 2) está visible
                if ui.get_active_view() == 2 {
                    let sys_peak = audio_mon_clone.system_peak();
                    let mic_peak = audio_mon_clone.mic_peak();
                    if (sys_peak - last_sys_peak).abs() > 0.008 {
                        ui.set_system_volume_peak(sys_peak);
                        last_sys_peak = sys_peak;
                    }
                    if (mic_peak - last_mic_peak).abs() > 0.008 {
                        ui.set_mic_volume_peak(mic_peak);
                        last_mic_peak = mic_peak;
                    }
                }

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

/// Actualiza las opciones de salida y micrófonos disponibles en PipeWire / PulseAudio / WASAPI.
fn update_audio_devices_model(ui: &AppWindow) {
    let (monitors, microphones) = sane_rec::get_audio_devices();
    if !monitors.is_empty() {
        let sink_entries: Vec<AudioDeviceEntry> = monitors
            .iter()
            .map(|m| {
                let badge = if m.is_default { "Por defecto" } else { "Salida" };
                AudioDeviceEntry {
                    name: m.name.clone().into(),
                    description: m.description.clone().into(),
                    badge: badge.into(),
                    is_default: m.is_default,
                }
            })
            .collect();
        ui.set_available_sinks(std::rc::Rc::new(slint::VecModel::from(sink_entries)).into());

        // Sincronizar sink en la UI con el predeterminado o primer dispositivo detectado
        let default_sink = monitors.iter().find(|m| m.is_default).unwrap_or(&monitors[0]);
        ui.set_audio_sink(default_sink.description.clone().into());
    } else {
        ui.set_audio_sink("Audio del sistema (Predeterminado)".into());
    }

    let mut mic_entries: Vec<AudioDeviceEntry> = microphones
        .iter()
        .map(|m| {
            let badge = if m.is_default { "Por defecto" } else { "Entrada" };
            AudioDeviceEntry {
                name: m.name.clone().into(),
                description: m.description.clone().into(),
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

    // Sincronizar micrófono en la UI con el predeterminado o primer micrófono detectado
    if let Some(first_mic) = microphones.iter().find(|m| m.is_default).or_else(|| microphones.first()) {
        ui.set_audio_mic_source(first_mic.description.clone().into());
    } else {
        ui.set_audio_mic_source("Micrófono predeterminado".into());
    }
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

/// Obtiene la duración real del clip de video formateada como MM:SS usando ffprobe sin ventanas CMD emergentes.
fn get_clip_duration_str(path: &std::path::Path) -> String {
    let ffprobe_bin = sane_rec::resolve_binary("ffprobe");
    let mut cmd = std::process::Command::new(ffprobe_bin);
    cmd.args([
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ])
    .arg(path);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let output = cmd.output();

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
                let is_discord = c.name.contains("_discord");

                ClipEntry {
                    filename: c.name.into(),
                    file_size: format!("{:.1} MB", c.size_mb).into(),
                    date_text: date_str.into(),
                    duration_text: duration_text.into(),
                    filepath: c.path.to_string_lossy().to_string().into(),
                    is_discord,
                }
            })
            .collect();
        let recent_entries: Vec<ClipEntry> = entries.iter().take(3).cloned().collect();
        ui.set_gallery_clips(std::rc::Rc::new(slint::VecModel::from(entries)).into());
        ui.set_recent_clips(std::rc::Rc::new(slint::VecModel::from(recent_entries)).into());
    }
}

/// Copia la ruta de un archivo de vídeo al portapapeles del sistema (wl-copy en Linux / Set-Clipboard en Windows).
#[allow(dead_code)]
fn copy_file_to_clipboard(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    {
        let path_str = path.to_string_lossy();
        let _ = std::process::Command::new("wl-copy")
            .arg(path_str.as_ref())
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let path_str = path.to_string_lossy().to_string();
        let escaped = path_str.replace("'", "''");
        let ps_cmd = format!("Set-Clipboard -Path '{}'", escaped);
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
            .creation_flags(0x08000000)
            .spawn();
    }
}

/// Resuelve el directorio de salida actual desde el motor en memoria, la configuración en disco o el fallback inicial.
fn resolve_active_output_dir(
    engine: &Option<Arc<ReplayEngine>>,
    fallback: &std::path::Path,
) -> PathBuf {
    if let Some(ref eng) = engine {
        eng.config().storage.output_dir
    } else if let Ok(cfg) = Config::load() {
        cfg.storage.output_dir
    } else {
        fallback.to_path_buf()
    }
}

/// Actualiza el modelo de perfiles de hardware disponibles en la interfaz.
fn update_profiles_model(ui: &AppWindow) {
    let profiles = sane_rec::profiles_manager::list_profiles();
    if ui.get_selected_profile_name().is_empty() {
        if let Some(first) = profiles.first() {
            ui.set_selected_profile_name(first.clone().into());
        }
    }
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
        assert_eq!(ui.get_output_path(), "");
        assert_eq!(ui.get_audio_sink(), "Audio del sistema (Predeterminado)");
        assert_eq!(ui.get_audio_mic_source(), "Micrófono predeterminado");

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

        assert_eq!(ui.get_shortcut_save(), "F10");
        assert_eq!(ui.get_shortcut_record(), "F9");
        ui.set_shortcut_save("Ctrl+F12".into());
        assert_eq!(ui.get_shortcut_save(), "Ctrl+F12");
        ui.set_shortcut_save("F10".into());

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
        assert_eq!(ui.get_shortcut_save(), "F10");
        assert_eq!(ui.get_shortcut_record(), "F9");
        assert_eq!(ui.get_max_memory_mb(), 150);
        assert_eq!(ui.get_custom_codec(), "libx264");
        assert_eq!(ui.get_custom_preset(), "ultrafast");
        assert_eq!(ui.get_custom_crf(), 23);

        // Modo doble clip y compresión Discord
        assert_eq!(ui.get_dual_clip(), false);
        ui.set_dual_clip(true);
        assert_eq!(ui.get_dual_clip(), true);

        assert_eq!(ui.get_discord_target_mb(), 10);
        ui.set_discord_target_mb(15);
        assert_eq!(ui.get_discord_target_mb(), 15);

        // Verificación de propiedades de Backend multiplataforma
        assert_eq!(ui.get_backend_title(), "Wayland / niri");
        ui.set_backend_title("Windows DXGI".into());
        assert_eq!(ui.get_backend_title(), "Windows DXGI");
        ui.set_backend_subtitle("WASAPI • Activo".into());
        assert_eq!(ui.get_backend_subtitle(), "WASAPI • Activo");

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
