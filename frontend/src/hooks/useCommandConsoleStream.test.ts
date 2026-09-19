import { describe, expect, it } from "vitest";
import type { CommandEvent } from "../lib/consoleTypes";
import {
  appendCommandEvent,
  appendCommandEvents,
  commandConsoleStreamPath,
  parseCommandEventLine,
} from "./useCommandConsoleStream";

function event(eventId: string, sequence: number): CommandEvent {
  return {
    event_id: eventId,
    kind: "output_chunk",
    session_id: "session-1",
    run_id: "run-1",
    block_id: "block-1",
    sequence,
    timestamp: "2026-06-21T00:00:00Z",
  };
}

describe("command console stream helpers", () => {
  it("builds replay paths for sessions and runners", () => {
    expect(commandConsoleStreamPath({ sessionId: "session/one" })).toBe(
      "/api/console/sessions/session%2Fone/stream",
    );
    expect(commandConsoleStreamPath({ runnerId: "runner one" }, 42)).toBe(
      "/api/console/runners/runner%20one/stream?afterSequence=42",
    );
    expect(commandConsoleStreamPath({ sessionId: 'one' }, 7, 'runtime-two')).toBe('/api/console/sessions/one/stream?afterSequence=7&epoch=runtime-two');
  });

  it("parses command event NDJSON lines", () => {
    const parsed = parseCommandEventLine(JSON.stringify({ type: "command_event", event: event("evt-1", 7) }));

    expect(parsed?.event_id).toBe("evt-1");
    expect(parsed?.sequence).toBe(7);
    expect(parseCommandEventLine("  ")).toBeNull();
    expect(parseCommandEventLine(JSON.stringify({ type: "status", event: event("evt-2", 8) }))).toBeNull();
  });

  it("dedupes, sorts, and caps replayed events", () => {
    const first = event("evt-1", 20);
    const second = event("evt-2", 10);
    const third = event("evt-3", 30);

    let events: CommandEvent[] = [];
    events = appendCommandEvent(events, first, 2);
    events = appendCommandEvent(events, second, 2);
    events = appendCommandEvent(events, first, 2);
    events = appendCommandEvent(events, third, 2);

    expect(events.map((item) => item.event_id)).toEqual(["evt-1", "evt-3"]);
    expect(events.map((item) => item.sequence)).toEqual([20, 30]);
  });

  it("dedupes, sorts, and caps batched events", () => {
    const events = appendCommandEvents(
      [event("evt-1", 20)],
      [event("evt-2", 10), event("evt-1", 20), event("evt-3", 30)],
      2,
    );

    expect(events.map((item) => item.event_id)).toEqual(["evt-1", "evt-3"]);
    expect(events.map((item) => item.sequence)).toEqual([20, 30]);
  });
});
