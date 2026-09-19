import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = (file) => readFileSync(new URL(file, import.meta.url), "utf8");

describe("tool widget style loading", () => {
  it("keeps the chat disclosure shell styled immediately without loading rich tool detail CSS", () => {
    expect(source("./ToolWidgetShell.tsx")).toContain('import "./toolWidgetShell.css"');
    expect(source("./ToolWidgetShell.tsx")).not.toContain('import "./toolWidgets.css"');
    const css = source("./toolWidgetShell.css");
    expect(css).toContain(".tool-widget__header:focus-visible");
    expect(css).toContain("@media (pointer: coarse)");
    expect(css).toContain("@media (max-width: 560px)");
    // Flat hover feedback is immediate for everyone, including reduced-motion users.
    expect(css).not.toMatch(/\b(?:transition|animation):/);
    expect(css).not.toMatch(/@import|\.engineering-|\.embedded-|\.workflow-|\.adaptive-data/);
  });

  it("loads rich detail styles separately without duplicating the already-loaded shell", () => {
    expect(source("./ToolWidgetPrimitives.tsx")).toContain('import "./toolWidgets.css"');
    const css = source("./toolWidgets.css");
    expect(css).not.toContain('@import "./toolWidgetShell.css"');
    expect(css).not.toContain(".tool-widget__header");
    expect(css).toContain(".engineering-widget__terminal-attach");
    expect(css).toContain(".tool-widget-fields");
    expect(css).toContain(".workflow-widget__agent-tree");
    expect(css).toContain(".embedded-code");
  });
});
