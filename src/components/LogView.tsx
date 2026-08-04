import { useEffect, useRef, useState } from "react";
import { getTerminal, hydrateFromBuffer } from "../lib/terminals";
import { ipc } from "../lib/ipc";

interface LogViewProps {
  projectId: string;
}

export function LogView({ projectId }: LogViewProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [atBottom, setAtBottom] = useState(true);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const entry = getTerminal(projectId);
    host.appendChild(entry.container);

    if (!entry.opened) {
      entry.terminal.open(entry.container);
      entry.opened = true;
      // First time this project's terminal is shown this session: seed it
      // from the backend's in-memory ring buffer, in case the process was
      // already running before this view ever mounted (e.g. a dev reload).
      ipc
        .getProcessOutput(projectId)
        .then((buffer) => {
          if (buffer) hydrateFromBuffer(projectId, buffer);
          entry.fitAddon.fit();
        })
        .catch(() => {
          // Non-fatal: the terminal just starts empty and fills in live.
        });
    }

    entry.fitAddon.fit();

    const checkAtBottom = () => {
      const buffer = entry.terminal.buffer.active;
      setAtBottom(buffer.viewportY >= buffer.baseY);
    };
    checkAtBottom();

    // xterm.js already only auto-scrolls on new data when the viewport was
    // already at the bottom; this just surfaces a way back down once the
    // user has scrolled up to read older output.
    const scrollSub = entry.terminal.onScroll(checkAtBottom);
    const resizeObserver = new ResizeObserver(() => {
      entry.fitAddon.fit();
      checkAtBottom();
    });
    resizeObserver.observe(host);

    return () => {
      scrollSub.dispose();
      resizeObserver.disconnect();
      // The container is intentionally left parked (not removed/destroyed):
      // it re-parents into whichever host mounts next for this project id.
    };
  }, [projectId]);

  function scrollToBottom() {
    getTerminal(projectId).terminal.scrollToBottom();
    setAtBottom(true);
  }

  return (
    <div className="log-view">
      <div ref={hostRef} className="log-view-host" />
      {!atBottom && (
        <button className="follow-button" onClick={scrollToBottom}>
          ↓ Seguir
        </button>
      )}
    </div>
  );
}
