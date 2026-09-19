import { describe, expect, it } from "vitest";
import {
  REALTIME_REPLAY_FIELD,
  REALTIME_RUN_ID_FIELD,
  REALTIME_SESSION_ID_FIELD,
  annotateRealtimeReplay,
  isRealtimeReplay,
  realtimeRunId,
  runtimeActiveForContinuityStatus,
  shouldApplyRealtimeEventToRun,
  shouldApplyRealtimeEventToSession,
  shouldRunRealtimeSideEffects,
} from "./realtimeReplay";

describe("realtime replay metadata", () => {
  it("marks replayed events without mutating journal payloads", () => {
    const payload = { type: "done", queuePending: true };

    const replayed = annotateRealtimeReplay(payload, true, "run-1", "session-1");

    expect(replayed).not.toBe(payload);
    expect(replayed[REALTIME_REPLAY_FIELD]).toBe(true);
    expect(replayed[REALTIME_RUN_ID_FIELD]).toBe("run-1");
    expect(replayed[REALTIME_SESSION_ID_FIELD]).toBe("session-1");
    expect(payload).toEqual({ type: "done", queuePending: true });
    expect(isRealtimeReplay(replayed)).toBe(true);
    expect(realtimeRunId(replayed)).toBe("run-1");
  });

  it("never applies an event from another session binding", () => {
    const event = annotateRealtimeReplay(
      { type: "text", text: "other chat" },
      false,
      "run-1",
      "session-other",
    );

    expect(shouldApplyRealtimeEventToSession(event, "session-current")).toBe(false);
    expect(shouldApplyRealtimeEventToSession(event, "session-other")).toBe(true);
  });

  it("keeps a reprompt isolated from the interrupted run replay", () => {
    const interrupted = annotateRealtimeReplay(
      { type: "message", text: "old work" },
      true,
      "run-old",
    );
    const reprompt = annotateRealtimeReplay(
      { type: "message", text: "new work" },
      false,
      "run-new",
    );

    expect(shouldApplyRealtimeEventToRun(interrupted, "run-new")).toBe(false);
    expect(shouldApplyRealtimeEventToRun(reprompt, "run-new")).toBe(true);
    expect(
      shouldApplyRealtimeEventToRun({ type: "streaming", active: true }, "run-new"),
    ).toBe(true);
  });

  it("always applies session ownership events across a run handoff", () => {
    for (const type of ["queue_updated", "interrupt_requested"]) {
      const event = annotateRealtimeReplay({ type }, true, "run-old");
      expect(shouldApplyRealtimeEventToRun(event, "run-new")).toBe(true);
    }
  });

  it("keeps the queued successor active when the predecessor finishes late", () => {
    for (const replay of [false, true]) {
      for (const status of ["idle", "error", "cancelled", "waiting_for_input"]) {
        const stale = annotateRealtimeReplay({ type: "continuity", status }, replay, "old");
        expect(shouldApplyRealtimeEventToRun(stale, "new")).toBe(false);
      }
      expect(shouldApplyRealtimeEventToRun(annotateRealtimeReplay({ type: "done" }, replay, "old"), "new")).toBe(false);
    }
    const handoff = annotateRealtimeReplay({ type: "continuity", status: "queued", activeRequestId: "new" }, false, "old");
    expect(shouldApplyRealtimeEventToRun(handoff, "old")).toBe(true);
  });

  it("projects authoritative continuity states onto runtime activity", () => {
    for (const status of [
      "queued",
      "starting",
      "active",
      "running",
      "recovering",
      "compacting",
    ]) {
      expect(runtimeActiveForContinuityStatus(status)).toBe(true);
    }
    for (const status of [
      "idle",
      "ready",
      "cancelled",
      "error",
      "failed",
      "dead",
    ]) {
      expect(runtimeActiveForContinuityStatus(status)).toBe(false);
    }
    expect(runtimeActiveForContinuityStatus("future-status")).toBeUndefined();
    expect(runtimeActiveForContinuityStatus(undefined)).toBeUndefined();
  });

  it("allows live completion effects and suppresses replayed completion effects", () => {
    const live = annotateRealtimeReplay({ type: "done" }, false);
    const replayed = annotateRealtimeReplay({ type: "done" }, true);

    expect(shouldRunRealtimeSideEffects(live)).toBe(true);
    expect(shouldRunRealtimeSideEffects(replayed)).toBe(false);
  });
});
