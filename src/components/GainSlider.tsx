import { Slider } from "@/components/ui/slider";
import { cn } from "@/lib/utils";

interface Props {
  label: string;
  value: number;
  onChange: (v: number) => void;
  disabled?: boolean;
}

export function GainSlider({ label, value, onChange, disabled }: Props) {
  // Convert 0.0-2.0 range to 0-100 for Shadcn Slider
  const sliderValue = [value * 50];

  const handleValueChange = (vals: number[]) => {
    onChange(vals[0] / 50);
  };

  return (
    <div className={cn("flex flex-1 flex-col gap-3", disabled && "opacity-40 grayscale")}>
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/80">
          {label}
        </span>
        <span className="text-[10px] font-mono text-primary/80">
          {Math.round(value * 100)}%
        </span>
      </div>
      <Slider
        disabled={disabled}
        value={sliderValue}
        max={100}
        step={1}
        onValueChange={handleValueChange}
        className="cursor-pointer"
      />
    </div>
  );
}
