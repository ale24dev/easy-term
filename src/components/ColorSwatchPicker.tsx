import { CheckIcon, XIcon } from "lucide-react";
import { COLOR_OPTIONS } from "../lib/colors";
import { cn } from "@/lib/utils";

interface ColorSwatchPickerProps {
  value: string | null;
  onChange: (color: string | null) => void;
}

export function ColorSwatchPicker({ value, onChange }: ColorSwatchPickerProps) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <button
        type="button"
        title="No color"
        onClick={() => onChange(null)}
        className={cn(
          "flex size-6 items-center justify-center rounded-full border border-dashed border-input text-muted-foreground",
          value === null && "ring-2 ring-ring ring-offset-2 ring-offset-background",
        )}
      >
        <XIcon className="size-3" />
      </button>
      {COLOR_OPTIONS.map((swatch) => (
        <button
          key={swatch}
          type="button"
          title={swatch}
          onClick={() => onChange(swatch)}
          className={cn(
            "flex size-6 items-center justify-center rounded-full",
            value === swatch && "ring-2 ring-ring ring-offset-2 ring-offset-background",
          )}
          style={{ backgroundColor: swatch }}
        >
          {value === swatch && <CheckIcon className="size-3 text-white" />}
        </button>
      ))}
    </div>
  );
}
