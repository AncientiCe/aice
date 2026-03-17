//! Desktop microphone capture via cpal.

use crate::capture::{AudioCapture, CaptureError};
use crate::SAMPLE_RATE;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Capture from the default system microphone using cpal.
/// The stream runs on a dedicated thread (cpal::Stream is !Send on some platforms).
pub struct CpalCapture {
    rx: mpsc::Receiver<Vec<i16>>,
    device_name: String,
}

impl CpalCapture {
    /// Build and start capture from the default input device (16 kHz, mono, i16).
    pub fn default_device() -> Result<Self, CaptureError> {
        Self::from_preferred_name(None)
    }

    /// Build and start capture from a preferred input device name (substring match, case-insensitive).
    /// Falls back to default input device when no preferred name is provided.
    pub fn from_preferred_name(preferred_name: Option<&str>) -> Result<Self, CaptureError> {
        let host = cpal::default_host();
        let preferred_lower = preferred_name.map(|s| s.to_lowercase());

        let device = if let Some(pref) = preferred_lower.as_deref() {
            let mut found = None;
            if let Ok(mut devices) = host.input_devices() {
                for dev in devices.by_ref() {
                    let name = dev.name().unwrap_or_default().to_lowercase();
                    if name.contains(pref) {
                        found = Some(dev);
                        break;
                    }
                }
            }
            found
        } else {
            None
        }
        .or_else(|| host.default_input_device())
        .ok_or_else(|| {
            CaptureError::Device("no input device found (check OS microphone settings)".to_string())
        })?;

        let device_name = device
            .name()
            .unwrap_or_else(|_| "unknown-input-device".to_string());

        let (tx, rx) = mpsc::sync_channel::<Vec<i16>>(32);
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<(), CaptureError>>(1);
        thread::spawn(move || {
            let supported = match device.default_input_config() {
                Ok(cfg) => cfg,
                Err(e) => {
                    let _ = init_tx.send(Err(CaptureError::Device(format!(
                        "failed to query default input config: {e}"
                    ))));
                    return;
                }
            };
            let config: cpal::StreamConfig = supported.clone().into();
            let in_rate = config.sample_rate.0;
            let channels = config.channels as usize;

            let stream = match supported.sample_format() {
                cpal::SampleFormat::I16 => device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let chunk = convert_to_pipeline_pcm_i16(data, channels, in_rate);
                        let _ = tx.send(chunk);
                    },
                    |err| {
                        tracing::warn!(%err, "cpal input stream error");
                    },
                    None,
                ),
                cpal::SampleFormat::U16 => device.build_input_stream(
                    &config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        let chunk = convert_to_pipeline_pcm_u16(data, channels, in_rate);
                        let _ = tx.send(chunk);
                    },
                    |err| {
                        tracing::warn!(%err, "cpal input stream error");
                    },
                    None,
                ),
                cpal::SampleFormat::F32 => device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let chunk = convert_to_pipeline_pcm_f32(data, channels, in_rate);
                        let _ = tx.send(chunk);
                    },
                    |err| {
                        tracing::warn!(%err, "cpal input stream error");
                    },
                    None,
                ),
                other => {
                    let _ = init_tx.send(Err(CaptureError::Device(format!(
                        "unsupported sample format: {other:?}"
                    ))));
                    return;
                }
            };
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    let _ = init_tx.send(Err(CaptureError::Device(format!(
                        "failed to build input stream: {e}"
                    ))));
                    return;
                }
            };
            if stream.play().is_err() {
                let _ = init_tx.send(Err(CaptureError::Device(
                    "failed to start input stream".to_string(),
                )));
                return;
            }
            let _ = init_tx.send(Ok(()));
            loop {
                thread::park();
            }
        });
        match init_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(Self { rx, device_name }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CaptureError::Device(
                "timed out waiting for microphone stream start".to_string(),
            )),
        }
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }
}

fn convert_to_pipeline_pcm_i16(data: &[i16], channels: usize, input_rate: u32) -> Vec<i16> {
    let mono = to_mono_f32(data, channels, |v| v as f32 / i16::MAX as f32);
    resample_to_16k_i16(&mono, input_rate)
}

fn convert_to_pipeline_pcm_u16(data: &[u16], channels: usize, input_rate: u32) -> Vec<i16> {
    let mono = to_mono_f32(data, channels, |v| (v as f32 / u16::MAX as f32) * 2.0 - 1.0);
    resample_to_16k_i16(&mono, input_rate)
}

fn convert_to_pipeline_pcm_f32(data: &[f32], channels: usize, input_rate: u32) -> Vec<i16> {
    let mono = to_mono_f32(data, channels, |v| v);
    resample_to_16k_i16(&mono, input_rate)
}

fn to_mono_f32<T: Copy>(data: &[T], channels: usize, to_f32: impl Fn(T) -> f32) -> Vec<f32> {
    if channels <= 1 {
        return data.iter().copied().map(to_f32).collect();
    }
    data.chunks(channels)
        .map(|frame| to_f32(frame[0]))
        .collect()
}

fn resample_to_16k_i16(input: &[f32], input_rate: u32) -> Vec<i16> {
    if input.is_empty() {
        return Vec::new();
    }
    if input_rate == SAMPLE_RATE {
        return input.iter().copied().map(f32_to_i16).collect();
    }
    let step = input_rate as f32 / SAMPLE_RATE as f32;
    if step <= 0.0 {
        return input.iter().copied().map(f32_to_i16).collect();
    }
    let mut out = Vec::with_capacity((input.len() as f32 / step).max(1.0) as usize);
    let mut idx = 0.0_f32;
    while (idx as usize) < input.len() {
        out.push(f32_to_i16(input[idx as usize]));
        idx += step;
    }
    out
}

fn f32_to_i16(sample: f32) -> i16 {
    let v = sample.clamp(-1.0, 1.0);
    (v * i16::MAX as f32).round() as i16
}

impl AudioCapture for CpalCapture {
    fn read_chunk(&mut self, timeout: Duration) -> Result<Vec<i16>, CaptureError> {
        self.rx
            .recv_timeout(timeout)
            .map_err(|_| CaptureError::Timeout)
    }
}
