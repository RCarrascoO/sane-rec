#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// Solicitud enviada por el cliente al socket IPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcRequest {
    pub action: String,
}

/// Respuesta devuelta por el servidor IPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IpcResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl IpcResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            status: "ok".to_string(),
            message: Some(message.into()),
            data: None,
        }
    }

    pub fn ok_with_data(message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            status: "ok".to_string(),
            message: Some(message.into()),
            data: Some(data),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            message: Some(message.into()),
            data: None,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == "ok"
    }

    pub fn is_error(&self) -> bool {
        self.status == "error"
    }
}

/// Comando IPC despachado hacia el bucle principal de sane-rec mediante Tokio mpsc.
#[derive(Debug)]
pub struct IpcCommand {
    pub action: String,
    pub responder: tokio::sync::oneshot::Sender<IpcResponse>,
}

impl IpcCommand {
    pub fn new(action: impl Into<String>, responder: tokio::sync::oneshot::Sender<IpcResponse>) -> Self {
        Self {
            action: action.into(),
            responder,
        }
    }

    pub fn reply(self, response: IpcResponse) {
        let _ = self.responder.send(response);
    }

    pub fn ok(self, message: impl Into<String>) {
        let _ = self.responder.send(IpcResponse::ok(message));
    }

    pub fn ok_with_data(self, message: impl Into<String>, data: serde_json::Value) {
        let _ = self.responder.send(IpcResponse::ok_with_data(message, data));
    }

    pub fn error(self, message: impl Into<String>) {
        let _ = self.responder.send(IpcResponse::error(message));
    }
}

#[derive(Debug)]
pub enum IpcError {
    Io(std::io::Error),
    Json(serde_json::Error),
    ChannelClosed,
    Custom(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::Io(e) => write!(f, "IPC IO error: {}", e),
            IpcError::Json(e) => write!(f, "IPC JSON error: {}", e),
            IpcError::ChannelClosed => write!(f, "IPC internal channel closed"),
            IpcError::Custom(s) => write!(f, "IPC error: {}", s),
        }
    }
}

impl std::error::Error for IpcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IpcError::Io(e) => Some(e),
            IpcError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for IpcError {
    fn from(e: std::io::Error) -> Self {
        IpcError::Io(e)
    }
}

impl From<serde_json::Error> for IpcError {
    fn from(e: serde_json::Error) -> Self {
        IpcError::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, IpcError>;

/// Retorna la ruta al socket Unix `/run/user/<UID>/sane-rec.sock` en Linux, o el endpoint TCP en Windows.
#[cfg(target_os = "linux")]
pub fn socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("sane-rec.sock")
    } else {
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/run/user/{}/sane-rec.sock", uid))
    }
}

#[cfg(not(target_os = "linux"))]
pub fn socket_path() -> PathBuf {
    PathBuf::from("127.0.0.1:45892")
}

/// Envía una acción mediante IPC por defecto de forma asíncrona y espera la respuesta.
#[cfg(target_os = "linux")]
pub async fn send_command(action: &str) -> Result<IpcResponse> {
    send_command_at(&socket_path(), action).await
}

#[cfg(target_os = "windows")]
pub async fn send_command(action: &str) -> Result<IpcResponse> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let mut stream = tokio::net::TcpStream::connect("127.0.0.1:45892").await?;
    let req = IpcRequest {
        action: action.to_string(),
    };
    let mut req_bytes = serde_json::to_vec(&req)?;
    req_bytes.push(b'\n');
    stream.write_all(&req_bytes).await?;
    stream.flush().await?;

    let (reader, _) = stream.split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    let bytes_read = buf_reader.read_line(&mut line).await?;
    if bytes_read == 0 {
        return Err(IpcError::Custom(
            "Connection closed by server without response".to_string(),
        ));
    }

    let resp: IpcResponse = serde_json::from_str(&line)?;
    Ok(resp)
}

/// Envía una acción a un socket Unix en una ruta específica de forma asíncrona.
#[cfg(unix)]
pub async fn send_command_at(path: &Path, action: &str) -> Result<IpcResponse> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let mut stream = tokio::net::UnixStream::connect(path).await?;
    let req = IpcRequest {
        action: action.to_string(),
    };
    let mut req_bytes = serde_json::to_vec(&req)?;
    req_bytes.push(b'\n');
    stream.write_all(&req_bytes).await?;
    stream.flush().await?;

    let (reader, _) = stream.split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    let bytes_read = buf_reader.read_line(&mut line).await?;
    if bytes_read == 0 {
        return Err(IpcError::Custom(
            "Connection closed by server without response".to_string(),
        ));
    }

    let resp: IpcResponse = serde_json::from_str(&line)?;
    Ok(resp)
}

/// Envía una acción mediante IPC por defecto de forma síncrona.
#[cfg(target_os = "linux")]
pub fn send_command_sync(action: &str) -> Result<IpcResponse> {
    send_command_sync_at(&socket_path(), action)
}

#[cfg(target_os = "windows")]
pub fn send_command_sync(action: &str) -> Result<IpcResponse> {
    use std::io::{BufRead, BufReader, Write};
    use std::net::SocketAddr;
    use std::time::Duration;

    let (conn_timeout, rw_timeout) = if action == "status" {
        (Duration::from_millis(500), Duration::from_millis(1500))
    } else {
        (Duration::from_millis(1500), Duration::from_secs(5))
    };

    let addr: SocketAddr = "127.0.0.1:45892".parse().map_err(|e| {
        IpcError::Custom(format!("Invalid socket address: {}", e))
    })?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, conn_timeout)?;
    let _ = stream.set_read_timeout(Some(rw_timeout));
    let _ = stream.set_write_timeout(Some(rw_timeout));

    let req = IpcRequest {
        action: action.to_string(),
    };
    let mut req_bytes = serde_json::to_vec(&req)?;
    req_bytes.push(b'\n');
    stream.write_all(&req_bytes)?;
    stream.flush()?;

    let mut buf_reader = BufReader::new(stream);
    let mut line = String::new();
    let bytes_read = buf_reader.read_line(&mut line)?;
    if bytes_read == 0 {
        return Err(IpcError::Custom(
            "Connection closed by server without response".to_string(),
        ));
    }

    let resp: IpcResponse = serde_json::from_str(&line)?;
    Ok(resp)
}

