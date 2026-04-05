import { useState } from "react";
import { invoke } from "@/lib/tauri";
import { Loader2, Settings, AlertCircle, CheckCircle2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

interface Props {
  onInstalled: () => void;
  onInstallStart?: () => void;
  onInstallError?: (e: string) => void;
}

export function DriverSetup({ onInstalled, onInstallStart, onInstallError }: Props) {
  const [status, setStatus] = useState<"idle" | "installing" | "success" | "error">("idle");
  const [error, setError] = useState<string | null>(null);

  const handleInstall = async () => {
    setStatus("installing");
    setError(null);
    onInstallStart?.();
    try {
      await invoke("install_driver");
      setStatus("success");
      setTimeout(onInstalled, 1500);
    } catch (e) {
      setError(e as string);
      setStatus("error");
      onInstallError?.(e as string);
    }
  };

  return (
    <div className={cn(
      "w-full max-w-sm mx-auto space-y-2.5 animate-in fade-in zoom-in-95 duration-500",
    )}>
      <div className={cn(
        "relative overflow-hidden rounded-xl border bg-zinc-950/40 backdrop-blur-sm p-3.5 text-card-foreground shadow-sm transition-all",
        status === "error" && "border-destructive/30"
      )}>
        <div className="flex flex-col gap-3">
          <div className="flex items-center gap-3">
            <div className={cn(
              "flex h-7 w-7 items-center justify-center rounded-md border bg-muted/50",
              status === "success" && "border-green-500/20 bg-green-500/10 text-green-500",
              status === "error" && "border-destructive/20 bg-destructive/10 text-destructive",
              status === "idle" && "text-muted-foreground"
            )}>
              {status === "success" ? (
                <CheckCircle2 className="h-4 w-4" />
              ) : status === "error" ? (
                <AlertCircle className="h-4 w-4" />
              ) : status === "installing" ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Settings className="h-4 w-4" />
              )}
            </div>
            <div>
              <h3 className="text-sm font-semibold tracking-tight">
                {status === "success" ? "Driver Installed" : "Driver Required"}
              </h3>
              <p className="text-xs text-muted-foreground">
                {status === "success"
                  ? "Virtual audio is now configured."
                  : "Setup your system for voice routing."}
              </p>
            </div>
          </div>

          {error && (
            <div className="rounded-md bg-destructive/10 p-3 text-xs text-destructive">
              {error}
            </div>
          )}

          {status !== "success" && (
            <Button
              variant={status === "error" ? "destructive" : "default"}
              className="w-full text-xs font-semibold"
              disabled={status === "installing"}
              onClick={handleInstall}
            >
              {status === "installing" ? "Installing..." : status === "error" ? "Retry Installation" : "Install Driver"}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
