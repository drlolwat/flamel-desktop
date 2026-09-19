export const REALTIME_REPLAY_FIELD = "__flamelRealtimeReplay";
export const REALTIME_RUN_ID_FIELD = "__flamelRealtimeRunId";
export const REALTIME_SESSION_ID_FIELD = "__flamelRealtimeSessionId";

type RealtimeLine = Record<string, unknown>;

/** Adds transport-only journal metadata without mutating the persisted payload. */
export function annotateRealtimeReplay(
  line: RealtimeLine,
  replay: boolean,
  runId?: string,
  sessionId?: string,
): RealtimeLine {
  if (!replay && !runId && !sessionId) return line;
  return {
    ...line,
    ...(replay ? { [REALTIME_REPLAY_FIELD]: true } : {}),
    ...(runId ? { [REALTIME_RUN_ID_FIELD]: runId } : {}),
    ...(sessionId ? { [REALTIME_SESSION_ID_FIELD]: sessionId } : {}),
  };
}

export function isRealtimeReplay(line: unknown): boolean {
  return (
    !!line &&
    typeof line === "object" &&
    (line as RealtimeLine)[REALTIME_REPLAY_FIELD] === true
  );
}

export function realtimeRunId(line: unknown): string | undefined {
  if (!line || typeof line !== "object") return undefined;
  const runId = (line as RealtimeLine)[REALTIME_RUN_ID_FIELD];
  return typeof runId === "string" && runId ? runId : undefined;
}

/** Rejects presentation events that belong to a stale or different tab binding. */
export function shouldApplyRealtimeEventToSession(
  line: unknown,
  sessionId: string,
): boolean {
  if (!line || typeof line !== "object") return false;
  const eventSessionId = (line as RealtimeLine)[REALTIME_SESSION_ID_FIELD];
  return typeof eventSessionId !== "string" || eventSessionId === sessionId;
}

/** Snapshot lines have no run ID and remain applicable to every subscription. */
export function shouldApplyRealtimeEventToRun(
  line: unknown,
  requestId?: string,
  retiredRunIds?: ReadonlySet<string>,
): boolean {
  if (
    line &&
    typeof line === "object" &&
    ["queue_updated", "interrupt_requested"].includes(
      String((line as RealtimeLine).type || ""),
    )
  ) {
    return true;
  }
  if (line && typeof line === "object" && (line as RealtimeLine).type === "continuity") {
    const event = line as RealtimeLine;
    const active = runtimeActiveForContinuityStatus(event.status);
    // A successor's active transition crosses the old run filter. A late
    // predecessor terminal transition must never clear that successor.
    if (active === true && typeof event.activeRequestId === "string") {
      if (retiredRunIds?.has(event.activeRequestId)) return false;
      if (isRealtimeReplay(event) && requestId && event.activeRequestId !== requestId) return false;
      return true;
    }
  }
  const runId = realtimeRunId(line);
  return !requestId || !runId || runId === requestId;
}

/**
 * Projects the authoritative session continuity state onto the client runtime.
 * Unknown statuses leave the existing runtime state unchanged.
 */
export function runtimeActiveForContinuityStatus(
  status: unknown,
): boolean | undefined {
  if (typeof status !== "string") return undefined;
  if (
    ["queued", "starting", "active", "running", "recovering", "compacting"].includes(
      status,
    )
  ) {
    return true;
  }
  if (
    ["idle", "ready", "cancelled", "error", "failed", "dead", "waiting_for_input", "blocked", "paused"].includes(status)
  ) {
    return false;
  }
  return undefined;
}

/** Replay rebuilds presentation state but must never dispatch new work. */
export function shouldRunRealtimeSideEffects(line: unknown): boolean {
  return !isRealtimeReplay(line);
}
