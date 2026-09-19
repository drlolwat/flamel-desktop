import { runtimeActiveForContinuityStatus } from "./realtimeReplay";

export type SendOptions = {
  model: string;
  reasoningEffort?: string;
  ponytailEnabled?: boolean;
  permissionMode: string;
  sessionId?: string;
  images?: Array<{ data: string; mediaType: string; name?: string }>;
  planningMode?: string;
  runnerId?: string;
  __isContinuation?: boolean;
  __reuseAssistantBlock?: boolean;
  __continuationCount?: number;
  __recoveryNote?: string;
  __recoverySummary?: string;
  __forceNewSession?: boolean;
  __interruptActive?: boolean;
};

export type QueuedMessage = {
  id: string;
  message: string;
  workingDirectory: string;
  options: SendOptions;
  createdAt: number;
  serverOwned?: boolean;
  admissionState?: "pending" | "accepted" | "failed";
  accepted?: Promise<boolean>;
};

export type ChatRuntimeState = {
  queue: QueuedMessage[];
  active: boolean;
  drainAfterAbort: boolean;
  subscribers: Set<() => void>;
};

export type SessionStreamOwner = {
  tabId: string;
  bindingGeneration: number;
  persistent: boolean;
  cancel: () => void;
  done: Promise<void>;
  snapshotReady?: Promise<boolean>;
};

export async function notifySessionSnapshotReady(
  owner: SessionStreamOwner,
  onReady: () => void,
  isCurrent: () => boolean,
  signal?: AbortSignal,
): Promise<void> {
  // Joined callers observe the existing snapshot, not the lifetime of a
  // persistent stream. Early close/cancellation resolves false, never ready.
  if (!owner.snapshotReady || signal?.aborted) return;
  let cancel: (() => void) | undefined;
  const aborted = new Promise<boolean>((resolve) => {
    cancel = () => resolve(false);
    signal?.addEventListener("abort", cancel, { once: true });
  });
  try {
    const ready = await Promise.race([owner.snapshotReady, aborted]);
    if (ready && !signal?.aborted && isCurrent()) onReady();
  } finally {
    if (cancel) signal?.removeEventListener("abort", cancel);
  }
}

type StreamClaim =
  | { status: "claimed"; owner: SessionStreamOwner }
  | { status: "joined"; owner: SessionStreamOwner };

type QueueDrain =
  | { status: "next"; item: QueuedMessage }
  | { status: "idle" }
  | { status: "server_owned" };

export class ChatRuntimeCoordinator {
  private readonly states = new Map<string, ChatRuntimeState>();
  private readonly streamOwners = new Map<string, SessionStreamOwner>();
  private readonly abortHandlers = new Map<string, () => void>();

  forTab(tabId: string): ChatRuntimeState {
    let runtime = this.states.get(tabId);
    if (!runtime) {
      runtime = {
        queue: [],
        active: false,
        drainAfterAbort: false,
        subscribers: new Set(),
      };
      this.states.set(tabId, runtime);
    }
    return runtime;
  }

  notify(runtime: ChatRuntimeState): void {
    for (const subscriber of Array.from(runtime.subscribers)) subscriber();
  }

  cleanupTab(tabId: string, runtime: ChatRuntimeState): void {
    if (
      this.states.get(tabId) === runtime &&
      runtime.subscribers.size === 0 &&
      !runtime.active &&
      runtime.queue.length === 0
    ) {
      this.states.delete(tabId);
      this.abortHandlers.delete(tabId);
    }
  }

  abortRef(tabId: string): { current: (() => void) | null } {
    const handlers = this.abortHandlers;
    return {
      get current() {
        return handlers.get(tabId) ?? null;
      },
      set current(handler: (() => void) | null) {
        if (handler) handlers.set(tabId, handler);
        else handlers.delete(tabId);
      },
    };
  }

  admit(runtime: ChatRuntimeState, item: QueuedMessage): QueuedMessage | null {
    runtime.queue.push(item);
    if (runtime.active) {
      this.notify(runtime);
      return null;
    }
    runtime.active = true;
    const next = runtime.queue.shift()!;
    this.notify(runtime);
    return next;
  }

  drain(runtime: ChatRuntimeState): QueueDrain {
    const nextIndex = runtime.queue.findIndex(
      (item) => !item.serverOwned && item.admissionState !== "failed",
    );
    if (nextIndex >= 0) {
      const [item] = runtime.queue.splice(nextIndex, 1);
      runtime.active = true;
      this.notify(runtime);
      return { status: "next", item };
    }
    if (runtime.queue.some((item) => item.serverOwned)) {
      return { status: "server_owned" };
    }
    runtime.active = false;
    this.notify(runtime);
    return { status: "idle" };
  }

  claimSessionStream(
    sessionId: string,
    candidate: SessionStreamOwner,
  ): StreamClaim {
    const current = this.streamOwners.get(sessionId);
    if (
      current &&
      current.tabId === candidate.tabId &&
      current.bindingGeneration === candidate.bindingGeneration &&
      (current.persistent || !candidate.persistent)
    ) {
      return { status: "joined", owner: current };
    }
    current?.cancel();
    this.streamOwners.set(sessionId, candidate);
    return { status: "claimed", owner: candidate };
  }

