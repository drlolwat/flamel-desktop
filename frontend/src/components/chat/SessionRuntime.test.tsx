import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionBootstrap } from "@flamel/shared";
import { useChatStore } from "../../stores/chatStore";
import { usePlanStore } from "../../stores/planStore";
import { useTabStore, type Tab } from "../../stores/tabStore";
import { useSessionReadinessStore } from "../../stores/sessionReadinessStore";
import SessionRuntime from "./SessionRuntime";

const harness = vi.hoisted(() => ({
  effects: [] as Array<() => void | (() => void)>,
  bootstrap: vi.fn(),
  inputs: vi.fn(),
  subscribe: vi.fn(),
  localPresentation: { current: undefined as unknown },
}));

// This non-rendering component only owns an effect. Run that real effect with
// real stores and a deferred API response, without adding a DOM dependency.
vi.mock("react", async (importOriginal) => ({
  ...await importOriginal<typeof import("react")>(),
  useEffect: (effect: () => void | (() => void)) => harness.effects.push(effect),
  useRef: () => harness.localPresentation,
}));
vi.mock("../../stores/chatStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../stores/chatStore")>();
  const store = actual.useChatStore;
  return { ...actual, useChatStore: Object.assign(
    (selector: (state: ReturnType<typeof store.getState>) => unknown) => selector(store.getState()), store,
  ) };
});
vi.mock("../../stores/tabStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../stores/tabStore")>();
  const store = actual.useTabStore;
  return { ...actual, useTabStore: Object.assign(
    (selector: (state: ReturnType<typeof store.getState>) => unknown) => selector(store.getState()), store,
  ) };
});
vi.mock("../../stores/planStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../stores/planStore")>();
  const store = actual.usePlanStore;
  return { ...actual, usePlanStore: Object.assign(
    (selector: (state: ReturnType<typeof store.getState>) => unknown) => selector(store.getState()), store,
  ) };
});
vi.mock("../../stores/sessionReadinessStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../stores/sessionReadinessStore")>();
  const store = actual.useSessionReadinessStore;
  return { ...actual, useSessionReadinessStore: Object.assign(
    (selector: (state: ReturnType<typeof store.getState>) => unknown) => selector(store.getState()), store,
  ) };
});
vi.mock("../../hooks/useChat", () => ({
  useChat: () => ({ subscribeToLiveStream: harness.subscribe }),
}));
vi.mock("../../lib/api", () => ({
  api: { sessions: { bootstrap: harness.bootstrap } }, apiFetch: harness.inputs,
}));

const TAB_ID = "bootstrap-tab";
const cleanups: Array<() => void> = [];

function tab(sessionId = "session-a"): Tab {
  return {
    id: TAB_ID, kind: "chat", sessionId, bindingGeneration: 0,
    title: "Initial title", projectPath: "/project", model: "gpt-5.6-sol",
    reasoningEffort: "high", ponytailEnabled: false,
    permissionMode: "bypassPermissions", isStreaming: false,
  };
}

function payload(sessionId = "session-a"): SessionBootstrap {
  return {
    session: {
      id: sessionId, title: "Persisted title", projectPath: "/persisted-project",
      model: "gpt-5.6-sol", requestedModel: "gpt-5.6-sol",
      requestedReasoningEffort: "medium", createdAt: "2026-09-05", updatedAt: "2026-09-05",
      messageCount: 1, pinned: false, inputTokens: 0, outputTokens: 0,
      requiresResume: true, isSessionWarm: true,
    },
    binding: {
      sessionId, streamSessionId: sessionId, activeRequestId: "old-request",
      activePlanId: null, realtimeSequence: 0, updatedAt: "2026-09-05",
    },
    activePlan: null,
    messages: [{ type: "user", uuid: "saved-prompt", message: { content: [{ type: "text", text: "Persisted opening prompt" }] } }],
  };
}

