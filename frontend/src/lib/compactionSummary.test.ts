import { describe, expect, it } from "vitest";
import { parseCompactionSummary } from "./compactionSummary";

describe("parseCompactionSummary", () => {
  it("structures the operational handoff and preserves markdown", () => {
    const sections = parseCompactionSummary(`Current session: Building the release flow.

Work in progress:
- Updating \`release.rs\`
- Last action: added the route

Next actions:
1. Run focused tests
2. Deploy

Risks and blockers: Waiting on a Windows runner.`);

    expect(sections.map((section) => section.key)).toEqual([
      "current",
      "active",
      "next",
      "risks",
    ]);
    expect(sections[1].body).toContain("`release.rs`");
    expect(sections[2].body).toContain("1. Run focused tests");
  });

  it("keeps an unstructured legacy summary readable", () => {
    expect(parseCompactionSummary("Continue the current implementation.")).toEqual([
      {
        key: "summary",
        title: "Checkpoint",
        body: "Continue the current implementation.",
      },
    ]);
  });

  it("omits empty and unknown sections", () => {
    const sections = parseCompactionSummary("Current session: Active\nGoals: Unknown\nNext actions:");
    expect(sections).toEqual([{ key: "current", title: "At checkpoint", body: "Active" }]);
  });
});
