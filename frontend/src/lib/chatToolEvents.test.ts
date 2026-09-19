import { describe, expect, it, vi } from "vitest";
import type { ToolCall } from "./messageProcessor";
import type { CommandEvent } from "./consoleTypes";
import type { ToolResultEventLike } from "./toolResultReducer";
import { applyStreamingToolResult, attachCommandEventToTool, shouldStartAssistantBlockAfterToolResult, toolFromAssistantSnapshot } from "./chatToolEvents";

const tool = (id: string): ToolCall => ({ id, name: "shell", input: {}, status: "running" });

describe("shared chat tool events", () => {
  it("keeps finished receipts when Codex repeats tool IDs in a cumulative snapshot", () => {
    const current = [tool("verify-passed"), tool("shell-failed"), tool("still-running")];
    const apply = (result: ToolResultEventLike) => applyStreamingToolResult(current, result,
      vi.fn(), vi.fn(), (_id, events) => events, () => undefined, new Map());
    apply({ tool_use_id: "verify-passed", content: '{"status":"passed"}' });
    apply({ tool_use_id: "shell-failed", content: "exit 1", is_error: true });
    apply({ tool_use_id: "still-running", content: "Verification is running", is_delta: true });

    // Production sequence: final receipt, then 28 -> 29 cumulative tool uses.
    const snapshot = [
      { type: "text", text: "Next check" },
      ...current.map(({ id }) => ({ type: "tool_use", id })),
      { type: "tool_use", id: "next" },
    ];
    expect(shouldStartAssistantBlockAfterToolResult(true, snapshot, current)).toBe(false);
    expect(current.map(({ status }) => status)).toEqual(["done", "error", "running"]);
    expect(current[0].output).toBe('{"status":"passed"}');
  });

  it("still separates disjoint assistant turns after a receipt", () => {
    const previous = [tool("old")];
    expect(shouldStartAssistantBlockAfterToolResult(true,
      [{ type: "tool_use", id: "new" }], previous)).toBe(true);
    expect(shouldStartAssistantBlockAfterToolResult(true,
      [{ type: "text" }], previous)).toBe(true);
    expect(shouldStartAssistantBlockAfterToolResult(false,
      [{ type: "tool_use", id: "new" }], previous)).toBe(false);
  });

  it("does not mistake arbitrary matching text or a missing ID for a cumulative tool", () => {
    expect(shouldStartAssistantBlockAfterToolResult(true,
      [null, { type: "text", id: "old" }, { type: "tool_use" }], [tool("old")])).toBe(true);
    expect(shouldStartAssistantBlockAfterToolResult(true, "malformed", [tool("old")])).toBe(true);
    expect(shouldStartAssistantBlockAfterToolResult(true, null, [tool("old")])).toBe(true);
  });

  it("retains a previous block's receipt if guidance split the cumulative snapshot", () => {
    const envelope = { id: "verify", name: "shell", input: { action: "verify" } };
    const previous = { ...tool("verify"), status: "done" as const, output: "passed" };
    expect(toolFromAssistantSnapshot(envelope, previous)).toMatchObject({
      ...envelope, status: "done", output: "passed",
    });
    expect(toolFromAssistantSnapshot(envelope)).toMatchObject({ status: "running" });
    expect(toolFromAssistantSnapshot(envelope, { ...previous, status: "error" })).toMatchObject({ status: "error" });
  });

  it("settles an earlier block without replacing the current tools", () => {
    const current = [tool("new")];
    const previous = { ...tool("old"), status: "unknown" as const };
    const update = vi.fn();
    applyStreamingToolResult(current, { tool_use_id: "old", content: "failed", is_error: true },
      update, vi.fn(), (_id, events) => events, () => previous, new Map());
    expect(current).toEqual([tool("new")]);
    expect(update).toHaveBeenCalledWith("old", expect.objectContaining({ status: "error", output: "failed" }));
  });

  it("retains a result arriving before its tool and applies it exactly once", () => {
    const pending = new Map<string, ToolResultEventLike>();
    const current: ToolCall[] = [];
    const apply = (result: ToolResultEventLike) => applyStreamingToolResult(current, result,
      vi.fn(), vi.fn(), (_id, events) => events, () => undefined, pending);
    apply({ tool_use_id: "late", content: "a", is_delta: true });
    apply({ tool_use_id: "late", content: "b", is_delta: true });
    expect(pending.get("late")?.content).toBe("ab");
    apply({ tool_use_id: "late", content: "complete" });
    apply({ tool_use_id: "late", content: "stale", is_delta: true });
    current.push(tool("late"));
    apply(pending.get("late")!);
    expect(current[0]).toMatchObject({ status: "done", output: "complete" });
    expect(pending.size).toBe(0);
  });

  it("does not attach an explicitly named command event to another active tool", () => {
    const event = { event_id: "event-1", kind: "session_started", session: {
      origin: { kind: "pty", tool_call_id: "missing" },
    } } as CommandEvent;
    const pending: CommandEvent[] = [];
    const update = vi.fn();
    attachCommandEventToTool(event, [tool("other")], pending, update);
    expect(update).not.toHaveBeenCalled();
    expect(pending).toEqual([event]);
  });
});
