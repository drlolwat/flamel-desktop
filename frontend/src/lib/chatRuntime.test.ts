import { describe, expect, it, vi } from "vitest";
import {
  ChatRuntimeCoordinator,
  applyRealtimeContinuityEvent,
  applyRealtimeQueueEvent,
  countUserMessagesBefore,
  notifySessionSnapshotReady,
  type QueuedMessage,
  type SessionStreamOwner,
} from "./chatRuntime";

function queued(
  id: string,
  overrides: Partial<QueuedMessage> = {},
): QueuedMessage {
  return {
    id,
    message: id,
    workingDirectory: "/project",
    options: { model: "auto", permissionMode: "default" },
    createdAt: 1,
    ...overrides,
  };
}

function owner(
  overrides: Partial<SessionStreamOwner> = {},
): SessionStreamOwner {
  return {
    tabId: "tab-1",
    bindingGeneration: 1,
    persistent: false,
    cancel: vi.fn(),
    done: Promise.resolve(),
    ...overrides,
  };
}

describe("ChatRuntimeCoordinator", () => {
  it("shares one tab runtime and admits exactly one active turn", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const runtime = coordinator.forTab("tab-1");
    const notify = vi.fn();
    runtime.subscribers.add(notify);

    const first = coordinator.admit(runtime, queued("first"));
    const second = coordinator.admit(runtime, queued("second"));

    expect(coordinator.forTab("tab-1")).toBe(runtime);
    expect(first?.id).toBe("first");
    expect(second).toBeNull();
    expect(runtime.active).toBe(true);
    expect(runtime.queue.map((item) => item.id)).toEqual(["second"]);
    expect(notify).toHaveBeenCalledTimes(2);
  });

  it("drains local turns in order and leaves server-owned work alone", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const runtime = coordinator.forTab("tab-1");
    runtime.active = true;
    runtime.queue.push(
      queued("failed", { admissionState: "failed" }),
      queued("server", { serverOwned: true }),
      queued("local"),
    );

    expect(coordinator.drain(runtime)).toEqual({
      status: "next",
      item: expect.objectContaining({ id: "local" }),
    });
    expect(coordinator.drain(runtime)).toEqual({ status: "server_owned" });
    expect(runtime.active).toBe(true);

    runtime.queue = [];
    expect(coordinator.drain(runtime)).toEqual({ status: "idle" });
    expect(runtime.active).toBe(false);
  });

  it("keeps abort ownership shared across hook instances", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const inputBar = coordinator.abortRef("tab-1");
    const chatView = coordinator.abortRef("tab-1");
    const abort = vi.fn();

    inputBar.current = abort;
    chatView.current?.();

    expect(abort).toHaveBeenCalledOnce();
    chatView.current = null;
    expect(inputBar.current).toBeNull();
  });

  it("joins duplicate owners without starting a second stream", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const first = owner();
    const duplicate = owner({ cancel: vi.fn() });

    expect(coordinator.claimSessionStream("session-1", first).status).toBe(
      "claimed",
    );
    expect(coordinator.claimSessionStream("session-1", duplicate)).toEqual({
      status: "joined",
      owner: first,
    });
    expect(first.cancel).not.toHaveBeenCalled();
    expect(duplicate.cancel).not.toHaveBeenCalled();
  });

  it("keeps a persistent owner through reconnect joins", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const persistent = owner({ persistent: true });
    const reconnect = owner();

    coordinator.claimSessionStream("session-1", persistent);

    expect(coordinator.claimSessionStream("session-1", reconnect)).toEqual({
      status: "joined",
      owner: persistent,
    });
    expect(persistent.cancel).not.toHaveBeenCalled();
  });

  it("replaces stale bindings and ignores stale cleanup", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const stale = owner();
    const current = owner({ bindingGeneration: 2 });

    coordinator.claimSessionStream("session-1", stale);
    expect(coordinator.claimSessionStream("session-1", current).status).toBe(
      "claimed",
    );

    expect(stale.cancel).toHaveBeenCalledOnce();
    expect(coordinator.releaseSessionStream("session-1", stale)).toBe(false);
    expect(coordinator.releaseSessionStream("session-1", current)).toBe(true);
  });

  it("lets a persistent subscriber replace a transient owner", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const transient = owner();
    const persistent = owner({ persistent: true });

    coordinator.claimSessionStream("session-1", transient);

    expect(coordinator.claimSessionStream("session-1", persistent).status).toBe(
      "claimed",
    );
    expect(transient.cancel).toHaveBeenCalledOnce();
  });

  it("does not notify an already-resolved snapshot after its stream owner is replaced", async () => {
    const coordinator = new ChatRuntimeCoordinator();
    let completeSnapshot!: (ready: boolean) => void;
    const transient = owner({ snapshotReady: new Promise(resolve => { completeSnapshot = resolve; }) });
    const persistent = owner({ persistent: true });
    const ready = vi.fn();
    coordinator.claimSessionStream("session-1", transient);
    const notification = notifySessionSnapshotReady(
      transient, ready, () => coordinator.ownsSessionStream("session-1", transient),
    );

    // Same tab/session/generation, but a replacement wins before the promise's
    // callback microtask runs. The old snapshot cannot unlock the new owner.
    completeSnapshot(true);
    coordinator.claimSessionStream("session-1", persistent);
    await notification;

    expect(ready).not.toHaveBeenCalled();
    expect(coordinator.ownsSessionStream("session-1", transient)).toBe(false);
    expect(coordinator.ownsSessionStream("session-1", persistent)).toBe(true);
  });

  it("only discards an idle unobserved tab runtime", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const runtime = coordinator.forTab("tab-1");
    const subscriber = vi.fn();
    runtime.subscribers.add(subscriber);

    coordinator.cleanupTab("tab-1", runtime);
    expect(coordinator.forTab("tab-1")).toBe(runtime);

    runtime.subscribers.delete(subscriber);
    coordinator.cleanupTab("tab-1", runtime);
    expect(coordinator.forTab("tab-1")).not.toBe(runtime);
  });
});

