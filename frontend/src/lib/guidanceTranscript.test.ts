import { describe, expect, it } from "vitest";
import { captureGuidanceAnchor, guidanceTranscript } from "./guidanceTranscript";
import type { DisplayBlock } from "./messageProcessor";
import type { InputDelivery } from "./chatInput";
import { useChatStore } from "../stores/chatStore";

const assistant = (text = "Before."): DisplayBlock => ({
  id: "assistant", type: "assistant", role: "assistant", model: "gpt-6-astra",
  content: text, segments: [{ type: "text", text }], isStreaming: true,
});
const receipt = (updates: Partial<InputDelivery> = {}): InputDelivery => ({
  requestId: "input-1", sessionId: "session", message: "finishhh", delivery: "sent", ...updates,
});

describe("delivered guidance transcript", () => {
  it("inserts acknowledged guidance between old and new streaming text without touching the source", () => {
    const anchor = captureGuidanceAnchor([assistant()]);
    const current = assistant("Before. After.");
    const original = structuredClone(current);
    const projected = guidanceTranscript([current], [receipt({ transcriptAnchor: anchor })]);
    expect(projected.map(entry => [entry.block.type, entry.block.content])).toEqual([
      ["assistant", "Before."], ["user", "finishhh"], ["assistant", " After."],
    ]);
    expect(projected[0].block.isStreaming).toBe(false);
    expect(projected[2].block.isStreaming).toBe(true);
    expect(projected[2].block.id).toBe("assistant");
    expect(current).toEqual(original);
  });

  it("keeps completed tool receipts with their original side of the guidance boundary", () => {
    const block = assistant();
    block.tools = [{ id: "read", name: "file_read", input: { file: "a.rs" }, status: "running" }];
    block.segments!.push({ type: "tool_ref", toolId: "read" });
    const anchor = captureGuidanceAnchor([block]);
    block.tools[0].status = "done";
    block.segments!.push({ type: "text", text: "Read done." });
    block.content += "Read done.";
    const projected = guidanceTranscript([block], [receipt({ transcriptAnchor: anchor })]);
    expect(projected[0].block.tools).toEqual(block.tools);
    expect(projected[2].block.tools).toEqual([]);
    expect(projected[2].block.content).toBe("Read done.");
  });

  it("keeps repeated identical guidance distinct by input ID and suppresses duplicate receipts", () => {
    const anchor = captureGuidanceAnchor([assistant()]);
    const first = receipt({ transcriptAnchor: anchor });
    const second = receipt({ requestId: "input-2", transcriptAnchor: anchor });
    const projected = guidanceTranscript([assistant("Before. After.")], [first, first, second]);
    expect(projected.filter(entry => entry.guidance).map(entry => entry.block.requestId)).toEqual(["input-1", "input-2"]);
    expect(projected.filter(entry => entry.block.type === "assistant").map(entry => entry.block.content).join("")).toBe("Before. After.");
  });

  it("preserves text exactly across multiple guidance positions and subsequent segments", () => {
    const first = captureGuidanceAnchor([assistant("One.")]);
    const second = captureGuidanceAnchor([assistant("One. Two.")]);
    const current = assistant("One. Two. Three.");
    current.segments!.push({ type: "text", text: "Four." });
    const projected = guidanceTranscript([current], [
      receipt({ transcriptAnchor: first }),
      receipt({ requestId: "input-2", message: "next guidance", transcriptAnchor: second }),
    ]);
    expect(projected.map(entry => entry.block.content)).toEqual([
      "One.", "finishhh", " Two.", "next guidance", " Three.Four.",
    ]);
  });

  it("does not display saving, waiting, failed, uncertain or queued receipts as delivered", () => {
    for (const delivery of ["saving", "waiting", "failed", "uncertain", "queued"] as const) {
      expect(guidanceTranscript([assistant()], [receipt({ delivery })])).toEqual([{ block: assistant(), globalIdx: 0 }]);
    }
  });

  it("deduplicates after REST reopen by request ID, not matching message text", () => {
    const restored: DisplayBlock = { id: "saved", type: "user", role: "user", requestId: "input-1", content: "finishhh" };
    const projected = guidanceTranscript([restored, assistant()], [receipt(), receipt({ requestId: "input-2" })]);
    expect(projected.filter(entry => entry.block.type === "user").map(entry => entry.block.requestId)).toEqual(["input-1", "input-2"]);
  });

  it("keeps a sent receipt visible when reconnect replaces its original assistant ID", () => {
    const anchor = captureGuidanceAnchor([assistant()]);
    const projected = guidanceTranscript([{ ...assistant(), id: "restored" }], [receipt({ transcriptAnchor: anchor })]);
    expect(projected.filter(entry => entry.guidance)).toHaveLength(1);
    expect(projected[0].globalIdx).toBe(0);
  });

  it("captures the position once and never puts synthetic guidance in the mutable stream store", () => {
    useChatStore.setState({ chats: {} });
    const store = useChatStore.getState();
    store.setSessionId("tab", "session");
    store.addBlock("tab", assistant());
    store.recordInputDelivery("tab", receipt({ delivery: "saving" }));
    store.updateLastBlock("tab", { content: "Before. After.", segments: [{ type: "text", text: "Before. After." }] });
    store.recordInputDelivery("tab", receipt());
    const chat = store.getChat("tab");
    expect(chat.blocks).toHaveLength(1);
    expect(chat.blocks[0].type).toBe("assistant");
    expect(chat.inputDeliveries![0].transcriptAnchor?.tailTextLength).toBe("Before.".length);
    expect(guidanceTranscript(chat.blocks, chat.inputDeliveries!).map(entry => entry.block.content)).toEqual(["Before.", "finishhh", " After."]);
    store.recordInputDelivery("tab", receipt({ sessionId: "other", requestId: "wrong" }));
    expect(store.getChat("tab").inputDeliveries).toHaveLength(1);
  });

  it("preserves source indices so editing a later real user message cannot target a synthetic entry", () => {
    const anchor = captureGuidanceAnchor([assistant()]);
    const nextUser: DisplayBlock = { id: "next", type: "user", role: "user", content: "Next question" };
    const projected = guidanceTranscript([assistant(), nextUser], [receipt({ transcriptAnchor: anchor })]);
    expect(projected.find(entry => entry.block.id === "next")?.globalIdx).toBe(1);
    expect(projected.find(entry => entry.guidance)?.guidance).toBe(true);
  });
});
