import * as React from "react";
import { cn } from "@/lib/utils";

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      className={cn(
        "flex h-8 w-full min-w-0 rounded-md border border-input bg-transparent px-2.5 py-1 text-[12px] shadow-xs outline-none transition-colors placeholder:text-muted-foreground disabled:pointer-events-none disabled:opacity-50",
        "focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/50",
        "read-only:bg-muted/50 read-only:text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

export { Input };
