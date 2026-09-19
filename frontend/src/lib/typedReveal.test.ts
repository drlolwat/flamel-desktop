import { describe, expect, it } from "vitest";
import {
  clampRevealLength,
  nextTypedRevealLength,
  normalizeTypedRevealOptions,
  shouldSnapTypedReveal,
  stabilizePartialMarkdown,
} from "./typedReveal";

describe("typed reveal helpers", () => {
  it("advances reveal length with bounded adaptive pacing", () => {
    const next = nextTypedRevealLength({
      ...normalizeTypedRevealOptions({ charsPerSecond: 600, maxStep: 20 }),
      currentLength: 10,
      targetLength: 100,
      deltaMs: 16,
    });

    expect(next).toBeGreaterThan(10);
    expect(next).toBeLessThanOrEqual(30);
  });

  it("snaps when backlog is too large", () => {
    expect(shouldSnapTypedReveal(0, 2500, 2400)).toBe(true);
    expect(nextTypedRevealLength({
      ...normalizeTypedRevealOptions({ snapBacklog: 1000 }),
      currentLength: 10,
      targetLength: 1500,
      deltaMs: 16,
    })).toBe(1500);
  });

  it("clamps reveal length when text shrinks", () => {
    expect(clampRevealLength(100, 20)).toBe(20);
    expect(clampRevealLength(-1, 20)).toBe(0);
  });

  it("stabilizes partial fenced code blocks for markdown rendering", () => {
    expect(stabilizePartialMarkdown("```ts\nconst value = 1;")).toBe("```ts\nconst value = 1;\n```");
  });

  it("stabilizes partial inline code without changing complete markdown", () => {
    expect(stabilizePartialMarkdown("Use `value")).toBe("Use `value`");
    expect(stabilizePartialMarkdown("Use `value`")).toBe("Use `value`");
  });

  it("ignores literal backticks inside fenced code when balancing inline markdown", () => {
    const markdown = "```txt\na literal ` character\n```";
    expect(stabilizePartialMarkdown(markdown)).toBe(markdown);
  });
});
