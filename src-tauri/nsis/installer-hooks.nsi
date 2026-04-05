; PureMic NSIS installer hooks — perMachine (runs as Administrator)
;
; POSTINSTALL: Files are already extracted to $INSTDIR here.
;   1. Launch VB-Cable installer WITH its normal UI (ExecWait — no /S)
;   2. After user finishes VB setup, stop AudioSrv, rename CABLE→PureMic, restart AudioSrv

!macro NSIS_HOOK_POSTINSTALL
  StrCpy $R9 "$INSTDIR\_up_\driver\windows\VBCABLE_Setup_x64.exe"
  IfFileExists "$R9" do_vb_install
    DetailPrint "VB-Cable installer not found — skipping driver setup."
    Goto driver_done

  do_vb_install:
    DetailPrint "Installing VB-Audio Virtual Cable driver..."
    ; ExecWait without /S = normal UI, user sees the VB-Cable installer and UAC prompt
    ExecWait '"$R9"'

    ; Wait for Windows Audio to enumerate the new devices
    Sleep 6000

    ; Stop Windows Audio service so our registry rename sticks (it caches names in memory)
    DetailPrint "Configuring PureMic audio device names..."
    ReadEnvStr $R8 "PROGRAMDATA"
    StrCmp $R8 "" 0 +2
      StrCpy $R8 "C:\ProgramData"
    CreateDirectory "$R8\PureMic"

    FileOpen $0 "$R8\PureMic\rename-devices.ps1" w
    FileWrite $0 "$$ErrorActionPreference='SilentlyContinue'$\r$\n"
    FileWrite $0 "Stop-Service -Name 'AudioSrv' -Force$\r$\n"
    FileWrite $0 "Stop-Service -Name 'AudioEndpointBuilder' -Force$\r$\n"
    FileWrite $0 "Start-Sleep -Seconds 1$\r$\n"
    FileWrite $0 "$$k='{b3f8fa53-0004-438e-9003-51a46e139bfc},6'$\r$\n"
    FileWrite $0 "$$cap='HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture'$\r$\n"
    FileWrite $0 "Get-ChildItem $$cap|%{$$p=Join-Path $$_.PSPath 'Properties';$$v=(Get-ItemProperty $$p -Name $$k -EA 0).$$k;if($$v -like '*CABLE*'){Set-ItemProperty $$p $$k 'PureMic'}}$\r$\n"
    FileWrite $0 "$$ren='HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render'$\r$\n"
    FileWrite $0 "Get-ChildItem $$ren|%{$$p=Join-Path $$_.PSPath 'Properties';$$v=(Get-ItemProperty $$p -Name $$k -EA 0).$$k;if($$v -like '*CABLE*'){Set-ItemProperty $$p $$k 'PureMic [Internal]'}}$\r$\n"
    FileWrite $0 "Start-Service -Name 'AudioEndpointBuilder'$\r$\n"
    FileWrite $0 "Start-Service -Name 'AudioSrv'$\r$\n"
    FileClose $0

    nsExec::Exec '"$WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$R8\PureMic\rename-devices.ps1"'

    ; Write uninstall script for PREUNINSTALL hook
    FileOpen $0 "$R8\PureMic\uninstall-driver.ps1" w
    FileWrite $0 "$$ErrorActionPreference='SilentlyContinue'$\r$\n"
    FileWrite $0 "$$out=& pnputil /enum-drivers /class 'Media' 2>&1$\r$\n"
    FileWrite $0 "$$lines=$$out -split '`r?`n';$$inf=$$null$\r$\n"
    FileWrite $0 "foreach($$l in $$lines){if($$l -match 'Published Name\s*:\s*(\S+\.inf)'){$$inf=$$Matches[1]};if(($$l -match 'VB-Audio' -or $$l -match 'vbMme' -or $$l -match 'VBCABLE') -and $$inf){& pnputil /delete-driver $$inf /uninstall /force;$$inf=$$null}}$\r$\n"
    FileClose $0

    DetailPrint "PureMic audio setup complete."

  driver_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ReadEnvStr $R8 "PROGRAMDATA"
  StrCmp $R8 "" 0 +2
    StrCpy $R8 "C:\ProgramData"
  IfFileExists "$R8\PureMic\uninstall-driver.ps1" 0 uninstall_done
    DetailPrint "Removing PureMic virtual audio driver..."
    nsExec::ExecToLog '"$WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$R8\PureMic\uninstall-driver.ps1"'
    RMDir /r "$R8\PureMic"
  uninstall_done:
!macroend
