import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Group, Project } from "../lib/ipc";

vi.mock("../lib/ipc", () => ({
  ipc: {
    listProjects: vi.fn(),
    listGroups: vi.fn(),
    saveProject: vi.fn(),
    deleteProject: vi.fn(),
    startProject: vi.fn(),
    stopProject: vi.fn(),
    restartProject: vi.fn(),
    startGroup: vi.fn(),
    stopGroup: vi.fn(),
    resetErrorCount: vi.fn(),
    listProcessStatuses: vi.fn(),
    toggleProjectPin: vi.fn(),
    toggleGroupPin: vi.fn(),
  },
}));

import { ipc } from "../lib/ipc";
import type { StatusEvent } from "../lib/ipc";
import { DEFAULT_RUNTIME, getRuntime, useProjectsStore } from "./projects";

function project(overrides: Partial<Project> = {}): Project {
  return {
    id: "p1",
    name: "api",
    path: "/tmp/api",
    command: "pnpm run dev",
    port: null,
    env: {},
    autoRestart: false,
    groupId: null,
    color: null,
    pinned: false,
    ...overrides,
  };
}

const initialState = useProjectsStore.getState();

beforeEach(() => {
  useProjectsStore.setState(initialState, true);
  vi.clearAllMocks();
});

describe("loadProjects", () => {
  it("stores the list and marks the store as loaded", async () => {
    vi.mocked(ipc.listProjects).mockResolvedValue([project()]);

    await useProjectsStore.getState().loadProjects();

    const state = useProjectsStore.getState();
    expect(state.projects).toEqual([project()]);
    expect(state.loaded).toBe(true);
  });

  it("still marks the store as loaded when the ipc call fails", async () => {
    vi.mocked(ipc.listProjects).mockRejectedValue(new Error("boom"));

    await useProjectsStore.getState().loadProjects();

    const state = useProjectsStore.getState();
    expect(state.projects).toEqual([]);
    expect(state.loaded).toBe(true);
  });
});

describe("loadGroups", () => {
  it("stores the list of groups", async () => {
    const groups: Group[] = [{ id: "g1", name: "backend", projectIds: [], pinned: false }];
    vi.mocked(ipc.listGroups).mockResolvedValue(groups);

    await useProjectsStore.getState().loadGroups();

    expect(useProjectsStore.getState().groups).toEqual(groups);
  });

  it("leaves groups untouched when the ipc call fails", async () => {
    useProjectsStore.setState({ groups: [{ id: "g1", name: "backend", projectIds: [], pinned: false }] });
    vi.mocked(ipc.listGroups).mockRejectedValue(new Error("boom"));

    await useProjectsStore.getState().loadGroups();

    expect(useProjectsStore.getState().groups).toHaveLength(1);
  });
});

describe("syncStatuses", () => {
  it("reconciles runtime status/pid from the backend without touching other runtime fields", async () => {
    useProjectsStore.setState({
      runtime: { p1: { ...DEFAULT_RUNTIME, status: "stopped", errorCount: 4 } },
    });
    const statuses: StatusEvent[] = [{ id: "p1", status: "running", pid: 999 }];
    vi.mocked(ipc.listProcessStatuses).mockResolvedValue(statuses);

    await useProjectsStore.getState().syncStatuses();

    const runtime = useProjectsStore.getState().runtime["p1"];
    expect(runtime.status).toBe("running");
    expect(runtime.pid).toBe(999);
    expect(runtime.errorCount).toBe(4);
  });

  it("is a no-op when the ipc call fails", async () => {
    useProjectsStore.setState({ runtime: { p1: { ...DEFAULT_RUNTIME, status: "running" } } });
    vi.mocked(ipc.listProcessStatuses).mockRejectedValue(new Error("boom"));

    await useProjectsStore.getState().syncStatuses();

    expect(useProjectsStore.getState().runtime["p1"].status).toBe("running");
  });
});

describe("saveProject", () => {
  it("appends a brand-new project to the list", async () => {
    const saved = project();
    vi.mocked(ipc.saveProject).mockResolvedValue(saved);

    const result = await useProjectsStore.getState().saveProject(saved);

    expect(result).toEqual(saved);
    expect(useProjectsStore.getState().projects).toEqual([saved]);
  });

  it("refreshes groups so a group created in the same submit shows up immediately", async () => {
    const saved = project();
    vi.mocked(ipc.saveProject).mockResolvedValue(saved);
    const groups: Group[] = [{ id: "g1", name: "backend", projectIds: [saved.id], pinned: false }];
    vi.mocked(ipc.listGroups).mockResolvedValue(groups);

    await useProjectsStore.getState().saveProject(saved);

    expect(useProjectsStore.getState().groups).toEqual(groups);
  });

  it("replaces an existing project in place instead of duplicating it", async () => {
    useProjectsStore.setState({ projects: [project({ command: "old" })] });
    const edited = project({ command: "new" });
    vi.mocked(ipc.saveProject).mockResolvedValue(edited);

    await useProjectsStore.getState().saveProject(edited);

    const { projects } = useProjectsStore.getState();
    expect(projects).toHaveLength(1);
    expect(projects[0].command).toBe("new");
  });

  it("returns null and leaves the list unchanged when the ipc call fails", async () => {
    vi.mocked(ipc.saveProject).mockRejectedValue(new Error("boom"));

    const result = await useProjectsStore.getState().saveProject(project());

    expect(result).toBeNull();
    expect(useProjectsStore.getState().projects).toEqual([]);
  });
});

