import { useEffect, useState } from "react";
import { ipc, type DiagnosticEvent } from "../lib/ipc";

const LEVEL_LABEL: Record<DiagnosticEvent["level"], string> = {
  warn: "warn",
  error: "error",
  fatal: "fatal",
};

export function Settings() {
  const [events, setEvents] = useState<DiagnosticEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [copied, setCopied] = useState(false);

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
  }, []);

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
    <div className="settings">
      <div className="settings-toolbar">
        <button type="button" onClick={() => ipc.openLogsFolder()}>
          Abrir carpeta de logs
        </button>
        <button type="button" onClick={copyLastError} disabled={events.length === 0}>
          {copied ? "Copiado" : "Copiar último error"}
        </button>
        <button type="button" onClick={load}>
          ↻
        </button>
      </div>

      {loading ? (
        <p className="hint">Cargando…</p>
      ) : events.length === 0 ? (
        <div className="empty-state">
          <p>Sin errores hoy.</p>
        </div>
      ) : (
        <ul className="diagnostics-list">
          {events.map((event, index) => (
            <li key={index} className={`diagnostic-row diagnostic-${event.level}`}>
              <span className="diagnostic-level">{LEVEL_LABEL[event.level]}</span>
              <div className="diagnostic-body">
                <span className="diagnostic-code">{event.code}</span>
                <span className="diagnostic-message">{event.message}</span>
              </div>
              <span className="diagnostic-time">
                {new Date(event.ts).toLocaleTimeString()}
                {event.repeats ? ` (+${event.repeats})` : ""}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
