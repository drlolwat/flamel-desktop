import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ToolCall } from "../../lib/messageProcessor";
import {
  AdaptiveDataToolWidget,
  adaptAdaptiveDataTool,
  isAdaptiveDataToolCall,
} from "./AdaptiveDataToolWidget";
import {
  redactAdaptiveString,
  redactAdaptiveValue,
} from "./adaptiveToolAdapters";

function tool(
  name: string,
  input: Record<string, unknown> = {},
  output: unknown = {},
  status: ToolCall["status"] = "done",
): ToolCall {
  return {
    id: `adaptive-${name}-${status}`,
    name,
    input,
    output: typeof output === "string" ? output : JSON.stringify(output),
    status,
  };
}

describe("adaptive data routing", () => {
  it("routes data, knowledge, integrations, and unknown MCP servers", () => {
    for (const fixture of [
      tool("mcp__flamel__db", { action: "query" }),
      tool("mcp__flamel__get_docs"),
      tool("mcp__flamel__capability_search"),
      tool("mcp__github__list_issues"),
      tool("mcp__custom_crm__accounts"),
    ]) {
      expect(isAdaptiveDataToolCall(fixture), fixture.name).toBe(true);
    }
    expect(isAdaptiveDataToolCall(tool("mcp__flamel__file_read"))).toBe(false);
  });

  it("uses provider and operation identity for unknown external calls", () => {
    const view = adaptAdaptiveDataTool(
      tool("mcp__github__list_issues", { state: "open" }, { count: 3 }),
    );
    expect(view).toMatchObject({
      family: "external",
      title: "Github · List issues",
      eyebrow: "Github",
      status: "success",
    });
  });
});