function mountDeferred() {
  let resolve!: (value: SessionBootstrap) => void;
  let reject!: (error: Error) => void;
  const response = new Promise<SessionBootstrap>((done, fail) => { resolve = done; reject = fail; });
  harness.bootstrap.mockReturnValueOnce(response);
  SessionRuntime({ tabId: TAB_ID });
  const effect = harness.effects.pop();
  expect(effect).toBeDefined();
  const cleanup = effect!();
  if (cleanup) cleanups.push(cleanup);
  return {
    cleanup,
    async fail(error: Error) {
      reject(error);
      await flushTasks();
    },
    async finish(value = payload()) {
      resolve(value);
      await response;
      await flushTasks();
    },
  };
}

async function flushTasks() {
  // Drain the parallel response/body promises and the actual effect continuation.
  for (let index = 0; index < 10; index++) await Promise.resolve();
}

function inputResponse(inputs: unknown[] = []) {
  return { ok: true, status: 200, json: async () => ({ inputs }) };
}

function loadState() {
  return useSessionReadinessStore.getState().entries[TAB_ID];
}

function optimisticSend() {
  const store = useChatStore.getState();
  store.addBlock(TAB_ID, { id: "local-user", requestId: "local-request", type: "user", role: "user", content: "Make my new plan" });
  store.addBlock(TAB_ID, { id: "local-assistant", type: "assistant", role: "assistant", content: "", isStreaming: true });
  useTabStore.getState().updateTab(TAB_ID, {
    requestId: "local-request", model: "gpt-6-astra", reasoningEffort: "xhigh",
    title: "Make my new plan", activePlanId: "live-plan", requiresResume: false,
    isStreaming: true, isSessionWarm: false,
  });
  usePlanStore.getState().setPlan(TAB_ID, "live-plan", "New live plan", undefined, []);
}

beforeEach(() => {
  vi.useFakeTimers();
  harness.effects.length = 0;
  harness.localPresentation.current = undefined;
  harness.bootstrap.mockReset();
  harness.inputs.mockReset().mockResolvedValue(inputResponse());
  harness.subscribe.mockReset().mockImplementation(async (_sessionId, options) => {
    options.onHistoryReady();
    return true;
  });
  useSessionReadinessStore.setState({ entries: {}, retries: {}, nextAttempt: 0 });
  useChatStore.setState({ chats: {} });
  usePlanStore.setState({ plans: {} });
  useTabStore.setState({ tabs: [tab()], activeTabId: TAB_ID });
});

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  vi.useRealTimers();
});

