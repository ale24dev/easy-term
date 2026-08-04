import { useState } from "react";
import {
  ChevronDownIcon,
  ChevronRightIcon,
  PencilIcon,
  PlayIcon,
  RotateCwIcon,
  SquareIcon,
  Trash2Icon,
  XIcon,
} from "lucide-react";
import { useProjectsStore, getRuntime } from "../stores/projects";
import { ipc, type PortOwner, type Project } from "../lib/ipc";
import { PortConflictDialog } from "./PortConflictDialog";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { IconTooltip } from "./ui/tooltip";
import { cn } from "@/lib/utils";

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

const STATUS_DOT: Record<string, string> = {
  running: "bg-status-running",
  starting: "bg-status-starting animate-pulse",
  crashed: "bg-status-crashed",
  stopped: "bg-status-stopped",
};

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
    <li className="group flex items-center gap-1.5 rounded-md px-2 py-1.5 hover:bg-accent/60">
      <button
        className="flex min-w-0 flex-1 items-center gap-2 text-left"
        onClick={() => onOpenLogs(project.id)}
      >
        <span className={cn("size-2 shrink-0 rounded-full", STATUS_DOT[status])} />
        <span className="truncate text-[12px]">{project.name}</span>
        {project.port !== null && (
          <span className="shrink-0 text-[10px] text-muted-foreground">:{project.port}</span>
        )}
        {status === "crashed" && restartInfo && (
          <Badge variant="warning" className="shrink-0">
            {restartInfo.attempt}/{restartInfo.maxAttempts}
          </Badge>
        )}
        {status === "running" && cpuPercent !== null && memoryBytes !== null && (
          <span className="shrink-0 whitespace-nowrap text-[10px] text-muted-foreground">
            {cpuPercent.toFixed(0)}% · {formatBytes(memoryBytes)}
          </span>
        )}
        {errorCount > 0 && <Badge variant="destructive">{errorCount}</Badge>}
      </button>

      <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 has-[[data-state=open]]:opacity-100">
        {isActive ? (
          <>
            <IconTooltip label="Reiniciar">
              <Button variant="ghost" size="icon" onClick={() => restart(project.id)}>
                <RotateCwIcon />
              </Button>
            </IconTooltip>
            <IconTooltip label="Detener">
              <Button variant="ghost" size="icon" onClick={() => stop(project.id)}>
                <SquareIcon />
              </Button>
            </IconTooltip>
          </>
        ) : isAutoRestarting ? (
          <IconTooltip label="Cancelar reintento automático">
            <Button variant="ghost" size="icon" onClick={() => stop(project.id)}>
              <XIcon />
            </Button>
          </IconTooltip>
        ) : (
          <IconTooltip label="Iniciar">
            <Button variant="ghost" size="icon" onClick={() => onStart(project)}>
              <PlayIcon />
            </Button>
          </IconTooltip>
        )}
        <IconTooltip label="Editar">
          <Button variant="ghost" size="icon" onClick={() => onEdit(project)}>
            <PencilIcon />
          </Button>
        </IconTooltip>
        <IconTooltip label="Eliminar">
          <Button
            variant="ghost"
            size="icon"
            className="hover:bg-destructive/10 hover:text-destructive"
            onClick={() => deleteProject(project.id)}
          >
            <Trash2Icon />
          </Button>
        </IconTooltip>
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
    <li className="mb-1">
      <div className="flex items-center justify-between px-2 py-1">
        <button
          className="flex items-center gap-1 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase"
          onClick={() => setCollapsed((c) => !c)}
        >
          {collapsed ? <ChevronRightIcon className="size-3" /> : <ChevronDownIcon className="size-3" />}
          {name}
        </button>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="sm" className="text-[10px]" onClick={() => startGroup(id)}>
            <PlayIcon className="size-3" />
            Todos
          </Button>
          <Button variant="ghost" size="sm" className="text-[10px]" onClick={() => stopGroup(id)}>
            <SquareIcon className="size-3" />
            Todos
          </Button>
        </div>
      </div>
      {!collapsed && (
        <ul className="flex flex-col">
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
      <div className="flex flex-1 flex-col items-center justify-center gap-1 px-6 text-center text-muted-foreground">
        <p className="text-[13px]">No hay proyectos todavía.</p>
        <p className="text-[11px] text-muted-foreground/70">Agrega uno para empezar.</p>
      </div>
    );
  }

  const groupedIds = new Set(groups.flatMap((g) => g.projectIds));
  const ungrouped = projects.filter((p) => !groupedIds.has(p.id));

  return (
    <>
      <ul className="scrollbar-thin flex-1 overflow-y-auto p-1">
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
