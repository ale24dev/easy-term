import { useEffect, useState } from "react";
import { isEnabled, enable, disable } from "@tauri-apps/plugin-autostart";
import { RefreshCwIcon } from "lucide-react";
import { ipc, type DiagnosticEvent } from "../lib/ipc";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { Switch } from "./ui/switch";
import { Label } from "./ui/label";
import { IconTooltip } from "./ui/tooltip";
import { ThemeToggle } from "./theme-toggle";
import { cn } from "@/lib/utils";

const LEVEL_LABEL: Record<DiagnosticEvent["level"], string> = {
  warn: "warn",
  error: "error",
  fatal: "fatal",
};

export function Settings() {
  const [events, setEvents] = useState<DiagnosticEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [copied, setCopied] = useState(false);
  const [autostart, setAutostart] = useState(false);

  async function load() {
    setLoading(true);
    try {
      const result = await ipc.readErrorLog(undefined, 100);
      setEvents(result.reverse());
    } catch {
      setEvents([]);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    load();
    isEnabled()
      .then(setAutostart)
      .catch(() => {
        // Autostart status just stays at its default (off) if unreadable.
      });
  }, []);

  async function toggleAutostart() {
    try {
      if (autostart) {
        await disable();
      } else {
        await enable();
      }
      setAutostart(!autostart);
    } catch {
      // Leave the toggle as-is — the OS-level state didn't change.
    }
  }

  async function copyLastError() {
    if (events.length === 0) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(events[0], null, 2));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access denied — nothing more we can do here.
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 p-3.5">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Switch id="autostart" checked={autostart} onCheckedChange={toggleAutostart} />
          <Label htmlFor="autostart" className="text-foreground">
            Iniciar con macOS
          </Label>
        </div>
        <ThemeToggle />
      </div>

      <div className="flex shrink-0 items-center gap-1.5">
        <Button variant="outline" size="sm" onClick={() => ipc.openLogsFolder()}>
          Abrir carpeta de logs
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={copyLastError}
          disabled={events.length === 0}
        >
          {copied ? "Copiado" : "Copiar último error"}
        </Button>
        <IconTooltip label="Actualizar">
          <Button variant="ghost" size="icon" className="ml-auto" onClick={load}>
            <RefreshCwIcon />
          </Button>
        </IconTooltip>
      </div>

      {loading ? (
        <p className="text-[12px] text-muted-foreground">Cargando…</p>
      ) : events.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-[12px] text-muted-foreground">
          <p>Sin errores hoy.</p>
        </div>
      ) : (
        <ul className="scrollbar-thin flex flex-1 flex-col gap-1.5 overflow-y-auto">
          {events.map((event, index) => (
            <li
              key={index}
              className="flex items-baseline gap-2 rounded-md bg-muted/50 px-2 py-1.5 text-[11px]"
            >
              <Badge
                variant={event.level === "warn" ? "warning" : "destructive"}
                className="shrink-0 rounded-sm px-1 uppercase"
              >
                {LEVEL_LABEL[event.level]}
              </Badge>
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="font-mono font-semibold">{event.code}</span>
                <span className="truncate text-muted-foreground">{event.message}</span>
              </div>
              <span className="shrink-0 text-[10px] text-muted-foreground">
                {new Date(event.ts).toLocaleTimeString()}
                {event.repeats ? ` (+${event.repeats})` : ""}
              </span>
            </li>
          ))}
        </ul>
      )}

      {/* No native "Quit" item in the tray's right-click menu — attaching
          any menu makes macOS show it on every click, left included, which
          would break click-to-toggle. This button is the only way out. */}
      <Button
        type="button"
        variant="outline"
        className={cn("shrink-0 border-destructive/30 text-destructive hover:bg-destructive/10")}
        onClick={() => ipc.quitApp()}
      >
        Salir de easy-term
      </Button>
    </div>
  );
}