describe("adaptive shape selection", () => {
  it("renders nested database rows as a bounded query grid", () => {
    const view = adaptAdaptiveDataTool(
      tool(
        "mcp__flamel__db",
        { action: "query" },
        {
          ok: true,
          result: {
            rows: [
              {
                id: 1,
                customer: "Northstar",
                total: 4200,
                active: true,
                api_key: "should-not-render",
              },
              {
                id: 2,
                customer: "Juniper",
                total: 1850,
                active: false,
                api_key: "also-secret",
              },
            ],
          },
        },
      ),
    );
    expect(view?.table).toMatchObject({
      title: "Query result",
      shown: 2,
      total: 2,
      truncated: false,
    });
    expect(view?.table?.columns.map((column) => column.key)).toEqual(
      expect.arrayContaining(["id", "customer", "total", "active", "api_key"]),
    );
    expect(view?.table?.rows[0].api_key).toBe("[redacted]");
  });

  it("renders schema records with purposeful labels", () => {
    const view = adaptAdaptiveDataTool(
      tool(
        "mcp__flamel__database",
        { action: "schema" },
        {
          tables: [
            { name: "users", columns: 12, primary_key: "id" },
            { name: "sessions", columns: 8, primary_key: "id" },
          ],
        },
      ),
    );
    expect(view?.table).toMatchObject({ title: "Schema", total: 2 });
  });

  it("renders document retrieval as readable prose", () => {
    const body =
      "Tool widgets keep the primary result human-readable. Raw provider data remains available only through an explicit source view.";
    const fixture = tool(
      "mcp__flamel__get_docs",
      { library: "flamel" },
      {
        document: { title: "Widget presentation", content: body },
      },
    );
    const view = adaptAdaptiveDataTool(fixture);
    expect(view).toMatchObject({
      family: "knowledge",
      document: body,
      table: undefined,
    });
    const html = renderToStaticMarkup(
      <AdaptiveDataToolWidget tool={fixture} defaultExpanded />,
    );
    expect(html).toContain("Widget presentation");
    expect(html).toContain("Tool widgets keep the primary result");
    expect(html).toContain(">Raw<");
    expect(html).not.toContain('{"document"');
  });

  it("renders search results as a linked knowledge collection", () => {
    const view = adaptAdaptiveDataTool(
      tool(
        "mcp__flamel__search_flamel_docs",
        { query: "tool widgets" },
        {
          results: [
            {
              id: "doc-1",
              title: "Tool presentation",
              snippet: "Authored card contracts and safe fallbacks.",
              url: "https://docs.flamel.dev/tool-presentation",
              status: "current",
            },
            {
              id: "doc-2",
              title: "Capability metadata",
              snippet: "Deterministic family and adapter routing.",
              url: "https://docs.flamel.dev/capabilities",
              status: "current",
            },
          ],
        },
      ),
    );
    expect(view?.table).toBeUndefined();
    expect(view?.collection).toHaveLength(2);
    expect(view?.collection[0]).toMatchObject({
      title: "Tool presentation",
      href: "https://docs.flamel.dev/tool-presentation",
    });
  });

  it("renders explicit metrics and event history alongside fields", () => {
    const view = adaptAdaptiveDataTool(
      tool(
        "mcp__metrics__release_health",
        {},
        {
          environment: "production",
          metrics: [
            {
              label: "Success rate",
              value: "99.98%",
              delta: "+0.12%",
              trend: "up",
            },
            { label: "p95", value: "182 ms", delta: "-14 ms", trend: "down" },
          ],
          history: [
            {
              id: "deploy-2",
              title: "Canary complete",
              status: "success",
              timestamp: "02:18 UTC",
            },
            {
              id: "deploy-1",
              title: "Health check",
              status: "failed",
              message: "One retry",
              timestamp: "02:12 UTC",
            },
          ],
        },
      ),
    );
    expect(view?.metrics).toHaveLength(2);
    expect(view?.timeline).toHaveLength(2);
    expect(view?.timeline[1]).toMatchObject({ status: "error" });
    expect(view?.fields).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ label: "Environment", value: "production" }),
      ]),
    );
  });

  it("renders capability discovery as a compact collection", () => {
    const view = adaptAdaptiveDataTool(
      tool(
        "mcp__flamel__capability_search",
        { task: "sync issue status" },
        {
          capabilities: [
            {
              id: "github.issues",
              name: "GitHub issues",
              description: "Read and update connected repository issues.",
              status: "connected",
            },
          ],
        },
      ),
    );
    expect(view).toMatchObject({
      family: "integrations",
      collection: [
        expect.objectContaining({
          title: "GitHub issues",
          meta: "connected",
        }),
      ],
    });
  });

  it("handles top-level arrays and scalar results without a code wall", () => {
    const arrayView = adaptAdaptiveDataTool(
      tool("mcp__custom__tags", {}, ["urgent", "design", "release"]),
    );
    const scalarView = adaptAdaptiveDataTool(tool("mcp__flamel__time", {}, 42));
    expect(arrayView?.collection.map((item) => item.title)).toEqual([
      "urgent",
      "design",
      "release",
    ]);
    expect(scalarView?.fields).toEqual([
      expect.objectContaining({ label: "Value", value: 42 }),
    ]);
  });
});

