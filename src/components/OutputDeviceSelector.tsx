import { Headphones } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { AudioDevice } from "@/lib/types";

interface Props {
  devices: AudioDevice[];
  selected: string | null;
  loading: boolean;
  onSelect: (id: string) => void;
}

export function OutputDeviceSelector({ devices, selected, loading, onSelect }: Props) {
  return (
    <div className="flex flex-col gap-2">
      <label className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground/70">
        <Headphones className="h-3.5 w-3.5 text-primary/70" />
        Monitoring Output
      </label>
      <Select
        value={selected ?? ""}
        onValueChange={onSelect}
        disabled={loading || devices.length === 0}
      >
        <SelectTrigger className="glass border-white/10 hover:border-white/20 transition-all">
          <SelectValue placeholder={loading ? "Scanning..." : "Select output"} />
        </SelectTrigger>
        <SelectContent className="glass border-white/10">
          {devices.map((d) => (
            <SelectItem key={d.id} value={d.id} className="focus:bg-primary/20">
              {d.name}
            </SelectItem>
          ))}
          {devices.length === 0 && !loading && (
            <p className="p-2 text-xs text-muted-foreground text-center">No devices found</p>
          )}
        </SelectContent>
      </Select>
    </div>
  );
}
