import { describe, expect, it } from "vitest";
import {
  capabilityForTool,
  isToolRenderer,
  rendererForTool,
} from "./toolCapabilities";

describe("tool capabilities", () => {
  it.each(["WebSearch", "WebFetch", "web_search", "webSearch"])(
    "renders %s as native web research",
    (name) => {
      expect(rendererForTool(name)).toBe("research");
      expect(capabilityForTool(name)?.readOnly).toBe(true);
    },
  );

  it.each([
    "browser_session",
    "browser_run",
    "browser_inspect",
    "mcp__browser__browser_run",
  ])("renders %s as persistent browser activity", (name) => {
    expect(isToolRenderer(name, "browser")).toBe(true);
  });

  it.each([
    ["file_edit", "filesystem", "diff", "tool-diff"],
    ["git", "source-control", "diff", "tool-diff"],
    ["image_gen.imagegen", "media", "gallery", "tool-image-generation"],
    ["mcp__flamel__assets", "media", "gallery", "tool-media"],
    ["database", "data", "table", "tool-data"],
  ])("assigns %s to an authored family and adapter", (name, family, adapter, spriteKey) => {
    expect(capabilityForTool(name)).toMatchObject({ family, adapter, spriteKey });
    expect(capabilityForTool(name).iconKey).not.toBe("");
    expect(capabilityForTool(name).safeActions.length).toBeGreaterThan(0);
  });

  it.each([
    ["assistant.status", "diagnostics", "status", true],
    ["brain.search", "knowledge", "document", false],
    ["skill.install", "integrations", "structured", false],
    ["automation.run", "workflow", "timeline", false],
  ])("adapts Assistant action %s", (action, family, adapter, readOnly) => {
    expect(capabilityForTool("mcp__flamel__assistant", { action })).toMatchObject({
      family,
      adapter,
      readOnly,
    });
  });

  it("treats unknown external MCP tools conservatively", () => {
    expect(capabilityForTool("mcp__partner__publish_everything")).toMatchObject({
      family: "external",
      adapter: "structured",
      iconKey: "plug",
      risk: "high",
      readOnly: false,
      destructive: true,
    });
  });
});
