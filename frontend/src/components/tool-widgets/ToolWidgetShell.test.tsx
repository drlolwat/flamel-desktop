import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  serializeInspectorValue,
  ToolWidgetFields,
  ToolWidgetInspector,
  ToolWidgetState,
} from "./ToolWidgetPrimitives";
import ToolWidgetGallery from "./ToolWidgetGallery";
import { spritePathFor, ToolWidgetShell } from "./ToolWidgetShell";

describe("ToolWidgetShell", () => {
  it("uses a native disclosure control and keeps curated detail out of collapsed markup", () => {
    const html = renderToStaticMarkup(
      <ToolWidgetShell
        title="Workspace read"
        summary="3 files matched"
        status="success"
      >
        <p>Curated evidence</p>
      </ToolWidgetShell>,
    );

    expect(html).toContain("<button");
    expect(html).toContain('aria-expanded="false"');
    expect(html).toContain("3 files matched");
    expect(html).not.toContain("Curated evidence");
    expect(html).not.toContain("Inspect raw data");
  });

  it("renders expanded semantic state and content without decorative sprites", () => {
    const html = renderToStaticMarkup(
      <ToolWidgetShell
        title="Verification"
        summary="Checks are running"
        status="running"
        sprite="terminal"
        defaultExpanded
      >
        <ToolWidgetState
          status="running"
          title="Collecting evidence"
          detail="Updates stay in place."
        />
      </ToolWidgetShell>,
    );

    expect(html).toContain('aria-expanded="true"');
    expect(html).toContain('role="region"');
    expect(html).toContain('aria-label="Verification details"');
    expect(html).toContain('tabindex="0"');
    expect(html).toContain('aria-busy="true"');
    expect(html).toContain('role="status"');
    expect(html).not.toContain("/tool-sprites/");
    expect(html).toContain("Collecting evidence");
  });

  it("shows unconfirmed completion without a successful receipt", () => {
    const html = renderToStaticMarkup(
      <ToolWidgetShell title="Shell" summary="Completion not confirmed" status="unknown" />,
    );
    expect(html).toContain("Unconfirmed");
    expect(html).not.toContain("Done");
    expect(html).not.toContain('aria-busy="true"');
  });

  it("keeps raw input and output behind an explicit inspector", () => {
    const html = renderToStaticMarkup(
      <ToolWidgetShell
        title="Database query"
        summary="2 rows"
        status="success"
        defaultExpanded
        inspector={
          <ToolWidgetInspector
            input={{ query: "select 1" }}
            output={{ rows: 2 }}
          />
        }
      >
        <ToolWidgetFields fields={[{ label: "Rows", value: "2" }]} />
      </ToolWidgetShell>,
    );

    expect(html).toContain("<details");
    expect(html).toContain("Inspect raw data");
    expect(html).not.toContain('role="tablist"');
    expect(html).toContain("Rows");
    expect(html).not.toContain("select 1");
  });

  it("serializes malformed inspector values without throwing and bounds giant payloads", () => {
    const circular: Record<string, unknown> = {};
    circular.self = circular;
    expect(serializeInspectorValue(circular)).toBe("[object Object]");

    const bounded = serializeInspectorValue("x".repeat(100_000));
    expect(bounded.length).toBeLessThan(66_000);
    expect(bounded).toContain("raw data clipped for this inspector");
  });

  it("maps every authored sprite family to a project asset", () => {
    expect([
      spritePathFor("workspace"),
      spritePathFor("terminal"),
      spritePathFor("source-control"),
      spritePathFor("browser-research"),
      spritePathFor("media"),
      spritePathFor("data"),
      spritePathFor("assistant-workflow"),
      spritePathFor("integrations"),
    ]).toEqual([
      "/tool-sprites/workspace.webp",
      "/tool-sprites/terminal.webp",
      "/tool-sprites/source-control.webp",
      "/tool-sprites/browser-research.webp",
      "/tool-sprites/media.webp",
      "/tool-sprites/data.webp",
      "/tool-sprites/assistant-workflow.webp",
      "/tool-sprites/integrations.webp",
    ]);
  });
});

describe("ToolWidgetGallery", () => {
  it("renders compact fixtures for every foundation state", () => {
    const html = renderToStaticMarkup(<ToolWidgetGallery />);

    expect(html).toContain("Expanded · successful");
    expect(html).toContain("Expanded · streaming");
    expect(html).toContain("Collapsed · compact receipt");
    expect(html).toContain("Error and empty states");
    expect(html).toContain("Pending workflow");
    expect(html).toContain("Navigation timed out");
    expect(html).toContain("No matching rows");
    expect(html).toContain(
      "Adaptive data, knowledge, and external receipts · live renderer",
    );
    expect(html).toContain("Northstar");
    expect(html).toContain("Tool presentation");
    expect(html).toContain("Success rate");
    expect(html).toContain("Preview clipped");
    expect(html).toContain("file_read-gallery.txt");
  });
});
