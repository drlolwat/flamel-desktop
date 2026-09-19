import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ToolCall } from "../../lib/messageProcessor";
import { FLAMEL_TOOL_CAPABILITIES } from "../../lib/toolCapabilities";
import { sanitizeSvgMarkup } from "../viz/SvgViz";
import { visualizationDetails } from "../viz/VizRenderer";
import {
  buildMediaToolView,
  isMediaToolCall,
  MediaToolWidget,
} from "./MediaToolWidget";
import {
  buildBrowserView,
  buildResearchView,
  isBrowserToolCall,
  isResearchToolCall,
  ResearchBrowserToolWidget,
} from "./ResearchBrowserToolWidget";

function tool(
  name: string,
  input: Record<string, unknown> = {},
  output = "",
  status: ToolCall["status"] = "done",
): ToolCall {
  return {
    id: `fixture-${name}-${status}`,
    name,
    input,
    output,
    status,
  };
}

describe("media tool widgets", () => {
  it("routes every registered media capability and alias", () => {
    const mediaCapabilities = FLAMEL_TOOL_CAPABILITIES.filter(
      (capability) => capability.renderer === "assets",
    );
    expect(mediaCapabilities.map((capability) => capability.name)).toEqual([
      "assets",
      "image_gen",
      "view_image",
    ]);
    for (const capability of mediaCapabilities) {
      for (const name of [capability.name, ...(capability.aliases ?? [])]) {
        expect(isMediaToolCall(tool(`mcp__flamel__${name}`)), name).toBe(true);
      }
    }
  });

  it("adapts generated images, output hints, sprite sheets, and managed metadata", () => {
    const generated: ToolCall = {
      ...tool(
        "image_gen",
        { prompt: "A pixel-art sprite sheet with transparent background" },
        JSON.stringify({ output_hint: "Saved to the workspace" }),
      ),
      artifacts: [
        {
          id: "sprite",
          kind: "image",
          name: "character-sprite-sheet.webp",
          mediaType: "image/webp",
          byteSize: 18432,
          sha256: "sha",
          contentUrl: "/api/tool-artifacts/sprite/content",
          downloadUrl: "/api/tool-artifacts/sprite/download",
        },
      ],
    };
    const view = buildMediaToolView(generated);
    expect(view).toMatchObject({
      status: "success",
      prompt: "A pixel-art sprite sheet with transparent background",
      outputHint: "Saved to the workspace",
      spriteSheet: true,
      summary: "1 artifact ready",
    });
    expect(view.items[0]).toMatchObject({
      kind: "image",
      mediaType: "image/webp",
      byteSize: 18432,
    });
  });

  it("normalizes MCP media, workspace paths, multiple files, and non-image media", () => {
    const mixed: ToolCall = {
      ...tool(
        "assets",
        { action: "inspect" },
        "Created /tmp/flamel/sheet.png and /tmp/flamel/preview.webp",
      ),
      presentation: {
        version: 1,
        text: "",
        summary: "",
        structuredContent: {},
        resources: [
          {
            uri: "https://media.example/demo.mp4",
            name: "demo.mp4",
            mediaType: "video/mp4",
            size: 2048,
          },
          {
            uri: "https://media.example/notes.pdf",
            name: "notes.pdf",
            mediaType: "application/pdf",
          },
        ],
        media: [
          {
            kind: "audio",
            mediaType: "audio/mpeg",
            resourceUri: "https://media.example/audio.mp3",
            alt: "Narration",
          },
          {
            kind: "image",
            mediaType: "image/png",
            resourceUri: "/api/tool-artifacts/mcp-image/content",
            alt: "MCP image",
          },
        ],
        artifacts: [],
        isError: false,
        truncated: false,
        legacy: false,
      },
    };
    const view = buildMediaToolView(mixed);
    expect(view.items.map((item) => item.kind)).toEqual([
      "audio",
      "image",
      "video",
      "file",
      "image",
      "image",
    ]);
    expect(
      view.items.filter((item) => item.source === "workspace"),
    ).toHaveLength(2);
  });

  it("never emits retained base64 as a preview URL", () => {
    const unsafe: ToolCall = {
      ...tool("image_gen"),
      artifacts: [
        {
          id: "unsafe",
          kind: "image",
          name: "unsafe.png",
          mediaType: "image/png",
          byteSize: 10,
          sha256: "sha",
          contentUrl: "data:image/png;base64,AAAA",
        },
      ],
    };
    const view = buildMediaToolView(unsafe);
    expect(view.items[0].url).toBeUndefined();
    const markup = renderToStaticMarkup(
      <MediaToolWidget tool={unsafe} defaultExpanded />,
    );
    expect(markup).not.toContain("data:image");
    expect(markup).toContain("Preview unavailable");
  });

  it("renders empty and failure states without raw JSON blocks", () => {
    const markup = renderToStaticMarkup(
      <MediaToolWidget
        tool={tool(
          "image_gen",
          { prompt: "missing asset" },
          "provider failed",
          "error",
        )}
        defaultExpanded
      />,
    );
    expect(markup).toContain("Media tool failed");
    expect(markup).toContain("No managed media");
    expect(markup).not.toContain(">INPUT<");
    expect(markup).not.toContain(">OUTPUT<");
  });
});

