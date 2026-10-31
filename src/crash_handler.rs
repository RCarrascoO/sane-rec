use std::backtrace::Backtrace;
use std::fs;
use std::path::PathBuf;

/// Ruta al archivo principal de registro de fallos (`~/.config/sane-rec/crash.log`).
pub fn get_crash_log_path() -> PathBuf {
    if let Some(user_dirs) = directories::BaseDirs::new() {
        let dir = user_dirs.config_dir().join("sane-rec");
        let _ = fs::create_dir_all(&dir);
        dir.join("crash.log")
    } else {
        PathBuf::from("/tmp/sane-rec-crash.log")
    }
}

/// Ruta al archivo de log del motor de grabación (`~/.config/sane-rec/last_engine.log`).
pub fn get_engine_log_path() -> PathBuf {
    if let Some(user_dirs) = directories::BaseDirs::new() {
        let dir = user_dirs.config_dir().join("sane-rec");
        let _ = fs::create_dir_all(&dir);
        dir.join("last_engine.log")
    } else {
        PathBuf::from("/tmp/sane-rec-engine.log")
    }
}

/// Directorio histórico de logs (`~/.config/sane-rec/logs/`).
pub fn get_logs_dir() -> PathBuf {
    if let Some(user_dirs) = directories::BaseDirs::new() {
        let dir = user_dirs.config_dir().join("sane-rec").join("logs");
        let _ = fs::create_dir_all(&dir);
        dir
    } else {
        let dir = PathBuf::from("/tmp/sane-rec-logs");
        let _ = fs::create_dir_all(&dir);
        dir
    }
}

/// Escribe un informe de crash exhaustivo y notifica visualmente al usuario.
pub fn write_crash_report(reason: &str, details: &str) {
    let now = chrono::Local::now();
    let timestamp_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let file_timestamp = now.format("%Y%m%d_%H%M%S").to_string();

    // Recopilar información del entorno del sistema
    let os_info = std::fs::read_to_string("/etc/os-release")
        .unwrap_or_else(|_| "Desconocido".to_string());
    let pretty_name = os_info
        .lines()
        .find(|l| l.starts_with("PRETTY_NAME="))
        .map(|l| l.trim_start_matches("PRETTY_NAME=").trim_matches('"'))
        .unwrap_or("Linux");

    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "desconocido".into());
    let current_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "desconocido".into());
    let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "no detectado".into());

    // Leer últimas líneas del log del motor si existe
    let engine_log_path = get_engine_log_path();
    let engine_tail = if engine_log_path.exists() {
        std::fs::read_to_string(&engine_log_path)
            .unwrap_or_default()
            .lines()
            .rev()
            .take(30)
            .collect::<Vec<&str>>()
            .into_iter()
            .rev()
            .collect::<Vec<&str>>()
            .join("\n")
    } else {
        "Sin registros de motor previos.".to_string()
    };

    let report = format!(
        r#"================================================================================
                    sane-rec — INFORME DE ERROR CRÍTICO (CRASH LOG)
================================================================================
Fecha y Hora:    {}
Versión de App:  sane-rec v{}
Sistema Operativo: {}
Entorno Gráfico: {} ({})
Display Wayland: {}

--------------------------------------------------------------------------------
🚨 RAZÓN DEL FALLO:
--------------------------------------------------------------------------------
{}

--------------------------------------------------------------------------------
🔍 DETALLES TÉCNICOS / BACKTRACE:
--------------------------------------------------------------------------------
{}

--------------------------------------------------------------------------------
📺 ÚLTIMOS REGISTROS DEL MOTOR (wf-recorder / GStreamer):
--------------------------------------------------------------------------------
{}
================================================================================
"#,
        timestamp_str,
        env!("CARGO_PKG_VERSION"),
        pretty_name,
        current_desktop,
        session_type,
        wayland_display,
        reason.trim(),
        details.trim(),
        engine_tail.trim()
    );

    // 1. Guardar en crash.log principal
    let main_log = get_crash_log_path();
    let _ = fs::write(&main_log, &report);

    // 2. Guardar copia histórica en ~/.config/sane-rec/logs/crash_YYYYMMDD_HHMMSS.log
    let history_log = get_logs_dir().join(format!("crash_{}.log", file_timestamp));
    let _ = fs::write(&history_log, &report);

    // 3. Imprimir a stderr
    eprintln!("\n❌ [sane-rec CRASH DETECTADO]\n{}\n", report);

    // 4. Disparar notificación visual de escritorio de alta prioridad
    let short_reason = if reason.len() > 120 {
        format!("{}...", &reason[..117])
    } else {
        reason.to_string()
    };

    let notif_body = format!(
        "Razón: {}\n\nLogs guardados en:\n{}",
        short_reason,
        main_log.display()
    );

    let _ = notify_rust::Notification::new()
        .appname("sane-rec")
        .summary("💥 sane-rec: Error Crítico")
        .body(&notif_body)
        .icon("dialog-error")
        .urgency(notify_rust::Urgency::Critical)
        .timeout(12000)
        .show();
}

/// Registra el Panic Hook global de Rust para atrapar cualquier error no controlado.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        let reason = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Error de pánico interno no especificado".to_string()
        };

        let location = panic_info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "Ubicación desconocida".to_string());

        let full_reason = format!("Pánico en [{}]: {}", location, reason);
        let backtrace = Backtrace::capture();
        let details = format!("Backtrace:\n{:?}", backtrace);

        write_crash_report(&full_reason, &details);
    }));
}

/// Registra un fallo del motor de grabación (wf-recorder) cuando termina de manera inesperada.
pub fn log_engine_failure(context: &str, exit_status: Option<i32>) {
    let reason = match exit_status {
        Some(code) => format!(
            "El proceso del motor (wf-recorder) terminó con código de error {} durante '{}'",
            code, context
        ),
        None => format!(
            "El proceso del motor (wf-recorder) fue terminado abruptamente por una señal durante '{}'",
            context
        ),
    };

    let engine_log_path = get_engine_log_path();
    let details = if engine_log_path.exists() {
        std::fs::read_to_string(&engine_log_path)
            .unwrap_or_else(|e| format!("No se pudo leer log: {}", e))
    } else {
        "No se encontraron logs del motor.".to_string()
    };

    write_crash_report(&reason, &details);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crash_log_paths_exist_or_can_be_created() {
        let crash_path = get_crash_log_path();
        assert!(crash_path.parent().unwrap().exists());

        let engine_path = get_engine_log_path();
        assert!(engine_path.parent().unwrap().exists());

        let logs_dir = get_logs_dir();
        assert!(logs_dir.exists());
    }

    #[test]
    fn test_write_crash_report_creates_file() {
        let test_reason = "Test Crash Verification";
        let test_details = "Detalle simulado de prueba para verificación de informe.";
        write_crash_report(test_reason, test_details);

        let path = get_crash_log_path();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Test Crash Verification"));
        assert!(content.contains("INFORME DE ERROR CRÍTICO"));
    }
}
