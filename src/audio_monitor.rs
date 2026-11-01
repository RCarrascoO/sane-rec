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

        #[cfg(target_os = "linux")]
        {
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
        }

        #[cfg(target_os = "windows")]
        {
            // 1. Hilo de monitoreo para audio del sistema (WASAPI Loopback)
            {
                let peak_store = Arc::clone(&system_peak);
                let is_running = Arc::clone(&running);
                let dev_sink = custom_sink;
                let handle = thread::Builder::new()
                    .name("sane-rec-vu-system".into())
                    .spawn(move || {
                        run_cpal_peak_loop(true, dev_sink, peak_store, is_running);
                    })
                    .ok();

                if let Some(h) = handle {
                    threads.push(h);
                }
            }

            // 2. Hilo de monitoreo para micrófono (WASAPI Input)
            {
                let peak_store = Arc::clone(&mic_peak);
                let is_running = Arc::clone(&running);
                let dev_mic = custom_mic;
                let handle = thread::Builder::new()
                    .name("sane-rec-vu-mic".into())
                    .spawn(move || {
                        run_cpal_peak_loop(false, dev_mic, peak_store, is_running);
                    })
                    .ok();

                if let Some(h) = handle {
                    threads.push(h);
                }
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

/// Bucle continuo que lee muestras PCM de 16-bit de `parec` y calcula el valor pico con suavizado exponencial en Linux.
#[cfg(target_os = "linux")]
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

/// Bucle continuo que lee muestras de audio mediante cpal (WASAPI Loopback/Input) en Windows.
#[cfg(target_os = "windows")]
fn run_cpal_peak_loop(
    is_loopback: bool,
    device_opt: Option<String>,
    peak_store: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::Mutex;

    let host = cpal::default_host();

    while running.load(Ordering::Relaxed) {
        let device = if is_loopback {
            if let Some(ref name) = device_opt {
                let clean_name = name.strip_prefix("loopback:").unwrap_or(name);
                if clean_name == "default_output" || clean_name.is_empty() {
                    host.default_output_device()
                } else {
                    host.output_devices()
                        .ok()
                        .and_then(|mut devs| devs.find(|d| d.name().map_or(false, |n| n == clean_name)))
                        .or_else(|| host.default_output_device())
                }
            } else {
                host.default_output_device()
            }
        } else {
            if let Some(ref name) = device_opt {
                if name == "Desactivado" {
                    return;
                }
                host.input_devices()
                    .ok()
                    .and_then(|mut devs| devs.find(|d| d.name().map_or(false, |n| n == name)))
                    .or_else(|| host.default_input_device())
            } else {
                host.default_input_device()
            }
        };

        let Some(device) = device else {
            thread::sleep(Duration::from_millis(1000));
            continue;
        };

        let config_res = if is_loopback {
            device.default_output_config()
        } else {
            device.default_input_config()
        };

        let Ok(config) = config_res else {
            thread::sleep(Duration::from_millis(1000));
            continue;
        };

        let sample_format = config.sample_format();
        let stream_config = config.into();

        let peak_store_cb = Arc::clone(&peak_store);
        let smoothed = Arc::new(Mutex::new(0.0f32));

        let err_fn = |_err| {};

        let stream_res = match sample_format {
            cpal::SampleFormat::F32 => {
                let peak_store_f32 = Arc::clone(&peak_store_cb);
                let smoothed_f32 = Arc::clone(&smoothed);
                device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let mut max_abs: f32 = 0.0;
                        for &sample in data {
                            let abs = sample.abs();
                            if abs > max_abs {
                                max_abs = abs;
                            }
                        }
                        if let Ok(mut sm) = smoothed_f32.lock() {
                            if max_abs > *sm {
                                *sm = *sm * 0.3 + max_abs * 0.7;
                            } else {
                                *sm = *sm * 0.82 + max_abs * 0.18;
                            }
                            peak_store_f32.store(
                                (*sm * 1000.0).clamp(0.0, 1000.0) as u32,
                                Ordering::Relaxed,
                            );
                        }
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let peak_store_i16 = Arc::clone(&peak_store_cb);
                let smoothed_i16 = Arc::clone(&smoothed);
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let mut max_abs: i16 = 0;
                        for &sample in data {
                            let abs = sample.abs();
                            if abs > max_abs {
                                max_abs = abs;
                            }
                        }
                        let instant_peak = (max_abs as f32) / 32768.0;
                        if let Ok(mut sm) = smoothed_i16.lock() {
                            if instant_peak > *sm {
                                *sm = *sm * 0.3 + instant_peak * 0.7;
                            } else {
                                *sm = *sm * 0.82 + instant_peak * 0.18;
                            }
                            peak_store_i16.store(
                                (*sm * 1000.0).clamp(0.0, 1000.0) as u32,
                                Ordering::Relaxed,
                            );
                        }
                    },
                    err_fn,
                    None,
                )
            }
            _ => {
                thread::sleep(Duration::from_millis(1000));
                continue;
            }
        };

        let Ok(stream) = stream_res else {
            thread::sleep(Duration::from_millis(1000));
            continue;
        };

        if stream.play().is_err() {
            thread::sleep(Duration::from_millis(1000));
            continue;
        }

        while running.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
        }

        peak_store.store(0, Ordering::Relaxed);
        break;
    }
}
