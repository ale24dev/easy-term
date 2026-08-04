import { useState, type FormEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectsStore } from "../stores/projects";
import type { Project } from "../lib/ipc";

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

  const [name, setName] = useState(initial?.name ?? "");
  const [path, setPath] = useState(initial?.path ?? "");
  const [command, setCommand] = useState(initial?.command ?? "pnpm run dev");
  const [port, setPort] = useState(initial?.port?.toString() ?? "");
  const [envRows, setEnvRows] = useState<EnvRow[]>(
    initial ? Object.entries(initial.env).map(([key, value]) => ({ key, value })) : [],
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handlePickFolder() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;

    setPath(selected);
    if (!name.trim()) {
      const base = selected.split("/").filter(Boolean).pop();
      if (base) setName(base);
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

    const saved = await saveProject({
      id: initial?.id,
      name: name.trim(),
      path: path.trim(),
      command: command.trim(),
      port: parsedPort,
      env,
      autoRestart: initial?.autoRestart ?? false,
      groupId: initial?.groupId ?? null,
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

      <label className="field">
        <span>Puerto (opcional)</span>
        <input
          value={port}
          onChange={(e) => setPort(e.target.value)}
          placeholder="3000"
          inputMode="numeric"
        />
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