describe("research and browser tool widgets", () => {
  it("recognizes provider-native web runs and returns ranked, safe sources", () => {
    const web = tool(
      "web__run",
      { search_query: [{ q: "Flamel tool widgets" }] },
      JSON.stringify({
        structuredContent: {
          results: [
            {
              title: "Tool widgets",
              url: "https://docs.example/widgets",
              snippet: "Purpose-built tool presentation.",
            },
            {
              title: "Unsafe",
              url: "javascript:alert(1)",
            },
          ],
        },
      }),
    );
    expect(isResearchToolCall(web)).toBe(true);
    const view = buildResearchView(web);
    expect(view.query).toBe("Flamel tool widgets");
    expect(view.sources).toEqual([
      expect.objectContaining({
        title: "Tool widgets",
        url: "https://docs.example/widgets",
        domain: "docs.example",
      }),
      expect.objectContaining({ title: "Unsafe", url: undefined }),
    ]);
  });

  it("extracts markdown links, deduplicates sources, and renders authored research", () => {
    const web = tool(
      "WebSearch",
      { query: "design systems" },
      "[Design system](https://example.com/design)\n[Duplicate](https://example.com/design)",
    );
    const view = buildResearchView(web);
    expect(view.sources).toHaveLength(1);
    const markup = renderToStaticMarkup(
      <ResearchBrowserToolWidget tool={web} kind="research" defaultExpanded />,
    );
    expect(markup).toContain("example.com");
    expect(markup).toContain("Sources");
    expect(markup).not.toContain(">INPUT<");
  });

  it("builds ordered browser timelines, counters, screenshots, and bounded errors", () => {
    const browser = tool(
      "mcp__flamel__browser_inspect",
      { action: "inspect", url: "https://example.com" },
      JSON.stringify({
        title: "Example",
        url: "https://example.com",
        actions: [
          { action: "navigate", status: "done", url: "https://example.com" },
          { action: "screenshot", status: "failed", result: "timeout" },
        ],
        console: [{ level: "error" }],
        network: [
          { url: "https://example.com" },
          { url: "https://example.com/app.js" },
        ],
        audits: [{ name: "accessibility" }],
        errors: ["Screenshot timed out"],
        capture: "/tmp/flamel/browser-capture.webp",
      }),
    );
    expect(isBrowserToolCall(browser)).toBe(true);
    const view = buildBrowserView(browser);
    expect(view).toMatchObject({
      title: "Example",
      consoleCount: 1,
      networkCount: 2,
      auditCount: 1,
      errors: ["Screenshot timed out"],
    });
    expect(view.timeline.map((item) => item.status)).toEqual([
      "success",
      "error",
    ]);
    const markup = renderToStaticMarkup(
      <ResearchBrowserToolWidget
        tool={browser}
        kind="browser"
        defaultExpanded
      />,
    );
    expect(markup).toContain("Captured frames");
    expect(markup).toContain("Screenshot timed out");
  });
});

describe("visualization widgets", () => {
  const fixtures: Array<[string, Record<string, unknown>, string]> = [
    [
      "render_chart",
      {
        type: "bar",
        title: "Revenue",
        data: [{ month: "Jan", value: 4 }],
        xKey: "month",
        yKeys: ["value"],
      },
      "chart",
    ],
    [
      "render_diagram",
      { title: "Flow", diagram: "flowchart\nA -> B" },
      "diagram",
    ],
    ["render_metric", { metrics: [{ label: "Latency", value: 42 }] }, "metric"],
    ["render_progress", { bars: [{ label: "Build", value: 80 }] }, "progress"],
    [
      "render_svg",
      { title: "Mark", svg: '<svg><circle cx="8" cy="8" r="4" /></svg>' },
      "svg",
    ],
  ];

  it("accepts every visualization type through the authored shell contract", () => {
    for (const [name, input, kind] of fixtures) {
      expect(
        visualizationDetails(tool(`mcp__viz__${name}`, input)),
      ).toMatchObject({
        kind,
        status: "success",
      });
    }
    expect(
      visualizationDetails(
        tool("render_metric", {
          metrics: [{ label: "Latency", value: "42 ms" }],
        }),
      ).title,
    ).toBe("Metric visualization");
  });

  it("distinguishes streaming, provider failure, and invalid completed input", () => {
    expect(
      visualizationDetails(tool("render_chart", {}, "", "running")),
    ).toMatchObject({
      status: "running",
      validationError: undefined,
    });
    expect(
      visualizationDetails(tool("render_chart", {}, "", "error")),
    ).toMatchObject({
      status: "error",
      validationError: "No visualization input was received.",
    });
    expect(
      visualizationDetails(tool("render_chart", { type: "line" })),
    ).toMatchObject({
      status: "error",
      validationError: "Chart data, xKey, and yKeys are required.",
    });
  });

  it("sanitizes active SVG content and external references", () => {
    const cleaned = sanitizeSvgMarkup(
      '<svg onload="alert(1)"><script>alert(1)</script><foreignObject><div>bad</div></foreignObject><path d="M0 0" onclick="x()" style="fill:red" href="javascript:x" /></svg>',
    );
    expect(cleaned).toContain("<svg");
    expect(cleaned).toContain("<path");
    expect(cleaned).not.toMatch(
      /script|foreignObject|onload|onclick|javascript:|style=/i,
    );
  });
});
