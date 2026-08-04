import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import "@xterm/xterm/css/xterm.css";

interface TerminalEntry {
  terminal: Terminal;
  fitAddon: FitAddon;
  searchAddon: SearchAddon;
  /** Owned by this registry, not by React — LogView re-parents it via
   * appendChild on mount instead of destroying/recreating it, so the
   * instance (and its scrollback) survives switching away from a project. */
  container: HTMLDivElement;
  opened: boolean;
}

const registry = new Map<string, TerminalEntry>();

function createEntry(): TerminalEntry {
  const terminal = new Terminal({
    convertEol: true,
    disableStdin: true,
    fontSize: 12,
    fontFamily: '"SF Mono", Menlo, Monaco, monospace',
    scrollback: 10000,
    theme: { background: "#00000000" },
  });
  const fitAddon = new FitAddon();
  const searchAddon = new SearchAddon();
  terminal.loadAddon(fitAddon);
  terminal.loadAddon(searchAddon);

  const container = document.createElement("div");
  container.className = "terminal-host";

  return { terminal, fitAddon, searchAddon, container, opened: false };
}

export function getTerminal(id: string): TerminalEntry {
  let entry = registry.get(id);
  if (!entry) {
    entry = createEntry();
    registry.set(id, entry);
  }
  return entry;
}

export function writeChunk(id: string, chunk: string): void {
  getTerminal(id).terminal.write(chunk);
}

export function writeExitMarker(id: string, code: number, crashed: boolean): void {
  const label = crashed ? `salió con código ${code}` : "detenido";
  const color = crashed ? "\x1b[31m" : "\x1b[2m";
  getTerminal(id).terminal.write(`\r\n${color}[proceso ${label}]\x1b[0m\r\n`);
}

export function hydrateFromBuffer(id: string, buffer: string): void {
  const entry = getTerminal(id);
  entry.terminal.clear();
  entry.terminal.write(buffer);
}

export function disposeTerminal(id: string): void {
  const entry = registry.get(id);
  if (entry) {
    entry.terminal.dispose();
    registry.delete(id);
  }
}
