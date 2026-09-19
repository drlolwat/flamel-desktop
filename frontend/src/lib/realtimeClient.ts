import { runtimeSocket, type RuntimeSocket } from './runtimeTransport';
import { apiFetch } from "./api";
import { getApiBase } from "../stores/connectionStore";
import { annotateRealtimeReplay } from "./realtimeReplay";

type SessionLine = Record<string, unknown>;
type SessionListener = (line: SessionLine) => void;

interface Subscription {
  listeners: Set<SessionListener>;
  errorListeners: Map<SessionListener, (error: Error) => void>;
  includeHistory: boolean;
  awaitingCatchUp?: boolean;
  catchUpStartedAt?: number;
  resubscribeAfterCatchUp?: boolean;
  cursor: number;
  snapshotListeners?: Set<SessionListener>;
}

interface RealtimeEnvelope {
  type?: string;
  error?: string;
  sessionId?: string;
  throughSequence?: number;
  sequence?: number;
  replay?: boolean;
  lines?: SessionLine[];
  event?: {
    sequence?: number;
    sessionId?: string;
    runId?: string;
    payload?: SessionLine;
  };
}

const IDLE_SOCKET_GRACE_MS = 15_000;
const HEARTBEAT_INTERVAL_MS = 15_000;
const STALE_SOCKET_MS = 45_000;
export const REALTIME_SNAPSHOT_COMPLETE = "snapshot_complete";
export const REALTIME_REPLAY_COMPLETE = "replay_complete";

class TerminalRealtimeError extends Error {
  override name = "RealtimeConnectionError";
}

export class RealtimeClient {
  private socket: RuntimeSocket | null = null;
  private connecting: Promise<void> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private idleCloseTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private reconnectAttempt = 0;
  private stopped = false;
  private lastMessageAt = 0;
  private subscriptions = new Map<string, Subscription>();
  private cursors = new Map<string, number>();