describe("adaptive safety and limits", () => {
  it("activates only allowlisted resource URLs from MCP result blocks", () => {
    const fixture = tool(
      "mcp__library__lookup",
      {},
      {
        content: [
          {
            type: "resource_link",
            name: "Reference",
            uri: "https://example.com/reference",
            mimeType: "text/html",
          },
          {
            type: "resource_link",
            name: "Unsafe",
            uri: "javascript:alert(1)",
          },
          {
            type: "resource_link",
            name: "Local export",
            uri: "/api/tool-artifacts/export/content",
          },
        ],
        structuredContent: { status: "ready" },
      },
    );
    const view = adaptAdaptiveDataTool(fixture);
    expect(view?.resources).toEqual([
      expect.objectContaining({
        title: "Reference",
        href: "https://example.com/reference",
      }),
      expect.objectContaining({ title: "Unsafe", href: undefined }),
      expect.objectContaining({
        title: "Local export",
        href: "/api/tool-artifacts/export/content",
      }),
    ]);
    const html = renderToStaticMarkup(
      <AdaptiveDataToolWidget tool={fixture} defaultExpanded />,
    );
    expect(html).not.toContain('href="javascript:');
    expect(html).toContain("Link disabled");
  });

  it("redacts secret-like keys and token values in structured and raw views", () => {
    expect(
      redactAdaptiveValue({
        api_key: "secret-value",
        nested: { authorization: "Bearer abcdefghijklmnopqrst" },
      }),
    ).toEqual({
      api_key: "[redacted]",
      nested: { authorization: "[redacted]" },
    });
    expect(redactAdaptiveString("Bearer abcdefghijklmnopqrst")).toBe(
      "Bearer [redacted]",
    );

    const view = adaptAdaptiveDataTool(
      tool(
        "mcp__custom__receipt",
        {},
        {
          account: "safe",
          api_key: "secret-value",
          note: "Bearer abcdefghijklmnopqrst",
        },
      ),
    );
    expect(view?.fields).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ label: "Api key", value: "[redacted]" }),
        expect.objectContaining({ label: "Note", value: "Bearer [redacted]" }),
      ]),
    );
    expect(view?.rawDownload).not.toContain("secret-value");
    expect(view?.rawDownload).not.toContain("abcdefghijklmnopqrst");
  });

  it("ignores provider-supplied renderer and markup instructions", () => {
    const fixture = tool(
      "mcp__unknown__present",
      {},
      {
        renderer: "admin-dashboard",
        component: "DangerousWidget",
        html: "<script>window.pwned=true</script>",
        ui: { execute: "delete everything" },
        title: "Provider receipt",
      },
    );
    const view = adaptAdaptiveDataTool(fixture);
    expect(view?.fields).toHaveLength(0);
    const html = renderToStaticMarkup(
      <AdaptiveDataToolWidget tool={fixture} defaultExpanded />,
    );
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("DangerousWidget");
    expect(html).toContain("No structured result");
  });

  it("bounds row, column, character, and raw-editor previews", () => {
    const rows = Array.from({ length: 72 }, (_, rowIndex) =>
      Object.fromEntries(
        Array.from({ length: 16 }, (_, columnIndex) => [
          `column_${columnIndex}`,
          columnIndex === 1 ? "x".repeat(600) : rowIndex * 100 + columnIndex,
        ]),
      ),
    );
    const fixture = tool(
      "mcp__flamel__db",
      { action: "query" },
      {
        rows,
        note: "z".repeat(90_000),
      },
    );
    const view = adaptAdaptiveDataTool(fixture);
    expect(view?.table).toMatchObject({
      shown: 50,
      total: 72,
      truncated: true,
    });
    expect(view?.table?.columns).toHaveLength(12);
    expect(String(view?.table?.rows[0].column_1).length).toBeLessThanOrEqual(
      240,
    );
    expect(view).toMatchObject({
      limited: true,
      rawTruncated: true,
    });
    const html = renderToStaticMarkup(
      <AdaptiveDataToolWidget tool={fixture} defaultExpanded />,
    );
    expect(html).toContain("50 of 72");
    expect(html).toContain(">Raw<");
    expect(html).not.toContain("Download redacted result");
  });

  it("keeps malformed prose readable and handles empty, running, and error states", () => {
    const prose = adaptAdaptiveDataTool(
      tool(
        "mcp__custom__explain",
        {},
        "Provider returned a readable explanation even though its structured payload was malformed.",
      ),
    );
    const empty = adaptAdaptiveDataTool(tool("mcp__custom__empty", {}, ""));
    const running = adaptAdaptiveDataTool(
      tool("mcp__custom__stream", {}, "", "running"),
    );
    const failedFixture = tool(
      "mcp__custom__receipt",
      {},
      { error: "Connection permission denied" },
      "error",
    );
    const failed = adaptAdaptiveDataTool(failedFixture);

    expect(prose?.document).toContain("readable explanation");
    expect(empty).toMatchObject({ status: "empty", empty: true });
    expect(running).toMatchObject({ status: "running" });
    expect(failed).toMatchObject({
      status: "error",
      summary: "Connection permission denied",
    });
    const html = renderToStaticMarkup(
      <AdaptiveDataToolWidget tool={failedFixture} defaultExpanded />,
    );
    expect(html).toContain("Tool call failed");
  });
});
