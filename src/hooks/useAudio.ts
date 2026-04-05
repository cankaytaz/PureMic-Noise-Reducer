import { invoke } from "@/lib/tauri";
import { useState } from "react";

export function useAudio() {
  const [pipelineRunning, setPipelineRunning] = useState(false);
  const [hardMode, setHardMode] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const startPipeline = async (
    micId: string | null,
    monitorId: string | null,
    denoise: boolean,
    virtualId?: string | null,
  ) => {
    setBusy(true);
    setError(null);
    try {
      await invoke("start_pipeline", {
        micId,
        monitorId,
        virtualId: virtualId ?? null,
        denoise,
      });
      setPipelineRunning(true);
    } catch (e: unknown) {
      setError(e as string);
    } finally {
      setBusy(false);
    }
  };

  const stopPipeline = async () => {
    setBusy(true);
    setError(null);
    try {
      await invoke("stop_pipeline");
      setPipelineRunning(false);
    } catch (e: unknown) {
      setError(e as string);
    } finally {
      setBusy(false);
    }
  };

  const setDenoiseEnabled = async (enabled: boolean) => {
    try {
      await invoke("set_denoise_enabled", { enabled });
    } catch (e: unknown) {
      setError(e as string);
    }
  };

  const setInputGain = async (gain: number) => {
    await invoke("set_input_gain", { gain });
  };

  const setOutputGain = async (gain: number) => {
    await invoke("set_output_gain", { gain });
  };

  const setHardModeEnabled = async (enabled: boolean) => {
    try {
      await invoke("set_denoise_hard_mode", { enabled });
      setHardMode(enabled);
    } catch (e: unknown) {
      setError(e as string);
    }
  };

  const detectVirtualDevice = async (): Promise<string | null> => {
    return invoke<string | null>("detect_virtual_device");
  };

  return {
    pipelineRunning,
    hardMode,
    busy,
    error,
    startPipeline,
    stopPipeline,
    setDenoiseEnabled,
    setHardModeEnabled,
    setInputGain,
    setOutputGain,
    detectVirtualDevice,
  };
}