describe("realtime runtime application", () => {
  it("applies continuity once and removes the admitted turn", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const runtime = coordinator.forTab("tab-1");
    const admitted = queued("request-1", { serverOwned: true });
    runtime.queue.push(admitted);
    const notify = vi.fn();
    const beginLiveRun = vi.fn();
    const clearLiveRun = vi.fn();
    const updateTab = vi.fn();

    expect(
      applyRealtimeContinuityEvent(
        {
          type: "continuity",
          status: "active",
          activeRequestId: "request-1",
          isSessionWarm: false,
        },
        {
          runtime,
          now: 42,
          notify,
          beginLiveRun,
          clearLiveRun,
          updateTab,
        },
      ),
    ).toBe(true);

    expect(beginLiveRun).toHaveBeenCalledWith("request-1", admitted);
    expect(runtime.queue).toEqual([]);
    expect(runtime.active).toBe(true);
    expect(clearLiveRun).not.toHaveBeenCalled();
    expect(updateTab).toHaveBeenCalledWith({
      requestId: "request-1",
      isStreaming: true,
      isSessionWarm: false,
      requiresResume: false,
      sessionWarmSince: undefined,
    });
  });

  it("makes an idle continuity event authoritative", () => {
    const coordinator = new ChatRuntimeCoordinator();
    const runtime = coordinator.forTab("tab-1");
    runtime.active = true;
    const clearLiveRun = vi.fn();
    const updateTab = vi.fn();

    applyRealtimeContinuityEvent(
      { type: "continuity", status: "idle" },
      {
        runtime,
        now: 42,
        notify: vi.fn(),
        beginLiveRun: vi.fn(),
        clearLiveRun,
        updateTab,
      },
    );

    expect(runtime.active).toBe(false);
    expect(clearLiveRun).toHaveBeenCalledOnce();
    expect(updateTab).toHaveBeenCalledWith({
      requestId: undefined,
      isStreaming: false,
      isSessionWarm: true,
      requiresResume: false,
      sessionWarmSince: 42,
    });
  });

  it("reconciles a hidden durable continuation without duplicating it", async () => {
    const coordinator = new ChatRuntimeCoordinator();
    const runtime = coordinator.forTab("tab-1");
    const context = {
      runtime,
      workingDirectory: "/project",
      sessionId: "session-1",
      defaults: {
        model: "auto",
        reasoningEffort: "high",
        permissionMode: "default",
      },
      now: 42,
      notify: vi.fn(),
    };

    expect(
      applyRealtimeQueueEvent(
        {
          type: "queue_updated",
          state: "queued",
          requestId: "request-1",
          message: "Continue",
          hiddenContinuation: true,
        },
        context,
      ),
    ).toBe(true);
    applyRealtimeQueueEvent(
      {
        type: "queue_updated",
        state: "queued",
        requestId: "request-1",
        message: "Continue safely",
        interruptActive: true,
      },
      context,
    );

    expect(runtime.queue).toHaveLength(1);
    expect(runtime.queue[0]).toMatchObject({
      id: "request-1",
      message: "Continue safely",
      serverOwned: true,
      options: {
        __isContinuation: true,
        __interruptActive: true,
        sessionId: "session-1",
      },
    });
    await expect(runtime.queue[0].accepted).resolves.toBe(true);
  });

  it("counts only user turns before an edit-and-rerun boundary", () => {
    const blocks = [
      { type: "user" },
      { type: "assistant" },
      { type: "user" },
      { type: "assistant" },
    ];

    expect(countUserMessagesBefore(blocks, 3)).toBe(2);
    expect(countUserMessagesBefore(blocks, 1)).toBe(1);
  });
});
