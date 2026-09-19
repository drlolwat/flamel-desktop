import { beforeEach, describe, expect, it } from "vitest";
import type { ContentSegment, DisplayBlock, ToolCall } from "../lib/messageProcessor";
import { useChatStore } from "./chatStore";

const TAB_ID = "tab-1";

function userBlock(id = "prompt-1"): DisplayBlock {
  return { id, role: "user", type: "user", content: "Make a plan" };
}

function assistantBlock(overrides: Partial<DisplayBlock> = {}): DisplayBlock {
  return {
    id: "assistant-1",
    role: "assistant",
    type: "assistant",
    content: "hello",
    ...overrides,
  };
}

describe("chat store streaming updates", () => {
  beforeEach(() => {
    useChatStore.setState({ chats: {} });
  });

  it("skips state publication for equivalent last-block updates", () => {
    const tool: ToolCall = {
      id: "tool-1",
      name: "shell",
      input: {},
      status: "running",
    };
    const segment: ContentSegment = { type: "text", text: "hello" };
    useChatStore.getState().addBlock(TAB_ID, assistantBlock({ tools: [tool], segments: [segment] }));

    const beforeState = useChatStore.getState();
    const beforeChat = beforeState.chats[TAB_ID];
    const beforeBlock = beforeChat.blocks[0];

    useChatStore.getState().updateLastBlock(TAB_ID, {
      content: "hello",
      tools: [tool],
      segments: [segment],
      statusText: undefined,
    });

    const afterState = useChatStore.getState();
    expect(afterState).toBe(beforeState);
    expect(afterState.chats[TAB_ID]).toBe(beforeChat);
    expect(afterState.chats[TAB_ID].blocks[0]).toBe(beforeBlock);
  });

  it("preserves stable tool and segment references during content-only updates", () => {
    const tool: ToolCall = {
      id: "tool-1",
      name: "shell",
      input: {},
      status: "running",
    };
    const segment: ContentSegment = { type: "text", text: "hello" };
    useChatStore.getState().addBlock(TAB_ID, assistantBlock({ tools: [tool], segments: [segment] }));

    const beforeBlock = useChatStore.getState().chats[TAB_ID].blocks[0];
    useChatStore.getState().updateLastBlock(TAB_ID, {
      content: "hello again",
      tools: [tool],
      segments: [segment],
    });

    const afterBlock = useChatStore.getState().chats[TAB_ID].blocks[0];
    expect(afterBlock).not.toBe(beforeBlock);
    expect(afterBlock.content).toBe("hello again");
    expect(afterBlock.tools).toBe(beforeBlock.tools);
    expect(afterBlock.segments).toBe(beforeBlock.segments);
  });

  it("does not republish an equivalent pending question replay", () => {
    const question = {
      requestId: "question-1",
      question: "Restart the connector?",
      choices: ["Approve", "Deny"],
      allowFreeText: false,
    };
    useChatStore.getState().setPendingQuestion(TAB_ID, question);
    const beforeState = useChatStore.getState();
    const beforeChat = beforeState.chats[TAB_ID];

    useChatStore.getState().setPendingQuestion(TAB_ID, {
      ...question,
      choices: [...question.choices],
    });

    const afterState = useChatStore.getState();
    expect(afterState).toBe(beforeState);
    expect(afterState.chats[TAB_ID]).toBe(beforeChat);
  });

  it("keeps pending questions isolated to their owning tab", () => {
    useChatStore.getState().setPendingQuestion(TAB_ID, {
      requestId: "question-1",
      question: "Choose a database?",
      choices: ["SQLite", "Postgres"],
      allowFreeText: false,
    });
    useChatStore.getState().setWaiting("tab-2", true);

    expect(useChatStore.getState().getChat(TAB_ID).pendingQuestion?.requestId).toBe("question-1");
    expect(useChatStore.getState().getChat("tab-2").pendingQuestion).toBeNull();
    expect(useChatStore.getState().getChat("tab-2").isWaiting).toBe(true);
  });

  it("clears transcript and interaction state when a tab is rebound", () => {
    useChatStore.getState().setSessionId(TAB_ID, "session-a");
    useChatStore.getState().addBlock(TAB_ID, assistantBlock());
    useChatStore.getState().setWaiting(TAB_ID, true);
    useChatStore.getState().setPendingQuestion(TAB_ID, {
      requestId: "question-a",
      question: "Old session question?",
      choices: [],
      allowFreeText: true,
    });

    useChatStore.getState().setSessionId(TAB_ID, "session-b");

    expect(useChatStore.getState().getChat(TAB_ID)).toMatchObject({
      sessionId: "session-b",
      blocks: [],
      isWaiting: false,
      pendingQuestion: null,
    });
  });

  it("clears unbound stale blocks when an existing session is attached", () => {
    useChatStore.getState().addBlock(TAB_ID, assistantBlock());
    useChatStore.getState().setSessionId(TAB_ID, "session-a");

    expect(useChatStore.getState().getChat(TAB_ID)).toMatchObject({
      sessionId: "session-a",
      blocks: [],
    });
  });

  it("finishes a tool in its owning block after a checkpoint was appended", () => {
    useChatStore.getState().addBlock(TAB_ID, assistantBlock({
      tools: [{
        id: "tool-1",
        name: "file_read",
        input: {},
        status: "running",
      }],
    }));
    useChatStore.getState().addBlock(TAB_ID, {
      id: "checkpoint-1",
      role: "system",
      type: "compact",
      isStreaming: true,
    });

    useChatStore.getState().updateTool(TAB_ID, "tool-1", {
      status: "done",
      output: "complete",
    });

    const blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    expect(blocks[0].tools?.[0]).toMatchObject({
      status: "done",
      output: "complete",
    });
    expect(blocks[1].id).toBe("checkpoint-1");
  });

  it("never regresses a terminal tool when an old running snapshot replays", () => {
    useChatStore.getState().addBlock(TAB_ID, assistantBlock({
      tools: [{
        id: "tool-1",
        name: "file_read",
        input: { path: "done.ts" },
        status: "done",
        output: "complete",
      }],
    }));

    useChatStore.getState().updateLastBlock(TAB_ID, {
      tools: [{
        id: "tool-1",
        name: "file_read",
        input: { path: "done.ts" },
        status: "running",
      }],
    });
    useChatStore.getState().updateTool(TAB_ID, "tool-1", { status: "running" });

    expect(useChatStore.getState().getChat(TAB_ID).blocks[0].tools?.[0]).toMatchObject({
      status: "done",
      output: "complete",
    });
  });

  it("does not turn a lost stream into success and accepts the eventual failure", () => {
    useChatStore.getState().addBlock(TAB_ID, assistantBlock({
      isStreaming: true,
      tools: [
        { id: "pending", name: "shell", input: {}, status: "running", output: "partial" },
        { id: "finished", name: "file_read", input: {}, status: "done", output: "file" },
      ],
    }));
    useChatStore.getState().stopAllStreaming(TAB_ID);
    expect(useChatStore.getState().getChat(TAB_ID).blocks[0]).toMatchObject({
      isStreaming: false,
      tools: [{ status: "unknown", output: "partial" }, { status: "done", output: "file" }],
    });
    useChatStore.getState().updateTool(TAB_ID, "pending", { status: "error", output: "failed" });
    expect(useChatStore.getState().getChat(TAB_ID).blocks[0].tools?.[0]).toMatchObject({
      status: "error", output: "failed",
    });
  });

  it("keeps routing progress separate from messages and clears it on a new session", () => {
    const store = useChatStore.getState();
    store.setSessionId(TAB_ID, "session-a");
    store.setReasoningReroute(TAB_ID, { sessionId: "other", requestId: "next", note: "Wrong session" });
    expect(useChatStore.getState().getChat(TAB_ID).reasoningReroute).toBeNull();
    store.setReasoningReroute(TAB_ID, { sessionId: "session-a", requestId: "next", note: "Auto is switching reasoning." });
    expect(useChatStore.getState().getChat(TAB_ID).blocks).toEqual([]);
    expect(useChatStore.getState().getChat(TAB_ID).reasoningReroute?.requestId).toBe("next");
    store.setSessionId(TAB_ID, "session-b");
    expect(useChatStore.getState().getChat(TAB_ID).reasoningReroute).toBeNull();
  });
});

