//! Audio pipeline: mic → RNNoise → virtual mic (BlackHole) + optional monitoring.
//!
//! Two independent output paths:
//! 1. Virtual device (BlackHole/VB-Cable): always active when NC is on,
//!    so other apps (Discord, games) pick up the denoised audio.
//! 2. Monitoring output: optional, for the user to hear themselves.
//!
//! RNNoise expects int16-scale samples: multiply by 32768 before, divide after.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig, SupportedStreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter};

use crate::rnnoise::denoiser::{Denoiser, FRAME_SIZE};
use super::eq::{WarmthEQ, EQ_ENABLED};
use super::AudioDevice;

// ─── Shared atomic controls ─────────────────────────────────────────────────
pub static AUDIO_LEVEL: AtomicU32 = AtomicU32::new(0);
pub static INPUT_GAIN: AtomicU32 = AtomicU32::new(0x3F800000);  // 1.0f32
pub static OUTPUT_GAIN: AtomicU32 = AtomicU32::new(0x3F800000); // 1.0f32
pub static DENOISE_ENABLED: AtomicBool = AtomicBool::new(true);
pub static DENOISE_HARD_MODE: AtomicBool = AtomicBool::new(false);
pub static ACTIVE_PIPELINE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn load_gain(atom: &AtomicU32) -> f32 {
    f32::from_bits(atom.load(Ordering::Relaxed))
}

// ─── Known virtual audio device name patterns ───────────────────────────────
const VIRTUAL_DEVICE_PATTERNS: &[&str] = &[
    "PureMic",             // Our own driver (highest priority)
    "Purea",               // Legacy driver name
    "NoiseCancellation",   // Legacy driver name
    "BlackHole",
    "VB-Cable",
    "VB-Audio",
    "CABLE Input",
    "Soundflower",
    "Loopback",
];

fn is_virtual_device(name: &str) -> bool {
    VIRTUAL_DEVICE_PATTERNS.iter().any(|p| name.contains(p))
}

// ─── Global pipeline state ──────────────────────────────────────────────────
struct PipelineState {
    _input_stream: Stream,
    _monitor_stream: Option<Stream>,
    _virtual_stream: Option<Stream>,
}
unsafe impl Send for PipelineState {}

static PIPELINE: Mutex<Option<PipelineState>> = Mutex::new(None);

// ─── Public API ─────────────────────────────────────────────────────────────
pub struct AudioPipeline;

impl AudioPipeline {
    pub fn list_input_devices() -> Result<Vec<AudioDevice>> {
        let host = cpal::default_host();
        let default_name = host
            .default_input_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        Ok(host
            .input_devices()?
            .filter_map(|d| {
                let name = d.name().ok()?;
                // Hide our own virtual device to prevent infinite loops and user confusion
                if name.contains("PureMic") || name.contains("Purea") || name.contains("NoiseCancellation") {
                    return None;
                }
                d.default_input_config().ok()?;
                let is_default = name == default_name;
                Some(AudioDevice { id: name.clone(), is_default, name })
            })
            .collect())
    }

    pub fn list_output_devices() -> Result<Vec<AudioDevice>> {
        let host = cpal::default_host();
        let default_name = host
            .default_output_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        Ok(host
            .output_devices()?
            .filter_map(|d| {
                let name = d.name().ok()?;
                d.default_output_config().ok()?;
                let is_default = name == default_name;
                Some(AudioDevice { id: name.clone(), is_default, name })
            })
            .collect())
    }

    /// Detect installed virtual audio devices (BlackHole, VB-Cable etc.)
    pub fn detect_virtual_device() -> Option<String> {
        let host = cpal::default_host();
        host.output_devices().ok()?.find_map(|d| {
            let name = d.name().ok()?;
            if is_virtual_device(&name) { Some(name) } else { None }
        })
    }

