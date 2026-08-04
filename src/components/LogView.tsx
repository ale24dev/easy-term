import { useEffect, useRef, useState } from "react";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { getTerminal, hydrateFromBuffer } from "../lib/terminals";
import { ipc } from "../lib/ipc";
import { useProjectsStore } from "../stores/projects";

interface LogViewProps {
  projectId: string;
}

export function LogView({ projectId }: LogViewProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchTerm, setSearchTerm] = useState("");

  const project = useProjectsStore((s) => s.projects.find((p) => p.id === projectId));
  const detectedUrl = useProjectsStore((s) => s.runtime[projectId]?.detectedUrl);
  const resetErrorCount = useProjectsStore((s) => s.resetErrorCount);

  const openableUrl = detectedUrl ?? (project?.port ? `http://localhost:${project.port}` : null);

  useEffect(() => {
    resetErrorCount(projectId);
  }, [projectId, resetErrorCount]);

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

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setSearchOpen(true);
        requestAnimationFrame(() => searchInputRef.current?.select());
      } else if (e.key === "Escape" && searchOpen) {
        e.preventDefault();
        closeSearch();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchOpen]);

  function closeSearch() {
    setSearchOpen(false);
    getTerminal(projectId).searchAddon.clearActiveDecoration();
  }

  function runSearch(term: string, backwards: boolean) {
    if (!term) return;
    const { searchAddon } = getTerminal(projectId);
    if (backwards) {
      searchAddon.findPrevious(term);
    } else {
      searchAddon.findNext(term);
    }
  }

  function scrollToBottom() {
    getTerminal(projectId).terminal.scrollToBottom();
    setAtBottom(true);
  }

  async function copyUrl() {
    if (!openableUrl) return;
    try {
      await navigator.clipboard.writeText(openableUrl);
    } catch {
      // Clipboard access denied — nothing more we can do here.
    }
  }

  return (
    <div className="log-view">
      {searchOpen && (
        <div className="log-search-bar">
          <input
            ref={searchInputRef}
            value={searchTerm}
            onChange={(e) => {
              setSearchTerm(e.target.value);
              runSearch(e.target.value, false);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                runSearch(searchTerm, e.shiftKey);
              }
            }}
            placeholder="Buscar en el log…"
          />
          <button type="button" onClick={() => runSearch(searchTerm, true)}>
            ↑
          </button>
          <button type="button" onClick={() => runSearch(searchTerm, false)}>
            ↓
          </button>
          <button type="button" onClick={closeSearch}>
            ✕
          </button>
        </div>
      )}

      <div ref={hostRef} className="log-view-host" />

      <div className="log-view-toolbar">
        {openableUrl && (
          <button className="open-url-button" onClick={() => openUrl(openableUrl)}>
            🌐 Abrir {openableUrl}
          </button>
        )}
        <div className="quick-actions">
          {project && (
            <>
              <button title="Abrir en el editor" onClick={() => ipc.openInEditor(project.path)}>
                🖥
              </button>
              <button title="Abrir en Finder" onClick={() => revealItemInDir(project.path)}>
                📁
              </button>
            </>
          )}
          {openableUrl && (
            <button title="Copiar URL" onClick={copyUrl}>
              📋
            </button>
          )}
        </div>
      </div>

      {!atBottom && (
        <button className="follow-button" onClick={scrollToBottom}>
          ↓ Seguir
        </button>
      )}
    </div>
  );
}
