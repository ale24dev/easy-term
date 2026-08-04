import { useEffect, useState } from "react";
import { onAction as onNotificationAction } from "@tauri-apps/plugin-notification";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./App.css";
import { ProjectList } from "./components/ProjectList";
import { ProjectForm } from "./components/ProjectForm";
import { LogView } from "./components/LogView";
import { Settings } from "./components/Settings";
import { installGlobalErrorHandlers } from "./lib/errorReporter";
import {
  ipc,
  onProcessStatus,
  onProcessOutput,
  onProcessExit,
  onUrlDetected,
  onErrorCount,
  onRestartScheduled,
  onRestartExhausted,
} from "./lib/ipc";
import { writeChunk, writeExitMarker } from "./lib/terminals";
import { useProjectsStore } from "./stores/projects";
import type { Project } from "./lib/ipc";

const RESOURCE_POLL_INTERVAL_MS = 2000;

type View =
  | { kind: "list" }
  | { kind: "form"; project: Project | null }
  | { kind: "logs"; projectId: string }
  | { kind: "settings" };

function App() {
  const [view, setView] = useState<View>({ kind: "list" });
  const projects = useProjectsStore((s) => s.projects);
  const loadProjects = useProjectsStore((s) => s.loadProjects);
  const loadGroups = useProjectsStore((s) => s.loadGroups);
  const setStatus = useProjectsStore((s) => s.setStatus);
  const setDetectedUrl = useProjectsStore((s) => s.setDetectedUrl);
  const setErrorCount = useProjectsStore((s) => s.setErrorCount);
  const setRestartInfo = useProjectsStore((s) => s.setRestartInfo);
  const setResourceStats = useProjectsStore((s) => s.setResourceStats);

  useEffect(() => {
    installGlobalErrorHandlers();
    loadProjects();
    loadGroups();

    const unlistenPromises = [
      onProcessStatus((e) => {
        setStatus(e.id, e.status, e.pid);
        if (e.status !== "crashed") setRestartInfo(e.id, null);
      }),
      onProcessOutput((e) => writeChunk(e.id, e.chunk)),
      onProcessExit((e) => {
        const crashed = useProjectsStore.getState().runtime[e.id]?.status === "crashed";
        writeExitMarker(e.id, e.code, crashed);
      }),
      onUrlDetected((e) => setDetectedUrl(e.id, e.url)),
      onErrorCount((e) => setErrorCount(e.id, e.count)),
      onRestartScheduled((e) =>
        setRestartInfo(e.id, { attempt: e.attempt, maxAttempts: e.maxAttempts }),
      ),
      onRestartExhausted((e) => setRestartInfo(e.id, null)),
    ];

    // Clicking a crash notification jumps straight to that project's logs
    // and brings the (normally hidden) popover to the front.
    onNotificationAction((notification) => {
      const projectId = notification.extra?.projectId;
      if (typeof projectId === "string") {
        setView({ kind: "logs", projectId });
      }
    }).catch(() => {
      // Notification action listening isn't available on every platform —
      // crash notifications still show, they just won't deep-link.
    });

    return () => {
      unlistenPromises.forEach((promise) => promise.then((unlisten) => unlisten()));
    };
  }, [
    loadProjects,
    loadGroups,
    setStatus,
    setDetectedUrl,
    setErrorCount,
    setRestartInfo,
    setResourceStats,
  ]);

  // CPU/RAM only costs a `ps` shell-out per running project, but there's no
  // reason to pay it while the popover is hidden — poll only while focused.
  useEffect(() => {
    let interval: ReturnType<typeof setInterval> | null = null;

    function poll() {
      const running = useProjectsStore
        .getState()
        .projects.filter((p) => useProjectsStore.getState().runtime[p.id]?.status === "running");
      for (const project of running) {
        ipc
          .getProjectStats(project.id)
          .then((stats) => {
            if (stats) setResourceStats(project.id, stats.cpuPercent, stats.memoryBytes);
          })
          .catch(() => {
            // Best-effort — a missed sample just leaves the last value shown.
          });
      }
    }

    const unlistenPromise = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) {
        poll();
        interval = setInterval(poll, RESOURCE_POLL_INTERVAL_MS);
      } else if (interval) {
        clearInterval(interval);
        interval = null;
      }
    });

    return () => {
      if (interval) clearInterval(interval);
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, [setResourceStats]);

  const activeProjectName =
    view.kind === "logs" ? (projects.find((p) => p.id === view.projectId)?.name ?? "") : "";

  return (
    <main className="popover">
      <header className="popover-header">
        {view.kind === "list" ? (
          <>
            <span className="popover-title">easy-term</span>
            <button title="Diagnóstico" onClick={() => setView({ kind: "settings" })}>
              ⚙
            </button>
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
                : view.kind === "settings"
                  ? "Diagnóstico"
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

        {view.kind === "settings" && <Settings />}
      </div>
    </main>
  );
}

export default App;
