import { useState, type FormEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { CheckIcon, PlusIcon, XIcon } from "lucide-react";
import { useProjectsStore } from "../stores/projects";
import { ipc, type DetectedScript, type Project } from "../lib/ipc";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Switch } from "./ui/switch";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./ui/select";
import { cn } from "@/lib/utils";

const DEFAULT_COMMAND = "pnpm run dev";

const COLOR_OPTIONS = [
  "#ef4444",
  "#f97316",
  "#eab308",
  "#22c55e",
  "#14b8a6",
  "#3b82f6",
  "#8b5cf6",
  "#ec4899",
];

interface EnvRow {
  key: string;
  value: string;
}

interface ProjectFormProps {
  initial: Project | null;
  onCancel: () => void;
  onSaved: (project: Project) => void;
}

export function ProjectForm({ initial, onCancel, onSaved }: ProjectFormProps) {
  const saveProject = useProjectsStore((s) => s.saveProject);
  const groups = useProjectsStore((s) => s.groups);

  const [name, setName] = useState(initial?.name ?? "");
  const [path, setPath] = useState(initial?.path ?? "");
  const [command, setCommand] = useState(initial?.command ?? DEFAULT_COMMAND);
  const [port, setPort] = useState(initial?.port?.toString() ?? "");
  const [envRows, setEnvRows] = useState<EnvRow[]>(
    initial ? Object.entries(initial.env).map(([key, value]) => ({ key, value })) : [],
  );
  const [scripts, setScripts] = useState<DetectedScript[]>([]);
  const [groupName, setGroupName] = useState(
    () => groups.find((g) => g.id === initial?.groupId)?.name ?? "",
  );
  const [autoRestart, setAutoRestart] = useState(initial?.autoRestart ?? false);
  const [color, setColor] = useState<string | null>(initial?.color ?? null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function applyPath(selected: string) {
    setPath(selected);
    setScripts([]);

    const base = selected.split("/").filter(Boolean).pop();
    if (!name.trim() && base) setName(base);

    try {
      const detected = await ipc.detectScripts(selected);
      setScripts(detected.scripts);
      if (detected.name && !name.trim()) setName(detected.name);
      // Only auto-pick a script for a brand-new project whose command is
      // still the untouched placeholder — never clobber an edit in progress.
      if (!initial && command === DEFAULT_COMMAND && detected.scripts.length > 0) {
        const preferred =
          detected.scripts.find((s) => s.name === "dev") ?? detected.scripts[0];
        setCommand(preferred.command);
      }
    } catch {
      // No package.json (or unreadable) — the user just types a command manually.
    }
  }

  async function handlePickFolder() {
    // The popover window hides itself on blur (see lib.rs), and on macOS
    // this native panel is a sheet attached to that window — losing focus
    // to it would hide the window and close the sheet with it. Bracket the
    // call so the backend skips that hide while the panel is open.
    await ipc.beginNativeDialog();
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === "string") applyPath(selected);
    } finally {
      await ipc.endNativeDialog();
    }
  }

  function updateEnvRow(index: number, field: keyof EnvRow, value: string) {
    setEnvRows((rows) => rows.map((row, i) => (i === index ? { ...row, [field]: value } : row)));
  }

  function addEnvRow() {
    setEnvRows((rows) => [...rows, { key: "", value: "" }]);
  }

  function removeEnvRow(index: number) {
    setEnvRows((rows) => rows.filter((_, i) => i !== index));
  }

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();

    if (!name.trim() || !path.trim() || !command.trim()) {
      setError("Name, path, and command are required.");
      return;
    }

    let parsedPort: number | null = null;
    if (port.trim()) {
      parsedPort = Number(port);
      if (!Number.isInteger(parsedPort) || parsedPort < 1 || parsedPort > 65535) {
        setError("Port must be a number between 1 and 65535.");
        return;
      }
    }

    const env: Record<string, string> = {};
    for (const row of envRows) {
      if (row.key.trim()) env[row.key.trim()] = row.value;
    }

    setSaving(true);
    setError(null);

    let groupId: string | null = null;
    if (groupName.trim()) {
      try {
        const group = await ipc.findOrCreateGroup(groupName.trim());
        groupId = group.id;
      } catch {
        setSaving(false);
        setError("Could not create/find the group.");
        return;
      }
    }

    const saved = await saveProject({
      id: initial?.id,
      name: name.trim(),
      path: path.trim(),
      command: command.trim(),
      port: parsedPort,
      env,
      autoRestart,
      groupId,
      color,
      pinned: initial?.pinned ?? false,
    });

    setSaving(false);

    if (saved) {
      onSaved(saved);
    } else {
      setError("Could not save the project.");
    }
  }

  return (
    <form
      className="scrollbar-thin flex flex-1 flex-col gap-3.5 overflow-y-auto p-3.5"
      onSubmit={handleSubmit}
    >
      <div className="flex flex-col gap-1">
        <Label>Project path</Label>
        <div className="flex gap-1.5">
          <Input value={path} readOnly placeholder="Choose a folder…" />
          <Button type="button" variant="outline" size="sm" onClick={handlePickFolder}>
            Choose…
          </Button>
        </div>
      </div>

      <div className="flex flex-col gap-1">
        <Label htmlFor="project-name">Name</Label>
        <Input
          id="project-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="my-project"
        />
      </div>

      <div className="flex flex-col gap-1">
        <Label htmlFor="project-command">Command</Label>
        <Input
          id="project-command"
          className="font-mono"
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          placeholder="pnpm run dev"
        />
      </div>

      {scripts.length > 0 && (
        <div className="flex flex-col gap-1">
          <Label>Detected scripts</Label>
          <Select
            value={scripts.find((s) => s.command === command)?.name ?? undefined}
            onValueChange={(value) => {
              const script = scripts.find((s) => s.name === value);
              if (script) setCommand(script.command);
            }}
          >
            <SelectTrigger>
              <SelectValue placeholder="Choose a script…" />
            </SelectTrigger>
            <SelectContent>
              {scripts.map((script) => (
                <SelectItem key={script.name} value={script.name}>
                  {script.name} — {script.command}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}

      <div className="flex flex-col gap-1">
        <Label htmlFor="project-port">Port (optional)</Label>
        <Input
          id="project-port"
          value={port}
          onChange={(e) => setPort(e.target.value)}
          placeholder="3000"
          inputMode="numeric"
        />
      </div>

      <div className="flex flex-col gap-1">
        <Label htmlFor="project-group">Group (optional)</Label>
        <Input
          id="project-group"
          value={groupName}
          onChange={(e) => setGroupName(e.target.value)}
          placeholder="backend"
          list="group-suggestions"
        />
        <datalist id="group-suggestions">
          {groups.map((group) => (
            <option key={group.id} value={group.name} />
          ))}
        </datalist>
      </div>

      <div className="flex flex-col gap-1.5">
        <Label>Color (optional)</Label>
        <div className="flex flex-wrap items-center gap-1.5">
          <button
            type="button"
            title="No color"
            onClick={() => setColor(null)}
            className={cn(
              "flex size-6 items-center justify-center rounded-full border border-dashed border-input text-muted-foreground",
              color === null && "ring-2 ring-ring ring-offset-2 ring-offset-background",
            )}
          >
            <XIcon className="size-3" />
          </button>
          {COLOR_OPTIONS.map((swatch) => (
            <button
              key={swatch}
              type="button"
              title={swatch}
              onClick={() => setColor(swatch)}
              className={cn(
                "flex size-6 items-center justify-center rounded-full",
                color === swatch && "ring-2 ring-ring ring-offset-2 ring-offset-background",
              )}
              style={{ backgroundColor: swatch }}
            >
              {color === swatch && <CheckIcon className="size-3 text-white" />}
            </button>
          ))}
        </div>
      </div>

      <div className="flex items-center gap-2">
        <Switch id="auto-restart" checked={autoRestart} onCheckedChange={setAutoRestart} />
        <Label htmlFor="auto-restart" className="text-foreground">
          Automatically restart on crash
        </Label>
      </div>

      <div className="flex flex-col gap-1.5">
        <Label>Environment variables</Label>
        <div className="flex flex-col gap-1.5">
          {envRows.map((row, index) => (
            <div className="flex items-center gap-1.5" key={index}>
              <Input
                value={row.key}
                onChange={(e) => updateEnvRow(index, "key", e.target.value)}
                placeholder="KEY"
                className="font-mono"
              />
              <Input
                value={row.value}
                onChange={(e) => updateEnvRow(index, "value", e.target.value)}
                placeholder="value"
                className="font-mono"
              />
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="shrink-0"
                onClick={() => removeEnvRow(index)}
              >
                <XIcon />
              </Button>
            </div>
          ))}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="self-start text-muted-foreground"
            onClick={addEnvRow}
          >
            <PlusIcon />
            Add variable
          </Button>
        </div>
      </div>

      {error && <p className="text-[12px] text-destructive">{error}</p>}

      <div className="mt-auto flex justify-end gap-2 pt-2">
        <Button type="button" variant="outline" onClick={onCancel} disabled={saving}>
          Cancel
        </Button>
        <Button type="submit" disabled={saving}>
          {saving ? "Saving…" : "Save"}
        </Button>
      </div>
    </form>
  );
}
