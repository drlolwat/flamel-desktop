import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ToolCall } from "../../lib/messageProcessor";
import {
  isToolHidden,
  toolRenderKind,
  ToolWidgetRenderer,
} from "./ToolWidgetRenderer";

function tool(name: string, input: Record<string, unknown> = {}): ToolCall {
  return {
    id: name,
    name,
    input,
    status: "done",
    output: "{}",
  };
}

describe("ToolWidgetRenderer registry", () => {
  it("preserves hidden, question, thinking, plan, agent, and viz semantics", () => {
    expect(isToolHidden(tool("plan", { action: "get" }))).toBe(true);
    expect(toolRenderKind(tool("AskUserQuestion"))).toBe("question");
    expect(toolRenderKind(tool("mcp__flamel__thinking"))).toBe("thinking");
    expect(toolRenderKind(tool("plan", { action: "create" }))).toBe("workflow");
    expect(toolRenderKind(tool("mcp__flamel__AgentBatch"))).toBe("workflow");
    expect(toolRenderKind(tool("mcp__viz__chart"))).toBe("viz");
  });

  it("routes every authored family before the adaptive fallback", () => {
    expect(toolRenderKind(tool("mcp__flamel__file_read"))).toBe("engineering");
    expect(toolRenderKind(tool("mcp__flamel__browser_run"))).toBe("browser");
    expect(toolRenderKind(tool("mcp__flamel__web_search"))).toBe("research");
    expect(toolRenderKind(tool("mcp__flamel__assets"))).toBe("media");
    expect(toolRenderKind(tool("mcp__warehouse__query"))).toBe("adaptive");
  });

  it("keeps unknown providers on the governed adaptive surface", () => {
    expect(toolRenderKind(tool("mcp__connected_provider__fetch_metrics"))).toBe(
      "adaptive",
    );
  });

  it("keeps every authored widget family collapsed while tools are running", () => {
    const EmptyRenderer = () => null;
    const fixtures: ToolCall[] = [
      { ...tool("mcp__flamel__code_nav", { action: "symbols" }), status: "running" },
      { ...tool("mcp__flamel__browser_run", { actions: [] }), status: "running" },
      { ...tool("mcp__flamel__web_search", { query: "Flamel" }), status: "running" },
      { ...tool("mcp__flamel__assets", { action: "inspect" }), status: "running" },
      { ...tool("mcp__warehouse__query", { query: "select 1" }), status: "running" },
      { ...tool("mcp__flamel__plan", { action: "create" }), status: "running" },
    ];

    for (const fixture of fixtures) {
      const html = renderToStaticMarkup(
        React.createElement(ToolWidgetRenderer, {
          tool: fixture,
          LegacyRenderer: EmptyRenderer,
          ThinkingRenderer: EmptyRenderer,
        }),
      );
      expect(html, fixture.name).toContain('aria-expanded="false"');
      expect(html, fixture.name).not.toContain('class="tool-widget__body"');
    }
  });
});
