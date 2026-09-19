import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { apiFetch } from "./api";
import {
  REALTIME_RUN_ID_FIELD,
  REALTIME_SESSION_ID_FIELD,
} from "./realtimeReplay";

vi.mock("./api", () => ({
  apiFetch: vi.fn(async () =>
    new Response(JSON.stringify({ ticket: "ticket-1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  ),
}));

vi.mock("../stores/connectionStore", () => ({
  getApiBase: () => "http://localhost:8181",
}));

class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readyState = MockWebSocket.CONNECTING;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;

  constructor(readonly url: string) {
    MockWebSocket.instances.push(this);
  }

  open() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.();
  }

  send(value: string) {
    this.sent.push(value);
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.();
  }
}

describe("RealtimeClient subscription handoff", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.mocked(apiFetch).mockReset().mockImplementation(async () => new Response(JSON.stringify({ ticket: "ticket-1" }), {
      status: 200, headers: { "content-type": "application/json" },
    }));
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    vi.stubGlobal("window", {
      location: { origin: "http://localhost:8181" },
      addEventListener: vi.fn(),
    });
    vi.stubGlobal("document", {
      visibilityState: "visible",
      addEventListener: vi.fn(),
    });
  });

  afterEach(() => {
    vi.clearAllTimers();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("emits the client snapshot boundary only after history and every control line", async () => {
    const { RealtimeClient, REALTIME_SNAPSHOT_COMPLETE, REALTIME_REPLAY_COMPLETE } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const received: Array<Record<string, unknown>> = [];
    const unsubscribe = client.subscribe("snapshot", line => received.push(line));
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    const lines = [
      { type: "message", message: { type: "assistant", message: { content: [] } } },
      { type: "streaming", active: false },
      { type: "continuity", status: "idle", activeRequestId: null },
      { type: "ask_question", requestId: "question" },
      { type: "queue_updated", requestId: "queued" },
    ];
    socket.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "snapshot", throughSequence: 20, lines }) });
    expect(received.map(line => line.type)).toEqual([...lines.map(line => line.type), REALTIME_SNAPSHOT_COMPLETE]);
    expect(received.at(-1)).toEqual({ type: REALTIME_SNAPSHOT_COMPLETE, throughSequence: 20, [REALTIME_SESSION_ID_FIELD]: "snapshot" });
    expect(lines).toHaveLength(5);
    socket.onmessage?.({ data: JSON.stringify({ type: "event", replay: true, event: {
      sessionId: "snapshot", sequence: 21, runId: "run", payload: { type: "message", message: { type: "tool_result", tool_use_id: "tool", content: "passed" } },
    } }) });
    expect(received.at(-1)?.type).toBe("message");
    socket.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "snapshot", sequence: 21 }) });
    expect(received.at(-1)).toEqual({ type: REALTIME_REPLAY_COMPLETE, throughSequence: 21, [REALTIME_SESSION_ID_FIELD]: "snapshot" });
    const count = received.length;
    socket.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "snapshot", sequence: 21 }) });
    expect(received).toHaveLength(count);
    unsubscribe();
  });

  it("does not report a replaced subscription's partial snapshot as complete", async () => {
    const { RealtimeClient, REALTIME_SNAPSHOT_COMPLETE } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const oldLines: Array<Record<string, unknown>> = [];
    const newLines: Array<Record<string, unknown>> = [];
    let stopNew = () => {};
    const stopOld = client.subscribe("replace", line => {
      oldLines.push(line);
      stopOld();
      stopNew = client.subscribe("replace", next => newLines.push(next));
    });
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    socket.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "replace", throughSequence: 16,
      lines: [{ type: "streaming", active: true }, { type: "continuity", status: "active" }],
    }) });
    expect(oldLines.map(line => line.type)).toEqual(["streaming"]);
    socket.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "replace", sequence: 16 }) });
    expect(newLines).toEqual([]);
    expect(socket.sent.map(value => JSON.parse(value))).toContainEqual({ type: "subscribe", sessionId: "replace", afterSequence: 16 });
    socket.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "replace", throughSequence: 17,
      lines: [{ type: "streaming", active: false }, { type: "continuity", status: "idle" }],
    }) });
    expect(newLines.map(line => line.type)).toEqual(["streaming", "continuity", REALTIME_SNAPSHOT_COMPLETE]);
    stopNew();
  });

  it("requires a new socket's own snapshot before acknowledging catch-up", async () => {
    const { RealtimeClient, REALTIME_REPLAY_COMPLETE } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const lines: Array<Record<string, unknown>> = [];
    const stop = client.subscribe("reconnect", line => lines.push(line));
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const first = MockWebSocket.instances[0];
    first.open();
    first.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "reconnect", throughSequence: 8, lines: [{ type: "streaming", active: true }] }) });
    first.close();
    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(2));
    const next = MockWebSocket.instances[1];
    next.open();
    next.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "reconnect", sequence: 8 }) });
    first.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "reconnect", throughSequence: 999, lines: [{ type: "stale" }] }) });
    expect(lines.some(line => line.type === REALTIME_REPLAY_COMPLETE || line.type === "stale")).toBe(false);
    next.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "reconnect", throughSequence: 8, lines: [{ type: "streaming", active: true }] }) });
    next.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "reconnect", sequence: 9 }) });
    expect(lines.at(-1)?.type).toBe(REALTIME_REPLAY_COMPLETE);
    stop();
  });

  it("reconnects a stale socket and resumes from the last journal cursor", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const lines: Array<Record<string, unknown>> = [];
    const unsubscribe = client.subscribe("session-1", (line) => lines.push(line));
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));

    const first = MockWebSocket.instances[0];
    first.open();
    first.onmessage?.({
      data: JSON.stringify({
        type: "event",
        event: {
          sessionId: "session-1",
          sequence: 7,
          runId: "run-1",
          payload: { type: "text", text: "durable" },
        },
      }),
    });
    expect(lines).toEqual([
      {
        type: "text",
        text: "durable",
        [REALTIME_RUN_ID_FIELD]: "run-1",
        [REALTIME_SESSION_ID_FIELD]: "session-1",
      },
    ]);

    await vi.advanceTimersByTimeAsync(45_000);
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(2));
    const resumed = MockWebSocket.instances[1];
    resumed.open();
    await Promise.resolve();

    expect(resumed.sent.map((value) => JSON.parse(value))).toContainEqual({
      type: "subscribe",
      sessionId: "session-1",
      afterSequence: 7,
    });
    unsubscribe();
  });

  it("keeps the socket alive while a queued run replaces its subscription", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const unsubscribeFirst = client.subscribe("session-1", () => {});
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));

    const socket = MockWebSocket.instances[0];
    socket.open();
    await Promise.resolve();
    expect(socket.sent.map((value) => JSON.parse(value))).toContainEqual({
      type: "subscribe",
      sessionId: "session-1",
      afterSequence: 0,
    });

    unsubscribeFirst();
    const unsubscribeSecond = client.subscribe("session-1", () => {});

    expect(MockWebSocket.instances).toHaveLength(1);
    expect(socket.readyState).toBe(MockWebSocket.OPEN);
    expect(socket.sent.slice(-2).map((value) => JSON.parse(value))).toEqual([
      { type: "unsubscribe", sessionId: "session-1" },
      { type: "subscribe", sessionId: "session-1", afterSequence: 0 },
    ]);

    unsubscribeSecond();
  });

  it("keeps session delivery attached when one of two presentation listeners unmounts", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const firstLines: Array<Record<string, unknown>> = [];
    const secondLines: Array<Record<string, unknown>> = [];
    const unsubscribeFirst = client.subscribe("session-shared", line => firstLines.push(line));
    const unsubscribeSecond = client.subscribe("session-shared", line => secondLines.push(line));
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));

    const socket = MockWebSocket.instances[0];
    socket.open();
    unsubscribeFirst();
    socket.onmessage?.({
      data: JSON.stringify({
        type: "event",
        event: {
          sessionId: "session-shared",
          sequence: 4,
          runId: "run-4",
          payload: { type: "text", text: "still live" },
        },
      }),
    });

    expect(firstLines).toEqual([]);
    expect(secondLines).toEqual([
      {
        type: "text",
        text: "still live",
        [REALTIME_RUN_ID_FIELD]: "run-4",
        [REALTIME_SESSION_ID_FIELD]: "session-shared",
      },
    ]);
    expect(socket.sent.map(value => JSON.parse(value))).not.toContainEqual({
      type: "unsubscribe",
      sessionId: "session-shared",
    });
    unsubscribeSecond();
  });

  it("restores its durable cursor after every listener unsubscribes", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const unsubscribe = client.subscribe("session-persisted", () => {});
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    socket.onmessage?.({
      data: JSON.stringify({
        type: "event",
        event: {
          sessionId: "session-persisted",
          sequence: 19,
          runId: "run-19",
          payload: { type: "text", text: "saved" },
        },
      }),
    });
    unsubscribe();

    const unsubscribeAgain = client.subscribe("session-persisted", () => {});
    expect(socket.sent.map((value) => JSON.parse(value))).toContainEqual({
      type: "subscribe",
      sessionId: "session-persisted",
      afterSequence: 19,
    });
    unsubscribeAgain();
  });

  it("requests a fresh snapshot when presentation state was discarded", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const unsubscribe = client.subscribe("session-reset", () => {});
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    socket.onmessage?.({
      data: JSON.stringify({
        type: "caughtUp",
        sessionId: "session-reset",
        sequence: 23,
      }),
    });
    unsubscribe();
    client.resetCursor("session-reset");
    const unsubscribeAgain = client.subscribe("session-reset", () => {});
    expect(socket.sent.map((value) => JSON.parse(value))).toContainEqual({
      type: "subscribe",
      sessionId: "session-reset",
      afterSequence: 0,
    });
    unsubscribeAgain();
  });

  it("reasserts subscriptions when a mobile webview returns to the foreground", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const unsubscribe = client.subscribe("session-mobile", () => {});
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    socket.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "session-mobile", lines: [{ type: "streaming", active: false }] }) });
    socket.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "session-mobile", sequence: 0 }) });
    socket.sent = [];

    const listener = (document.addEventListener as ReturnType<typeof vi.fn>).mock.calls
      .find(([name]) => name === "visibilitychange")?.[1] as (() => void) | undefined;
    listener?.();

    expect(socket.sent.map((value) => JSON.parse(value))).toEqual([{
      type: "subscribe",
      sessionId: "session-mobile",
      afterSequence: 0,
    }]);
    unsubscribe();
  });

  it("reasserts durable subscriptions on every healthy heartbeat", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const unsubscribe = client.subscribe("session-heartbeat", () => {});
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    socket.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "session-heartbeat", lines: [{ type: "streaming", active: false }] }) });
    socket.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "session-heartbeat", sequence: 0 }) });
    socket.sent = [];

    await vi.advanceTimersByTimeAsync(15_000);

    expect(socket.sent.map((value) => JSON.parse(value))).toEqual([
      { type: "ping" },
      {
        type: "subscribe",
        sessionId: "session-heartbeat",
        afterSequence: 0,
      },
    ]);
    unsubscribe();
  });

  it("does not queue duplicate heartbeat or foreground snapshots while catch-up is pending", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const stop = client.subscribe("large", () => {}, undefined, true);
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    socket.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "large", lines: [{ type: "streaming", active: true }] }) });
    const resume = (document.addEventListener as ReturnType<typeof vi.fn>).mock.calls
      .find(([name]) => name === "visibilitychange")?.[1] as (() => void);
    resume();
    await vi.advanceTimersByTimeAsync(15_000);
    expect(socket.sent.map(value => JSON.parse(value))).toEqual([
      { type: "subscribe", sessionId: "large", afterSequence: 0, includeHistory: false },
      { type: "ping" },
    ]);
    socket.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "large", sequence: 7 }) });
    resume();
    expect(JSON.parse(socket.sent.at(-1)!)).toEqual({ type: "subscribe", sessionId: "large", afterSequence: 7, includeHistory: false });
    stop();
  });

  it("keeps authoritative history suppression on a new socket without skipping journal replay", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const stop = client.subscribe("bootstrapped", () => {}, undefined, true);
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const first = MockWebSocket.instances[0];
    first.open();
    expect(JSON.parse(first.sent[0])).toEqual({ type: "subscribe", sessionId: "bootstrapped", afterSequence: 0, includeHistory: false });
    first.close();
    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(2));
    const next = MockWebSocket.instances[1];
    next.open();
    expect(JSON.parse(next.sent[0])).toEqual({ type: "subscribe", sessionId: "bootstrapped", afterSequence: 0, includeHistory: false });
    stop();
  });

  it("recovers a lost catch-up after45s even while healthy pongs keep arriving", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const stop = client.subscribe("lost-catchup", () => {}, undefined, true);
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const first = MockWebSocket.instances[0];
    first.open();
    for (let interval = 0; interval < 3; interval++) {
      await vi.advanceTimersByTimeAsync(14_999);
      first.onmessage?.({ data: JSON.stringify({ type: "pong" }) });
      await vi.advanceTimersByTimeAsync(1);
    }
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(2));
    expect(first.sent.map(value => JSON.parse(value)).filter(line => line.type === "subscribe")).toHaveLength(1);
    const next = MockWebSocket.instances[1];
    next.open();
    expect(JSON.parse(next.sent[0])).toEqual({ type: "subscribe", sessionId: "lost-catchup", afterSequence: 0, includeHistory: false });
    stop();
  });

  it("gives a late unbootstrapped listener its own snapshot after the pending batch", async () => {
    const { RealtimeClient, REALTIME_REPLAY_COMPLETE } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const first = client.subscribe("joined", () => {}, undefined, true);
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    socket.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "joined", lines: [{ type: "streaming", active: false }] }) });
    const late = vi.fn();
    const stopLate = client.subscribe("joined", late);
    expect(socket.sent).toHaveLength(1);
    socket.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "joined", sequence: 0 }) });
    expect(late).not.toHaveBeenCalled();
    expect(JSON.parse(socket.sent.at(-1)!)).toEqual({ type: "subscribe", sessionId: "joined", afterSequence: 0 });
    socket.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "joined", lines: [{ type: "streaming", active: false }] }) });
    socket.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "joined", sequence: 0 }) });
    expect(late.mock.calls.some(([line]) => line.type === REALTIME_REPLAY_COMPLETE)).toBe(true);
    stopLate(); first();
  });

  it("reports forbidden tickets promptly without repeatedly requesting authorization", async () => {
    vi.mocked(apiFetch).mockResolvedValue(new Response("Forbidden", { status: 403 }));
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const error = vi.fn();
    const stop = client.subscribe("denied", () => {}, error);
    await vi.waitFor(() => expect(error).toHaveBeenCalledOnce());
    expect(error.mock.calls[0][0].message).toMatch(/authorization failed \(403\)/);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(apiFetch).toHaveBeenCalledOnce();
    expect(MockWebSocket.instances).toHaveLength(0);
    stop();
  });

  it("retries a transient ticket failure and still completes the real snapshot", async () => {
    vi.mocked(apiFetch).mockRejectedValueOnce(new TypeError("Failed to fetch"));
    const { RealtimeClient, REALTIME_REPLAY_COMPLETE } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const error = vi.fn();
    const lines = vi.fn();
    const stop = client.subscribe("retry", lines, error);
    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    socket.onmessage?.({ data: JSON.stringify({ type: "snapshot", sessionId: "retry", lines: [{ type: "streaming", active: false }] }) });
    socket.onmessage?.({ data: JSON.stringify({ type: "caughtUp", sessionId: "retry", sequence: 0 }) });
    expect(error).not.toHaveBeenCalled();
    expect(lines.mock.calls.at(-1)?.[0].type).toBe(REALTIME_REPLAY_COMPLETE);
    stop();
  });

  it("separates subscription rejections from provider errors and other sessions", async () => {
    const { RealtimeClient } = await import("./realtimeClient");
    const client = new RealtimeClient();
    const lines = vi.fn();
    const denied = vi.fn();
    const other = vi.fn();
    const stop = client.subscribe("denied", lines, denied);
    const stopOther = client.subscribe("other", () => {}, other);
    await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1));
    const socket = MockWebSocket.instances[0];
    socket.open();
    socket.onmessage?.({ data: JSON.stringify({ type: "event", event: { sessionId: "denied", sequence: 1,
      payload: { type: "error", error: "Provider failed" } } }) });
    expect(denied).not.toHaveBeenCalled();
    expect(lines.mock.calls.at(-1)?.[0].error).toBe("Provider failed");
    socket.onmessage?.({ data: JSON.stringify({ type: "error", sessionId: "denied", error: "Forbidden" }) });
    expect(denied).toHaveBeenCalledOnce();
    expect(other).not.toHaveBeenCalled();
    stop(); stopOther();
  });
});