/// Envía una acción a un socket Unix en una ruta específica de forma síncrona.
#[cfg(unix)]
pub fn send_command_sync_at(path: &Path, action: &str) -> Result<IpcResponse> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let (_conn_timeout, rw_timeout) = if action == "status" {
        (Duration::from_millis(500), Duration::from_millis(1500))
    } else {
        (Duration::from_millis(1500), Duration::from_secs(5))
    };

    let mut stream = UnixStream::connect(path)?;
    let _ = stream.set_read_timeout(Some(rw_timeout));
    let _ = stream.set_write_timeout(Some(rw_timeout));

    let req = IpcRequest {
        action: action.to_string(),
    };
    let mut req_bytes = serde_json::to_vec(&req)?;
    req_bytes.push(b'\n');
    stream.write_all(&req_bytes)?;
    stream.flush()?;

    let mut buf_reader = BufReader::new(stream);
    let mut line = String::new();
    let bytes_read = buf_reader.read_line(&mut line)?;
    if bytes_read == 0 {
        return Err(IpcError::Custom(
            "Connection closed by server without response".to_string(),
        ));
    }

    let resp: IpcResponse = serde_json::from_str(&line)?;
    Ok(resp)
}

/// Inicia el listener IPC en Linux (Unix domain socket) o en Windows (TCP localhost 45892).
#[cfg(target_os = "linux")]
pub async fn start_listener(tx: tokio::sync::mpsc::Sender<IpcCommand>) -> Result<()> {
    start_listener_at(&socket_path(), tx).await
}

#[cfg(target_os = "windows")]
pub async fn start_listener(tx: tokio::sync::mpsc::Sender<IpcCommand>) -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:45892").await?;

    loop {
        let (mut stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => break,
        };

        let tx = tx.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

            let (reader, mut writer) = stream.split();
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();

            match buf_reader.read_line(&mut line).await {
                Ok(0) => {}
                Ok(_) => {
                    let req: IpcRequest = match serde_json::from_str(&line) {
                        Ok(r) => r,
                        Err(e) => {
                            let resp = IpcResponse::error(format!("Invalid request format: {}", e));
                            if let Ok(mut bytes) = serde_json::to_vec(&resp) {
                                bytes.push(b'\n');
                                let _ = writer.write_all(&bytes).await;
                                let _ = writer.flush().await;
                            }
                            return;
                        }
                    };

                    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                    let cmd = IpcCommand {
                        action: req.action,
                        responder: resp_tx,
                    };

                    let response = if tx.send(cmd).await.is_err() {
                        IpcResponse::error("Server backend unavailable or shutting down")
                    } else {
                        match resp_rx.await {
                            Ok(resp) => resp,
                            Err(_) => {
                                IpcResponse::error("Backend dropped request without response")
                            }
                        }
                    };

                    if let Ok(mut bytes) = serde_json::to_vec(&response) {
                        bytes.push(b'\n');
                        let _ = writer.write_all(&bytes).await;
                        let _ = writer.flush().await;
                    }
                }
                Err(_) => {}
            }
        });
    }

    Ok(())
}

pub const WIN_MOD_ALT: u32 = 0x0001;
pub const WIN_MOD_CONTROL: u32 = 0x0002;
pub const WIN_MOD_SHIFT: u32 = 0x0004;
pub const WIN_MOD_WIN: u32 = 0x0008;
pub const WIN_MOD_NOREPEAT: u32 = 0x4000;