describe("SessionRuntime delayed bootstrap", () => {
  it.each(["during-fetch", "before-fetch"])("preserves a local send made %s but still attaches its stream", async (timing) => {
    useChatStore.getState().setSessionId(TAB_ID, "session-a");
    if (timing === "before-fetch") optimisticSend();
    const request = mountDeferred();
    if (timing === "during-fetch") optimisticSend();
    const blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    const currentTab = useTabStore.getState().tabs[0];
    const plan = usePlanStore.getState().getPlan(TAB_ID);
    await request.finish();
    expect(useChatStore.getState().getChat(TAB_ID).blocks).toBe(blocks);
    expect(blocks[0].content).toBe("Make my new plan");
    expect(useTabStore.getState().tabs[0]).toBe(currentTab);
    expect(usePlanStore.getState().getPlan(TAB_ID)).toBe(plan);
    expect(harness.subscribe).toHaveBeenCalledOnce();
    expect(harness.subscribe).toHaveBeenCalledWith("session-a", expect.objectContaining({ persistent: true, bindingGeneration: 0, historyLoaded: true }));
    expect(loadState().phase).toBe("ready");
  });

  it("loads unchanged idle history and metadata, then subscribes", async () => {
    usePlanStore.getState().setPlan(TAB_ID, "stale-plan", "Old plan", undefined, []);
    const request = mountDeferred();
    await request.finish();
    expect(useChatStore.getState().getChat(TAB_ID).blocks[0].content).toBe("Persisted opening prompt");
    expect(useTabStore.getState().tabs[0]).toMatchObject({
      title: "Persisted title", projectPath: "/persisted-project", requestId: "old-request",
      reasoningEffort: "medium", activePlanId: undefined,
    });
    expect(usePlanStore.getState().getPlan(TAB_ID).planId).toBeUndefined();
    expect(harness.subscribe).toHaveBeenCalledOnce();
  });

  it.each([false, true])("rejects a stale binding, including return-to-same-session=%s", async (returnToSameSession) => {
    const request = mountDeferred();
    useTabStore.getState().updateTab(TAB_ID, { sessionId: "session-b" });
    useChatStore.getState().setSessionId(TAB_ID, "session-b");
    if (returnToSameSession) {
      useTabStore.getState().updateTab(TAB_ID, { sessionId: "session-a" });
      useChatStore.getState().setSessionId(TAB_ID, "session-a");
    }
    const current = useChatStore.getState().getChat(TAB_ID);
    const currentTab = useTabStore.getState().tabs[0];
    await request.finish();
    expect(useChatStore.getState().getChat(TAB_ID)).toBe(current);
    expect(useTabStore.getState().tabs[0]).toBe(currentTab);
    expect(harness.subscribe).not.toHaveBeenCalled();
  });

  it("ignores a response after effect cleanup", async () => {
    const request = mountDeferred();
    request.cleanup?.();
    await request.finish();
    expect(useChatStore.getState().getChat(TAB_ID).blocks).toEqual([]);
    expect(harness.subscribe).not.toHaveBeenCalled();
  });

  it("rejects a response bound to a different stream session", async () => {
    const request = mountDeferred();
    const wrong = payload();
    wrong.binding.streamSessionId = "session-b";
    await request.finish(wrong);
    expect(useChatStore.getState().getChat(TAB_ID).blocks).toEqual([]);
    expect(harness.subscribe).not.toHaveBeenCalled();
    expect(loadState().phase).toBe("error");
  });
});

