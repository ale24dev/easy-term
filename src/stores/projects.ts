import { create } from "zustand";
import { ipc, type Group, type Project, type ProjectInput, type ProjectStatus } from "../lib/ipc";

interface RestartInfo {
  attempt: number;
  maxAttempts: number;
}

interface ProjectRuntime {
  status: ProjectStatus;
  pid: number | null;
  detectedUrl: string | null;
  errorCount: number;
  restartInfo: RestartInfo | null;
  cpuPercent: number | null;
  memoryBytes: number | null;
}

export const DEFAULT_RUNTIME: ProjectRuntime = {
  status: "stopped",
  pid: null,
  detectedUrl: null,
  errorCount: 0,
  restartInfo: null,
  cpuPercent: null,
  memoryBytes: null,
};

interface ProjectsState {
  projects: Project[];
  groups: Group[];
  runtime: Record<string, ProjectRuntime>;
  loaded: boolean;

  loadProjects: () => Promise<void>;
  loadGroups: () => Promise<void>;
  saveProject: (input: ProjectInput) => Promise<Project | null>;
  deleteProject: (id: string) => Promise<void>;
  start: (id: string) => Promise<void>;
  stop: (id: string) => Promise<void>;
  restart: (id: string) => Promise<void>;
  startGroup: (groupId: string) => Promise<void>;
  stopGroup: (groupId: string) => Promise<void>;

  setStatus: (id: string, status: ProjectStatus, pid: number | null) => void;
  setDetectedUrl: (id: string, url: string) => void;
  setErrorCount: (id: string, count: number) => void;
  resetErrorCount: (id: string) => Promise<void>;
  setRestartInfo: (id: string, info: RestartInfo | null) => void;
  setResourceStats: (id: string, cpuPercent: number | null, memoryBytes: number | null) => void;
}

export const useProjectsStore = create<ProjectsState>((set) => ({
  projects: [],
  groups: [],
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

  loadGroups: async () => {
    try {
      const groups = await ipc.listGroups();
      set({ groups });
    } catch {
      // Groups are an enhancement — an empty list just means no sections.
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

  startGroup: async (groupId) => {
    try {
      await ipc.startGroup(groupId);
    } catch {
      //
    }
  },

  stopGroup: async (groupId) => {
    try {
      await ipc.stopGroup(groupId);
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

  setErrorCount: (id, count) =>
    set((state) => ({
      runtime: {
        ...state.runtime,
        [id]: { ...(state.runtime[id] ?? DEFAULT_RUNTIME), errorCount: count },
      },
    })),

  resetErrorCount: async (id) => {
    try {
      await ipc.resetErrorCount(id);
    } catch {
      // best-effort — the badge will just be off by whatever came in meanwhile
    }
    set((state) => ({
      runtime: {
        ...state.runtime,
        [id]: { ...(state.runtime[id] ?? DEFAULT_RUNTIME), errorCount: 0 },
      },
    }));
  },

  setRestartInfo: (id, info) =>
    set((state) => ({
      runtime: {
        ...state.runtime,
        [id]: { ...(state.runtime[id] ?? DEFAULT_RUNTIME), restartInfo: info },
      },
    })),

  setResourceStats: (id, cpuPercent, memoryBytes) =>
    set((state) => ({
      runtime: {
        ...state.runtime,
        [id]: { ...(state.runtime[id] ?? DEFAULT_RUNTIME), cpuPercent, memoryBytes },
      },
    })),
}));

export function getRuntime(id: string): ProjectRuntime {
  return useProjectsStore.getState().runtime[id] ?? DEFAULT_RUNTIME;
}

export type { ProjectRuntime, RestartInfo };