  releaseSessionStream(sessionId: string, owner: SessionStreamOwner): boolean {
    if (this.streamOwners.get(sessionId) !== owner) return false;
    this.streamOwners.delete(sessionId);
    return true;
  }

  ownsSessionStream(sessionId: string, owner: SessionStreamOwner): boolean {
    return this.streamOwners.get(sessionId) === owner;
  }
}

type ContinuityEvent = {
  type?: unknown;
  status?: unknown;
  activeRequestId?: unknown;
  isSessionWarm?: unknown;
  requiresResume?: unknown;
  retryAt?: unknown;
  completionManaged?: unknown;
};

type ContinuityContext = {
  runtime: ChatRuntimeState;
  now: number;
  notify: () => void;
  beginLiveRun: (requestId?: string, queuedItem?: QueuedMessage) => void;
  clearLiveRun: () => void;
  updateTab: (updates: {
    requestId?: string;
    currentTool?: string;
    isStreaming?: boolean;
    isSessionWarm: boolean;
    requiresResume: boolean;
    sessionWarmSince?: number;
  }) => void;
};

export function applyRealtimeContinuityEvent(
  event: ContinuityEvent,
  context: ContinuityContext,
): boolean {
  if (event.type !== "continuity") return false;
  const status = typeof event.status === "string" ? event.status : undefined;
  const activeRequestId =
    typeof event.activeRequestId === "string"
      ? event.activeRequestId
      : undefined;
  const queuedItem = activeRequestId
    ? context.runtime.queue.find((item) => item.id === activeRequestId)
    : undefined;

  if (status === "active" || status === "recovering") {
    context.beginLiveRun(activeRequestId, queuedItem);
  }
  if (activeRequestId) {
    const queueLength = context.runtime.queue.length;
    context.runtime.queue = context.runtime.queue.filter(
      (item) => item.id !== activeRequestId,
    );
    if (context.runtime.queue.length !== queueLength) context.notify();
  }

  const continuityActive = runtimeActiveForContinuityStatus(status);
  if (
    continuityActive !== undefined &&
    context.runtime.active !== continuityActive
  ) {
    context.runtime.active = continuityActive;
    context.notify();
  }
  if (continuityActive === false) context.clearLiveRun();

  const isSessionWarm =
    typeof event.isSessionWarm === "boolean"
      ? event.isSessionWarm
      : status === "idle";
  const requiresResume =
    typeof event.requiresResume === "boolean"
      ? event.requiresResume
      : status === "idle" && !isSessionWarm;
  context.updateTab({
    requestId: continuityActive === true ? activeRequestId : undefined,
    ...(continuityActive !== undefined && {
      isStreaming: continuityActive,
    }),
    isSessionWarm,
    requiresResume: event.completionManaged === "server" && continuityActive ? false : requiresResume,
    ...(status === "recovering" ? {currentTool: typeof event.retryAt === "string"
      ? `Recovering · retry at ${new Date(event.retryAt).toLocaleTimeString()}` : "Recovering…"} : {}),
    sessionWarmSince: isSessionWarm ? context.now : undefined,
  });
  return true;
}

type QueueEvent = {
  type?: unknown;
  state?: unknown;
  requestId?: unknown;
  message?: unknown;
  interruptActive?: unknown;
  hiddenContinuation?: unknown;
  createdAt?: unknown;
};

type QueueEventContext = {
  runtime: ChatRuntimeState;
  workingDirectory: string;
  sessionId: string;
  defaults: Pick<SendOptions, "model" | "reasoningEffort" | "permissionMode">;
  now: number;
  notify: () => void;
};

export function applyRealtimeQueueEvent(
  event: QueueEvent,
  context: QueueEventContext,
): boolean {
  if (
    event.type !== "queue_updated" ||
    event.state !== "queued" ||
    typeof event.requestId !== "string" ||
    typeof event.message !== "string"
  ) {
    return false;
  }

  const existingQueued = context.runtime.queue.some(
    (item) => item.id === event.requestId,
  );
  if (existingQueued) {
    context.runtime.queue = context.runtime.queue.map((item) =>
      item.id === event.requestId
        ? {
            ...item,
            message: event.message as string,
            options: {
              ...item.options,
              __interruptActive:
                item.options.__interruptActive ||
                event.interruptActive === true,
            },
          }
        : item,
    );
  } else {
    context.runtime.queue.push({
      id: event.requestId,
      message: event.message,
      workingDirectory: context.workingDirectory,
      options: {
        ...context.defaults,
        sessionId: context.sessionId,
        __isContinuation: event.hiddenContinuation === true,
        __interruptActive: event.interruptActive === true,
      },
      createdAt:
        typeof event.createdAt === "number" ? event.createdAt : context.now,
      serverOwned: true,
      accepted: Promise.resolve(true),
    });
  }
  context.notify();
  return true;
}

export function countUserMessagesBefore(
  blocks: readonly { type?: string }[],
  blockIndex: number,
): number {
  return blocks.slice(0, blockIndex).filter((block) => block.type === "user")
    .length;
}

export const chatRuntime = new ChatRuntimeCoordinator();
