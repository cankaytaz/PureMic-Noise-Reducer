//! Virtual audio driver installation.
//!
//! macOS:   Installs PureMicDriver.driver (Core Audio HAL plugin) via osascript (admin prompt).
//! Windows: Installs VB-Cable (WASAPI virtual audio device) via UAC-elevated PowerShell.

use anyhow::Result;


/// Returns true if the virtual audio driver is already installed.
pub fn is_driver_installed() -> bool {
    #[cfg(target_os = "macos")]
    { macos::is_installed() }
    #[cfg(target_os = "windows")]
    { windows::is_installed() }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { false }
}

/// Install the virtual audio driver bundled inside the app's resources.
pub fn install_driver(app_handle: &tauri::AppHandle) -> Result<()> {
    #[cfg(target_os = "macos")]
    { macos::install(app_handle) }
    #[cfg(target_os = "windows")]
    { windows::install(app_handle) }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { let _ = app_handle; Err(anyhow!("Driver installation is not supported on this platform")) }
}

/// Uninstall the virtual audio driver.
pub fn uninstall_driver() -> Result<()> {
    #[cfg(target_os = "macos")]
    { macos::uninstall() }
    #[cfg(target_os = "windows")]
    { windows::uninstall() }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { Err(anyhow!("Driver uninstallation is not supported on this platform")) }
}

// ── macOS ──────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod macos {
    use std::path::Path;
    use std::process::Command;
    use anyhow::{anyhow, Result};
    use tauri::Manager;

    const DRIVER_NAME: &str = "PureMicDriver.driver";
    const INSTALL_DIR: &str = "/Library/Audio/Plug-Ins/HAL";

    pub fn is_installed() -> bool {
        // Check for Info.plist inside the bundle to avoid false positives
        // from an incomplete or broken copy.
        let target = format!("{}/{}/Contents/Info.plist", INSTALL_DIR, DRIVER_NAME);
        Path::new(&target).exists()
    }

    pub fn uninstall() -> Result<()> {
        let target = format!("{}/{}", INSTALL_DIR, DRIVER_NAME);
        let script = format!(
            r#"do shell script "rm -rf '{}' && killall coreaudiod" with administrator privileges"#,
            target
        );

        tracing::info!("Uninstalling virtual audio driver...");

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| anyhow!("Failed to run osascript: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("User canceled") || stderr.contains("-128") {
                return Err(anyhow!("Permission denied. Driver was not removed."));
            }
            return Err(anyhow!("Driver uninstallation failed: {}", stderr));
        }

        tracing::info!("Virtual audio driver uninstalled successfully.");
        Ok(())
    }

    pub fn install(app_handle: &tauri::AppHandle) -> Result<()> {
        let resource_path = app_handle
            .path()
            .resource_dir()
            .map_err(|e| anyhow!("Cannot find resource dir: {}", e))?;

        // Tauri maps "../driver/build/X" → Resources/_up_/driver/build/X
        let primary  = resource_path.join("_up_").join("driver").join("build").join(DRIVER_NAME);
        let fallback = resource_path.join(DRIVER_NAME);

        let source = if primary.exists() {
            primary
        } else if fallback.exists() {
            fallback
        } else {
            return Err(anyhow!(
                "Driver bundle not found at {:?} or {:?}",
                primary, fallback
            ));
        };

        install_from(&source)
    }

    fn install_from(source: &Path) -> Result<()> {
        let tmp_source = "/tmp/PureMicDriver.driver";

        // Stage to /tmp as the regular user first.
        // The root process started by osascript often lacks read access to the
        // user's home directory due to macOS TCC policies.
        let _ = Command::new("rm").arg("-rf").arg(tmp_source).output();
        let cp_out = Command::new("cp")
            .args(["-R", &source.to_string_lossy(), tmp_source])
            .output()
            .map_err(|e| anyhow!("Failed to copy driver to temp folder: {}", e))?;

        if !cp_out.status.success() {
            return Err(anyhow!(
                "Failed to stage driver: {}",
                String::from_utf8_lossy(&cp_out.stderr)
            ));
        }

        let target = format!("{}/{}", INSTALL_DIR, DRIVER_NAME);
        let script = format!(
            r#"do shell script "rm -rf '{}' && ditto '{}' '{}' && chown -R root:wheel '{}' && killall coreaudiod" with administrator privileges"#,
            target, tmp_source, target, target
        );

        tracing::info!("Installing virtual audio driver from {:?}...", tmp_source);

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| anyhow!("Failed to run osascript: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("User canceled") || stderr.contains("-128") {
                return Err(anyhow!("Permission denied. Virtual microphone could not be installed."));
            }
            return Err(anyhow!("Driver installation failed: {}", stderr));
        }

        tracing::info!("Virtual audio driver installed successfully!");
        // Give CoreAudio time to pick up the new device before the caller queries devices.
        std::thread::sleep(std::time::Duration::from_secs(2));
        Ok(())
    }
}