describe("SessionRuntime preload readiness", () => {
  it("loads history and inputs in parallel, then waits for the full live snapshot", async () => {
    harness.subscribe.mockReturnValue(new Promise(() => {}));
    const receipt = { type: "input_delivery", accepted: true, requestId: "saved-input", sessionId: "session-a", message: "Do this next", delivery: "waiting", updatedAt: "2026-09-05T00:00:00Z" };
    harness.inputs.mockResolvedValue(inputResponse([receipt]));
    const request = mountDeferred();
    expect(loadState().phase).toBe("history");
    expect(harness.bootstrap).toHaveBeenCalledOnce();
    expect(harness.inputs).toHaveBeenCalledOnce();
    const signal = harness.bootstrap.mock.calls[0][1].signal;
    expect(harness.inputs).toHaveBeenCalledWith("/api/chat/session-a/inputs", { signal });
    await request.finish();
    expect(loadState().phase).toBe("state");
    expect(loadState().historyLoaded).toBe(true);
    expect(useChatStore.getState().getChat(TAB_ID).inputDeliveries?.[0]).toMatchObject({ requestId: "saved-input", delivery: "waiting" });
    harness.subscribe.mock.calls[0][1].onHistoryReady();
    expect(loadState().phase).toBe("ready");
    await vi.advanceTimersByTimeAsync(30_000);
    expect(loadState().phase).toBe("ready");
    request.cleanup?.();
    expect(signal.aborted).toBe(true);
    expect(loadState()).toBeUndefined();
  });

  it("does not mark ready until input loading also completes", async () => {
    let finishInputs!: (value: ReturnType<typeof inputResponse>) => void;
    harness.inputs.mockReturnValue(new Promise(resolve => { finishInputs = resolve; }));
    const request = mountDeferred();
    await request.finish();
    expect(loadState().phase).toBe("history");
    expect(harness.subscribe).not.toHaveBeenCalled();
    finishInputs(inputResponse());
    await flushTasks();
    expect(loadState().phase).toBe("ready");
  });

  it.each(["http", "shape", "identity"])("reports invalid input loading (%s) without pretending ready", async (failure) => {
    harness.inputs.mockResolvedValue(failure === "http" ? { ok: false, status: 503 }
      : failure === "shape" ? { ok: true, json: async () => ({ wrong: [] }) }
        : inputResponse([{ type: "input_delivery", accepted: true, sessionId: "other", requestId: "wrong", message: "Wrong session", delivery: "sent" }]));
    const request = mountDeferred();
    await request.finish();
    expect(loadState().phase).toBe("error");
    expect(loadState().error).toBeTruthy();
    expect(harness.subscribe).not.toHaveBeenCalled();
    expect(useChatStore.getState().getChat(TAB_ID).blocks).toEqual([]);
    expect(harness.bootstrap.mock.calls[0][1].signal.aborted).toBe(true);
  });

  it.each(["history", "state"])("times out during %s and ignores late success callbacks", async (phase) => {
    harness.subscribe.mockReturnValue(new Promise(() => {}));
    const request = mountDeferred();
    if (phase === "state") await request.finish();
    expect(loadState().phase).toBe(phase);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(loadState()).toMatchObject({ phase: "error", error: phase === "state"
      ? "Chat history loaded, but the live connection did not finish syncing. Try again."
      : "Chat history took too long to download. Try again." });
    expect(harness.bootstrap.mock.calls[0][1].signal.aborted).toBe(true);
    if (phase === "history") await request.finish();
    else harness.subscribe.mock.calls[0][1].onHistoryReady();
    expect(loadState().phase).toBe("error");
  });

  it("does not treat a subscription return value as snapshot readiness", async () => {
    harness.subscribe.mockResolvedValue(true);
    const request = mountDeferred();
    await request.finish();
    expect(loadState()).toMatchObject({ phase: "error", error: "The live session state could not be loaded. Try again." });
  });

  it("retries a failed attempt without letting late cleanup erase the new one", async () => {
    const first = mountDeferred();
    await first.fail(new Error("Network unavailable"));
    expect(loadState().phase).toBe("error");
    const oldAttempt = loadState().attempt;
    useSessionReadinessStore.getState().retry(TAB_ID);
    const second = mountDeferred();
    expect(loadState().attempt).toBeGreaterThan(oldAttempt);
    first.cleanup?.();
    expect(loadState().phase).toBe("history");
    await second.finish();
    expect(loadState().phase).toBe("ready");
  });

  it.each([false, true])("keeps its original local-prompt permission across cleanup and retry (local=%s)", async (local) => {
    useChatStore.getState().setSessionId(TAB_ID, "session-a");
    if (local) optimisticSend();
    harness.subscribe.mockReturnValue(new Promise(() => {}));
    const first = mountDeferred();
    await first.finish();
    expect(loadState().allowLocalPresentation).toBe(local);
    // An active live snapshot adds a streaming block before catch-up finishes.
    // It is not a local send and must not bypass the next attempt's barrier.
    if (!local) useChatStore.getState().addBlock(TAB_ID, {
      id: "replay-assistant", type: "assistant", role: "assistant", content: "", isStreaming: true,
    });
    await vi.advanceTimersByTimeAsync(30_000);
    expect(loadState().phase).toBe("error");
    useSessionReadinessStore.getState().retry(TAB_ID);
    first.cleanup?.();
    expect(loadState()).toBeUndefined();
    const second = mountDeferred();
    expect(loadState().allowLocalPresentation).toBe(local);
    await second.finish();
    expect(loadState().allowLocalPresentation).toBe(local);
    expect(loadState().phase).toBe("state");
  });

  it("retries a brief 404 only while admitting an optimistic local send", async () => {
    useChatStore.getState().setSessionId(TAB_ID, "session-a");
    optimisticSend();
    const request = mountDeferred();
    harness.bootstrap.mockResolvedValue(payload());
    await request.fail(new Error("404: Session is not saved yet"));
    expect(loadState().phase).toBe("history");
    await vi.advanceTimersByTimeAsync(500);
    expect(harness.bootstrap).toHaveBeenCalledTimes(2);
    expect(loadState().phase).toBe("ready");
    expect(useChatStore.getState().getChat(TAB_ID).blocks[0].content).toBe("Make my new plan");
  });

  it("keeps a new prompt and its current choices when preload fails", async () => {
    useChatStore.getState().setSessionId(TAB_ID, "session-a");
    optimisticSend();
    const blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    const currentTab = useTabStore.getState().tabs[0];
    const plan = usePlanStore.getState().getPlan(TAB_ID);
    const request = mountDeferred();
    await request.fail(new Error("503: Session loading is temporarily unavailable"));
    expect(loadState()).toMatchObject({ phase: "error", error: "503: Session loading is temporarily unavailable" });
    expect(useChatStore.getState().getChat(TAB_ID).blocks).toBe(blocks);
    expect(useTabStore.getState().tabs[0]).toBe(currentTab);
    expect(usePlanStore.getState().getPlan(TAB_ID)).toBe(plan);
    expect(harness.subscribe).not.toHaveBeenCalled();
    expect(harness.bootstrap.mock.calls[0][1].signal.aborted).toBe(true);
  });

  it("stops retrying a new session that never persists without dropping its prompt", async () => {
    useChatStore.getState().setSessionId(TAB_ID, "session-a");
    optimisticSend();
    const blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    const missing = new Error("404: Session is not saved yet");
    const request = mountDeferred();
    harness.bootstrap.mockRejectedValue(missing);
    await request.fail(missing);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(harness.bootstrap).toHaveBeenCalledTimes(2);
    expect(loadState().phase).toBe("history");
    await vi.advanceTimersByTimeAsync(29_000);
    expect(harness.bootstrap.mock.calls.length).toBeGreaterThan(3);
    expect(harness.bootstrap.mock.calls.length).toBeLessThanOrEqual(20);
    expect(loadState()).toMatchObject({ phase: "error", error: "Chat history took too long to download. Try again." });
    expect(useChatStore.getState().getChat(TAB_ID).blocks).toBe(blocks);
    expect(harness.subscribe).not.toHaveBeenCalled();
    expect(harness.bootstrap.mock.calls.every(call => call[1].signal.aborted)).toBe(true);
    const attempts = harness.bootstrap.mock.calls.length;
    await vi.advanceTimersByTimeAsync(30_000);
    expect(harness.bootstrap).toHaveBeenCalledTimes(attempts);
  });

  it("aborts pending admission reads when the retry window expires", async () => {
    useChatStore.getState().setSessionId(TAB_ID, "session-a");
    optimisticSend();
    const request = mountDeferred();
    harness.bootstrap.mockReturnValue(new Promise(() => {}));
    await request.fail(new Error("404: Session is not saved yet"));
    await vi.advanceTimersByTimeAsync(500);
    expect(harness.bootstrap).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(29_500);
    expect(loadState().phase).toBe("error");
    expect(harness.bootstrap.mock.calls[1][1].signal.aborted).toBe(true);
    expect(harness.subscribe).not.toHaveBeenCalled();
  });

  it("rejects a mismatched session body even when the binding fields match", async () => {
    const request = mountDeferred();
    const wrong = payload();
    wrong.session.id = "session-b";
    await request.finish(wrong);
    expect(loadState()).toMatchObject({ phase: "error", error: "The server returned a different session. Try again." });
    expect(useChatStore.getState().getChat(TAB_ID).blocks).toEqual([]);
    expect(harness.subscribe).not.toHaveBeenCalled();
  });

  it("never treats a missing persisted session as ready or retries it indefinitely", async () => {
    const request = mountDeferred();
    await request.fail(new Error("404: Session not found"));
    await vi.advanceTimersByTimeAsync(30_000);
    expect(harness.bootstrap).toHaveBeenCalledOnce();
    expect(loadState().phase).toBe("error");
    expect(harness.subscribe).not.toHaveBeenCalled();
  });
});