/// Parsea un string de atajos textual a `(modifiers, vk_code)`.
/// Soporta cadenas como `"F9"`, `"F10"`, `"Mod+F10"`, `"Alt+F9"`, `"Ctrl+Shift+F10"`, `"Ctrl+F9"`, `"F1"`..`"F24"`, `"Insert"`, `"Delete"`, `"Home"`, `"End"`, `"PageUp"`, `"PageDown"`, `"Num0"`..`"Num9"`, `"A"`..`"Z"`, `"0"`..`"9"`.
pub fn parse_windows_hotkey(shortcut: &str) -> Option<(u32, u32)> {
    let s = shortcut.trim();
    if s.is_empty() {
        return None;
    }

    let mut modifiers: u32 = WIN_MOD_NOREPEAT;
    let mut vk_code: Option<u32> = None;

    for part in s.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let lower = part.to_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => modifiers |= WIN_MOD_CONTROL,
            "shift" => modifiers |= WIN_MOD_SHIFT,
            "alt" => modifiers |= WIN_MOD_ALT,
            "mod" | "super" | "win" | "windows" | "logo" => modifiers |= WIN_MOD_WIN,
            _ => {
                let code = match lower.as_str() {
                    "f1" => Some(0x70),
                    "f2" => Some(0x71),
                    "f3" => Some(0x72),
                    "f4" => Some(0x73),
                    "f5" => Some(0x74),
                    "f6" => Some(0x75),
                    "f7" => Some(0x76),
                    "f8" => Some(0x77),
                    "f9" => Some(0x78),
                    "f10" => Some(0x79),
                    "f11" => Some(0x7A),
                    "f12" => Some(0x7B),
                    "f13" => Some(0x7C),
                    "f14" => Some(0x7D),
                    "f15" => Some(0x7E),
                    "f16" => Some(0x7F),
                    "f17" => Some(0x80),
                    "f18" => Some(0x81),
                    "f19" => Some(0x82),
                    "f20" => Some(0x83),
                    "f21" => Some(0x84),
                    "f22" => Some(0x85),
                    "f23" => Some(0x86),
                    "f24" => Some(0x87),
                    "num0" | "numpad0" => Some(0x60),
                    "num1" | "numpad1" => Some(0x61),
                    "num2" | "numpad2" => Some(0x62),
                    "num3" | "numpad3" => Some(0x63),
                    "num4" | "numpad4" => Some(0x64),
                    "num5" | "numpad5" => Some(0x65),
                    "num6" | "numpad6" => Some(0x66),
                    "num7" | "numpad7" => Some(0x67),
                    "num8" | "numpad8" => Some(0x68),
                    "num9" | "numpad9" => Some(0x69),
                    "space" => Some(0x20),
                    "enter" | "return" => Some(0x0D),
                    "esc" | "escape" => Some(0x1B),
                    "tab" => Some(0x09),
                    "backspace" | "back" => Some(0x08),
                    "delete" | "del" => Some(0x2E),
                    "insert" | "ins" => Some(0x2D),
                    "home" => Some(0x24),
                    "end" => Some(0x23),
                    "pageup" | "pgup" => Some(0x21),
                    "pagedown" | "pgdn" => Some(0x22),
                    "printscreen" | "prtscn" | "snapshot" => Some(0x2C),
                    "pause" => Some(0x13),
                    "scrolllock" | "scroll" => Some(0x91),
                    // Botones del mouse
                    "mousemiddle" | "middleclick" | "mouse3" | "btn_middle" => Some(0x04),
                    "mouse4" | "xbutton1" | "mouseback" | "btn_side" => Some(0x05),
                    "mouse5" | "xbutton2" | "mouseforward" | "btn_extra" => Some(0x06),
                    single if single.len() == 1 => {
                        let ch = single.chars().next().unwrap();
                        if ch.is_ascii_digit() {
                            Some(ch as u32)
                        } else if ch.is_ascii_alphabetic() {
                            Some(ch.to_ascii_uppercase() as u32)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(c) = code {
                    vk_code = Some(c);
                }
            }
        }
    }

    vk_code.map(|vk| (modifiers, vk))
}

/// Helper para determinar si un Virtual Key Code de Windows corresponde a un botón del mouse.
pub fn is_mouse_vk(vk: u32) -> bool {
    matches!(vk, 0x04..=0x06)
}

pub const LINUX_MOD_ALT: u16 = 0x0001;
pub const LINUX_MOD_CONTROL: u16 = 0x0002;
pub const LINUX_MOD_CTRL: u16 = 0x0002;
pub const LINUX_MOD_SHIFT: u16 = 0x0004;
pub const LINUX_MOD_SUPER: u16 = 0x0008;
pub const LINUX_MOD_WIN: u16 = 0x0008;
pub const LINUX_MOD_MOD: u16 = 0x0008;

/// Parsea un string de atajos textual para evdev en Linux a `(modifiers, evdev_code)`.
/// Soporta cadenas como `"F10"`, `"F9"`, `"Mouse4"`, `"Mouse5"`, `"MouseMiddle"`, `"Alt+F10"`, `"Mod+Shift+S"`, `"Ctrl+Mouse4"`.
pub fn parse_linux_evdev_shortcut(shortcut: &str) -> Option<(u16, u16)> {
    let s = shortcut.trim();
    if s.is_empty() {
        return None;
    }

    let mut modifiers: u16 = 0;
    let mut ev_code: Option<u16> = None;

    for part in s.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let lower = part.to_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => modifiers |= LINUX_MOD_CONTROL,
            "shift" => modifiers |= LINUX_MOD_SHIFT,
            "alt" => modifiers |= LINUX_MOD_ALT,
            "mod" | "super" | "win" | "windows" | "logo" => modifiers |= LINUX_MOD_SUPER,
            _ => {
                let code = match lower.as_str() {
                    // Botones del mouse
                    "mousemiddle" | "middleclick" | "mouse3" | "btn_middle" => Some(274),
                    "mouse4" | "xbutton1" | "mouseback" | "btn_side" => Some(275),
                    "mouse5" | "xbutton2" | "mouseforward" | "btn_extra" => Some(276),
                    "mouse1" | "mouseleft" | "btn_left" => Some(272),
                    "mouse2" | "mouseright" | "btn_right" => Some(273),

                    // Teclas de función F1..F24
                    "f1" => Some(59),
                    "f2" => Some(60),
                    "f3" => Some(61),
                    "f4" => Some(62),
                    "f5" => Some(63),
                    "f6" => Some(64),
                    "f7" => Some(65),
                    "f8" => Some(66),
                    "f9" => Some(67),
                    "f10" => Some(68),
                    "f11" => Some(87),
                    "f12" => Some(88),
                    "f13" => Some(183),
                    "f14" => Some(184),
                    "f15" => Some(185),
                    "f16" => Some(186),
                    "f17" => Some(187),
                    "f18" => Some(188),
                    "f19" => Some(189),
                    "f20" => Some(190),
                    "f21" => Some(191),
                    "f22" => Some(192),
                    "f23" => Some(193),
                    "f24" => Some(194),

                    // Teclado numérico
                    "num0" | "numpad0" => Some(82),
                    "num1" | "numpad1" => Some(79),
                    "num2" | "numpad2" => Some(80),
                    "num3" | "numpad3" => Some(81),
                    "num4" | "numpad4" => Some(75),
                    "num5" | "numpad5" => Some(76),
                    "num6" | "numpad6" => Some(77),
                    "num7" | "numpad7" => Some(71),
                    "num8" | "numpad8" => Some(72),
                    "num9" | "numpad9" => Some(73),

                    // Navegación y edición
                    "space" => Some(57),
                    "enter" | "return" => Some(28),
                    "esc" | "escape" => Some(1),
                    "tab" => Some(15),
                    "backspace" | "back" => Some(14),
                    "delete" | "del" => Some(111),
                    "insert" | "ins" => Some(110),
                    "home" => Some(102),
                    "end" => Some(107),
                    "pageup" | "pgup" => Some(104),
                    "pagedown" | "pgdn" => Some(109),
                    "printscreen" | "prtscn" | "snapshot" => Some(99),
                    "pause" => Some(119),
                    "scrolllock" | "scroll" => Some(70),

                    // Letras A-Z y dígitos 0-9
                    single if single.len() == 1 => {
                        let ch = single.chars().next().unwrap();
                        match ch {
                            '0' => Some(11),
                            '1' => Some(2),
                            '2' => Some(3),
                            '3' => Some(4),
                            '4' => Some(5),
                            '5' => Some(6),
                            '6' => Some(7),
                            '7' => Some(8),
                            '8' => Some(9),
                            '9' => Some(10),
                            'a' | 'A' => Some(30),
                            'b' | 'B' => Some(48),
                            'c' | 'C' => Some(46),
                            'd' | 'D' => Some(32),
                            'e' | 'E' => Some(18),
                            'f' | 'F' => Some(33),
                            'g' | 'G' => Some(34),
                            'h' | 'H' => Some(35),
                            'i' | 'I' => Some(23),
                            'j' | 'J' => Some(36),
                            'k' | 'K' => Some(37),
                            'l' | 'L' => Some(38),
                            'm' | 'M' => Some(50),
                            'n' | 'N' => Some(49),
                            'o' | 'O' => Some(24),
                            'p' | 'P' => Some(25),
                            'q' | 'Q' => Some(16),
                            'r' | 'R' => Some(19),
                            's' | 'S' => Some(31),
                            't' | 'T' => Some(20),
                            'u' | 'U' => Some(22),
                            'v' | 'V' => Some(47),
                            'w' | 'W' => Some(17),
                            'x' | 'X' => Some(45),
                            'y' | 'Y' => Some(21),
                            'z' | 'Z' => Some(44),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(c) = code {
                    ev_code = Some(c);
                }
            }
        }
    }

    ev_code.map(|code| (modifiers, code))
}

static HOTKEY_CONFIG: std::sync::OnceLock<std::sync::Mutex<(String, String)>> = std::sync::OnceLock::new();
#[allow(dead_code)]
static HOTKEY_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[allow(dead_code)]
static HOTKEY_LISTENER_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static HOTKEYS_DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Notifica y actualiza en caliente los atajos globales sin necesidad de reiniciar la app.
pub fn reload_windows_hotkeys(save: &str, record: &str) {
    let mutex = HOTKEY_CONFIG.get_or_init(|| std::sync::Mutex::new(Default::default()));
    if let Ok(mut lock) = mutex.lock() {
        *lock = (save.to_string(), record.to_string());
    }
    mark_hotkeys_dirty();
}

/// Alias multiplataforma para actualizar en caliente los atajos globales sin necesidad de reiniciar la app.
pub fn reload_hotkeys(save: &str, record: &str) {
    reload_windows_hotkeys(save, record);
}

/// Marca los atajos como modificados/sucios para forzar su reinscripción o actualización.
pub fn mark_hotkeys_dirty() {
    HOTKEYS_DIRTY.store(true, std::sync::atomic::Ordering::SeqCst);

    #[cfg(target_os = "windows")]
    {
        let tid = HOTKEY_THREAD_ID.load(std::sync::atomic::Ordering::SeqCst);
        if tid != 0 {
            use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_USER};
            unsafe {
                PostThreadMessageW(tid, WM_USER + 101, 0, 0);
            }
        }
    }
}

/// Inicia el listener de Hotkeys Globales en Windows.
#[cfg(target_os = "windows")]
pub fn spawn_global_hotkeys_listener() {
    use std::sync::atomic::Ordering;

    if HOTKEY_LISTENER_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    if std::thread::Builder::new()
        .name("sane-rec-win-hotkeys".into())
        .spawn(move || {
            run_windows_hotkey_loop();
            HOTKEY_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
        })
        .is_err()
    {
        HOTKEY_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
    }
}

#[cfg(target_os = "windows")]
fn run_windows_hotkey_loop() {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, KillTimer, PeekMessageW, SetTimer, TranslateMessage, MSG, PM_NOREMOVE,
        WM_HOTKEY, WM_TIMER, WM_USER,
    };

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    let tid = unsafe { GetCurrentThreadId() };
    HOTKEY_THREAD_ID.store(tid, Ordering::SeqCst);

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    unsafe {
        PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
    }

    const TIMER_ID_DIRTY: usize = 1;
    const TIMER_ID_MOUSE: usize = 2;
    let dirty_timer = unsafe { SetTimer(std::ptr::null_mut(), TIMER_ID_DIRTY, 500, None) };
    let mut mouse_timer: usize = 0;

    const VK_F9: u32 = 0x78;
    const VK_F10: u32 = 0x79;
    const HOTKEY_ID_RECORD: i32 = 1;
    const HOTKEY_ID_SAVE: i32 = 2;

    // Leer atajos iniciales desde Config
    let initial_cfg = crate::config::Config::load().unwrap_or_default();
    let mut current_save = initial_cfg.shortcuts.save_replay;
    let mut current_record = initial_cfg.shortcuts.toggle_record;

    if let Some(mutex) = HOTKEY_CONFIG.get() {
        if let Ok(lock) = mutex.lock() {
            if !lock.0.is_empty() || !lock.1.is_empty() {
                current_save = lock.0.clone();
                current_record = lock.1.clone();
            }
        }
    } else {
        let _ = HOTKEY_CONFIG.set(std::sync::Mutex::new((current_save.clone(), current_record.clone())));
    }

    let mut rec_binding = parse_windows_hotkey(&current_record).unwrap_or((MOD_NOREPEAT, VK_F9));
    let mut save_binding = parse_windows_hotkey(&current_save).unwrap_or((MOD_NOREPEAT, VK_F10));

    unsafe {
        let ok_rec = if !is_mouse_vk(rec_binding.1) {
            RegisterHotKey(
                std::ptr::null_mut(),
                HOTKEY_ID_RECORD,
                rec_binding.0,
                rec_binding.1,
            )
        } else {
            1
        };

        let ok_save = if !is_mouse_vk(save_binding.1) {
            if save_binding != rec_binding || is_mouse_vk(rec_binding.1) {
                RegisterHotKey(
                    std::ptr::null_mut(),
                    HOTKEY_ID_SAVE,
                    save_binding.0,
                    save_binding.1,
                )
            } else {
                eprintln!(
                    "Aviso: Los atajos de grabar ('{}') y guardar ('{}') colisionan; se omitió el registro duplicado.",
                    current_record, current_save
                );
                1
            }
        } else {
            1
        };

        let mut hotkeys_registered: bool = (ok_rec != 0) && (ok_save != 0);
        let mut last_trigger = std::time::Instant::now() - std::time::Duration::from_secs(1);

        if is_mouse_vk(rec_binding.1) || is_mouse_vk(save_binding.1) {
            mouse_timer = SetTimer(std::ptr::null_mut(), TIMER_ID_MOUSE, 20, None);
        }

        let mut rec_was_down = false;
        let mut save_was_down = false;

        let check_hotkey_down = |mods: u32, vk: u32| -> bool {
            let key_down = (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0;
            if !key_down {
                return false;
            }
            let ctrl_down = (GetAsyncKeyState(0x11) as u16 & 0x8000) != 0;
            let ctrl_req = (mods & WIN_MOD_CONTROL) != 0;
            if ctrl_down != ctrl_req {
                return false;
            }
            let shift_down = (GetAsyncKeyState(0x10) as u16 & 0x8000) != 0;
            let shift_req = (mods & WIN_MOD_SHIFT) != 0;
            if shift_down != shift_req {
                return false;
            }
            let alt_down = (GetAsyncKeyState(0x12) as u16 & 0x8000) != 0;
            let alt_req = (mods & WIN_MOD_ALT) != 0;
            if alt_down != alt_req {
                return false;
            }
            let win_down = ((GetAsyncKeyState(0x5B) as u16 & 0x8000) != 0)
                || ((GetAsyncKeyState(0x5C) as u16 & 0x8000) != 0);
            let win_req = (mods & WIN_MOD_WIN) != 0;
            if win_down != win_req {
                return false;
            }
            true
        };

        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == WM_HOTKEY {
                if last_trigger.elapsed() >= std::time::Duration::from_millis(350) {
                    last_trigger = std::time::Instant::now();
                    match msg.wParam as i32 {
                        HOTKEY_ID_RECORD => {
                            std::thread::spawn(|| {
                                let _ = send_command_sync("record_toggle");
                            });
                        }
                        HOTKEY_ID_SAVE => {
                            std::thread::spawn(|| {
                                let _ = send_command_sync("save");
                            });
                        }
                        _ => {}
                    }
                }
            } else if msg.message == WM_TIMER && mouse_timer != 0 && msg.wParam == mouse_timer {
                if is_mouse_vk(rec_binding.1) {
                    let is_down = check_hotkey_down(rec_binding.0, rec_binding.1);
                    if is_down && !rec_was_down {
                        if last_trigger.elapsed() >= std::time::Duration::from_millis(350) {
                            last_trigger = std::time::Instant::now();
                            std::thread::spawn(|| {
                                let _ = send_command_sync("record_toggle");
                            });
                        }
                    }
                    rec_was_down = is_down;
                }

                if is_mouse_vk(save_binding.1) {
                    let is_down = check_hotkey_down(save_binding.0, save_binding.1);
                    if is_down && !save_was_down {
                        if last_trigger.elapsed() >= std::time::Duration::from_millis(350) {
                            last_trigger = std::time::Instant::now();
                            std::thread::spawn(|| {
                                let _ = send_command_sync("save");
                            });
                        }
                    }
                    save_was_down = is_down;
                }
            } else if msg.message == (WM_USER + 101) || (msg.message == WM_TIMER && dirty_timer != 0 && msg.wParam == dirty_timer) {
                let dirty = HOTKEYS_DIRTY.swap(false, Ordering::SeqCst);
                let (new_save, new_rec) = if dirty {
                    if let Some(mutex) = HOTKEY_CONFIG.get() {
                        if let Ok(lock) = mutex.lock() {
                            lock.clone()
                        } else {
                            (current_save.clone(), current_record.clone())
                        }
                    } else {
                        (current_save.clone(), current_record.clone())
                    }
                } else {
                    (current_save.clone(), current_record.clone())
                };

                let changed = (new_save != current_save) || (new_rec != current_record);

                if changed || dirty || !hotkeys_registered {
                    UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID_RECORD);
                    UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID_SAVE);
                    current_save = new_save;
                    current_record = new_rec;

                    rec_binding = parse_windows_hotkey(&current_record).unwrap_or((MOD_NOREPEAT, VK_F9));
                    save_binding = parse_windows_hotkey(&current_save).unwrap_or((MOD_NOREPEAT, VK_F10));

                    let ok_rec = if !is_mouse_vk(rec_binding.1) {
                        RegisterHotKey(
                            std::ptr::null_mut(),
                            HOTKEY_ID_RECORD,
                            rec_binding.0,
                            rec_binding.1,
                        )
                    } else {
                        1
                    };

                    let ok_save = if !is_mouse_vk(save_binding.1) {
                        if save_binding != rec_binding || is_mouse_vk(rec_binding.1) {
                            RegisterHotKey(
                                std::ptr::null_mut(),
                                HOTKEY_ID_SAVE,
                                save_binding.0,
                                save_binding.1,
                            )
                        } else {
                            1
                        }
                    } else {
                        1
                    };

                    hotkeys_registered = (ok_rec != 0) && (ok_save != 0);

                    let need_mouse = is_mouse_vk(rec_binding.1) || is_mouse_vk(save_binding.1);
                    if need_mouse && mouse_timer == 0 {
                        mouse_timer = SetTimer(std::ptr::null_mut(), TIMER_ID_MOUSE, 20, None);
                    } else if !need_mouse && mouse_timer != 0 {
                        KillTimer(std::ptr::null_mut(), mouse_timer);
                        mouse_timer = 0;
                        rec_was_down = false;
                        save_was_down = false;
                    }
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if mouse_timer != 0 {
            KillTimer(std::ptr::null_mut(), mouse_timer);
        }
        if dirty_timer != 0 {
            KillTimer(std::ptr::null_mut(), dirty_timer);
        }
    }
}

/// Inicia el listener de Hotkeys Globales nativo en Linux mediante /dev/input/event*.
#[cfg(target_os = "linux")]
pub fn spawn_linux_global_hotkeys_listener() {
    use std::sync::atomic::Ordering;

    if HOTKEY_LISTENER_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    if std::thread::Builder::new()
        .name("sane-rec-linux-hotkeys".into())
        .spawn(move || {
            run_linux_hotkey_loop();
            HOTKEY_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
        })
        .is_err()
    {
        HOTKEY_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
    }
}

#[cfg(target_os = "linux")]
fn scan_input_devices() -> Vec<std::fs::File> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev/input") {
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.starts_with("event"))
                    .unwrap_or(false)
            })
            .collect();
        paths.sort();

        for path in paths {
            if let Ok(file) = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
            {
                files.push(file);
            }
        }
    }
    files
}

