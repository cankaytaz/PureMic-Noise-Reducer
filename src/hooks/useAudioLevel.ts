import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";

const HISTORY = 60;

export function useAudioLevel(active: boolean) {
  const [levels, setLevels] = useState<number[]>(Array(HISTORY).fill(0));

  useEffect(() => {
    if (!active) {
      setLevels(Array(HISTORY).fill(0));
      return;
    }

    let cancelled = false;
    let unlisten: UnlistenFn | null = null;

    const setup = async () => {
      unlisten = await listen<number>("audio-level", (event) => {
        if (cancelled) return;
        const raw = Math.min(event.payload * 4, 1.0);
        setLevels((prev) => [...prev.slice(1), raw]);
      });
    };

    setup();

    return () => {
      cancelled = true;
      // unlisten may not be set yet if the promise hasn't resolved
      if (unlisten) {
        unlisten();
      } else {
        // Wait for it then clean up
        const interval = setInterval(() => {
          if (unlisten) {
            unlisten();
            clearInterval(interval);
          }
        }, 10);
        // Safety: clear after 2s regardless
        setTimeout(() => clearInterval(interval), 2000);
      }
    };
  }, [active]);

  return levels;
}
