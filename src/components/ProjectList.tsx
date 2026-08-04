import { useProjectsStore, getRuntime } from "../stores/projects";
import type { Project } from "../lib/ipc";

interface ProjectListProps {
  onEdit: (project: Project) => void;
  onOpenLogs: (id: string) => void;
}

export function ProjectList({ onEdit, onOpenLogs }: ProjectListProps) {
  const projects = useProjectsStore((s) => s.projects);
  // Subscribed so status-dot updates trigger a re-render for every row.
  useProjectsStore((s) => s.runtime);
  const start = useProjectsStore((s) => s.start);
  const stop = useProjectsStore((s) => s.stop);
  const restart = useProjectsStore((s) => s.restart);
  const deleteProject = useProjectsStore((s) => s.deleteProject);

  if (projects.length === 0) {
    return (
      <div className="empty-state">
        <p>No hay proyectos todavía.</p>
        <p className="hint">Agrega uno para empezar.</p>
      </div>
    );
  }

  return (
    <ul className="project-list">
      {projects.map((project) => {
        const { status } = getRuntime(project.id);
        const isActive = status === "running" || status === "starting";

        return (
          <li key={project.id} className="project-row">
            <button className="project-main" onClick={() => onOpenLogs(project.id)}>
              <span className={`status-dot status-${status}`} />
              <span className="project-name">{project.name}</span>
              {project.port !== null && <span className="project-port">:{project.port}</span>}
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
                <button title="Iniciar" onClick={() => start(project.id)}>
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
  );
}
