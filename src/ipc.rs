use std::path::{Path, PathBuf};
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

/// Retorna la ruta al socket Unix `/run/user/<UID>/sane-rec.sock`.
pub fn socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("sane-rec.sock")
    } else {
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/run/user/{}/sane-rec.sock", uid))
    }
}

/// Envía una acción mediante el socket Unix por defecto de forma asíncrona y espera la respuesta.
pub async fn send_command(action: &str) -> Result<IpcResponse> {
    send_command_at(&socket_path(), action).await
}

/// Envía una acción a un socket Unix en una ruta específica de forma asíncrona.
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

/// Envía una acción mediante el socket Unix por defecto de forma síncrona.
pub fn send_command_sync(action: &str) -> Result<IpcResponse> {
    send_command_sync_at(&socket_path(), action)
}

/// Envía una acción a un socket Unix en una ruta específica de forma síncrona.
pub fn send_command_sync_at(path: &Path, action: &str) -> Result<IpcResponse> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(path)?;
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

/// Inicia el listener IPC en la ruta por defecto `/run/user/<UID>/sane-rec.sock`.
pub async fn start_listener(tx: tokio::sync::mpsc::Sender<IpcCommand>) -> Result<()> {
    start_listener_at(&socket_path(), tx).await
}

/// Inicia el listener IPC en una ruta arbitraria de socket Unix.
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
}