  constructor() {
    if (typeof window !== "undefined") {
      window.addEventListener("online", this.handleResume);
      window.addEventListener("pageshow", this.handleResume);
    }
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", this.handleVisibilityChange);
    }
  }

  private handleVisibilityChange = () => {
    if (document.visibilityState === "visible") this.resumeConnection();
  };

  private handleResume = () => this.resumeConnection();

  private resumeConnection() {
    if (this.subscriptions.size === 0) return;
    this.stopped = false;
    const socket = this.socket;
    if (
      socket?.readyState === WebSocket.OPEN
      && Date.now() - this.lastMessageAt <= STALE_SOCKET_MS
      && !this.hasStalledCatchUp()
    ) {
      // Mobile browsers can preserve an apparently open WebSocket while dropping
      // its server-side subscription. Reassert every cursor on every foreground
      // transition; subscribe is idempotent and replays any missed journal rows.
      for (const sessionId of this.subscriptions.keys()) this.sendSubscribe(sessionId);
      return;
    }
    this.forceReconnect("Client resumed after connection became stale");
  }

  subscribe(
    sessionId: string,
    listener: SessionListener,
    onError?: (error: Error) => void,
    historyLoaded = false,
  ): () => void {
    if (this.idleCloseTimer) clearTimeout(this.idleCloseTimer);
    this.idleCloseTimer = null;
    let subscription = this.subscriptions.get(sessionId);
    if (!subscription) {
      subscription = {
        listeners: new Set(), errorListeners: new Map(), includeHistory: !historyLoaded,
        cursor: this.cursors.get(sessionId) || 0,
      };
      this.subscriptions.set(sessionId, subscription);
    }
    // Never withhold history from another listener that has not bootstrapped.
    const needsHistoryUpgrade = !historyLoaded && !subscription.includeHistory;
    if (needsHistoryUpgrade) {
      subscription.includeHistory = true;
      if (subscription.awaitingCatchUp) subscription.resubscribeAfterCatchUp = true;
    }
    const isNewListener = !subscription.listeners.has(listener);
    subscription.listeners.add(listener);
    if (onError) subscription.errorListeners.set(listener, onError);
    this.stopped = false;
    if (isNewListener && this.socket?.readyState === WebSocket.OPEN) {
      this.sendSubscribe(sessionId);
    } else {
      // `connect` subscribes every registered session exactly once on open.
      void this.ensureConnected().catch(error => this.handleConnectionError(error));
    }

    return () => {
      const current = this.subscriptions.get(sessionId);
      if (!current) return;
      current.listeners.delete(listener);
      current.errorListeners.delete(listener);
      if (current.listeners.size > 0) return;
      this.subscriptions.delete(sessionId);
      this.send({ type: "unsubscribe", sessionId });
      if (this.subscriptions.size === 0) this.scheduleIdleClose();
    };
  }

  resetCursor(sessionId: string) {
    this.cursors.delete(sessionId);
    const subscription = this.subscriptions.get(sessionId);
    if (subscription) subscription.cursor = 0;
  }

  private async ensureConnected(): Promise<void> {
    if (this.socket?.readyState === WebSocket.OPEN) return;
    if (this.connecting) return this.connecting;
    this.connecting = this.connect();
    try {
      await this.connecting;
    } finally {
      this.connecting = null;
    }
  }

  private async connect(): Promise<void> {
    const response = await apiFetch("/api/realtime/ticket", { method: "POST" });
    if (response.status === 401 || response.status === 403) {
      throw new TerminalRealtimeError(`Live session authorization failed (${response.status}). Sign in again.`);
    }
    if (!response.ok) throw new Error(`Realtime authorization failed (${response.status})`);
    const { ticket } = (await response.json()) as { ticket: string };
    if (typeof ticket !== "string" || !ticket) {
      throw new TerminalRealtimeError("The server returned an invalid live session ticket.");
    }
    const base = getApiBase() || window.location.origin;
    const url = new URL("/api/realtime", base);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.searchParams.set("ticket", ticket);

    await new Promise<void>((resolve, reject) => {
      const socket = runtimeSocket(url);
      let opened = false;
      let settled = false;
      this.socket = socket;
      socket.onopen = () => {
        opened = true;
        settled = true;
        this.reconnectAttempt = 0;
        this.lastMessageAt = Date.now();
        this.startHeartbeat();
        for (const [sessionId, subscription] of this.subscriptions) {
          subscription.snapshotListeners = undefined;
          subscription.awaitingCatchUp = false;
          subscription.catchUpStartedAt = undefined;
          this.sendSubscribe(sessionId);
        }
        resolve();
      };
      socket.onmessage = (event) => {
        if (this.socket !== socket) return;
        this.lastMessageAt = Date.now();
        this.handleMessage(event.data);
      };
      socket.onerror = () => {
        if (!settled) {
          settled = true;
          reject(new Error("Realtime connection failed"));
        }
      };
      socket.onclose = () => {
        if (this.socket === socket) {
          this.socket = null;
          this.stopHeartbeat();
        }
        if (!opened && !settled) {
          settled = true;
          reject(new Error("Realtime connection closed before it was ready"));
        }
        if (!this.stopped && this.subscriptions.size > 0) this.scheduleReconnect();
      };
    });
  }

  private sendSubscribe(sessionId: string) {
    const subscription = this.subscriptions.get(sessionId);
    if (!subscription || subscription.awaitingCatchUp || this.socket?.readyState !== WebSocket.OPEN) return;
    this.send({
      type: "subscribe", sessionId, afterSequence: subscription.cursor,
      ...(subscription.includeHistory ? {} : { includeHistory: false }),
    });
    subscription.awaitingCatchUp = true;
    subscription.catchUpStartedAt = Date.now();
    subscription.resubscribeAfterCatchUp = false;
  }

  private send(value: Record<string, unknown>) {
    if (this.socket?.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify(value));
  }

  private emit(sessionId: string, line: SessionLine) {
    const subscription = this.subscriptions.get(sessionId);
    if (!subscription) return;
    for (const listener of subscription.listeners) listener(line);
  }

  private failSubscription(sessionId: string, error: Error) {
    const subscription = this.subscriptions.get(sessionId);
    if (!subscription) return;
    for (const [listener, onError] of [...subscription.errorListeners]) {
      if (subscription.listeners.has(listener)) onError(error);
    }
  }

  private handleConnectionError(error: unknown) {
    if (!(error instanceof TerminalRealtimeError)) {
      // Network failures still use the existing backoff. Initial hydration is
      // bounded by its own watchdog; an already-live stream keeps reconnecting.
      if (!this.stopped && this.subscriptions.size > 0) this.scheduleReconnect();
      return;
    }
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.stopped = true;
    for (const sessionId of [...this.subscriptions.keys()]) this.failSubscription(sessionId, error);
  }

  private handleMessage(raw: unknown) {
    if (typeof raw !== "string") return;
    let envelope: RealtimeEnvelope;
    try {
      envelope = JSON.parse(raw) as RealtimeEnvelope;
    } catch {
      return;
    }
    if (envelope.type === "error" && envelope.sessionId) {
      // This is the socket protocol's subscription rejection, not a provider
      // error inside an event payload. Fail only the rejected session.
      this.failSubscription(envelope.sessionId, new TerminalRealtimeError(
        envelope.error === "Forbidden"
          ? "You no longer have access to this live session. Sign in again or reopen the chat."
          : "The server could not subscribe to this live session. Try again.",
      ));
      return;
    }
    if (envelope.type === "snapshot" && envelope.sessionId) {
      const subscription = this.subscriptions.get(envelope.sessionId);
      if (!subscription) return;
      subscription.cursor = Math.max(subscription.cursor, envelope.throughSequence || 0);
      this.cursors.set(envelope.sessionId, subscription.cursor);
      // A snapshot line can synchronously replace the subscription (for
      // example, when it restores an active compaction). Save the boundary
      // first so the replacement resumes after this snapshot.
      const listeners = [...subscription.listeners];
      subscription.snapshotListeners = new Set(listeners);
      const emitSnapshotLine = (line: SessionLine) => {
        // A replacement must receive its own complete snapshot, not the tail
        // (or readiness marker) of the subscription it just replaced.
        if (this.subscriptions.get(envelope.sessionId!) !== subscription) return;
        for (const listener of listeners) {
          if (subscription.listeners.has(listener)) {
            listener(annotateRealtimeReplay(line, false, undefined, envelope.sessionId));
          }
        }
      };
      for (const line of envelope.lines || []) {
        emitSnapshotLine(line);
      }
      // Transport-only: no server schema or durable journal event is changed.
      // The NDJSON consumer handles all control lines before this boundary.
      emitSnapshotLine({ type: REALTIME_SNAPSHOT_COMPLETE, throughSequence: subscription.cursor });
      return;
    }
    if (envelope.type === "event" && envelope.event?.sessionId && envelope.event.payload) {
      const subscription = this.subscriptions.get(envelope.event.sessionId);
      if (!subscription) return;
      const sequence = envelope.event.sequence || 0;
      if (sequence <= subscription.cursor) return;
      // Persist progress before invoking listeners. Compaction and run handoff
      // handlers may synchronously replace their subscription; a re-entrant
      // subscriber must resume after this event rather than replaying it.
      subscription.cursor = sequence;
      this.cursors.set(envelope.event.sessionId, sequence);
      this.emit(
        envelope.event.sessionId,
        annotateRealtimeReplay(
          envelope.event.payload,
          envelope.replay === true,
          envelope.event.runId,
          envelope.event.sessionId,
        ),
      );
      return;
    }
    if (envelope.type === "caughtUp" && envelope.sessionId) {
      const subscription = this.subscriptions.get(envelope.sessionId);
      if (!subscription) return;
      subscription.cursor = Math.max(subscription.cursor, envelope.sequence || 0);
      this.cursors.set(envelope.sessionId, subscription.cursor);
      const listeners = subscription.snapshotListeners;
      subscription.snapshotListeners = undefined;
      subscription.awaitingCatchUp = false;
      subscription.catchUpStartedAt = undefined;
      for (const listener of listeners || []) {
        if (this.subscriptions.get(envelope.sessionId) === subscription && subscription.listeners.has(listener)) {
          listener(annotateRealtimeReplay({
            type: REALTIME_REPLAY_COMPLETE, throughSequence: subscription.cursor,
          }, false, undefined, envelope.sessionId));
        }
      }
      if (this.subscriptions.get(envelope.sessionId) === subscription && (
        subscription.resubscribeAfterCatchUp
        || [...subscription.listeners].some(listener => !listeners?.has(listener))
      )) {
        // A listener added after snapshot emission needs its own full boundary.
        // Wait for this batch to finish instead of adding duplicate work ahead.
        this.sendSubscribe(envelope.sessionId);
      }
    }
  }

  private scheduleReconnect() {
    if (this.reconnectTimer) return;
    const attempt = this.reconnectAttempt++;
    const baseDelay = Math.min(10_000, 300 * 2 ** Math.min(attempt, 5));
    const delay = baseDelay + Math.floor(Math.random() * Math.max(100, baseDelay / 3));
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      void this.ensureConnected().catch(error => this.handleConnectionError(error));
    }, delay);
  }

  private startHeartbeat() {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      if (Date.now() - this.lastMessageAt >= STALE_SOCKET_MS || this.hasStalledCatchUp()) {
        this.forceReconnect("Realtime heartbeat timed out");
        return;
      }
      this.send({ type: "ping" });
      // Reassert subscriptions as part of the heartbeat. Mobile WebViews can
      // retain an apparently healthy socket after the server-side cursor was
      // lost; subscribe is idempotent and replays from the durable cursor.
      for (const sessionId of this.subscriptions.keys()) {
        this.sendSubscribe(sessionId);
      }
    }, HEARTBEAT_INTERVAL_MS);
  }

  private hasStalledCatchUp() {
    return [...this.subscriptions.values()].some(subscription =>
      subscription.awaitingCatchUp && subscription.catchUpStartedAt !== undefined
      && Date.now() - subscription.catchUpStartedAt >= STALE_SOCKET_MS,
    );
  }

  private forceReconnect(reason: string) {
    if (this.subscriptions.size === 0) return;
    const socket = this.socket;
    this.socket = null;
    this.stopHeartbeat();
    if (socket && socket.readyState !== WebSocket.CLOSED) socket.close(4000, reason);
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    void this.ensureConnected().catch(error => this.handleConnectionError(error));
  }

  private stopHeartbeat() {
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
    this.heartbeatTimer = null;
  }

  private scheduleIdleClose() {
    this.stopped = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    if (this.idleCloseTimer) clearTimeout(this.idleCloseTimer);
    this.idleCloseTimer = setTimeout(() => {
      this.idleCloseTimer = null;
      if (this.subscriptions.size === 0) this.stop();
    }, IDLE_SOCKET_GRACE_MS);
  }

  private stop() {
    this.stopped = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    if (this.idleCloseTimer) clearTimeout(this.idleCloseTimer);
    this.idleCloseTimer = null;
    this.stopHeartbeat();
    this.lastMessageAt = 0;
    this.socket?.close(1000, "No active subscriptions");
    this.socket = null;
  }
}

