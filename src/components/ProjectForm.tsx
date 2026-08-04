import { useState, type FormEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectsStore } from "../stores/projects";
import { ipc, type DetectedScript, type Project } from "../lib/ipc";

const DEFAULT_COMMAND = "pnpm run dev";

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
      setError("Nombre, ruta y comando son obligatorios.");
      return;
    }

    let parsedPort: number | null = null;
    if (port.trim()) {
      parsedPort = Number(port);
      if (!Number.isInteger(parsedPort) || parsedPort < 1 || parsedPort > 65535) {
        setError("El puerto debe ser un número entre 1 y 65535.");
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
        setError("No se pudo crear/encontrar el grupo.");
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
    });

    setSaving(false);

    if (saved) {
      onSaved(saved);
    } else {
      setError("No se pudo guardar el proyecto.");
    }
  }

  return (
    <form className="project-form" onSubmit={handleSubmit}>
      <label className="field">
        <span>Ruta del proyecto</span>
        <div className="path-picker">
          <input value={path} readOnly placeholder="Elige una carpeta…" />
          <button type="button" onClick={handlePickFolder}>
            Elegir…
          </button>
        </div>
      </label>

      <label className="field">
        <span>Nombre</span>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="mi-proyecto" />
      </label>

      <label className="field">
        <span>Comando</span>
        <input
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          placeholder="pnpm run dev"
        />
      </label>

      {scripts.length > 0 && (
        <label className="field">
          <span>Scripts detectados</span>
          <select
            value={scripts.find((s) => s.command === command)?.name ?? ""}
            onChange={(e) => {
              const script = scripts.find((s) => s.name === e.target.value);
              if (script) setCommand(script.command);
            }}
          >
            <option value="" disabled>
              Elegir un script…
            </option>
            {scripts.map((script) => (
              <option key={script.name} value={script.name}>
                {script.name} — {script.command}
              </option>
            ))}
          </select>
        </label>
      )}

      <label className="field">
        <span>Puerto (opcional)</span>
        <input
          value={port}
          onChange={(e) => setPort(e.target.value)}
          placeholder="3000"
          inputMode="numeric"
        />
      </label>

      <label className="field">
        <span>Grupo (opcional)</span>
        <input
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
      </label>

      <label className="checkbox-field">
        <input
          type="checkbox"
          checked={autoRestart}
          onChange={(e) => setAutoRestart(e.target.checked)}
        />
        <span>Reiniciar automáticamente si crashea</span>
      </label>

      <div className="field">
        <span>Variables de entorno</span>
        <div className="env-rows">
          {envRows.map((row, index) => (
            <div className="env-row" key={index}>
              <input
                value={row.key}
                onChange={(e) => updateEnvRow(index, "key", e.target.value)}
                placeholder="CLAVE"
              />
              <input
                value={row.value}
                onChange={(e) => updateEnvRow(index, "value", e.target.value)}
                placeholder="valor"
              />
              <button type="button" className="icon-button" onClick={() => removeEnvRow(index)}>
                ✕
              </button>
            </div>
          ))}
          <button type="button" className="add-env" onClick={addEnvRow}>
            + Agregar variable
          </button>
        </div>
      </div>

      {error && <p className="form-error">{error}</p>}

      <div className="form-actions">
        <button type="button" onClick={onCancel} disabled={saving}>
          Cancelar
        </button>
        <button type="submit" className="primary" disabled={saving}>
          {saving ? "Guardando…" : "Guardar"}
        </button>
      </div>
    </form>
  );
}
