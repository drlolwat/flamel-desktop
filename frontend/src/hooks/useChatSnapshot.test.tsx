import React from "react";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useChat } from "./useChat";
import { useChatStore } from "../stores/chatStore";
import { useTabStore } from "../stores/tabStore";
import { chatRuntime } from "../lib/chatRuntime";

const harness = vi.hoisted(() => ({
  controllers: [] as ReadableStreamDefaultController<Uint8Array>[],
  open: vi.fn(),
}));
vi.mock("../lib/realtimeClient", () => ({
  REALTIME_SNAPSHOT_COMPLETE: "snapshot_complete",
  REALTIME_REPLAY_COMPLETE: "replay_complete",
  openRealtimeSessionStream: harness.open,
}));

const cleanup: Array<() => Promise<unknown>> = [];
let serial = 0;
let sessionId: string;
let tabId: string;
let chat: ReturnType<typeof useChat>;
const encoder = new TextEncoder();
const emit = (line: Record<string, unknown>, index = 0) => {
  harness.controllers[index].enqueue(encoder.encode(`${JSON.stringify(line)}\n`));
};
const tick = async () => { for (let n = 0; n < 8; n++) await Promise.resolve(); };

beforeEach(() => {
  serial++;
  sessionId = `snapshot-session-${serial}`;
  tabId = `snapshot-tab-${serial}`;
  harness.controllers = [];
  harness.open.mockReset();
  harness.open.mockImplementation((_session: string, signal: AbortSignal) => new Response(new ReadableStream<Uint8Array>({
    start(controller) {
      harness.controllers.push(controller);
      signal.addEventListener("abort", () => { try { controller.close(); } catch {} }, { once: true });
    },
  })));
  useChatStore.setState({ chats: {} });
  useTabStore.setState({ tabs: [] });
  useTabStore.getState().addTab({ id: tabId, sessionId, bindingGeneration: 1 });
  useChatStore.getState().setSessionId(tabId, sessionId);
  // Exercise the actual hook/parser and stores; SSR only supplies React's hook
  // context. No effects, network, provider calls or DOM dependency are needed.
  function Capture() { chat = useChat(tabId); return null; }
  renderToString(<Capture />);
});

afterEach(async () => {
  for (const close of cleanup.splice(0)) await close();
});

function subscribe(onHistoryReady = vi.fn(), persistent = true) {
  const abort = new AbortController();
  const done = chat.subscribeToLiveStream(sessionId, {
    onHistoryReady, persistent, signal: abort.signal, bindingGeneration: 1,
  });
  cleanup.push(async () => { abort.abort(); await done.catch(() => {}); });
  return { abort, done, ready: onHistoryReady };
}

