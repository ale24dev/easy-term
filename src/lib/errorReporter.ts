import { invoke } from "@tauri-apps/api/core";

export type ErrorLevel = "warn" | "error" | "fatal";

interface ReportErrorInput {
  level?: ErrorLevel;
  module: string;
  code: string;
  message: string;
  context?: unknown;
  stack?: string;
}

/** Sends an internal app error to the Rust-side JSONL diagnostics log (PLAN.md section 7). */
export function reportError(input: ReportErrorInput): void {
  const payload = {
    level: input.level ?? "error",
    module: input.module,
    code: input.code,
    message: input.message,
    context: input.context ?? null,
    stack: input.stack ?? null,
  };

  // Fire-and-forget: a failure to log a failure must never throw into the caller.
  invoke("log_app_error", { payload }).catch(() => {
    // The Rust side already falls back to eprintln when the logger isn't
    // reachable; nothing more useful to do here without risking a loop.
  });
}

export function installGlobalErrorHandlers(): void {
  window.addEventListener("error", (event) => {
    reportError({
      module: "window",
      code: "UI_UNHANDLED_ERROR",
      message: event.message,
      stack: event.error instanceof Error ? event.error.stack : undefined,
    });
  });

  window.addEventListener("unhandledrejection", (event) => {
    const reason = event.reason;
    reportError({
      module: "window",
      code: "UI_UNHANDLED_REJECTION",
      message: reason instanceof Error ? reason.message : String(reason),
      stack: reason instanceof Error ? reason.stack : undefined,
    });
  });
}
