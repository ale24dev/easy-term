import * as React from "react";
import * as LabelPrimitive from "@radix-ui/react-label";
import { cn } from "@/lib/utils";

function Label({ className, ...props }: React.ComponentProps<typeof LabelPrimitive.Root>) {
  return (
    <LabelPrimitive.Root
      className={cn(
        "flex items-center gap-1.5 text-[12px] font-medium text-muted-foreground select-none",
        className,
      )}
      {...props}
    />
  );
}

export { Label };
