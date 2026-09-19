import { beforeEach, describe, expect, it } from "vitest";
import { useChatStore } from "./chatStore";

const TAB_ID = "hydration-edge";

beforeEach(() => {
  useChatStore.setState({ chats: {} });
  useChatStore.getState().setSessionId(TAB_ID, "session-a");
});

describe("history hydration overlap", () => {
  it("accepts only the first response from two concurrent requests", () => {
    const store = useChatStore.getState();
    const before = store.getChat(TAB_ID).blocks;
    const history = [{ id: "saved", role: "user" as const, type: "user" as const, content: "Saved prompt" }];
    expect(store.hydrateHistory(TAB_ID, "session-a", before, history)).toBe(true);
    expect(store.hydrateHistory(TAB_ID, "session-a", before, [])).toBe(false);
    expect(store.getChat(TAB_ID).blocks).toBe(history);
  });

  it("keeps finished optimistic work even if it stopped streaming before hydration", () => {
    const store = useChatStore.getState();
    store.addBlock(TAB_ID, { id: "reply", role: "assistant", type: "assistant", content: "", isStreaming: true });
    const before = store.getChat(TAB_ID).blocks;
    store.updateLastBlock(TAB_ID, { content: "Done", isStreaming: false });
    expect(store.hydrateHistory(TAB_ID, "session-a", before, [])).toBe(false);
    expect(store.getChat(TAB_ID).blocks[0].content).toBe("Done");
  });

  it("preserves independent interaction state while accepting unchanged empty history", () => {
    const store = useChatStore.getState();
    const before = store.getChat(TAB_ID).blocks;
    const question = { requestId: "question", question: "Ready?", choices: [], allowFreeText: true };
    store.setWaiting(TAB_ID, true);
    store.setPendingQuestion(TAB_ID, question);
    expect(store.hydrateHistory(TAB_ID, "session-a", before, [])).toBe(true);
    expect(store.getChat(TAB_ID)).toMatchObject({ isWaiting: true, pendingQuestion: question, blocks: [] });
  });
});
