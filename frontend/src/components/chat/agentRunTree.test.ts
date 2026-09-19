import { describe, expect, it } from "vitest";
import { buildNestedAgentRunTree } from "./agentRunTree";

describe("buildNestedAgentRunTree", () => {
  it("preserves event order and attaches children even when they arrive first", () => {
    const roots = buildNestedAgentRunTree([
      { type: "tool_use", run_id: "child", parent_run_id: "root" },
      { type: "nested_start", run_id: "root" },
      { type: "tool_result", run_id: "child", parent_run_id: "root" },
      { type: "ignored" },
      { type: "nested_start", run_id: "orphan", parent_run_id: "missing" },
    ]);

    expect(roots.map((node) => node.runId)).toEqual(["root", "orphan"]);
    expect(roots[0].children.map((node) => node.runId)).toEqual(["child"]);
    expect(roots[0].children[0].events.map((event) => event.type)).toEqual([
      "tool_use",
      "tool_result",
    ]);
  });
});
