import { useEffect, useState } from "react";
import "./App.css";
import { ProjectList } from "./components/ProjectList";
import { ProjectForm } from "./components/ProjectForm";
import { LogView } from "./components/LogView";
import { installGlobalErrorHandlers } from "./lib/errorReporter";
import { onProcessStatus, onProcessOutput, onProcessExit, onUrlDetected } from "./lib/ipc";
import { writeChunk, writeExitMarker } from "./lib/terminals";
import { useProjectsStore } from "./stores/projects";
import type { Project } from "./lib/ipc";

type View =
  | { kind: "list" }
  | { kind: "form"; project: Project | null }
  | { kind: "logs"; projectId: string };

function App() {
  const [view, setView] = useState<View>({ kind: "list" });
  const projects = useProjectsStore((s) => s.projects);
  const loadProjects = useProjectsStore((s) => s.loadProjects);
  const setStatus = useProjectsStore((s) => s.setStatus);
  const setDetectedUrl = useProjectsStore((s) => s.setDetectedUrl);

  useEffect(() => {
    installGlobalErrorHandlers();
    loadProjects();

    const unlistenPromises = [
      onProcessStatus((e) => setStatus(e.id, e.status, e.pid)),
      onProcessOutput((e) => writeChunk(e.id, e.chunk)),
      onProcessExit((e) => {
        const crashed = useProjectsStore.getState().runtime[e.id]?.status === "crashed";
        writeExitMarker(e.id, e.code, crashed);
      }),
      onUrlDetected((e) => setDetectedUrl(e.id, e.url)),
    ];

    return () => {
      unlistenPromises.forEach((promise) => promise.then((unlisten) => unlisten()));
    };
  }, [loadProjects, setStatus, setDetectedUrl]);

  const activeProjectName =
    view.kind === "logs" ? (projects.find((p) => p.id === view.projectId)?.name ?? "") : "";

  return (
    <main className="popover">
      <header className="popover-header">
        {view.kind === "list" ? (
          <>
            <span className="popover-title">easy-term</span>
            <button
              className="header-action"
              onClick={() => setView({ kind: "form", project: null })}
            >
              + Proyecto
            </button>
          </>
        ) : (
          <>
            <button className="header-back" onClick={() => setView({ kind: "list" })}>
              ‹
            </button>
            <span className="popover-title">
              {view.kind === "form"
                ? view.project
                  ? "Editar proyecto"
                  : "Nuevo proyecto"
                : activeProjectName}
            </span>
          </>
        )}
      </header>

      <div className="popover-body">
        {view.kind === "list" && (
          <ProjectList
            onEdit={(project) => setView({ kind: "form", project })}
            onOpenLogs={(id) => setView({ kind: "logs", projectId: id })}
          />
        )}

        {view.kind === "form" && (
          <ProjectForm
            initial={view.project}
            onCancel={() => setView({ kind: "list" })}
            onSaved={() => setView({ kind: "list" })}
          />
        )}

        {view.kind === "logs" && <LogView projectId={view.projectId} />}
      </div>
    </main>
  );
}

export default App;
