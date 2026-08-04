import { useState } from "react";
import { useProjectsStore, getRuntime } from "../stores/projects";
import { ipc, type PortOwner, type Project } from "../lib/ipc";
import { PortConflictDialog } from "./PortConflictDialog";

interface ProjectListProps {
  onEdit: (project: Project) => void;
  onOpenLogs: (id: string) => void;
}

interface Conflict {
  project: Project;
  owner: PortOwner;
}

function formatBytes(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)}GB` : `${mb.toFixed(0)}MB`;
}

interface ProjectRowProps {
  project: Project;
  onEdit: (project: Project) => void;
  onOpenLogs: (id: string) => void;
  onStart: (project: Project) => void;
}

function ProjectRow({ project, onEdit, onOpenLogs, onStart }: ProjectRowProps) {
  const { status, errorCount, restartInfo, cpuPercent, memoryBytes } = getRuntime(project.id);
  const stop = useProjectsStore((s) => s.stop);
  const restart = useProjectsStore((s) => s.restart);
  const deleteProject = useProjectsStore((s) => s.deleteProject);
  const isActive = status === "running" || status === "starting";
  const isAutoRestarting = status === "crashed" && restartInfo !== null;

  return (
    <li className="project-row">
      <button className="project-main" onClick={() => onOpenLogs(project.id)}>
        <span className={`status-dot status-${status}`} />
        <span className="project-name">{project.name}</span>
        {project.port !== null && <span className="project-port">:{project.port}</span>}
        {status === "crashed" && restartInfo && (
          <span className="restart-badge">
            reintentando {restartInfo.attempt}/{restartInfo.maxAttempts}
          </span>
        )}
        {status === "running" && cpuPercent !== null && memoryBytes !== null && (
          <span className="resource-stats">
            {cpuPercent.toFixed(0)}% · {formatBytes(memoryBytes)}
          </span>
        )}
        {errorCount > 0 && <span className="error-badge">{errorCount}</span>}
      </button>

      <div className="project-actions">
        {isActive ? (
          <>
            <button title="Reiniciar" onClick={() => restart(project.id)}>
              ⟳
            </button>
            <button title="Detener" onClick={() => stop(project.id)}>
              ■
            </button>
          </>
        ) : isAutoRestarting ? (
          <button title="Cancelar reintento automático" onClick={() => stop(project.id)}>
            ✕
          </button>
        ) : (
          <button title="Iniciar" onClick={() => onStart(project)}>
            ▶
          </button>
        )}
        <button title="Editar" onClick={() => onEdit(project)}>
          ✎
        </button>
        <button title="Eliminar" onClick={() => deleteProject(project.id)}>
          🗑
        </button>
      </div>
    </li>
  );
}

interface GroupSectionProps {
  id: string;
  name: string;
  projects: Project[];
  onEdit: (project: Project) => void;
  onOpenLogs: (id: string) => void;
  onStart: (project: Project) => void;
}

function GroupSection({ id, name, projects, onEdit, onOpenLogs, onStart }: GroupSectionProps) {
  const [collapsed, setCollapsed] = useState(false);
  const startGroup = useProjectsStore((s) => s.startGroup);
  const stopGroup = useProjectsStore((s) => s.stopGroup);

  return (
    <li className="group-section">
      <div className="group-header">
        <button className="group-toggle" onClick={() => setCollapsed((c) => !c)}>
          {collapsed ? "▸" : "▾"} {name}
        </button>
        <div className="group-actions">
          <button title="Iniciar todos" onClick={() => startGroup(id)}>
            ▶ Todos
          </button>
          <button title="Detener todos" onClick={() => stopGroup(id)}>
            ■ Todos
          </button>
        </div>
      </div>
      {!collapsed && (
        <ul className="project-list group-members">
          {projects.map((project) => (
            <ProjectRow
              key={project.id}
              project={project}
              onEdit={onEdit}
              onOpenLogs={onOpenLogs}
              onStart={onStart}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

export function ProjectList({ onEdit, onOpenLogs }: ProjectListProps) {
  const projects = useProjectsStore((s) => s.projects);
  const groups = useProjectsStore((s) => s.groups);
  // Subscribed so status-dot/badge updates trigger a re-render for every row.
  useProjectsStore((s) => s.runtime);
  const start = useProjectsStore((s) => s.start);

  const [conflict, setConflict] = useState<Conflict | null>(null);
  const [resolving, setResolving] = useState(false);

  async function handleStart(project: Project) {
    if (project.port !== null) {
      try {
        const result = await ipc.checkPort(project.port);
        if (!result.free && result.owner) {
          setConflict({ project, owner: result.owner });
          return;
        }
      } catch {
        // Couldn't check — fall through and let start_project surface any
        // real failure itself rather than blocking the user here.
      }
    }
    start(project.id);
  }

  async function handleFreeAndStart() {
    if (!conflict) return;
    setResolving(true);
    try {
      if (conflict.project.port !== null) {
        await ipc.killPortOwner(conflict.project.port);
      }
      await start(conflict.project.id);
    } catch {
      // ipc already reported the failure to the diagnostics log.
    } finally {
      setResolving(false);
      setConflict(null);
    }
  }

  if (projects.length === 0) {
    return (
      <div className="empty-state">
        <p>No hay proyectos todavía.</p>
        <p className="hint">Agrega uno para empezar.</p>
      </div>
    );
  }

  const groupedIds = new Set(groups.flatMap((g) => g.projectIds));
  const ungrouped = projects.filter((p) => !groupedIds.has(p.id));

  return (
    <>
      <ul className="project-list">
        {groups.map((group) => {
          const members = group.projectIds
            .map((id) => projects.find((p) => p.id === id))
            .filter((p): p is Project => p !== undefined);
          if (members.length === 0) return null;

          return (
            <GroupSection
              key={group.id}
              id={group.id}
              name={group.name}
              projects={members}
              onEdit={onEdit}
              onOpenLogs={onOpenLogs}
              onStart={handleStart}
            />
          );
        })}

        {ungrouped.map((project) => (
          <ProjectRow
            key={project.id}
            project={project}
            onEdit={onEdit}
            onOpenLogs={onOpenLogs}
            onStart={handleStart}
          />
        ))}
      </ul>

      {conflict && (
        <PortConflictDialog
          port={conflict.project.port ?? 0}
          owner={conflict.owner}
          busy={resolving}
          onCancel={() => setConflict(null)}
          onFreeAndStart={handleFreeAndStart}
        />
      )}
    </>
  );
}
