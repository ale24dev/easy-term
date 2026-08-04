import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { reportError } from "./errorReporter";

export interface Project {
  id: string;
  name: string;
  path: string;
  command: string;
  port: number | null;
  env: Record<string, string>;
  autoRestart: boolean;
  groupId: string | null;
}

export interface ProjectInput {
  id?: string;
  name: string;
  path: string;
  command: string;
  port: number | null;
  env: Record<string, string>;
  autoRestart: boolean;
  groupId: string | null;
}

export type ProjectStatus = "stopped" | "starting" | "running" | "crashed";

export interface StatusEvent {
  id: string;
  status: ProjectStatus;
  pid: number | null;
}

export interface OutputEvent {
  id: string;
  chunk: string;
}

export interface ExitEvent {
  id: string;
  code: number;
}

export interface UrlDetectedEvent {
  id: string;
  url: string;
}

export interface ErrorCountEvent {
  id: string;
  count: number;
}

export interface DetectedScript {
  name: string;
  command: string;
}

export interface DetectedConfig {
  name: string | null;
  packageManager: string;
  scripts: DetectedScript[];
}

export interface PortOwner {
  pid: number;
  name: string;
}

export interface PortCheckResult {
  free: boolean;
  owner: PortOwner | null;
}

export interface Group {
  id: string;
  name: string;
  projectIds: string[];
}

export interface RestartScheduledEvent {
  id: string;
  attempt: number;
  maxAttempts: number;
  delayMs: number;
}

export interface RestartExhaustedEvent {
  id: string;
}

export interface ProcessStats {
  cpuPercent: number;
  memoryBytes: number;
}

export interface DiagnosticEvent {
  ts: string;
  level: "warn" | "error" | "fatal";
  source: "backend" | "frontend";
  module: string;
  code: string;
  message: string;
  context?: unknown;
  stack?: string;
  session: string;
  appVersion: string;
  osVersion: string;
  repeats?: number;
}

interface AppErrorPayload {
  code: string;
  message: string;
}

function isAppError(value: unknown): value is AppErrorPayload {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value
  );
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    // A well-formed AppError already logged itself on the Rust side when
    // serialized as the command's Err — logging it again here would
    // double-count. Only report failures that never took that shape
    // (transport-level invoke failures, e.g. the command name is wrong).
    if (!isAppError(err)) {
      reportError({
        module: "ipc",
        code: "IPC_COMMAND_PANIC",
        message: err instanceof Error ? err.message : String(err),
        context: { command },
      });
    }
    throw err;
  }
}

export const ipc = {
  listProjects: () => call<Project[]>("list_projects"),
  saveProject: (project: ProjectInput) => call<Project>("save_project", { project }),
  deleteProject: (id: string) => call<void>("delete_project", { id }),
  startProject: (id: string) => call<void>("start_project", { id }),
  stopProject: (id: string) => call<void>("stop_project", { id }),
  restartProject: (id: string) => call<void>("restart_project", { id }),
  getProcessOutput: (id: string) => call<string>("get_process_output", { id }),
  getErrorCount: (id: string) => call<number>("get_error_count", { id }),
  resetErrorCount: (id: string) => call<void>("reset_error_count", { id }),
  detectScripts: (path: string) => call<DetectedConfig>("detect_scripts", { path }),
  checkPort: (port: number) => call<PortCheckResult>("check_port", { port }),
  killPortOwner: (port: number) => call<void>("kill_port_owner", { port }),
  readErrorLog: (day?: string, limit?: number) =>
    call<DiagnosticEvent[]>("read_error_log", { day, limit }),
  openLogsFolder: () => call<void>("open_logs_folder"),
  listGroups: () => call<Group[]>("list_groups"),
  findOrCreateGroup: (name: string) => call<Group>("find_or_create_group", { name }),
  startGroup: (groupId: string) => call<void>("start_group", { groupId }),
  stopGroup: (groupId: string) => call<void>("stop_group", { groupId }),
  getProjectStats: (id: string) => call<ProcessStats | null>("get_project_stats", { id }),
  openInEditor: (path: string) => call<void>("open_in_editor", { path }),
  quitApp: () => call<void>("quit_app"),
  beginNativeDialog: () => call<void>("begin_native_dialog"),
  endNativeDialog: () => call<void>("end_native_dialog"),
};

export function onProcessStatus(cb: (event: StatusEvent) => void) {
  return listen<StatusEvent>("process:status", (event) => cb(event.payload));
}

export function onProcessOutput(cb: (event: OutputEvent) => void) {
  return listen<OutputEvent>("process:output", (event) => cb(event.payload));
}

export function onProcessExit(cb: (event: ExitEvent) => void) {
  return listen<ExitEvent>("process:exit", (event) => cb(event.payload));
}

export function onUrlDetected(cb: (event: UrlDetectedEvent) => void) {
  return listen<UrlDetectedEvent>("process:url-detected", (event) => cb(event.payload));
}

export function onErrorCount(cb: (event: ErrorCountEvent) => void) {
  return listen<ErrorCountEvent>("process:error-count", (event) => cb(event.payload));
}

export function onRestartScheduled(cb: (event: RestartScheduledEvent) => void) {
  return listen<RestartScheduledEvent>("process:restart-scheduled", (event) => cb(event.payload));
}

export function onRestartExhausted(cb: (event: RestartExhaustedEvent) => void) {
  return listen<RestartExhaustedEvent>("process:restart-exhausted", (event) => cb(event.payload));
}
