import { useEffect, useRef, useState } from "react";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  ArrowDownIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  ClipboardIcon,
  FolderIcon,
  GlobeIcon,
  MonitorIcon,
  XIcon,
} from "lucide-react";
import { getTerminal, hydrateFromBuffer } from "../lib/terminals";
import { ipc } from "../lib/ipc";
import { useProjectsStore } from "../stores/projects";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { IconTooltip } from "./ui/tooltip";

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
    <div className="relative flex min-h-0 flex-1 flex-col">
      {searchOpen && (
        <div className="flex shrink-0 items-center gap-1 border-b border-border px-2 py-1.5">
          <Input
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
          <Button
            type="button"
            variant="ghost"
            size="icon"
            onClick={() => runSearch(searchTerm, true)}
          >
            <ChevronUpIcon />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            onClick={() => runSearch(searchTerm, false)}
          >
            <ChevronDownIcon />
          </Button>
          <Button type="button" variant="ghost" size="icon" onClick={closeSearch}>
            <XIcon />
          </Button>
        </div>
      )}

      <div ref={hostRef} className="min-h-0 flex-1 px-2 py-1" />

      <div className="flex shrink-0 items-center gap-1.5 px-2 pt-1 pb-2">
        {openableUrl && (
          <Button
            variant="ghost"
            size="sm"
            className="min-w-0 flex-1 justify-start truncate text-muted-foreground"
            onClick={() => openUrl(openableUrl)}
          >
            <GlobeIcon className="shrink-0" />
            <span className="truncate">Abrir {openableUrl}</span>
          </Button>
        )}
        <div className="flex shrink-0 items-center gap-0.5">
          {project && (
            <>
              <IconTooltip label="Abrir en el editor">
                <Button variant="ghost" size="icon" onClick={() => ipc.openInEditor(project.path)}>
                  <MonitorIcon />
                </Button>
              </IconTooltip>
              <IconTooltip label="Abrir en Finder">
                <Button variant="ghost" size="icon" onClick={() => revealItemInDir(project.path)}>
                  <FolderIcon />
                </Button>
              </IconTooltip>
            </>
          )}
          {openableUrl && (
            <IconTooltip label="Copiar URL">
              <Button variant="ghost" size="icon" onClick={copyUrl}>
                <ClipboardIcon />
              </Button>
            </IconTooltip>
          )}
        </div>
      </div>

      {!atBottom && (
        <Button
          size="sm"
          className="absolute right-5 bottom-11 rounded-full bg-foreground text-background shadow-md hover:bg-foreground/90"
          onClick={scrollToBottom}
        >
          <ArrowDownIcon />
          Seguir
        </Button>
      )}
    </div>
  );
}