const realtimeClient = new RealtimeClient();

/** Adapts the shared WebSocket to the existing NDJSON parser contract. */
export function openRealtimeSessionStream(
  sessionId: string,
  signal?: AbortSignal,
  freshSnapshot = false,
  historyLoaded = false,
): Response {
  if (freshSnapshot) realtimeClient.resetCursor(sessionId);
  const encoder = new TextEncoder();
  let unsubscribe: (() => void) | undefined;
  let abortHandler: (() => void) | undefined;
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      const close = () => {
        unsubscribe?.();
        unsubscribe = undefined;
        if (abortHandler && signal) signal.removeEventListener("abort", abortHandler);
        try { controller.close(); } catch { /* stream is already closed */ }
      };
      unsubscribe = realtimeClient.subscribe(sessionId, (line) => {
        controller.enqueue(encoder.encode(`${JSON.stringify(line)}\n`));
      }, error => {
        controller.error(error);
        close();
      }, historyLoaded);
      abortHandler = close;
      if (signal?.aborted) close();
      else signal?.addEventListener("abort", close, { once: true });
    },
    cancel() {
      unsubscribe?.();
      unsubscribe = undefined;
      if (abortHandler && signal) signal.removeEventListener("abort", abortHandler);
    },
  });
  return new Response(stream, {
    status: 200,
    headers: { "content-type": "application/x-ndjson" },
  });
}
