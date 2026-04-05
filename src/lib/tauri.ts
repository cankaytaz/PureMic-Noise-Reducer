import { invoke as tauriInvoke } from "@tauri-apps/api/core";

/**
 * Safe invoke wrapper that handles the case where Tauri IPC
 * is not yet available (e.g., during early page load or in browser).
 */
export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  // Check if we're in Tauri context
  if (typeof window === "undefined" || !(window as any).__TAURI_INTERNALS__) {
    throw new Error(`Not in Tauri context (command: ${cmd})`);
  }
  return tauriInvoke<T>(cmd, args);
}