    /// Start the pipeline.
    /// - `input_id`    — mic device name (None = system default)
    /// - `monitor_id`  — monitoring output for user to hear (None = no monitoring)
    /// - `virtual_id`  — virtual device for other apps (None = auto-detect)
    pub fn start(
        input_id: Option<String>,
        monitor_id: Option<String>,
        virtual_id: Option<String>,
        app: AppHandle,
    ) -> Result<()> {
        // Increment and capture unique ID for this specific pipeline session
        let my_id = ACTIVE_PIPELINE_ID.fetch_add(1, Ordering::SeqCst) + 1;

        // Stop any existing pipeline first
        Self::stop()?;
        
        // Re-set the ID to ours (stop() sets it to 0)
        ACTIVE_PIPELINE_ID.store(my_id, Ordering::SeqCst);

        let host = cpal::default_host();

        // ── Input device ─────────────────────────────────────────────────────
        let input_device = Self::find_input(&host, input_id.as_deref())?;
        let in_supported = Self::best_f32_config(&input_device, true)?;
        let in_cfg: StreamConfig = in_supported.clone().into();
        let in_channels = in_cfg.channels as usize;
        let in_rate = in_cfg.sample_rate.0 as f64;

        tracing::info!(
            "Input:  {} | {} ch | {} Hz | {:?}",
            input_device.name()?,
            in_channels,
            in_cfg.sample_rate.0,
            in_supported.sample_format()
        );

        let rnnoise_rate = 48_000f64;

        // ── Helper: build an output stream + ring buffer producer ────────────
        struct OutputInfo {
            stream: Stream,
            prod: ringbuf::HeapProd<f32>,
            rate: f64,
        }

        let build_output = |device_name: &str, label: &str| -> Result<OutputInfo> {
            let device = Self::find_output(&host, Some(device_name))?;
            // Use the device's default config to avoid AUDCLNT_E_WRONG_ENDPOINT_BUFFER_SIZE
            // (0x8889000A) on Windows WASAPI — especially for virtual devices like VB-Cable.
            let supported = device.default_output_config()
                .or_else(|_| Self::best_f32_config(&device, false))?;
            let cfg: StreamConfig = supported.into();
            let channels = cfg.channels as usize;
            let rate = cfg.sample_rate.0 as f64;

            tracing::info!("{}: {} | {} ch | {} Hz", label, device_name, channels, rate);

            // Keep ring buffer small: ~8 frames (~80ms at 48kHz).
            // Larger buffers cause latency buildup = metallic/phaser sound.
            let rb = ringbuf::HeapRb::<f32>::new(FRAME_SIZE * 8);
            let (prod, mut cons) = rb.split();

            let is_monitor = label == "Monitor";

            // Pre-allocate temp buffer outside the closure
            let out_frames_max = 8192; // generous upper bound
            let mut temp = vec![0f32; out_frames_max];

            let stream = device.build_output_stream(
                &cfg,
                move |data: &mut [f32], _| {
                    if ACTIVE_PIPELINE_ID.load(Ordering::SeqCst) != my_id {
                        for s in data.iter_mut() { *s = 0.0; }
                        return;
                    }
                    let gain = if is_monitor { load_gain(&OUTPUT_GAIN) } else { 1.0 };
                    let frames = data.len() / channels;

                    // Anti-latency: if ring buffer has way more data than we need,
                    // skip ahead to stay near real-time. This prevents the
                    // "metallic/phaser" effect caused by growing delay.
                    let buffered = cons.occupied_len();
                    let max_buffered = FRAME_SIZE * 4; // ~40ms max
                    if buffered > max_buffered + frames {
                        let skip = buffered - max_buffered;
                        let mut discard = vec![0f32; skip];
                        cons.pop_slice(&mut discard);
                    }

                    if frames > temp.len() {
                        temp.resize(frames, 0.0);
                    }
                    let read = cons.pop_slice(&mut temp[..frames]);
                    for (i, ch_frame) in data.chunks_mut(channels).enumerate() {
                        let s = if i < read { temp[i] * gain } else { 0.0 };
                        for ch in ch_frame.iter_mut() {
                            *ch = s;
                        }
                    }
                },
                |err| tracing::error!("Output error: {err}"),
                None,
            )?;
            stream.play()?;
            Ok(OutputInfo { stream, prod, rate })
        };

        // ── Monitor output (optional) ────────────────────────────────────────
        let monitor = monitor_id
            .as_deref()
            .map(|id| build_output(id, "Monitor"))
            .transpose()?;

        // ── Virtual device output (auto-detect if not specified) ─────────────
        let virt_name = virtual_id.or_else(Self::detect_virtual_device);
        let virtual_out = virt_name
            .as_deref()
            // Don't route to virtual device if it's the same as monitor
            .filter(|v| monitor_id.as_deref() != Some(*v))
            .map(|id| build_output(id, "Virtual"))
            .transpose()?;

        if virtual_out.is_none() {
            tracing::warn!("No virtual audio device found (BlackHole/VB-Cable). \
                Other apps won't receive denoised audio.");
        }

        // ── Unpack into closures ─────────────────────────────────────────────
        let (monitor_stream, mut mon_prod) = match monitor {
            Some(m) => (Some(m.stream), Some((m.rate, m.prod))),
            None => (None, None),
        };

        let (virtual_stream, mut virt_prod) = match virtual_out {
            Some(v) => (Some(v.stream), Some((v.rate, v.prod))),
            None => (None, None),
        };

        // ── Input callback ───────────────────────────────────────────────────
        let denoiser = Arc::new(Mutex::new(Denoiser::new()));
        let mut warmth_eq = WarmthEQ::new(rnnoise_rate);

        // Use a fixed ring-style accumulator to avoid unbounded Vec growth.
        // Max accumulator size: 4 frames worth. If more arrives, drop oldest.
        let max_accum = FRAME_SIZE * 4;
        let mut accumulator: Vec<f32> = Vec::with_capacity(max_accum);

        let mut level_accum = 0f32;
        let mut level_count = 0usize;
        let level_emit_every = (rnnoise_rate * 0.05) as usize;
        let mut frames_processed = 0usize;
        let log_every = (rnnoise_rate * 5.0) as usize;
        let app_clone = app.clone();

        // Smoothing state for "Hard Reduce" noise gate
        let mut hard_gate_gain = 1.0f32;

        // Pre-allocated reusable buffers to avoid per-callback allocations
        let mut resample_buf: Vec<f32> = Vec::with_capacity(FRAME_SIZE * 2);

        let input_stream = input_device.build_input_stream(
            &in_cfg,
            move |data: &[f32], _| {
                if ACTIVE_PIPELINE_ID.load(Ordering::SeqCst) != my_id {
                    return;
                }
                let i_gain = load_gain(&INPUT_GAIN);
                let denoise = DENOISE_ENABLED.load(Ordering::Relaxed);

                // 1. Downmix to mono + input gain
                let mono: Vec<f32> = data
                    .chunks(in_channels)
                    .map(|ch| (ch.iter().sum::<f32>() / in_channels as f32) * i_gain)
                    .collect();

                // 2. Resample to 48 kHz
                let at_48k = if (in_rate - rnnoise_rate).abs() < 1.0 {
                    mono
                } else {
                    resample_linear(&mono, in_rate, rnnoise_rate)
                };

                // 3. RMS for visualizer (pre-denoise)
                for &s in &at_48k {
                    level_accum += s * s;
                    level_count += 1;
                }
                if level_count >= level_emit_every {
                    let rms = (level_accum / level_count as f32).sqrt();
                    AUDIO_LEVEL.store(rms.to_bits(), Ordering::Relaxed);
                    let _ = app_clone.emit("audio-level", rms);
                    level_accum = 0.0;
                    level_count = 0;
                }

                frames_processed += at_48k.len();
                if frames_processed >= log_every {
                    tracing::info!("Audio engine heartbeat: processed {} samples", frames_processed);
                    frames_processed = 0;
                }

                // 4. Accumulate samples for FRAME_SIZE processing
                accumulator.extend_from_slice(&at_48k);

                // Safety valve: if accumulator grows too large (consumer can't keep up),
                // drop oldest samples to prevent unbounded latency buildup.
                // This is the key fix for the "metallic sound over time" issue.
                if accumulator.len() > max_accum {
                    let excess = accumulator.len() - max_accum;
                    accumulator.drain(..excess);
                    tracing::warn!("Accumulator overflow: dropped {} samples to prevent latency buildup", excess);
                }

                // Process complete frames
                let mut read_pos = 0;
                while read_pos + FRAME_SIZE <= accumulator.len() {
                    let mut frame = [0f32; FRAME_SIZE];
                    frame.copy_from_slice(&accumulator[read_pos..read_pos + FRAME_SIZE]);
                    read_pos += FRAME_SIZE;

                    if denoise {
                        // RNNoise expects int16-scale samples
                        for s in frame.iter_mut() {
                            *s *= 32768.0;
                        }

                        let vad = denoiser.lock().unwrap().process_frame(&mut frame);

                        // Hard mode: Aggressive VAD-based noise gate
                        let is_hard = DENOISE_HARD_MODE.load(Ordering::Relaxed);
                        if is_hard {
                            let target_gain = if vad < 0.20 { 0.05 } else { 1.0 };
                            hard_gate_gain = hard_gate_gain * 0.8 + target_gain * 0.2;

                            for s in frame.iter_mut() {
                                *s *= hard_gate_gain;
                            }
                        } else {
                            hard_gate_gain = 1.0;
                        }

                        for s in frame.iter_mut() {
                            *s /= 32768.0;
                        }

                        // Post-denoise EQ: warm up the thin/metallic RNNoise output
                        if EQ_ENABLED.load(Ordering::Relaxed) {
                            warmth_eq.process_frame(&mut frame);
                        }
                    }

                    // 5a. Push to virtual device (drop oldest in ring buffer if full)
                    if let Some((vrate, ref mut prod)) = virt_prod {
                        if (rnnoise_rate - vrate).abs() < 1.0 {
                            let _ = prod.push_slice(&frame);
                        } else {
                            resample_buf.clear();
                            resample_into(&frame, rnnoise_rate, vrate, &mut resample_buf);
                            let _ = prod.push_slice(&resample_buf);
                        }
                    }

                    // 5b. Push to monitoring output
                    if let Some((mrate, ref mut prod)) = mon_prod {
                        if (rnnoise_rate - mrate).abs() < 1.0 {
                            let _ = prod.push_slice(&frame);
                        } else {
                            resample_buf.clear();
                            resample_into(&frame, rnnoise_rate, mrate, &mut resample_buf);
                            let _ = prod.push_slice(&resample_buf);
                        }
                    }
                }

                // Efficiently remove processed samples: shift remaining to front
                if read_pos > 0 {
                    accumulator.drain(..read_pos);
                }
            },
            |err| tracing::error!("Input error: {err}"),
            None,
        )?;

        input_stream.play()?;

        *PIPELINE.lock().unwrap() = Some(PipelineState {
            _input_stream: input_stream,
            _monitor_stream: monitor_stream,
            _virtual_stream: virtual_stream,
        });

        tracing::info!("Pipeline started (monitoring: {}, virtual: {}, denoise: {})",
            monitor_id.is_some(),
            virt_name.is_some(),
            DENOISE_ENABLED.load(Ordering::Relaxed));
        Ok(())
    }

