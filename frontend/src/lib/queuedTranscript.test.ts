import { describe, expect, it } from "vitest";
import type { DisplayBlock } from "./messageProcessor";
import {
  hasQueuedUserBlock,
  reconcileQueuedOptimisticBlocks,
  visibleQueuedMessages,
} from "./queuedTranscript";

describe("queued transcript reconciliation", () => {
  it("keeps an accepted interrupt visible while removing its empty assistant", () => {
    const blocks: DisplayBlock[] = [
      { id: "old", role: "assistant", type: "assistant", content: "Working" },
      { id: "user", role: "user", type: "user", content: "Stop and check this" },
      { id: "assistant", role: "assistant", type: "assistant", content: "", isStreaming: true },
    ];

    const next = reconcileQueuedOptimisticBlocks(
      blocks,
      new Set(["user", "assistant"]),
      true,
      "request-2",
    );

    expect(next.map(block => block.id)).toEqual(["old", "user"]);
    expect(next[1].requestId).toBe("request-2");
    expect(hasQueuedUserBlock(next, "request-2", "Stop and check this")).toBe(true);
  });

  it("keeps ordinary queued prompts editable outside the transcript", () => {
    const blocks: DisplayBlock[] = [
      { id: "user", role: "user", type: "user", content: "Run later" },
      { id: "assistant", role: "assistant", type: "assistant", content: "" },
    ];

    expect(reconcileQueuedOptimisticBlocks(
      blocks,
      new Set(["user", "assistant"]),
      false,
      "request-3",
    )).toEqual([]);
  });

  it("projects an admitted interrupt as an immediately visible queue item", () => {
    const visible = visibleQueuedMessages([
      {
        id: "request-4",
        message: "Stop and use this instead",
        workingDirectory: "/project",
        createdAt: 42,
        options: { __interruptActive: true },
        serverOwned: true,
        admissionState: "accepted",
      },
      {
        id: "hidden-continuation",
        message: "Continue",
        workingDirectory: "/project",
        createdAt: 43,
        options: { __isContinuation: true },
      },
    ]);

    expect(visible).toEqual([
      {
        id: "request-4",
        message: "Stop and use this instead",
        workingDirectory: "/project",
        createdAt: 42,
        imageCount: 0,
        serverOwned: true,
        interruptActive: true,
        admissionState: "accepted",
      },
    ]);
  });
});
