// sane-rec: Monitor en tiempo real de picos de audio (VU meter) usando parec/PipeWire
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct AudioPeakMonitor {
    system_peak: Arc<AtomicU32>,
    mic_peak: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
    _threads: Vec<JoinHandle<()>>,
}

impl AudioPeakMonitor {
    /// Inicia los hilos de monitoreo en tiempo real para el monitor del sistema y el micrófono.
    pub fn new(custom_sink: Option<String>, custom_mic: Option<String>) -> Self {
        let system_peak = Arc::new(AtomicU32::new(0));
        let mic_peak = Arc::new(AtomicU32::new(0));
        let running = Arc::new(AtomicBool::new(true));

        let mut threads = Vec::new();

        // 1. Hilo de monitoreo para audio del sistema (Sink Monitor)
        {
            let peak_store = Arc::clone(&system_peak);
            let is_running = Arc::clone(&running);
            let device = custom_sink
                .filter(|s| !s.is_empty() && s != "PipeWire Default Sink")
                .map(|s| {
                    if s.ends_with(".monitor") {
                        s
                    } else {
                        format!("{}.monitor", s)
                    }
                })
                .unwrap_or_else(|| "@DEFAULT_MONITOR@".to_string());

            let handle = thread::Builder::new()
                .name("sane-rec-vu-system".into())
                .spawn(move || {
                    run_peak_loop(&device, peak_store, is_running);
                })
                .ok();

            if let Some(h) = handle {
                threads.push(h);
            }
        }

        // 2. Hilo de monitoreo para micrófono (Source)
        {
            let peak_store = Arc::clone(&mic_peak);
            let is_running = Arc::clone(&running);
            let device = custom_mic
                .filter(|m| !m.is_empty() && m != "Desactivado")
                .unwrap_or_else(|| "@DEFAULT_SOURCE@".to_string());

            let handle = thread::Builder::new()
                .name("sane-rec-vu-mic".into())
                .spawn(move || {
                    run_peak_loop(&device, peak_store, is_running);
                })
                .ok();

            if let Some(h) = handle {
                threads.push(h);
            }
        }

        Self {
            system_peak,
            mic_peak,
            running,
            _threads: threads,
        }
    }

    /// Retorna el nivel de pico de audio del sistema (normalizado de 0.0 a 1.0).
    pub fn system_peak(&self) -> f32 {
        (self.system_peak.load(Ordering::Relaxed) as f32) / 1000.0
    }

    /// Retorna el nivel de pico del micrófono (normalizado de 0.0 a 1.0).
    pub fn mic_peak(&self) -> f32 {
        (self.mic_peak.load(Ordering::Relaxed) as f32) / 1000.0
    }
}

impl Drop for AudioPeakMonitor {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

/// Bucle continuo que lee muestras PCM de 16-bit de `parec` y calcula el valor pico con suavizado exponencial.
fn run_peak_loop(device: &str, peak_store: Arc<AtomicU32>, running: Arc<AtomicBool>) {
    let mut smoothed: f32 = 0.0;
    let mut buffer = [0u8; 640]; // 320 muestras de 16-bit mono @ 8000Hz = 40ms

    while running.load(Ordering::Relaxed) {
        let child = Command::new("parec")
            .args([
                "-d",
                device,
                "--latency-msec=40",
                "--raw",
                "--rate=8000",
                "--channels=1",
                "--format=s16le",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();

        let Ok(mut child) = child else {
            thread::sleep(Duration::from_millis(500));
            continue;
        };

        if let Some(mut stdout) = child.stdout.take() {
            while running.load(Ordering::Relaxed) {
                match stdout.read_exact(&mut buffer) {
                    Ok(()) => {
                        let mut max_abs: i16 = 0;
                        for chunk in buffer.chunks_exact(2) {
                            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                            let abs = sample.abs();
                            if abs > max_abs {
                                max_abs = abs;
                            }
                        }

                        let instant_peak = (max_abs as f32) / 32768.0;
                        // Suavizado visual: subida rápida, caída suave
                        if instant_peak > smoothed {
                            smoothed = smoothed * 0.3 + instant_peak * 0.7;
                        } else {
                            smoothed = smoothed * 0.82 + instant_peak * 0.18;
                        }

                        peak_store.store(
                            (smoothed * 1000.0).clamp(0.0, 1000.0) as u32,
                            Ordering::Relaxed,
                        );
                    }
                    Err(_) => {
                        // Flujo interrumpido o dispositivo desconectado
                        break;
                    }
                }
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        peak_store.store(0, Ordering::Relaxed);

        if running.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(300));
        }
    }
}