describe("full realtime snapshot readiness", () => {
  it("passes history suppression only when authoritative bootstrap is explicitly confirmed", async () => {
    const first = subscribe();
    expect(harness.open.mock.calls[0][3]).toBe(false);
    first.abort.abort();
    await first.done;
    const abort = new AbortController();
    const done = chat.subscribeToLiveStream(sessionId, {
      persistent: true, historyLoaded: true, onHistoryReady: vi.fn(), signal: abort.signal,
    });
    cleanup.push(async () => { abort.abort(); await done.catch(() => {}); });
    expect(harness.open.mock.calls[1][3]).toBe(true);
  });

  it("propagates setup transport errors instead of swallowing the startup reason", async () => {
    const stream = subscribe();
    const rejected = expect(stream.done).rejects.toThrow("Live session authorization failed (403)");
    harness.controllers[0].error(new Error("Live session authorization failed (403)"));
    await rejected;
    expect(stream.ready).not.toHaveBeenCalled();
  });

  it("keeps provider error events in chat without treating them as setup transport failure", async () => {
    const stream = subscribe();
    emit({ type: "streaming", active: true });
    emit({ type: "error", error: "Provider declined the request" });
    emit({ type: "snapshot_complete" });
    emit({ type: "replay_complete" });
    await tick();
    expect(stream.ready).toHaveBeenCalledOnce();
    expect(useChatStore.getState().getChat(tabId).blocks.at(-1)?.content).toBe("Error: Provider declined the request");
  });

  it("applies replayed terminal tool receipts before reporting ready", async () => {
    const ready = vi.fn(() => expect(useChatStore.getState().getChat(tabId).blocks.flatMap(block => block.tools || []))
      .toEqual([expect.objectContaining({ id: "verify", status: "done", output: "passed" })]));
    subscribe(ready);
    emit({ type: "message", message: { type: "assistant", message: { content: [
      { type: "tool_use", id: "verify", name: "shell", input: {} },
    ] } } });
    emit({ type: "streaming", active: true });
    emit({ type: "continuity", status: "active", activeRequestId: "active-run" });
    emit({ type: "snapshot_complete" });
    await tick();
    expect(ready).not.toHaveBeenCalled();
    emit({ type: "message", __flamelRealtimeReplay: true, message: {
      type: "tool_result", tool_use_id: "verify", content: "passed", is_error: false,
    } });
    emit({ type: "replay_complete" });
    await tick();
    expect(ready).toHaveBeenCalledOnce();
  });

  it("keeps the successor spinner, Stop target and runtime after late predecessor events", async () => {
    subscribe();
    emit({ type: "streaming", active: true });
    emit({ type: "continuity", status: "active", activeRequestId: "old", __flamelRealtimeRunId: "old" });
    emit({ type: "snapshot_complete" });
    await tick();
    emit({ type: "queue_updated", state: "queued", requestId: "new", message: "Next", sessionId });
    emit({ type: "continuity", status: "active", activeRequestId: "new", __flamelRealtimeRunId: "new" });
    await tick();
    for (const event of [
      {type:"continuity",status:"idle"},
      {type:"done",queuePending:false,completionManaged:"server"},
      {type:"continuity",status:"recovering",activeRequestId:"old"},
    ]) emit({...event,__flamelRealtimeRunId:"old"});
    await tick();
    const tab=useTabStore.getState().tabs.find(t=>t.id===tabId);
    expect(tab?.isStreaming).toBe(true);
    expect(tab?.requestId).toBe("new");
    expect(chatRuntime.forTab(tabId).active).toBe(true);
    expect(chatRuntime.forTab(tabId).queue).toHaveLength(0);
  });

  it("waits for continuity, pending questions and queue before notifying", async () => {
    const ready = vi.fn(() => {
      expect(useTabStore.getState().tabs[0].requestId).toBe("active-run");
      expect(useChatStore.getState().getChat(tabId).pendingQuestion?.requestId).toBe("question-1");
      expect(chatRuntime.forTab(tabId).queue.map(item => item.id)).toContain("queued-run");
    });
    subscribe(ready);
    emit({ type: "streaming", active: true });
    await tick();
    expect(ready).not.toHaveBeenCalled();
    emit({ type: "continuity", status: "active", activeRequestId: "active-run" });
    emit({ type: "ask_question", requestId: "question-1", sessionId, question: "Continue?", choices: [], allowFreeText: true });
    emit({ type: "queue_updated", state: "queued", requestId: "queued-run", message: "Next", sessionId });
    await tick();
    expect(ready).not.toHaveBeenCalled();
    emit({ type: "snapshot_complete" });
    await tick();
    expect(ready).not.toHaveBeenCalled();
    emit({ type: "replay_complete" });
    await tick();
    expect(ready).toHaveBeenCalledOnce();
    emit({ type: "snapshot_complete" });
    emit({ type: "replay_complete" });
    await tick();
    expect(ready).toHaveBeenCalledOnce();
  });

  it("consumes the full inactive snapshot before a nonpersistent reader exits", async () => {
    const stream = subscribe(vi.fn(), false);
    emit({ type: "streaming", active: false });
    await tick();
    expect(stream.ready).not.toHaveBeenCalled();
    emit({ type: "continuity", status: "idle", isSessionWarm: false, requiresResume: true });
    emit({ type: "ask_question", requestId: "idle-question", sessionId, question: "Choose", choices: [] });
    emit({ type: "snapshot_complete" });
    emit({ type: "replay_complete" });
    await stream.done;
    expect(stream.ready).toHaveBeenCalledOnce();
    expect(useTabStore.getState().tabs[0].requiresResume).toBe(true);
    expect(useChatStore.getState().getChat(tabId).pendingQuestion?.requestId).toBe("idle-question");
  });

  it("lets a joined persistent subscriber wait for or reuse the real snapshot boundary", async () => {
    const first = subscribe();
    const pendingJoin = subscribe();
    await tick();
    expect(harness.open).toHaveBeenCalledOnce();
    expect(pendingJoin.ready).not.toHaveBeenCalled();
    emit({ type: "streaming", active: false });
    emit({ type: "continuity", status: "idle" });
    emit({ type: "snapshot_complete" });
    await tick();
    expect(pendingJoin.ready).not.toHaveBeenCalled();
    emit({ type: "replay_complete" });
    await pendingJoin.done;
    await tick();
    expect(first.ready).toHaveBeenCalledOnce();
    expect(pendingJoin.ready).toHaveBeenCalledOnce();
    const readyJoin = subscribe();
    await readyJoin.done;
    expect(readyJoin.ready).toHaveBeenCalledOnce();
    expect(harness.open).toHaveBeenCalledOnce();
  });

  it("does not signal readiness on abort, early stream closure or stale tab binding", async () => {
    const first = subscribe();
    emit({ type: "streaming", active: false });
    await tick();
    first.abort.abort();
    await first.done;
    expect(first.ready).not.toHaveBeenCalled();

    const early = subscribe();
    harness.controllers[1].close();
    await early.done;
    expect(early.ready).not.toHaveBeenCalled();

    const stale = subscribe();
    useTabStore.getState().updateTab(tabId, { sessionId: "different-session" });
    emit({ type: "streaming", active: false }, 2);
    emit({ type: "snapshot_complete" }, 2);
    emit({ type: "replay_complete" }, 2);
    await stale.done;
    expect(stale.ready).not.toHaveBeenCalled();
  });

  it("requires the replacement persistent owner's own complete snapshot", async () => {
    const transient = subscribe(vi.fn(), false);
    emit({ type: "streaming", active: true });
    await tick();
    const persistent = subscribe();
    await transient.done;
    expect(transient.ready).not.toHaveBeenCalled();
    expect(harness.open).toHaveBeenCalledTimes(2);
    emit({ type: "streaming", active: false }, 1);
    emit({ type: "continuity", status: "idle" }, 1);
    expect(persistent.ready).not.toHaveBeenCalled();
    emit({ type: "snapshot_complete" }, 1);
    emit({ type: "replay_complete" }, 1);
    await tick();
    expect(persistent.ready).toHaveBeenCalledOnce();
  });

  it("cancels a waiting join without cancelling the shared persistent stream", async () => {
    const first = subscribe();
    const joined = subscribe();
    joined.abort.abort();
    await joined.done;
    expect(joined.ready).not.toHaveBeenCalled();
    emit({ type: "streaming", active: false });
    emit({ type: "continuity", status: "idle" });
    emit({ type: "snapshot_complete" });
    emit({ type: "replay_complete" });
    await tick();
    expect(first.ready).toHaveBeenCalledOnce();
    expect(joined.ready).not.toHaveBeenCalled();
    expect(harness.open).toHaveBeenCalledOnce();
  });
});