describe("deleteProject", () => {
  it("removes the project and its runtime entry", async () => {
    useProjectsStore.setState({
      projects: [project()],
      runtime: { p1: { ...DEFAULT_RUNTIME, status: "running" } },
    });
    vi.mocked(ipc.deleteProject).mockResolvedValue(undefined);

    await useProjectsStore.getState().deleteProject("p1");

    const state = useProjectsStore.getState();
    expect(state.projects).toEqual([]);
    expect(state.runtime["p1"]).toBeUndefined();
  });

  it("leaves state unchanged when the ipc call fails", async () => {
    useProjectsStore.setState({ projects: [project()] });
    vi.mocked(ipc.deleteProject).mockRejectedValue(new Error("boom"));

    await useProjectsStore.getState().deleteProject("p1");

    expect(useProjectsStore.getState().projects).toHaveLength(1);
  });
});

describe("start/stop/restart/startGroup/stopGroup", () => {
  it("delegate to the matching ipc call and never throw, even on failure", async () => {
    vi.mocked(ipc.startProject).mockRejectedValue(new Error("boom"));
    vi.mocked(ipc.stopProject).mockResolvedValue(undefined);
    vi.mocked(ipc.restartProject).mockRejectedValue(new Error("boom"));
    vi.mocked(ipc.startGroup).mockResolvedValue(undefined);
    vi.mocked(ipc.stopGroup).mockRejectedValue(new Error("boom"));

    const state = useProjectsStore.getState();
    await expect(state.start("p1")).resolves.toBeUndefined();
    await expect(state.stop("p1")).resolves.toBeUndefined();
    await expect(state.restart("p1")).resolves.toBeUndefined();
    await expect(state.startGroup("g1")).resolves.toBeUndefined();
    await expect(state.stopGroup("g1")).resolves.toBeUndefined();

    expect(ipc.startProject).toHaveBeenCalledWith("p1");
    expect(ipc.stopProject).toHaveBeenCalledWith("p1");
    expect(ipc.restartProject).toHaveBeenCalledWith("p1");
    expect(ipc.startGroup).toHaveBeenCalledWith("g1");
    expect(ipc.stopGroup).toHaveBeenCalledWith("g1");
  });
});

describe("togglePin", () => {
  it("replaces the project with the backend's toggled version", async () => {
    useProjectsStore.setState({ projects: [project({ pinned: false })] });
    vi.mocked(ipc.toggleProjectPin).mockResolvedValue(project({ pinned: true }));

    await useProjectsStore.getState().togglePin("p1");

    expect(useProjectsStore.getState().projects[0].pinned).toBe(true);
  });

  it("leaves state unchanged when the ipc call fails", async () => {
    useProjectsStore.setState({ projects: [project({ pinned: false })] });
    vi.mocked(ipc.toggleProjectPin).mockRejectedValue(new Error("boom"));

    await useProjectsStore.getState().togglePin("p1");

    expect(useProjectsStore.getState().projects[0].pinned).toBe(false);
  });
});

describe("toggleGroupPin", () => {
  it("replaces the group with the backend's toggled version", async () => {
    useProjectsStore.setState({
      groups: [{ id: "g1", name: "backend", projectIds: [], pinned: false }],
    });
    vi.mocked(ipc.toggleGroupPin).mockResolvedValue({
      id: "g1",
      name: "backend",
      projectIds: [],
      pinned: true,
    });

    await useProjectsStore.getState().toggleGroupPin("g1");

    expect(useProjectsStore.getState().groups[0].pinned).toBe(true);
  });
});

describe("runtime setters", () => {
  it("merge into an existing runtime entry instead of replacing it", () => {
    const state = useProjectsStore.getState();
    state.setStatus("p1", "running", 1234);
    state.setDetectedUrl("p1", "http://localhost:3000");

    const runtime = useProjectsStore.getState().runtime["p1"];
    expect(runtime.status).toBe("running");
    expect(runtime.pid).toBe(1234);
    expect(runtime.detectedUrl).toBe("http://localhost:3000");
  });

  it("default to DEFAULT_RUNTIME fields for a project never touched before", () => {
    useProjectsStore.getState().setErrorCount("p1", 3);

    const runtime = useProjectsStore.getState().runtime["p1"];
    expect(runtime.errorCount).toBe(3);
    expect(runtime.status).toBe(DEFAULT_RUNTIME.status);
    expect(runtime.pid).toBe(DEFAULT_RUNTIME.pid);
  });

  it("setRestartInfo and setResourceStats update their own fields only", () => {
    const state = useProjectsStore.getState();
    state.setRestartInfo("p1", { attempt: 2, maxAttempts: 5 });
    state.setResourceStats("p1", 12.5, 2048);

    const runtime = useProjectsStore.getState().runtime["p1"];
    expect(runtime.restartInfo).toEqual({ attempt: 2, maxAttempts: 5 });
    expect(runtime.cpuPercent).toBe(12.5);
    expect(runtime.memoryBytes).toBe(2048);
  });
});

describe("resetErrorCount", () => {
  it("zeroes the local error count even if the ipc call fails", async () => {
    useProjectsStore.setState({ runtime: { p1: { ...DEFAULT_RUNTIME, errorCount: 7 } } });
    vi.mocked(ipc.resetErrorCount).mockRejectedValue(new Error("boom"));

    await useProjectsStore.getState().resetErrorCount("p1");

    expect(useProjectsStore.getState().runtime["p1"].errorCount).toBe(0);
  });
});

describe("getRuntime", () => {
  it("returns DEFAULT_RUNTIME for a project with no runtime entry yet", () => {
    expect(getRuntime("unknown")).toEqual(DEFAULT_RUNTIME);
  });

  it("returns the stored runtime for a known project", () => {
    useProjectsStore.getState().setStatus("p1", "crashed", null);
    expect(getRuntime("p1").status).toBe("crashed");
  });
});