// ── Windows ────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod windows {
    use std::process::Command;
    use anyhow::{anyhow, Result};
    use cpal::traits::{DeviceTrait, HostTrait};
    use tauri::Manager;

    pub fn is_installed() -> bool {
        // After installation and rename, the virtual mic appears as "PureMic".
        // Before rename (or if rename failed), it appears as "CABLE Output" / "VB-Audio".
        let host = cpal::default_host();
        host.input_devices()
            .ok()
            .map(|mut devs| {
                devs.any(|d| {
                    d.name()
                        .map(|n| n.contains("PureMic") || n.contains("CABLE") || n.contains("VB-Audio"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    pub fn install(app_handle: &tauri::AppHandle) -> Result<()> {
        let resource_path = app_handle
            .path()
            .resource_dir()
            .map_err(|e| anyhow!("Cannot find resource dir: {}", e))?;

        // Tauri maps "../driver/windows/X" → Resources/_up_/driver/windows/X
        let driver_dir_primary = resource_path.join("_up_").join("driver").join("windows");
        let driver_dir_fallback = resource_path.clone();

        // Look for VBCABLE_Setup_x64.exe — try primary path then fallback
        let setup_exe = if driver_dir_primary.join("VBCABLE_Setup_x64.exe").exists() {
            driver_dir_primary.join("VBCABLE_Setup_x64.exe")
        } else if driver_dir_fallback.join("VBCABLE_Setup_x64.exe").exists() {
            driver_dir_fallback.join("VBCABLE_Setup_x64.exe")
        } else {
            return Err(anyhow!(
                "VB-Cable installer not found. Expected VBCABLE_Setup_x64.exe at: {:?}",
                driver_dir_primary
            ));
        };

        tracing::info!("Launching VB-Cable installer (with UI) from {:?}...", setup_exe);

        // Launch VB-Cable installer WITH its UI so user can see and confirm it.
        // We use ShellExecuteW "runas" verb so Windows shows the UAC prompt properly.
        // CREATE_NO_WINDOW (0x08000000) prevents a stray cmd/console window.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let status = Command::new(&setup_exe)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| anyhow!("Failed to launch VB-Cable installer: {}", e))?;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            // Exit code 1 = user cancelled NSIS installer
            if code == 1 {
                return Err(anyhow!("Installation was cancelled."));
            }
            return Err(anyhow!("VB-Cable installer exited with code {}", code));
        }

        tracing::info!("VB-Cable installed. Waiting for Windows Audio to register devices...");
        std::thread::sleep(std::time::Duration::from_secs(5));

        // Rename CABLE→PureMic via a hidden PowerShell process (no visible window).
        // Stop Windows Audio first so it releases its device name cache, then rename, then restart.
        let rename_ps1 = r#"$ErrorActionPreference = 'SilentlyContinue'
Stop-Service -Name 'AudioSrv' -Force
Stop-Service -Name 'AudioEndpointBuilder' -Force
Start-Sleep -Seconds 1
$k = '{b3f8fa53-0004-438e-9003-51a46e139bfc},6'
$cap = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture'
Get-ChildItem $cap | ForEach-Object {
    $p = Join-Path $_.PSPath 'Properties'
    $v = (Get-ItemProperty $p -Name $k -EA 0).$k
    if ($v -like '*CABLE*') { Set-ItemProperty $p $k 'PureMic' }
}
$ren = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render'
Get-ChildItem $ren | ForEach-Object {
    $p = Join-Path $_.PSPath 'Properties'
    $v = (Get-ItemProperty $p -Name $k -EA 0).$k
    if ($v -like '*CABLE*') { Set-ItemProperty $p $k 'PureMic [Internal]' }
}
Start-Service -Name 'AudioEndpointBuilder'
Start-Service -Name 'AudioSrv'
"#;
        let rename_path = std::env::temp_dir().join("puremic_rename.ps1");
        std::fs::write(&rename_path, rename_ps1)
            .map_err(|e| anyhow!("Failed to write rename script: {}", e))?;

        // Run rename elevated + hidden. Start-Process -Verb RunAs handles UAC,
        // -WindowStyle Hidden + CREATE_NO_WINDOW ensures no console appears.
        let elevate = format!(
            "Start-Process powershell -Verb RunAs -WindowStyle Hidden -Wait -ArgumentList '-NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File \"{}\"'",
            rename_path.display()
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &elevate])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let _ = std::fs::remove_file(&rename_path);

        // Write uninstall script to ProgramData for NSIS PREUNINSTALL hook.
        let programdata = std::env::var("PROGRAMDATA")
            .unwrap_or_else(|_| r"C:\ProgramData".to_string());
        let script_dir = std::path::Path::new(&programdata).join("PureMic");
        let _ = std::fs::create_dir_all(&script_dir);
        let uninstall_ps1 = r#"$ErrorActionPreference = 'SilentlyContinue'
$out = & pnputil /enum-drivers /class 'Media' 2>&1
$lines = $out -split "`r?`n"; $inf = $null
foreach ($l in $lines) {
    if ($l -match 'Published Name\s*:\s*(\S+\.inf)') { $inf = $Matches[1] }
    if (($l -match 'VB-Audio' -or $l -match 'vbMme' -or $l -match 'VBCABLE') -and $inf) {
        & pnputil /delete-driver $inf /uninstall /force; $inf = $null
    }
}
"#;
        let _ = std::fs::write(script_dir.join("uninstall-driver.ps1"), uninstall_ps1);

        tracing::info!("VB-Cable installed and renamed to PureMic.");
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        let script_content = r#"
$ErrorActionPreference = 'SilentlyContinue'

# Find and delete the VB-Cable/PureMic driver package from the driver store
$driverOutput = & pnputil /enum-drivers /class "Media" 2>&1
$lines = $driverOutput -split "`r?`n"
$currentInf = $null
foreach ($line in $lines) {
    if ($line -match 'Published Name\s*:\s*(\S+\.inf)') {
        $currentInf = $Matches[1]
    }
    if (($line -match 'VB-Audio' -or $line -match 'vbMme' -or $line -match 'VBCABLE') -and $currentInf) {
        & pnputil /delete-driver $currentInf /uninstall /force
        $currentInf = $null
    }
}
"#;

        let script_path = std::env::temp_dir().join("puremic_driver_uninstall.ps1");
        std::fs::write(&script_path, script_content)
            .map_err(|e| anyhow!("Failed to write uninstall script: {}", e))?;

        let elevate_cmd = format!(
            "Start-Process powershell -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File \"{}\"' -Verb RunAs -Wait",
            script_path.display()
        );

        tracing::info!("Uninstalling VB-Cable driver...");

        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &elevate_cmd])
            .output()
            .map_err(|e| anyhow!("Failed to launch uninstaller: {}", e))?;

        let _ = std::fs::remove_file(&script_path);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("canceled") || stderr.contains("cancelled") || stderr.contains("1223") {
                return Err(anyhow!("Permission denied. Driver was not removed."));
            }
            return Err(anyhow!("Driver uninstallation failed: {}", stderr));
        }

        tracing::info!("VB-Cable driver uninstalled successfully.");
        Ok(())
    }
}
