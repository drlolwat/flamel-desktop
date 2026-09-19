import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { DisplayBlock, ToolCall } from "../../lib/messageProcessor";
import ClaudeBlock from "./ClaudeBlock";
import { AgentRow } from "./AgentToolRenderer";
import { LegacyToolRow } from "./LegacyToolRenderer";

function renderBlock(block: DisplayBlock) {
  return renderToStaticMarkup(<ClaudeBlock block={block} />);
}

describe("ClaudeBlock renderer boundaries", () => {
  it("attributes the response before its first streaming text arrives", () => {
    const html = renderBlock({
      id: "starting", role: "assistant", type: "assistant",
      model: "gpt-6-astra", isStreaming: true,
    });
    expect(html).toContain("chat-assistant-header");
    expect(html).toContain("Codex");
    expect(html).toContain("gpt-6-astra");
    expect(html.indexOf("Codex")).toBeLessThan(html.indexOf("Working…"));
  });

  it("does not guess a provider before model metadata arrives", () => {
    const html = renderBlock({
      id: "starting", role: "assistant", type: "assistant", isStreaming: true,
    });
    expect(html).toContain(">Assistant</span>");
    expect(html).not.toContain(">Claude</span>");
  });

  it("preserves the model version while removing a dated Claude suffix", () => {
    const html = renderBlock({
      id: "claude", role: "assistant", type: "assistant",
      model: "claude-sonnet-4-6-20260217", content: "Ready.",
    });
    expect(html).toContain(">Claude</span>");
    expect(html).toContain(">sonnet-4-6</span>");
  });

  it("keeps segment-only commentary visible with its attribution", () => {
    const html = renderBlock({
      id: "segment", role: "assistant", type: "assistant", model: "gpt-6-astra",
      segments: [{ type: "text", text: "I’ll check the release." }],
    });
    expect(html).toContain("chat-assistant-header");
    expect(html).toContain("I’ll check the release.");
  });

  it("uses the compact expandable card shell for completed reads without mounting hidden children", () => {
    const tools: ToolCall[] = ["a", "b", "c"].map(id => ({
      id, name: "file_read", input: { file: `src/${id}.rs` }, status: "done",
    }));
    const html = renderBlock({
      id: "reads", role: "assistant", type: "assistant", model: "gpt-6-astra", tools,
      segments: tools.map(tool => ({ type: "tool_ref", toolId: tool.id })),
    });
    expect(html).toContain('tool-widget--success chat-activity-group');
    expect(html).toContain("Reads and searches");
    expect(html).toContain("3 completed");
    expect(html).toContain('aria-expanded="false"');
    expect(html).toContain('aria-controls="tool-widget-');
    expect(html).toContain(">Done</span>");
    expect(html).not.toContain('class="tool-widget__body"');
    expect(html).not.toContain("src/a.rs");
  });

  it("preserves authored segment order and the selected provider label", () => {
    const html = renderBlock({
      id: "authored",
      role: "assistant",
      type: "assistant",
      model: "gpt-5.6-codex",
      content: "The extraction is complete.",
      segments: [
        { type: "thinking", text: "Trace the shared rendering path." },
        { type: "text", text: "The extraction is complete." },
      ],
    });

    expect(html).toContain("Codex");
    expect(html).toContain("Reasoning");
    expect(html).toContain("Trace the shared rendering path.");
    expect(html).toContain("The extraction is complete.");
    expect(html.indexOf("Reasoning")).toBeLessThan(
      html.indexOf("The extraction is complete."),
    );
  });

  it("renders provider-native compaction checkpoints without a synthetic summary", () => {
    const html = renderBlock({
      id: "checkpoint",
      role: "assistant",
      type: "compact",
      compaction: { providerNative: true, preTokens: 128_000 },
    });

    expect(html).toContain("Context checkpoint");
    expect(html).toContain("Provider checkpoint");
  });

  it("keeps agent and unknown-tool fallback labels intact", () => {
    const agent: ToolCall = {
      id: "agent",
      name: "mcp__flamel__Agent",
      status: "done",
      input: {
        subagent_type: "frontend-developer",
        description: "Check narrow message layout",
      },
    };
    const unknown: ToolCall = {
      id: "unknown",
      name: "mcp__connected_provider__fetch_metrics",
      status: "done",
      input: { query: "latency" },
      output: "p95 42ms",
    };

    const agentHtml = renderToStaticMarkup(<AgentRow tool={agent} />);
    const fallbackHtml = renderToStaticMarkup(<LegacyToolRow tool={unknown} />);

    expect(agentHtml).toContain("frontend developer");
    expect(agentHtml).toContain("Check narrow message layout");
    expect(fallbackHtml).toContain("Connected Provider");
    expect(fallbackHtml).toContain("fetch metrics");
  });
});
