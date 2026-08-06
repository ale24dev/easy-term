import { useRef, useState } from "react";
import {
  ChevronDownIcon,
  ChevronRightIcon,
  PencilIcon,
  PinIcon,
  PlayIcon,
  RotateCwIcon,
  SquareIcon,
  Trash2Icon,
  XIcon,
} from "lucide-react";
import { useProjectsStore, getRuntime } from "../stores/projects";
import { ipc, type PortOwner, type Project } from "../lib/ipc";
import { ColorSwatchPicker } from "./ColorSwatchPicker";
import { PortConflictDialog } from "./PortConflictDialog";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { Checkbox } from "./ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
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

// Pinned projects/groups float to the top of their own list — stable sort
// keeps everything else in its existing relative order.
function sortPinnedFirst<T extends { pinned: boolean }>(items: T[]): T[] {
  return [...items].sort((a, b) => Number(b.pinned) - Number(a.pinned));
}

const LONG_PRESS_MS = 500;

interface ProjectRowProps {
  project: Project;
  onEdit: (project: Project) => void;
  onOpenLogs: (id: string) => void;
  onStart: (project: Project) => void;
  selectionMode: boolean;
  selected: boolean;
  onToggleSelect: (id: string) => void;
}

function ProjectRow({
  project,
  onEdit,
  onOpenLogs,
  onStart,
  selectionMode,
  selected,
  onToggleSelect,
}: ProjectRowProps) {
  const { status, errorCount, restartInfo, cpuPercent, memoryBytes } = getRuntime(project.id);
  const stop = useProjectsStore((s) => s.stop);
  const restart = useProjectsStore((s) => s.restart);
  const deleteProject = useProjectsStore((s) => s.deleteProject);
  const togglePin = useProjectsStore((s) => s.togglePin);
  const saveProject = useProjectsStore((s) => s.saveProject);
  const isActive = status === "running" || status === "starting";
  const isAutoRestarting = status === "crashed" && restartInfo !== null;

  function handleColorChange(color: string | null) {
    saveProject({
      id: project.id,
      name: project.name,
      path: project.path,
      command: project.command,
      port: project.port,
      env: project.env,
      autoRestart: project.autoRestart,
      groupId: project.groupId,
      color,
      pinned: project.pinned,
    });
  }

  // The checkbox column is hidden until the user long-presses a row to
  // enter selection mode — no permanent selector cluttering the list.
  const pressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const longPressFired = useRef(false);

  function startPressTimer() {
    longPressFired.current = false;
    pressTimer.current = setTimeout(() => {
      longPressFired.current = true;
      onToggleSelect(project.id);
    }, LONG_PRESS_MS);
  }

  function cancelPressTimer() {
    if (pressTimer.current) {
      clearTimeout(pressTimer.current);
      pressTimer.current = null;
    }
  }

  function handleRowClick() {
    // The long press already acted (selected/deselected this row) — the
    // click that follows the mouseup is just its trailing synthetic event.
    if (longPressFired.current) {
      longPressFired.current = false;
      return;
    }
    if (selectionMode) {
      onToggleSelect(project.id);
    } else {
      onOpenLogs(project.id);
    }
  }

  return (
    <li className="group flex items-center gap-1.5 rounded-md px-2 py-1.5 hover:bg-accent/60">
      {selectionMode && (
        <Checkbox
          checked={selected}
          onCheckedChange={() => onToggleSelect(project.id)}
          className="shrink-0"
          aria-label={`Select ${project.name}`}
        />
      )}
      <Popover>
        <PopoverTrigger asChild>
          <button
            type="button"
            title={project.color ? "Change color" : "Set color"}
            className="flex shrink-0 items-center justify-center rounded-sm p-1 outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            <span
              className={cn(
                "h-4 w-1 rounded-full",
                !project.color && "border border-dashed border-muted-foreground/50",
              )}
              style={project.color ? { backgroundColor: project.color } : undefined}
            />
          </button>
        </PopoverTrigger>
        <PopoverContent className="w-auto" align="start">
          <ColorSwatchPicker value={project.color} onChange={handleColorChange} />
        </PopoverContent>
      </Popover>
      <button
        className="flex min-w-0 flex-1 select-none items-center gap-2 text-left"
        onPointerDown={startPressTimer}
        onPointerUp={cancelPressTimer}
        onPointerLeave={cancelPressTimer}
        onPointerCancel={cancelPressTimer}
        onContextMenu={(e) => e.preventDefault()}
        onClick={handleRowClick}
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
        <IconTooltip label={project.pinned ? "Unpin" : "Pin"}>
          <Button
            variant="ghost"
            size="icon"
            className={cn(project.pinned && "text-foreground")}
            onClick={() => togglePin(project.id)}
          >
            <PinIcon className={cn(project.pinned && "fill-current")} />
          </Button>
        </IconTooltip>
        {isActive ? (
          <>
            <IconTooltip label="Restart">
              <Button variant="ghost" size="icon" onClick={() => restart(project.id)}>
                <RotateCwIcon />
              </Button>
            </IconTooltip>
            <IconTooltip label="Stop">
              <Button variant="ghost" size="icon" onClick={() => stop(project.id)}>
                <SquareIcon />
              </Button>
            </IconTooltip>
          </>
        ) : isAutoRestarting ? (
          <IconTooltip label="Cancel automatic retry">
            <Button variant="ghost" size="icon" onClick={() => stop(project.id)}>
              <XIcon />
            </Button>
          </IconTooltip>
        ) : (
          <IconTooltip label="Start">
            <Button
              variant="ghost"
              size="icon"
              className="text-status-running hover:text-status-running"
              onClick={() => onStart(project)}
            >
              <PlayIcon />
            </Button>
          </IconTooltip>
        )}
        <IconTooltip label="Edit">
          <Button variant="ghost" size="icon" onClick={() => onEdit(project)}>
            <PencilIcon />
          </Button>
        </IconTooltip>
        <IconTooltip label="Delete">
          <Button
            variant="ghost"
            size="icon"
            className="text-destructive hover:bg-destructive/10 hover:text-destructive"
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
  pinned: boolean;
  projects: Project[];
  onEdit: (project: Project) => void;
  onOpenLogs: (id: string) => void;
  onStart: (project: Project) => void;
  selectionMode: boolean;
  selectedIds: Set<string>;
  onToggleSelect: (id: string) => void;
  onToggleSelectMany: (ids: string[], select: boolean) => void;
}

function GroupSection({
  id,
  name,
  pinned,
  projects,
  onEdit,
  onOpenLogs,
  onStart,
  selectionMode,
  selectedIds,
  onToggleSelect,
  onToggleSelectMany,
}: GroupSectionProps) {
  const [collapsed, setCollapsed] = useState(false);
  const toggleGroupPin = useProjectsStore((s) => s.toggleGroupPin);

  const memberIds = projects.map((p) => p.id);
  const selectedCount = memberIds.filter((id) => selectedIds.has(id)).length;
  const allSelected = memberIds.length > 0 && selectedCount === memberIds.length;
  const someSelected = selectedCount > 0 && !allSelected;

  return (
    <li className="mb-1">
      <div className="flex items-center justify-between px-2 py-1">
        <div className="flex items-center gap-1.5">
          {selectionMode && (
            <Checkbox
              checked={allSelected ? true : someSelected ? "indeterminate" : false}
              onCheckedChange={() => onToggleSelectMany(memberIds, !allSelected)}
              aria-label={`Select all in ${name}`}
            />
          )}
          <button
            className="flex items-center gap-1 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase"
            onClick={() => setCollapsed((c) => !c)}
          >
            {collapsed ? (
              <ChevronRightIcon className="size-3" />
            ) : (
              <ChevronDownIcon className="size-3" />
            )}
            {name}
          </button>
        </div>
        <IconTooltip label={pinned ? "Unpin group" : "Pin group"}>
          <Button
            variant="ghost"
            size="icon"
            className={cn("size-6", pinned && "text-foreground")}
            onClick={() => toggleGroupPin(id)}
          >
            <PinIcon className={cn("size-3", pinned && "fill-current")} />
          </Button>
        </IconTooltip>
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
              selectionMode={selectionMode}
              selected={selectedIds.has(project.id)}
              onToggleSelect={onToggleSelect}
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
  const stop = useProjectsStore((s) => s.stop);
  const deleteProject = useProjectsStore((s) => s.deleteProject);

  const [conflict, setConflict] = useState<Conflict | null>(null);
  const [resolving, setResolving] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  // Derived, not its own state: selection mode starts the moment a long
  // press adds the first id, and ends the moment the set empties out again
  // (via the clear button or deselecting the last selected row).
  const selectionMode = selectedIds.size > 0;

  function toggleSelect(id: string) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  function toggleSelectMany(ids: string[], select: boolean) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      for (const id of ids) {
        if (select) next.add(id);
        else next.delete(id);
      }
      return next;
    });
  }

  function clearSelection() {
    setSelectedIds(new Set());
  }

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

  async function handleBulkStart() {
    const targets = projects.filter((p) => selectedIds.has(p.id));
    clearSelection();
    for (const project of targets) {
      await handleStart(project);
    }
  }

  async function handleBulkStop() {
    const ids = [...selectedIds];
    clearSelection();
    for (const id of ids) {
      await stop(id);
    }
  }

  async function handleBulkDelete() {
    const ids = [...selectedIds];
    clearSelection();
    for (const id of ids) {
      await deleteProject(id);
    }
  }

  if (projects.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-1 px-6 text-center text-muted-foreground">
        <p className="text-[13px]">No projects yet.</p>
        <p className="text-[11px] text-muted-foreground/70">Add one to get started.</p>
      </div>
    );
  }

  const groupedIds = new Set(groups.flatMap((g) => g.projectIds));
  const ungrouped = sortPinnedFirst(projects.filter((p) => !groupedIds.has(p.id)));
  const sortedGroups = sortPinnedFirst(groups);

  return (
    <>
      <ul className="scrollbar-thin flex-1 overflow-y-auto p-1">
        {sortedGroups.map((group) => {
          const members = sortPinnedFirst(
            group.projectIds
              .map((id) => projects.find((p) => p.id === id))
              .filter((p): p is Project => p !== undefined),
          );
          if (members.length === 0) return null;

          return (
            <GroupSection
              key={group.id}
              id={group.id}
              name={group.name}
              pinned={group.pinned}
              projects={members}
              onEdit={onEdit}
              onOpenLogs={onOpenLogs}
              onStart={handleStart}
              selectionMode={selectionMode}
              selectedIds={selectedIds}
              onToggleSelect={toggleSelect}
              onToggleSelectMany={toggleSelectMany}
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
            selectionMode={selectionMode}
            selected={selectedIds.has(project.id)}
            onToggleSelect={toggleSelect}
          />
        ))}
      </ul>

      {selectionMode && (
        <div className="flex shrink-0 items-center gap-1.5 border-t border-border px-2.5 py-1.5">
          <span className="text-[11px] text-muted-foreground">{selectedIds.size} selected</span>
          <div className="ml-auto flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              className="text-status-running hover:text-status-running"
              onClick={handleBulkStart}
            >
              <PlayIcon className="size-3" />
              Start
            </Button>
            <Button variant="ghost" size="sm" onClick={handleBulkStop}>
              <SquareIcon className="size-3" />
              Stop
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="text-destructive hover:bg-destructive/10 hover:text-destructive"
              onClick={handleBulkDelete}
            >
              <Trash2Icon className="size-3" />
              Delete
            </Button>
            <Button variant="ghost" size="icon" className="size-6" onClick={clearSelection}>
              <XIcon className="size-3" />
            </Button>
          </div>
        </div>
      )}

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
