use std::sync::Arc;
use tauri::{AppHandle, State, Manager};
use tokio::sync::Mutex;
use std::sync::atomic::Ordering;

use crate::{
    audio::pipeline::{AudioPipeline, AUDIO_LEVEL, INPUT_GAIN, OUTPUT_GAIN, DENOISE_ENABLED, DENOISE_HARD_MODE},
    audio::eq::{EQ_ENABLED, EQ_BASS_DB10, EQ_MID_DB10, EQ_TREBLE_DB10},
    audio::driver_installer,
    state::AppState,
};
use super::AudioDevice;

#[tauri::command]
pub async fn get_microphones(
    _state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<AudioDevice>, String> {
    AudioPipeline::list_input_devices()
        .map(|devs| devs.into_iter().map(|d| AudioDevice { id: d.id, name: d.name, is_default: d.is_default }).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_output_devices(
    _state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<Vec<AudioDevice>, String> {
    AudioPipeline::list_output_devices()
        .map(|devs| devs.into_iter().map(|d| AudioDevice { id: d.id, name: d.name, is_default: d.is_default }).collect())
        .map_err(|e| e.to_string())
}

/// Returns the name of the detected virtual audio device.
#[tauri::command]
pub async fn detect_virtual_device() -> Option<String> {
    AudioPipeline::detect_virtual_device()
}

/// Check if our custom driver is installed.
#[tauri::command]
pub async fn is_driver_installed() -> bool {
    driver_installer::is_driver_installed()
}

/// Install the virtual audio driver (prompts for admin).
#[tauri::command]
pub async fn install_driver(app: AppHandle) -> Result<(), String> {
    driver_installer::install_driver(&app).map_err(|e| e.to_string())
}

/// Uninstall the virtual audio driver (prompts for admin).
#[tauri::command]
pub async fn uninstall_driver() -> Result<(), String> {
    driver_installer::uninstall_driver().map_err(|e| e.to_string())
}

/// Returns the current OS: "macos", "windows", or "other".
#[tauri::command]
pub async fn get_platform() -> &'static str {
    #[cfg(target_os = "macos")]   { "macos" }
    #[cfg(target_os = "windows")] { "windows" }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))] { "other" }
}

/// Returns the resolved resource directory path (debug helper).
#[tauri::command]
pub async fn get_resource_dir(app: AppHandle) -> Result<String, String> {
    app.path().resource_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_audio_level() -> f32 {
    f32::from_bits(AUDIO_LEVEL.load(Ordering::Relaxed))
}

/// Start the audio pipeline.
/// - `mic_id`:      input device (None = system default)
/// - `monitor_id`:  output for user to hear (None = no monitoring)
/// - `virtual_id`:  virtual device for other apps (None = auto-detect)
/// - `denoise`:     whether to run RNNoise
#[tauri::command]
pub async fn start_pipeline(
    mic_id: Option<String>,
    monitor_id: Option<String>,
    virtual_id: Option<String>,
    denoise: bool,
    state: State<'_, Arc<Mutex<AppState>>>,
    app: AppHandle,
) -> Result<(), String> {
    DENOISE_ENABLED.store(denoise, Ordering::Relaxed);

    let mut guard = state.lock().await;
    let input = mic_id.clone().or_else(|| guard.selected_mic_id.clone());

    AudioPipeline::start(input, monitor_id.clone(), virtual_id, app.clone())
        .map_err(|e| e.to_string())?;

    guard.is_active = true;
    if let Some(id) = mic_id { guard.selected_mic_id = Some(id); }
    if let Some(id) = monitor_id { guard.selected_output_id = Some(id); }
    
    if let Some(tray) = app.tray_by_id("main_tray") {
        let active_icon = tauri::image::Image::from_bytes(include_bytes!("../../icons/tray_active.png")).unwrap();
        let _ = tray.set_icon(Some(active_icon));
    }
    
    Ok(())
}

#[tauri::command]
pub async fn stop_pipeline(
    state: State<'_, Arc<Mutex<AppState>>>,
    app: AppHandle,
) -> Result<(), String> {
    let mut guard = state.lock().await;
    AudioPipeline::stop().map_err(|e| e.to_string())?;
    guard.is_active = false;
    
    if let Some(tray) = app.app_handle().tray_by_id("main_tray") {
        let inactive_icon = tauri::image::Image::from_bytes(include_bytes!("../../icons/tray_inactive.png")).unwrap();
        let _ = tray.set_icon(Some(inactive_icon));
    }
    
    Ok(())
}

/// Toggle denoising without restarting the pipeline.
#[tauri::command]
pub async fn set_denoise_enabled(enabled: bool) -> Result<(), String> {
    DENOISE_ENABLED.store(enabled, Ordering::Relaxed);
    tracing::info!("Denoise: {}", enabled);
    Ok(())
}

#[tauri::command]
pub async fn set_denoise_hard_mode(enabled: bool) -> Result<(), String> {
    DENOISE_HARD_MODE.store(enabled, Ordering::Relaxed);
    tracing::info!("Hard Reduce: {}", enabled);
    Ok(())
}

/// Toggle EQ on/off.
#[tauri::command]
pub async fn set_eq_enabled(enabled: bool) -> Result<(), String> {
    EQ_ENABLED.store(enabled, Ordering::Relaxed);
    tracing::info!("EQ enabled: {}", enabled);
    Ok(())
}

/// Get EQ enabled state.
#[tauri::command]
pub async fn get_eq_enabled() -> bool {
    EQ_ENABLED.load(Ordering::Relaxed)
}

/// Set EQ bands. Values in dB: range -12.0 to +12.0.
#[tauri::command]
pub async fn set_eq_bands(bass: f32, mid: f32, treble: f32) -> Result<(), String> {
    EQ_BASS_DB10.store((bass * 10.0) as i32, Ordering::Relaxed);
    EQ_MID_DB10.store((mid * 10.0) as i32, Ordering::Relaxed);
    EQ_TREBLE_DB10.store((treble * 10.0) as i32, Ordering::Relaxed);
    tracing::info!("EQ: bass={:.1}dB mid={:.1}dB treble={:.1}dB", bass, mid, treble);
    Ok(())
}

/// Get current EQ bands (dB values).
#[tauri::command]
pub async fn get_eq_bands() -> Result<(f32, f32, f32), String> {
    let bass = EQ_BASS_DB10.load(Ordering::Relaxed) as f32 / 10.0;
    let mid = EQ_MID_DB10.load(Ordering::Relaxed) as f32 / 10.0;
    let treble = EQ_TREBLE_DB10.load(Ordering::Relaxed) as f32 / 10.0;
    Ok((bass, mid, treble))
}

#[tauri::command]
pub async fn set_input_gain(gain: f32) -> Result<(), String> {
    INPUT_GAIN.store(gain.to_bits(), Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn set_output_gain(gain: f32) -> Result<(), String> {
    OUTPUT_GAIN.store(gain.to_bits(), Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn set_microphone(
    device_id: String,
    state: State<'_, Arc<Mutex<AppState>>>,
    app: AppHandle,
) -> Result<(), String> {
    let mut guard = state.lock().await;
    guard.selected_mic_id = Some(device_id.clone());
    if guard.is_active {
        let output = guard.selected_output_id.clone();
        AudioPipeline::start(Some(device_id), output, None, app.clone())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_output_device(
    device_id: String,
    state: State<'_, Arc<Mutex<AppState>>>,
    app: AppHandle,
) -> Result<(), String> {
    let mut guard = state.lock().await;
    guard.selected_output_id = Some(device_id.clone());
    if guard.is_active {
        let input = guard.selected_mic_id.clone();
        AudioPipeline::start(input, Some(device_id), None, app.clone())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