#[cfg(target_os = "linux")]
fn run_linux_hotkey_loop() {
    use std::sync::atomic::Ordering;

    let initial_cfg = crate::config::Config::load().unwrap_or_default();
    let mut current_save = initial_cfg.shortcuts.save_replay;
    let mut current_record = initial_cfg.shortcuts.toggle_record;

    if let Some(mutex) = HOTKEY_CONFIG.get() {
        if let Ok(lock) = mutex.lock() {
            if !lock.0.is_empty() || !lock.1.is_empty() {
                current_save = lock.0.clone();
                current_record = lock.1.clone();
            }
        }
    } else {
        let _ = HOTKEY_CONFIG.set(std::sync::Mutex::new((current_save.clone(), current_record.clone())));
    }

    let mut save_binding = parse_linux_evdev_shortcut(&current_save);
    let mut rec_binding = parse_linux_evdev_shortcut(&current_record);

    let mut last_trigger = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let mut mod_ctrl = false;
    let mut mod_shift = false;
    let mut mod_alt = false;
    let mut mod_super = false;

    let mut devices = scan_input_devices();
    let mut last_rescan = std::time::Instant::now();

    while HOTKEY_LISTENER_ACTIVE.load(Ordering::SeqCst) {
        if devices.is_empty() {
            std::thread::sleep(std::time::Duration::from_secs(3));
            devices = scan_input_devices();
            last_rescan = std::time::Instant::now();
            continue;
        }

        if HOTKEYS_DIRTY.swap(false, Ordering::SeqCst) {
            if let Some(mutex) = HOTKEY_CONFIG.get() {
                if let Ok(lock) = mutex.lock() {
                    let (new_save, new_rec) = lock.clone();
                    if new_save != current_save || new_rec != current_record {
                        current_save = new_save;
                        current_record = new_rec;
                        save_binding = parse_linux_evdev_shortcut(&current_save);
                        rec_binding = parse_linux_evdev_shortcut(&current_record);
                    }
                }
            }
        }

        use std::os::unix::io::AsRawFd;
        let mut pollfds: Vec<libc::pollfd> = devices
            .iter()
            .map(|f| libc::pollfd {
                fd: f.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();

        let ret = unsafe {
            libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, 500)
        };

        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }

        let mut needs_rescan = false;
        if ret > 0 {
            use std::io::Read;
            let ev_size = std::mem::size_of::<libc::input_event>();
            let mut read_buf = vec![0u8; ev_size * 16];

            for (i, pfd) in pollfds.iter().enumerate() {
                if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                    needs_rescan = true;
                    continue;
                }
                if (pfd.revents & libc::POLLIN) != 0 {
                    match devices[i].read(&mut read_buf) {
                        Ok(bytes_read) if bytes_read >= ev_size => {
                            let count = bytes_read / ev_size;
                            for idx in 0..count {
                                let offset = idx * ev_size;
                                let ev: libc::input_event = unsafe {
                                    std::ptr::read_unaligned(
                                        read_buf[offset..offset + ev_size].as_ptr() as *const libc::input_event,
                                    )
                                };

                                const EV_KEY: u16 = 0x01;
                                if ev.type_ == EV_KEY {
                                    match ev.code {
                                        29 | 97 => mod_ctrl = ev.value != 0,
                                        42 | 54 => mod_shift = ev.value != 0,
                                        56 | 100 => mod_alt = ev.value != 0,
                                        125 | 126 => mod_super = ev.value != 0,
                                        _ => {}
                                    }

                                    if ev.value == 1 {
                                        let mut active_mods: u16 = 0;
                                        if mod_alt {
                                            active_mods |= LINUX_MOD_ALT;
                                        }
                                        if mod_ctrl {
                                            active_mods |= LINUX_MOD_CONTROL;
                                        }
                                        if mod_shift {
                                            active_mods |= LINUX_MOD_SHIFT;
                                        }
                                        if mod_super {
                                            active_mods |= LINUX_MOD_SUPER;
                                        }

                                        if let Some(rec) = rec_binding {
                                            if ev.code == rec.1 && active_mods == rec.0 {
                                                if last_trigger.elapsed() >= std::time::Duration::from_millis(350) {
                                                    last_trigger = std::time::Instant::now();
                                                    std::thread::spawn(|| {
                                                        let _ = send_command_sync("record_toggle");
                                                    });
                                                }
                                                continue;
                                            }
                                        }

                                        if let Some(save) = save_binding {
                                            if ev.code == save.1 && active_mods == save.0 {
                                                if last_trigger.elapsed() >= std::time::Duration::from_millis(350) {
                                                    last_trigger = std::time::Instant::now();
                                                    std::thread::spawn(|| {
                                                        let _ = send_command_sync("save");
                                                    });
                                                }
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(_) => {
                            needs_rescan = true;
                        }
                    }
                }
            }
        }

        if needs_rescan || last_rescan.elapsed() >= std::time::Duration::from_secs(5) {
            last_rescan = std::time::Instant::now();
            devices = scan_input_devices();
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn spawn_global_hotkeys_listener() {
    #[cfg(target_os = "linux")]
    {
        spawn_linux_global_hotkeys_listener();
    }
}

/// Inicia el listener IPC en una ruta arbitraria de socket Unix.
#[cfg(unix)]
pub async fn start_listener_at(
    path: &Path,
    tx: tokio::sync::mpsc::Sender<IpcCommand>,
) -> Result<()> {
    // Si el archivo del socket ya existe, verificamos si es huérfano
    if path.exists() {
        if tokio::net::UnixStream::connect(path).await.is_ok() {
            return Err(IpcError::Custom(format!(
                "Socket {} is already in use by an active instance",
                path.display()
            )));
        }
        let _ = std::fs::remove_file(path);
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let listener = tokio::net::UnixListener::bind(path)?;

    loop {
        let (mut stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => break,
        };

        let tx = tx.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

            let (reader, mut writer) = stream.split();
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();

            match buf_reader.read_line(&mut line).await {
                Ok(0) => {}
                Ok(_) => {
                    let req: IpcRequest = match serde_json::from_str(&line) {
                        Ok(r) => r,
                        Err(e) => {
                            let resp = IpcResponse::error(format!("Invalid request format: {}", e));
                            if let Ok(mut bytes) = serde_json::to_vec(&resp) {
                                bytes.push(b'\n');
                                let _ = writer.write_all(&bytes).await;
                                let _ = writer.flush().await;
                            }
                            return;
                        }
                    };

                    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                    let cmd = IpcCommand {
                        action: req.action,
                        responder: resp_tx,
                    };

                    let response = if tx.send(cmd).await.is_err() {
                        IpcResponse::error("Server backend unavailable or shutting down")
                    } else {
                        match resp_rx.await {
                            Ok(resp) => resp,
                            Err(_) => {
                                IpcResponse::error("Backend dropped request without response")
                            }
                        }
                    };

                    if let Ok(mut bytes) = serde_json::to_vec(&response) {
                        bytes.push(b'\n');
                        let _ = writer.write_all(&bytes).await;
                        let _ = writer.flush().await;
                    }
                }
                Err(_) => {}
            }
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_protocol_serialization() {
        // Peticiones
        let req_save = IpcRequest { action: "save".to_string() };
        assert_eq!(serde_json::to_string(&req_save).unwrap(), r#"{"action":"save"}"#);

        let req_record = IpcRequest { action: "record_toggle".to_string() };
        assert_eq!(serde_json::to_string(&req_record).unwrap(), r#"{"action":"record_toggle"}"#);

        let req_status = IpcRequest { action: "status".to_string() };
        assert_eq!(serde_json::to_string(&req_status).unwrap(), r#"{"action":"status"}"#);

        let req_stop = IpcRequest { action: "stop".to_string() };
        assert_eq!(serde_json::to_string(&req_stop).unwrap(), r#"{"action":"stop"}"#);

        // Respuestas
        let resp_ok = IpcResponse::ok("Replay saved successfully");
        assert!(resp_ok.is_ok());
        let json_ok = serde_json::to_string(&resp_ok).unwrap();
        assert!(json_ok.contains(r#""status":"ok""#));
        assert!(json_ok.contains(r#""message":"Replay saved successfully""#));

        let resp_err = IpcResponse::error("Recording already stopped");
        assert!(resp_err.is_error());
        let json_err = serde_json::to_string(&resp_err).unwrap();
        assert!(json_err.contains(r#""status":"error""#));
        assert!(json_err.contains(r#""message":"Recording already stopped""#));

        let data = serde_json::json!({ "recording": true, "fps": 60 });
        let resp_data = IpcResponse::ok_with_data("Current status", data);
        assert!(resp_data.data.is_some());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_ipc_listener_and_send_command_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!("sane_rec_ipc_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let socket_file = temp_dir.join("test.sock");

        let (tx, mut rx) = tokio::sync::mpsc::channel::<IpcCommand>(16);

        let s_path = socket_file.clone();
        tokio::spawn(async move {
            let _ = start_listener_at(&s_path, tx).await;
        });

        // Esperar brevemente a que el socket esté disponible
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Tarea simulada del servidor para responder a los comandos
        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd.action.as_str() {
                    "save" => cmd.ok("Replay saved to disk"),
                    "status" => cmd.ok_with_data("Running", serde_json::json!({ "recording": true })),
                    "unknown" => cmd.error("Unknown action"),
                    _ => cmd.ok("Generic ok"),
                }
            }
        });

        // Test comando async
        let resp_save = send_command_at(&socket_file, "save").await.expect("Failed to send save command");
        assert!(resp_save.is_ok());
        assert_eq!(resp_save.message.unwrap(), "Replay saved to disk");

        // Test comando status con data
        let resp_status = send_command_at(&socket_file, "status").await.expect("Failed to send status command");
        assert!(resp_status.is_ok());
        assert_eq!(resp_status.data.unwrap()["recording"], true);

        // Test comando síncrono
        let resp_sync = send_command_sync_at(&socket_file, "unknown").expect("Failed to send sync command");
        assert!(resp_sync.is_error());
        assert_eq!(resp_sync.message.unwrap(), "Unknown action");

        let _ = std::fs::remove_file(&socket_file);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_parse_windows_hotkey() {
        // Atajos simples de función
        assert_eq!(parse_windows_hotkey("F4"), Some((WIN_MOD_NOREPEAT, 0x73)));
        assert_eq!(parse_windows_hotkey("F9"), Some((WIN_MOD_NOREPEAT, 0x78)));
        assert_eq!(parse_windows_hotkey("F10"), Some((WIN_MOD_NOREPEAT, 0x79)));
        assert_eq!(parse_windows_hotkey("f12"), Some((WIN_MOD_NOREPEAT, 0x7B)));
        assert_eq!(parse_windows_hotkey("F13"), Some((WIN_MOD_NOREPEAT, 0x7C)));
        assert_eq!(parse_windows_hotkey("F24"), Some((WIN_MOD_NOREPEAT, 0x87)));

        // Teclas de navegación y numpad
        assert_eq!(parse_windows_hotkey("Insert"), Some((WIN_MOD_NOREPEAT, 0x2D)));
        assert_eq!(parse_windows_hotkey("Delete"), Some((WIN_MOD_NOREPEAT, 0x2E)));
        assert_eq!(parse_windows_hotkey("Home"), Some((WIN_MOD_NOREPEAT, 0x24)));
        assert_eq!(parse_windows_hotkey("End"), Some((WIN_MOD_NOREPEAT, 0x23)));
        assert_eq!(parse_windows_hotkey("PageUp"), Some((WIN_MOD_NOREPEAT, 0x21)));
        assert_eq!(parse_windows_hotkey("PgDn"), Some((WIN_MOD_NOREPEAT, 0x22)));
        assert_eq!(parse_windows_hotkey("Num0"), Some((WIN_MOD_NOREPEAT, 0x60)));
        assert_eq!(parse_windows_hotkey("Num9"), Some((WIN_MOD_NOREPEAT, 0x69)));

        // Modificadores combinados
        assert_eq!(
            parse_windows_hotkey("Alt+F4"),
            Some((WIN_MOD_NOREPEAT | WIN_MOD_ALT, 0x73))
        );
        assert_eq!(
            parse_windows_hotkey("Mod+F10"),
            Some((WIN_MOD_NOREPEAT | WIN_MOD_WIN, 0x79))
        );
        assert_eq!(
            parse_windows_hotkey("Alt+F9"),
            Some((WIN_MOD_NOREPEAT | WIN_MOD_ALT, 0x78))
        );
        assert_eq!(
            parse_windows_hotkey("Ctrl+Shift+F10"),
            Some((WIN_MOD_NOREPEAT | WIN_MOD_CONTROL | WIN_MOD_SHIFT, 0x79))
        );
        assert_eq!(
            parse_windows_hotkey("Ctrl+F9"),
            Some((WIN_MOD_NOREPEAT | WIN_MOD_CONTROL, 0x78))
        );
        assert_eq!(
            parse_windows_hotkey("Ctrl+Shift+R"),
            Some((WIN_MOD_NOREPEAT | WIN_MOD_CONTROL | WIN_MOD_SHIFT, 0x52))
        );
        assert_eq!(
            parse_windows_hotkey("Alt+0"),
            Some((WIN_MOD_NOREPEAT | WIN_MOD_ALT, 0x30))
        );

        // Botones de mouse en Windows
        assert_eq!(parse_windows_hotkey("Mouse4"), Some((WIN_MOD_NOREPEAT, 0x05)));
        assert_eq!(parse_windows_hotkey("XButton1"), Some((WIN_MOD_NOREPEAT, 0x05)));
        assert_eq!(parse_windows_hotkey("MouseBack"), Some((WIN_MOD_NOREPEAT, 0x05)));
        assert_eq!(parse_windows_hotkey("Mouse5"), Some((WIN_MOD_NOREPEAT, 0x06)));
        assert_eq!(parse_windows_hotkey("XButton2"), Some((WIN_MOD_NOREPEAT, 0x06)));
        assert_eq!(parse_windows_hotkey("MouseForward"), Some((WIN_MOD_NOREPEAT, 0x06)));
        assert_eq!(parse_windows_hotkey("MouseMiddle"), Some((WIN_MOD_NOREPEAT, 0x04)));
        assert_eq!(parse_windows_hotkey("MiddleClick"), Some((WIN_MOD_NOREPEAT, 0x04)));
        assert_eq!(parse_windows_hotkey("Mouse3"), Some((WIN_MOD_NOREPEAT, 0x04)));
        assert_eq!(
            parse_windows_hotkey("Ctrl+Mouse4"),
            Some((WIN_MOD_NOREPEAT | WIN_MOD_CONTROL, 0x05))
        );
        assert_eq!(
            parse_windows_hotkey("Alt+Mouse5"),
            Some((WIN_MOD_NOREPEAT | WIN_MOD_ALT, 0x06))
        );
        assert!(is_mouse_vk(0x04));
        assert!(is_mouse_vk(0x05));
        assert!(is_mouse_vk(0x06));
        assert!(!is_mouse_vk(0x79));

        // Inválidos
        assert_eq!(parse_windows_hotkey(""), None);
        assert_eq!(parse_windows_hotkey("Ctrl+Shift+"), None);
    }

    #[test]
    fn test_parse_linux_evdev_shortcut() {
        assert_eq!(parse_linux_evdev_shortcut("F10"), Some((0, 68)));
        assert_eq!(parse_linux_evdev_shortcut("F9"), Some((0, 67)));
        assert_eq!(parse_linux_evdev_shortcut("Mouse4"), Some((0, 275)));
        assert_eq!(parse_linux_evdev_shortcut("Mouse5"), Some((0, 276)));
        assert_eq!(parse_linux_evdev_shortcut("MouseMiddle"), Some((0, 274)));
        assert_eq!(parse_linux_evdev_shortcut("xbutton1"), Some((0, 275)));
        assert_eq!(parse_linux_evdev_shortcut("xbutton2"), Some((0, 276)));
        assert_eq!(parse_linux_evdev_shortcut("middleclick"), Some((0, 274)));
        assert_eq!(parse_linux_evdev_shortcut("Alt+F10"), Some((LINUX_MOD_ALT, 68)));
        assert_eq!(
            parse_linux_evdev_shortcut("Mod+Shift+S"),
            Some((LINUX_MOD_SUPER | LINUX_MOD_SHIFT, 31))
        );
        assert_eq!(
            parse_linux_evdev_shortcut("Ctrl+Mouse4"),
            Some((LINUX_MOD_CONTROL, 275))
        );
        assert_eq!(
            parse_linux_evdev_shortcut("Alt+Mouse5"),
            Some((LINUX_MOD_ALT, 276))
        );
        assert_eq!(parse_linux_evdev_shortcut(""), None);
        assert_eq!(parse_linux_evdev_shortcut("Ctrl+Alt+"), None);
    }
}
