import { create } from "zustand";
import { ipc, type Project, type ProjectInput, type ProjectStatus } from "../lib/ipc";

interface ProjectRuntime {
  status: ProjectStatus;
  pid: number | null;
  detectedUrl: string | null;
}

export const DEFAULT_RUNTIME: ProjectRuntime = {
  status: "stopped",
  pid: null,
  detectedUrl: null,
};

interface ProjectsState {
  projects: Project[];
  runtime: Record<string, ProjectRuntime>;
  loaded: boolean;

  loadProjects: () => Promise<void>;
  saveProject: (input: ProjectInput) => Promise<Project | null>;
  deleteProject: (id: string) => Promise<void>;
  start: (id: string) => Promise<void>;
  stop: (id: string) => Promise<void>;
  restart: (id: string) => Promise<void>;

  setStatus: (id: string, status: ProjectStatus, pid: number | null) => void;
  setDetectedUrl: (id: string, url: string) => void;
}

export const useProjectsStore = create<ProjectsState>((set) => ({
  projects: [],
  runtime: {},
  loaded: false,

  loadProjects: async () => {
    try {
      const projects = await ipc.listProjects();
      set({ projects, loaded: true });
    } catch {
      set({ loaded: true });
    }
  },

  saveProject: async (input) => {
    try {
      const saved = await ipc.saveProject(input);
      set((state) => {
        const exists = state.projects.some((p) => p.id === saved.id);
        const projects = exists
          ? state.projects.map((p) => (p.id === saved.id ? saved : p))
          : [...state.projects, saved];
        return { projects };
      });
      return saved;
    } catch {
      return null;
    }
  },

  deleteProject: async (id) => {
    try {
      await ipc.deleteProject(id);
      set((state) => {
        const runtime = { ...state.runtime };
        delete runtime[id];
        return {
          projects: state.projects.filter((p) => p.id !== id),
          runtime,
        };
      });
    } catch {
      // ipc already reported the failure to the diagnostics log.
    }
  },

  start: async (id) => {
    try {
      await ipc.startProject(id);
    } catch {
      // process:status events (or their absence) already reflect the outcome.
    }
  },

  stop: async (id) => {
    try {
      await ipc.stopProject(id);
    } catch {
      //
    }
  },

  restart: async (id) => {
    try {
      await ipc.restartProject(id);
    } catch {
      //
    }
  },

  setStatus: (id, status, pid) =>
    set((state) => ({
      runtime: {
        ...state.runtime,
        [id]: { ...(state.runtime[id] ?? DEFAULT_RUNTIME), status, pid },
      },
    })),

  setDetectedUrl: (id, url) =>
    set((state) => ({
      runtime: {
        ...state.runtime,
        [id]: { ...(state.runtime[id] ?? DEFAULT_RUNTIME), detectedUrl: url },
      },
    })),
}));

export function getRuntime(id: string): ProjectRuntime {
  return useProjectsStore.getState().runtime[id] ?? DEFAULT_RUNTIME;
}

export type { ProjectRuntime };