    pub fn stop() -> Result<()> {
        ACTIVE_PIPELINE_ID.store(0, Ordering::SeqCst);
        let prev = PIPELINE.lock().unwrap().take();
        if prev.is_some() {
            // Drop streams explicitly, then give CoreAudio a moment to release resources.
            // This prevents stale audio data from bleeding into the next pipeline session.
            drop(prev);
            std::thread::sleep(std::time::Duration::from_millis(50));
            AUDIO_LEVEL.store(0f32.to_bits(), Ordering::Relaxed);
            tracing::info!("Pipeline stopped");
        }
        Ok(())
    }

    pub fn is_running() -> bool {
        PIPELINE.lock().unwrap().is_some()
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn best_f32_config(device: &Device, is_input: bool) -> Result<SupportedStreamConfig> {
        let configs: Vec<_> = if is_input {
            device.supported_input_configs()?.collect()
        } else {
            device.supported_output_configs()?.collect()
        };

        let f32_cfg = configs.iter()
            .filter(|c| c.sample_format() == SampleFormat::F32)
            .min_by_key(|c| {
                let rate_diff = (c.max_sample_rate().0 as i64 - 48_000).abs();
                let ch_score = if c.channels() <= 2 { 0 } else { c.channels() as i64 };
                rate_diff + ch_score * 1000
            })
            .map(|c| {
                let rate = c.max_sample_rate().0.min(48_000).max(c.min_sample_rate().0);
                c.clone().with_sample_rate(cpal::SampleRate(rate))
            });

        if let Some(cfg) = f32_cfg {
            return Ok(cfg);
        }

        if is_input {
            device.default_input_config().map_err(|e| anyhow!(e))
        } else {
            device.default_output_config().map_err(|e| anyhow!(e))
        }
    }

    fn find_input(host: &cpal::Host, name: Option<&str>) -> Result<Device> {
        match name {
            None => host.default_input_device().ok_or_else(|| anyhow!("No default input device")),
            Some(id) => host
                .input_devices()?
                .find(|d| d.name().map(|n| n == id).unwrap_or(false))
                .ok_or_else(|| anyhow!("Input device '{}' not found", id)),
        }
    }

    fn find_output(host: &cpal::Host, name: Option<&str>) -> Result<Device> {
        match name {
            None => host.default_output_device().ok_or_else(|| anyhow!("No default output device")),
            Some(id) => host
                .output_devices()?
                .find(|d| d.name().map(|n| n == id).unwrap_or(false))
                .ok_or_else(|| anyhow!("Output device '{}' not found", id)),
        }
    }
}

/// Linear interpolation resampler (allocating version).
fn resample_linear(input: &[f32], from_rate: f64, to_rate: f64) -> Vec<f32> {
    let mut out = Vec::new();
    resample_into(input, from_rate, to_rate, &mut out);
    out
}

/// Linear interpolation resampler (non-allocating: appends to existing buffer).
fn resample_into(input: &[f32], from_rate: f64, to_rate: f64, out: &mut Vec<f32>) {
    if input.is_empty() { return; }
    let ratio = from_rate / to_rate;
    let out_len = (input.len() as f64 / ratio).ceil() as usize;
    out.reserve(out_len);
    for i in 0..out_len {
        let src = i as f64 * ratio;
        let lo = src.floor() as usize;
        let hi = (lo + 1).min(input.len() - 1);
        let t = (src - lo as f64) as f32;
        out.push(input[lo] * (1.0 - t) + input[hi] * t);
    }
}
