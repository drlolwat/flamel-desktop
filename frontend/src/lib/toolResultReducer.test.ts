import { describe, expect, it } from "vitest";
import type { ToolCall } from "./messageProcessor";
import {
  legacySkillBatchText,
  skillBatchResult,
} from "../components/tool-widgets/skillBatch.fixture";
import { applyToolResult, reduceToolResult } from "./toolResultReducer";

function tool(overrides: Partial<ToolCall> = {}): ToolCall {
  return {
    id: "tool-1",
    name: "mcp__flamel__file_edit",
    input: { action: "replace", file: "src/app.ts" },
    partialInput: '{"action":"replace"',
    status: "running",
    commandEvents: [],
    ...overrides,
  };
}

const artifact = {
  id: "artifact-1",
  kind: "file" as const,
  name: "result.json",
  mediaType: "application/json",
  byteSize: 12,
  sha256: "abc",
  contentUrl: "/api/artifacts/artifact-1/content",
};

describe("tool result reducer", () => {
  it("accepts the authoritative result after a connection loss", () => {
    const reduced = reduceToolResult(tool({ status: "unknown", output: "partial" }), {
      tool_use_id: "tool-1", content: "permission denied", is_error: true,
    });
    expect(reduced.tool).toMatchObject({ status: "error", output: "permission denied" });
  });
  it("preserves partial input and artifacts while deltas settle in place", () => {
    const first = reduceToolResult(tool(), {
      tool_use_id: "tool-1",
      content: "one\n",
      artifacts: [artifact],
      is_delta: true,
    }).tool;
    const second = reduceToolResult(first, {
      tool_use_id: "tool-1",
      content: "two\n",
      is_delta: true,
    }).tool;
    const final = reduceToolResult(second, {
      tool_use_id: "tool-1",
      content: "final\n",
    }).tool;

    expect(second.output).toBe("one\ntwo\n");
    expect(final).toMatchObject({
      output: "final\n",
      status: "done",
      partialInput: '{"action":"replace"',
      artifacts: [artifact],
    });
  });

  it("retains structured presentation when a later snapshot only carries text", () => {
    const structured = reduceToolResult(tool(), {
      tool_use_id: "tool-1",
      content: { content: [{ type: "text", text: "Found rows" }] },
      presentation: {
        version: 1,
        text: "Found rows",
        structuredContent: { rows: [{ id: 1 }] },
      },
      is_delta: true,
    }).tool;
    const final = reduceToolResult(structured, {
      tool_use_id: "tool-1",
      content: "Found rows",
    }).tool;

    expect(final.presentation?.structuredContent).toEqual({
      rows: [{ id: 1 }],
    });
    expect(final.presentation?.legacy).toBe(false);
  });

  it("keeps the file batch protocol through streamed and final snapshots", () => {
    const running = reduceToolResult(
      tool({
        name: "mcp__flamel__file_read",
        input: { action: "batch", files: [] },
      }),
      {
        tool_use_id: "tool-1",
        content: {
          content: [{ type: "text", text: legacySkillBatchText }],
          structuredContent: {
            ...skillBatchResult,
            text: legacySkillBatchText,
          },
        },
        is_delta: true,
      },
    ).tool;
    const final = reduceToolResult(running, {
      tool_use_id: "tool-1",
      content: legacySkillBatchText,
    }).tool;

    expect(running.presentation?.structuredContent).toMatchObject({
      schema: "flamel.engineering_result.v1",
      tool: "file_read",
      action: "batch",
      counts: { files: 6 },
    });
    expect(final.presentation?.structuredContent).toEqual(
      running.presentation?.structuredContent,
    );
    expect(final.output).toBe(legacySkillBatchText);
  });

  it("stores a semantic preview instead of the bounded transport envelope", () => {
    const envelope = JSON.stringify({
      schema: "flamel.oversized_result.v1",
      status: "ok",
      summary: "The tool completed; the full output is available as an artifact.",
      data: {
        preview: "export const reduced = true;\n",
        truncated: true,
        original: { media_type: "text/plain", bytes: 12_289 },
      },
      artifacts: [
        {
          path: "/tmp/flamel-mcp-artifacts/file_read-reducer.txt",
          media_type: "text/plain",
          bytes: 12_289,
        },
      ],
      diagnostics: {
        reason: "inline_budget_exceeded",
        inline_budget_bytes: 12_288,
      },
    });
    const reduced = reduceToolResult(
      tool({
        name: "file_read",
        input: { action: "read", file: "src/reduced.ts" },
      }),
      {
        tool_use_id: "tool-1",
        content: { content: [{ type: "text", text: envelope }] },
      },
    ).tool;

    expect(reduced.output).toBe("export const reduced = true;\n");
    expect(reduced.output).not.toContain("inline_budget");
    expect(reduced.presentation).toMatchObject({
      truncated: true,
      legacy: false,
      transport: {
        reason: "inline_budget_exceeded",
        inlineBudgetBytes: 12_288,
      },
    });
    expect(reduced.artifacts?.[0].contentUrl).toBe(
      "/api/mcp-artifacts/file_read-reducer.txt",
    );
  });

  it("extracts agent correlation and preserves nested events", () => {
    const nestedEvents = [{ type: "nested_done", run_id: "run-1" }];
    const reduced = reduceToolResult(
      tool({
        name: "mcp__flamel__Agent",
        nestedEvents,
      }),
      {
        tool_use_id: "tool-1",
        content: "AGENT_RUN:run-1\nFinished",
      },
    ).tool;

    expect(reduced.runId).toBe("run-1");
    expect(reduced.output).toBe("Finished");
    expect(reduced.nestedEvents).toBe(nestedEvents);
  });

  it("returns the original object and array for a repeated terminal snapshot", () => {
    const initial = reduceToolResult(tool(), {
      tool_use_id: "tool-1",
      content: "complete",
    }).tool;
    const tools = [initial];
    const repeated = applyToolResult(tools, {
      tool_use_id: "tool-1",
      content: "complete",
    });

    expect(repeated?.changed).toBe(false);
    expect(repeated?.tool).toBe(initial);
    expect(repeated?.tools).toBe(tools);
  });

  it("never downgrades terminal results when stale deltas or replay snapshots arrive", () => {
    const done = reduceToolResult(tool(), {
      tool_use_id: "tool-1",
      content: "complete",
    }).tool;
    const lateDelta = reduceToolResult(done, {
      tool_use_id: "tool-1",
      content: "stale partial",
      is_delta: true,
    });
    expect(lateDelta.tool).toBe(done);
    expect(lateDelta.tool).toMatchObject({
      status: "done",
      output: "complete",
    });

    const failed = tool({
      status: "error",
      output: "permission denied",
      presentation: {
        version: 1,
        text: "permission denied",
        summary: "permission denied",
        resources: [],
        media: [],
        artifacts: [],
        isError: true,
        truncated: false,
        legacy: false,
      },
    });
    const replay = reduceToolResult(failed, {
      tool_use_id: "tool-1",
      content: "older successful snapshot",
    });
    expect(replay.tool).toBe(failed);
    expect(replay.tool).toMatchObject({
      status: "error",
      output: "permission denied",
    });
  });

  it("correlates by id without changing order or unrelated object identity", () => {
    const first = tool({ id: "first" });
    const target = tool({ id: "target" });
    const last = tool({ id: "last" });
    const tools = [first, target, last];
    const applied = applyToolResult(tools, {
      tool_use_id: "target",
      content: "done",
    });

    expect(applied?.tools.map((candidate) => candidate.id)).toEqual([
      "first",
      "target",
      "last",
    ]);
    expect(applied?.tools[0]).toBe(first);
    expect(applied?.tools[2]).toBe(last);
    expect(
      applyToolResult(tools, {
        tool_use_id: "missing",
        content: "ignored",
      }),
    ).toBeUndefined();
  });

  it("settles errors and computes unified diff statistics", () => {
    const reduced = reduceToolResult(tool(), {
      tool_use_id: "tool-1",
      content: "--- a/src/app.ts\n+++ b/src/app.ts\n-old\n+new\n+more",
      is_error: true,
    }).tool;

    expect(reduced.status).toBe("error");
    expect(reduced.diffStat).toEqual({ added: 2, removed: 1 });
  });
});
