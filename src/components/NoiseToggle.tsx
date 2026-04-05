import { Loader2, Power } from "lucide-react";

interface Props {
  enabled: boolean;
  busy: boolean;
  onToggle: (next: boolean) => void;
}

export function NoiseToggle({ enabled, busy, onToggle }: Props) {
  return (
    <button
      type="button"
      onClick={() => {
        if (!busy) onToggle(!enabled);
      }}
      disabled={busy}
      className={`
        relative z-10 h-32 w-32 rounded-full
        flex items-center justify-center
        transition-all duration-300 cursor-pointer
        focus:outline-none focus:ring-2 focus:ring-primary/50
        ${enabled
          ? "bg-background shadow-[0_0_40px_rgba(16,185,129,0.1)] active:scale-95"
          : "bg-background active:scale-95 text-zinc-400 hover:text-zinc-200"
        }
        ${busy ? "opacity-60 cursor-wait" : ""}
      `}
    >
      {busy ? (
        <Loader2 className="h-10 w-10 animate-spin text-zinc-400" />
      ) : (
        <Power className={`h-12 w-12 transition-all duration-500 ${enabled ? "rotate-180 text-emerald-500 drop-shadow-[0_0_8px_rgba(16,185,129,0.5)]" : "text-zinc-600"}`} />
      )}
    </button>
  );
}
