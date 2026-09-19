import { describe, expect, it } from "vitest";
import { tabRequiresInput } from "./TabBar";

describe("tabRequiresInput", () => {
  it("marks only the tab that owns a pending question", () => {
    const chats = {
      "tab-1": {
        blocks: [],
        isWaiting: true,
        pendingQuestion: {
          requestId: "question-1",
          question: "Choose a database?",
          choices: ["SQLite", "Postgres"],
          allowFreeText: false,
        },
      },
      "tab-2": {
        blocks: [],
        isWaiting: false,
        pendingQuestion: null,
      },
    };

    expect(tabRequiresInput(chats, "tab-1")).toBe(true);
    expect(tabRequiresInput(chats, "tab-2")).toBe(false);
    expect(tabRequiresInput(chats, "missing")).toBe(false);
  });
});
