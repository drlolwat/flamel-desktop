import { describe, expect, it } from "vitest";
import {
  applyCompactionStreamLine,
  initialCompactionStreamState,
  type CompactionStreamState,
} from "./sessionCompaction";
import type { WorkflowEvent } from "./workflowEvents";

function apply(line: unknown, state: CompactionStreamState, events: WorkflowEvent[]) {
  return applyCompactionStreamLine(JSON.stringify(line), state, (event) => events.push(event));
}

describe("compaction stream processing", () => {
  it("emits normalized compaction events and tracks completion", () => {
    const events: WorkflowEvent[] = [];
    let state = initialCompactionStreamState();

    state = apply(
      { type: "system_status", status: "compacting", label: "Summarizing tools", summaryDelta: "A" },
      state,
      events,
    );
    state = apply(
      { type: "workflow_event", kind: "system.compact_complete", source: "backend", payload: { runId: "run-1" } },
      state,
      events,
    );

    expect(events.map((event) => event.kind)).toEqual(["system.compacting", "system.compact_complete"]);
    expect(events[0].payload.label).toBe("Summarizing tools");
    expect(state).toEqual({ completed: true, failed: false });
  });

  it("keeps failure sticky even if a later complete event arrives", () => {
    const events: WorkflowEvent[] = [];
    let state = initialCompactionStreamState();

    state = apply({ type: "system_status", status: "compact_failed" }, state, events);
    state = apply({ type: "system_status", status: "compact_complete" }, state, events);

    expect(events.map((event) => event.kind)).toEqual(["system.compact_failed", "system.compact_complete"]);
    expect(state).toEqual({ completed: true, failed: true });
  });

  it("ignores empty lines and non-workflow payloads", () => {
    const events: WorkflowEvent[] = [];
    let state = initialCompactionStreamState();

    state = applyCompactionStreamLine("  ", state, (event) => events.push(event));
    state = apply({ type: "unknown" }, state, events);

    expect(events).toHaveLength(0);
    expect(state).toEqual({ completed: false, failed: false });
  });
});