describe("chat history bootstrap", () => {
  beforeEach(() => {
    useChatStore.setState({ chats: {} });
    useChatStore.getState().setSessionId(TAB_ID, "session-a");
  });

  it("loads persisted history into an unchanged empty session", () => {
    const store = useChatStore.getState();
    const before = store.getChat(TAB_ID).blocks;
    const history = [userBlock(), assistantBlock()];
    expect(store.hydrateHistory(TAB_ID, "session-a", before, history)).toBe(true);
    expect(store.getChat(TAB_ID).blocks).toBe(history);
  });

  it("preserves the opening prompt sent while bootstrap is in flight", () => {
    const store = useChatStore.getState();
    const before = store.getChat(TAB_ID).blocks;
    store.addBlock(TAB_ID, userBlock());
    store.addBlock(TAB_ID, assistantBlock({ content: "", isStreaming: true }));
    const live = store.getChat(TAB_ID).blocks;
    expect(store.hydrateHistory(TAB_ID, "session-a", before, [])).toBe(false);
    expect(store.getChat(TAB_ID).blocks).toBe(live);
    expect(live[0].content).toBe("Make a plan");
  });

  it("preserves an optimistic prompt when bootstrap starts before it is persisted", () => {
    const store = useChatStore.getState();
    store.addBlock(TAB_ID, userBlock());
    store.addBlock(TAB_ID, assistantBlock({ content: "", isStreaming: true }));
    const before = store.getChat(TAB_ID).blocks;
    const state = useChatStore.getState();
    expect(store.hydrateHistory(TAB_ID, "session-a", before, [assistantBlock()])).toBe(false);
    expect(useChatStore.getState()).toBe(state);
  });

  it("does not roll back a turn that finished during the history request", () => {
    const store = useChatStore.getState();
    const before = store.getChat(TAB_ID).blocks;
    store.addBlock(TAB_ID, userBlock());
    store.addBlock(TAB_ID, assistantBlock({ isStreaming: true }));
    store.updateLastBlock(TAB_ID, { isStreaming: false, content: "Finished plan" });
    expect(store.hydrateHistory(TAB_ID, "session-a", before, [])).toBe(false);
    expect(store.getChat(TAB_ID).blocks.at(-1)?.content).toBe("Finished plan");
  });

  it("rejects history belonging to a previous session binding", () => {
    const store = useChatStore.getState();
    const before = store.getChat(TAB_ID).blocks;
    store.setSessionId(TAB_ID, "session-b");
    expect(store.hydrateHistory(TAB_ID, "session-a", before, [userBlock()])).toBe(false);
    expect(store.getChat(TAB_ID)).toMatchObject({ sessionId: "session-b", blocks: [] });
  });

  it("can refresh an idle unchanged transcript without deduplicating repeated prompts", () => {
    const store = useChatStore.getState();
    store.addBlock(TAB_ID, userBlock());
    const before = store.getChat(TAB_ID).blocks;
    const history = [userBlock(), assistantBlock(), userBlock("prompt-2")];
    expect(store.hydrateHistory(TAB_ID, "session-a", before, history)).toBe(true);
    expect(store.getChat(TAB_ID).blocks.filter(block => block.type === "user")).toHaveLength(2);
  });

  it("does not touch another tab or recreate one closed while loading", () => {
    const store = useChatStore.getState();
    const before = store.getChat(TAB_ID).blocks;
    store.setSessionId("tab-2", "session-b");
    const other = store.getChat("tab-2");
    store.clearChat(TAB_ID);
    const cleared = store.getChat(TAB_ID);
    expect(store.hydrateHistory(TAB_ID, "session-a", before, [userBlock()])).toBe(false);
    expect(store.getChat("tab-2")).toBe(other);
    expect(store.getChat(TAB_ID)).toBe(cleared);
    expect(cleared.sessionId).toBeUndefined();
  });
});
