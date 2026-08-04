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

export function ProjectList({ onEdit, onOpenLogs }: ProjectListProps) {
  const projects = useProjectsStore((s) => s.projects);
  // Subscribed so status-dot/badge updates trigger a re-render for every row.
  useProjectsStore((s) => s.runtime);
  const start = useProjectsStore((s) => s.start);
  const stop = useProjectsStore((s) => s.stop);
  const restart = useProjectsStore((s) => s.restart);
  const deleteProject = useProjectsStore((s) => s.deleteProject);

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

  return (
    <>
      <ul className="project-list">
        {projects.map((project) => {
          const { status, errorCount } = getRuntime(project.id);
          const isActive = status === "running" || status === "starting";

          return (
            <li key={project.id} className="project-row">
              <button className="project-main" onClick={() => onOpenLogs(project.id)}>
                <span className={`status-dot status-${status}`} />
                <span className="project-name">{project.name}</span>
                {project.port !== null && <span className="project-port">:{project.port}</span>}
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
                ) : (
                  <button title="Iniciar" onClick={() => handleStart(project)}>
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
        })}
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
