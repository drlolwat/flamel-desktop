import { describe, expect, it } from "vitest";
import { processHistoryMessages } from "./historyProcessor";
import {
  legacySkillBatchText,
  skillBatchResult,
} from "../components/tool-widgets/skillBatch.fixture";
import { adaptEngineeringTool } from "../components/tool-widgets/engineeringToolAdapters";
import { normalizeToolPresentation } from "./messageProcessor";

describe("processHistoryMessages", () => {
  it("retains a delivered guidance request ID on its persisted user message", () => {
    const blocks = processHistoryMessages([{
      type: "user",
      message: {
        content: [{ type: "text", text: "finishhh" }],
        flamel: { requestId: "guidance-1" },
      },
    }]);
    expect(blocks[0]).toMatchObject({ role: "user", content: "finishhh", requestId: "guidance-1" });
  });

  it("restores embedded receipts without claiming success for unfinished verification", () => {
    const blocks = processHistoryMessages([{
      type: "assistant",
      message: { content: [
        { type: "tool_use", id: "verify", name: "shell", input: { action: "verify" } },
        { type: "tool_use", id: "failed", name: "shell", input: {} },
        { type: "tool_use", id: "active", name: "shell", input: {} },
        { type: "tool_use", id: "missing", name: "shell", input: {} },
        { type: "text", text: "Next check" },
        { type: "tool_result", tool_use_id: "verify", content: '{"status":"passed"}', is_delta: false },
        { type: "tool_result", tool_use_id: "failed", content: "exit 1", is_error: true },
        { type: "tool_result", tool_use_id: "active", content: "Verification is running", is_delta: true },
      ] },
    }]);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].content).toBe("Next check");
    expect(blocks[0].tools?.map(({ status }) => status)).toEqual(["done", "error", "unknown", "unknown"]);
    expect(blocks[0].tools?.[0].output).toBe('{"status":"passed"}');
    expect(blocks[0].segments?.filter(({ type }) => type === "tool_ref")).toHaveLength(4);
  });

  it("buffers receipts before envelopes and ignores stale deltas after a terminal result", () => {
    const blocks = processHistoryMessages([
      { type: "user", message: { content: [
        { type: "tool_result", tool_use_id: "late", content: "partial", is_delta: true },
        { type: "tool_result", tool_use_id: "late", content: "failed", is_error: true },
        { type: "tool_result", tool_use_id: "late", content: "stale", is_delta: true },
      ] } },
      { type: "assistant", message: { content: [
        { type: "tool_use", id: "late", name: "shell", input: {} },
      ] } },
    ]);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].tools?.[0]).toMatchObject({ status: "error", output: "failed" });
  });

  it("keeps an earlier receipt on a repeated tool envelope in legacy history", () => {
    const envelope = { type: "tool_use", id: "repeat", name: "shell", input: {} };
    const blocks = processHistoryMessages([
      { type: "assistant", message: { content: [envelope] } },
      { type: "tool_result", message: { content: [
        { type: "tool_result", tool_use_id: "repeat", content: "complete" },
      ] } },
      { type: "assistant", message: { content: [envelope, { type: "text", text: "Continuing" }] } },
    ]);
    expect(blocks[0].tools?.[0]).toMatchObject({ status: "done", output: "complete" });
    expect(blocks[1].tools?.[0]).toMatchObject({ status: "done", output: "complete" });
  });

  it("attaches a late receipt across guidance messages without adding a tool-result chat row", () => {
    const blocks = processHistoryMessages([
      { type: "assistant", message: { content: [
        { type: "tool_use", id: "slow", name: "shell", input: {} },
      ] } },
      { type: "user", message: { content: [{ type: "text", text: "First guidance" }] } },
      { type: "tool_result", message: { content: [
        { type: "tool_result", tool_use_id: "slow", content: "finished" },
      ] } },
      { type: "user", message: { content: [{ type: "text", text: "Second guidance" }] } },
    ]);
    expect(blocks.map(({ role }) => role)).toEqual(["assistant", "user", "user"]);
    expect(blocks[0].tools?.[0]).toMatchObject({ status: "done", output: "finished" });
  });

  it("deduplicates envelopes merged across hidden continuations and retains their latest input", () => {
    const blocks = processHistoryMessages([{
      type: "assistant", message: { content: [
        { type: "tool_use", id: "same", name: "file_read", input: {} },
        { type: "tool_use", id: "same", name: "file_read", input: { file: "src/app.ts" } },
        { type: "tool_result", tool_use_id: "same", content: "source", is_error: false },
      ] },
    }]);
    expect(blocks[0].tools).toHaveLength(1);
    expect(blocks[0].tools?.[0]).toMatchObject({ status: "done", output: "source", input: { file: "src/app.ts" } });
    expect(blocks[0].segments).toEqual([{ type: "tool_ref", toolId: "same" }]);
  });

  it("does not claim success when history has no tool result", () => {
    const blocks = processHistoryMessages([{
      type: "assistant",
      message: { content: [{ type: "tool_use", id: "unfinished", name: "shell", input: {} }] },
    }]);
    expect(blocks[0].tools?.[0].status).toBe("unknown");
  });
  it("rehydrates a durable completed compaction checkpoint in order", () => {
    const blocks = processHistoryMessages([
      {
        type: "user",
        message: { content: [{ type: "text", text: "Keep implementing" }] },
      },
      {
        type: "compaction",
        uuid: "compact-run-1",
        message: {
          runId: "compact-run-1",
          phase: "complete",
          label: "Compaction complete",
          summary: "Current session: implementation is active.",
          preTokens: 229_000,
          summarizedTurns: 15,
        },
      },
      {
        type: "assistant",
        message: {
          content: [{ type: "text", text: "Continuing now" }],
        },
      },
    ]);

    expect(blocks).toHaveLength(3);
    expect(blocks[1]).toMatchObject({
      id: "compaction:compact-run-1",
      role: "system",
      type: "compact",
      isStreaming: false,
      preTokens: 229_000,
    });
    expect(blocks[1].compaction?.summary).toContain("implementation is active");
    expect(blocks[2].content).toBe("Continuing now");
  });

  it("rehydrates the same structured widget data as a live result", () => {
    const content = {
      content: [
        { type: "text", text: "Found 2 files" },
        { type: "resource_link", uri: "file:///repo/src/app.ts", title: "app.ts" },
      ],
    };
    const envelope = {
      version: 1,
      text: "Found 2 files",
      structuredContent: { files: ["src/app.ts", "src/main.ts"] },
    };
    const artifacts = [
      {
        id: "artifact-1",
        kind: "file",
        name: "results.json",
        mediaType: "application/json",
        byteSize: 120,
        sha256: "abc",
        contentUrl: "/api/artifacts/artifact-1/content",
      },
    ];
    const blocks = processHistoryMessages([
      {
        type: "assistant",
        uuid: "assistant-1",
        message: {
          content: [
            {
              type: "tool_use",
              id: "tool-1",
              name: "file_read",
              input: { file: "src/app.ts" },
            },
          ],
        },
      },
      {
        type: "user",
        message: {
          content: [
            {
              type: "tool_result",
              tool_use_id: "tool-1",
              content,
              presentation: envelope,
              artifacts,
              is_error: false,
            },
          ],
        },
      },
    ]);

    const expected = normalizeToolPresentation({
      content,
      envelope,
      fallbackText:
        "Found 2 files" +
        JSON.stringify({
          type: "resource_link",
          uri: "file:///repo/src/app.ts",
          title: "app.ts",
        }),
      artifacts,
      isError: false,
    });
    expect(blocks[0].tools?.[0].presentation).toEqual(expected);
    expect(blocks[0].tools?.[0].artifacts).toEqual(expected.artifacts);
  });

  it("restores typed and string-only file batches as the same source viewer", () => {
    const history = (content: unknown, id: string) =>
      processHistoryMessages([
        {
          type: "assistant",
          message: {
            content: [
              {
                type: "tool_use",
                id,
                name: "file_read",
                input: { action: "batch", files: [] },
              },
            ],
          },
        },
        {
          type: "user",
          message: {
            content: [{ type: "tool_result", tool_use_id: id, content }],
          },
        },
      ])[0].tools?.[0];

    const typed = history(
      {
        content: [{ type: "text", text: legacySkillBatchText }],
        structuredContent: {
          ...skillBatchResult,
          text: legacySkillBatchText,
        },
      },
      "typed-batch",
    );
    const legacy = history(legacySkillBatchText, "legacy-batch");

    expect(typed?.presentation?.structuredContent).toMatchObject({
      schema: "flamel.engineering_result.v1",
      action: "batch",
      counts: { files: 6 },
    });
    expect(adaptEngineeringTool(typed!)?.structuredResult?.counts.files).toBe(
      6,
    );
    expect(adaptEngineeringTool(legacy!)?.structuredResult?.counts.files).toBe(
      6,
    );
  });

  it("rehydrates persisted legacy budget envelopes as clipped previews", () => {
    const legacyEnvelope = JSON.stringify({
      status: "ok",
      summary: "Output exceeded the 12288 byte inline budget",
      data: { preview: "const historyPreview = true;\n" },
      artifacts: [
        {
          path: "/tmp/flamel-mcp-artifacts/file_read-history.txt",
          media_type: "text/plain",
          bytes: 17_000,
        },
      ],
      warnings: [],
      next_actions: ["Read the artifact only if the preview is insufficient"],
    });
    const blocks = processHistoryMessages([
      {
        type: "assistant",
        message: {
          content: [
            {
              type: "tool_use",
              id: "legacy-budget",
              name: "file_read",
              input: { action: "read", file: "src/history.ts" },
            },
          ],
        },
      },
      {
        type: "user",
        message: {
          content: [
            {
              type: "tool_result",
              tool_use_id: "legacy-budget",
              content: legacyEnvelope,
            },
          ],
        },
      },
    ]);
    const restored = blocks[0].tools?.[0];

    expect(restored?.output).toBe("const historyPreview = true;\n");
    expect(restored?.presentation).toMatchObject({
      text: "const historyPreview = true;\n",
      summary: "Clipped tool result",
      truncated: true,
      legacy: true,
    });
    expect(restored?.artifacts?.[0].contentUrl).toBe(
      "/api/mcp-artifacts/file_read-history.txt",
    );
  });

  it("keeps old string-only history compatible", () => {
    const blocks = processHistoryMessages([
      {
        type: "assistant",
        message: {
          content: [
            { type: "tool_use", id: "legacy-1", name: "shell", input: {} },
          ],
        },
      },
      {
        type: "user",
        message: {
          content: [
            {
              type: "tool_result",
              tool_use_id: "legacy-1",
              content: "legacy output",
              is_error: true,
            },
          ],
        },
      },
    ]);

    expect(blocks[0].tools?.[0]).toMatchObject({
      output: "legacy output",
      status: "error",
      presentation: {
        text: "legacy output",
        legacy: true,
        isError: true,
      },
    });
  });
});
