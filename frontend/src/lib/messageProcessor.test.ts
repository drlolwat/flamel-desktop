import { describe, expect, it } from "vitest";
import {
  appendThinkingSegment,
  mergeToolOutput,
  normalizeToolPresentation,
  textFromToolResultContent,
  type ContentSegment,
} from "./messageProcessor";

describe("streamed tool output", () => {
  it("appends deltas and replaces with the final aggregate", () => {
    const first = mergeToolOutput(undefined, "line one\n", true);
    const second = mergeToolOutput(first, "line two\n", true);
    expect(second).toBe("line one\nline two\n");
    expect(mergeToolOutput(second, "final aggregate\n", false)).toBe(
      "final aggregate\n",
    );
    expect(mergeToolOutput(second, "", false)).toBe(second);
  });
});

describe("reasoning segments", () => {
  it("coalesces adjacent visible phases into one transcript block", () => {
    const segments: ContentSegment[] = [];
    appendThinkingSegment(segments, "Checking containers", false);
    appendThinkingSegment(segments, "Checking listeners", false);
    expect(segments).toEqual([
      {
        type: "thinking",
        text: "Checking containers\n\nChecking listeners",
        redacted: false,
      },
    ]);
  });

  it("ignores an identical adjacent reasoning snapshot", () => {
    const segments: ContentSegment[] = [];
    appendThinkingSegment(segments, "Preparing the release", false);
    appendThinkingSegment(segments, "Preparing the release", false);
    expect(segments).toEqual([
      {
        type: "thinking",
        text: "Preparing the release",
        redacted: false,
      },
    ]);
  });
});

describe("tool result presentation", () => {
  it("keeps legacy text and error state without inventing structured data", () => {
    const presentation = normalizeToolPresentation({
      content: "command failed",
      isError: true,
    });

    expect(presentation).toMatchObject({
      text: "command failed",
      summary: "command failed",
      isError: true,
      legacy: true,
      truncated: false,
    });
    expect(presentation.structuredContent).toBeUndefined();
  });

  it("retains safe structured metadata without retaining inline binary payloads", () => {
    const secret = "sensitive-base64-payload";
    const content = {
      content: [
        { type: "text", text: "Rendered preview" },
        { type: "image", mimeType: "image/webp", data: secret },
        {
          type: "resource_link",
          uri: "artifact://render/preview.webp",
          title: "Preview",
        },
      ],
    };
    const presentation = normalizeToolPresentation({
      content,
      envelope: {
        version: 1,
        text: "Rendered preview",
        structuredContent: {
          dimensions: { width: 1280, height: 720 },
          data: secret,
        },
        resources: [
          {
            uri: "artifact://render/preview.webp",
            title: "Preview",
            mediaType: "image/webp",
          },
          { uri: "javascript:alert(1)", title: "Unsafe" },
        ],
        media: [
          {
            kind: "image",
            mediaType: "image/webp",
            resourceUri: "artifact://render/preview.webp",
          },
        ],
      },
      artifacts: [
        {
          id: "artifact-1",
          kind: "image",
          name: "preview.webp",
          mediaType: "image/webp",
          byteSize: 2048,
          sha256: "abc123",
          contentUrl: "/api/artifacts/artifact-1/content",
        },
      ],
    });

    expect(textFromToolResultContent(content)).not.toContain(secret);
    expect(JSON.stringify(presentation)).not.toContain(secret);
    expect(presentation.structuredContent).toMatchObject({
      data: "[binary payload omitted]",
    });
    expect(presentation.resources).toEqual([
      {
        uri: "artifact://render/preview.webp",
        title: "Preview",
        mediaType: "image/webp",
      },
    ]);
    expect(presentation.media).toHaveLength(1);
    expect(presentation.artifacts).toHaveLength(1);
    expect(presentation.legacy).toBe(false);
    expect(presentation.truncated).toBe(true);
  });

  it("decodes current and legacy oversized envelopes without presenting transport notices as content", () => {
    const currentEnvelope = {
      schema: "flamel.oversized_result.v1",
      status: "ok",
      summary: "The tool completed; the full output is available as an artifact.",
      data: {
        preview: "export const ready = true;\n",
        truncated: true,
        original: { media_type: "text/plain", bytes: 12_289 },
      },
      artifacts: [
        {
          path: "/tmp/flamel-mcp-artifacts/file_read-current.txt",
          media_type: "text/plain",
          bytes: 12_289,
        },
      ],
      diagnostics: {
        reason: "inline_budget_exceeded",
        inline_budget_bytes: 12_288,
      },
    };
    const current = normalizeToolPresentation({
      content: {
        content: [{ type: "text", text: JSON.stringify(currentEnvelope) }],
      },
    });

    expect(current).toMatchObject({
      text: "export const ready = true;\n",
      summary: "The tool completed; the full output is available as an artifact.",
      truncated: true,
      legacy: false,
    });
    expect(current.text).not.toContain("Output exceeded");
    expect(current.artifacts[0]).toMatchObject({
      kind: "file",
      name: "file_read-current.txt",
      mediaType: "text/plain",
      byteSize: 12_289,
      contentUrl: "/api/mcp-artifacts/file_read-current.txt",
      downloadUrl: "/api/mcp-artifacts/file_read-current.txt?download=1",
    });

    const legacy = normalizeToolPresentation({
      content: JSON.stringify({
        status: "ok",
        summary: "Output exceeded the 12288 byte inline budget",
        data: { preview: "const persisted = true;\n" },
        artifacts: [
          {
            path: "/tmp/flamel-mcp-artifacts/file_read-legacy.txt",
            media_type: "text/plain",
            bytes: 14_000,
          },
        ],
        warnings: [],
        next_actions: ["Read the artifact only if the preview is insufficient"],
      }),
    });
    expect(legacy).toMatchObject({
      text: "const persisted = true;\n",
      summary: "Clipped tool result",
      truncated: true,
      legacy: true,
    });
    expect(legacy.summary).not.toContain("12288");
  });

  it("bounds summaries and structured collections", () => {
    const presentation = normalizeToolPresentation({
      content: "",
      fallbackText: "x".repeat(500),
      envelope: {
        version: 1,
        structuredContent: Array.from({ length: 80 }, (_, index) => ({ index })),
      },
    });

    expect(presentation.summary).toHaveLength(240);
    expect(presentation.summary.endsWith("…")).toBe(true);
    expect(presentation.structuredContent).toHaveLength(64);
    expect(presentation.truncated).toBe(true);
  });
});
