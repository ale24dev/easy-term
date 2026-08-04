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
