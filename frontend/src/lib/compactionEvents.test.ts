import { beforeEach, describe, expect, it } from "vitest";
import { useChatStore } from "../stores/chatStore";
import { useTabStore } from "../stores/tabStore";
import { applyCompactionEventToStores, clearCompactionBlock } from "./compactionEvents";
import type { WorkflowEvent, WorkflowEventKind } from "./workflowEvents";

const TAB_ID = "compaction-tab";

function event(kind: WorkflowEventKind, payload: Record<string, unknown>): WorkflowEvent {
  return { type: "workflow_event", kind, source: "chat_stream", payload };
}

describe("compaction event rendering", () => {
  beforeEach(() => {
    useChatStore.setState({ chats: {} });
    useTabStore.getState().addTab({
      id: TAB_ID,
      title: "Compaction test",
      projectPath: "/tmp/project",
      model: "gpt-5.6-sol",
      permissionMode: "bypassPermissions",
    });
    useChatStore.getState().addBlock(TAB_ID, {
      id: "assistant-1",
      role: "assistant",
      type: "assistant",
      content: "Existing work",
      isStreaming: true,
      tools: [{
        id: "tool-1",
        name: "file_read",
        input: {},
        status: "running",
      }],
    });
  });

  it("targets one run, ignores replayed deltas, and completes idempotently", () => {
    applyCompactionEventToStores(TAB_ID, event("system.compacting", {
      runId: "run-1",
      phase: "writing_summary",
      label: "Writing summary",
      summaryDelta: "A",
      summaryChars: 1,
    }));
    applyCompactionEventToStores(TAB_ID, event("system.compacting", {
      runId: "run-1",
      phase: "writing_summary",
      label: "Writing summary",
      summaryDelta: "A",
      summaryChars: 1,
    }));
    applyCompactionEventToStores(TAB_ID, event("system.compacting", {
      runId: "run-1",
      phase: "writing_summary",
      label: "Writing summary",
      summaryDelta: "B",
      summaryChars: 2,
    }));

    let blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    expect(blocks).toHaveLength(2);
    expect(blocks[1].type).toBe("compact");
    expect(blocks[1].isStreaming).toBe(true);
    expect(blocks[1].compaction?.summary).toBe("AB");
    expect(blocks[1].compaction?.runId).toBe("run-1");
    expect(blocks[0].isStreaming).toBe(false);
    expect(blocks[0].tools?.[0].status).toBe("unknown");

    const complete = event("system.compact_complete", {
      runId: "run-1",
      phase: "complete",
      label: "Compaction complete",
      summary: "AB",
      queuedPrompt: true,
    });
    applyCompactionEventToStores(TAB_ID, complete);
    applyCompactionEventToStores(TAB_ID, complete);

    blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    expect(blocks).toHaveLength(2);
    expect(blocks[0].statusText).toBeUndefined();
    expect(blocks[1].id).toBe("compaction:run-1");
    expect(blocks[1].isStreaming).toBe(false);
    expect(blocks[1].compaction?.summary).toBe("AB");
  });

  it("ignores provider-native compaction without mutating active tools", () => {
    applyCompactionEventToStores(TAB_ID, event("system.compacting", {
      label: "Compacting context",
      providerNative: true,
    }));
    applyCompactionEventToStores(TAB_ID, event("system.compacting", {
      label: "Compacting context",
      providerNative: true,
    }));

    let blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    expect(blocks).toHaveLength(1);
    expect(blocks[0].tools?.[0].status).toBe("running");

    const complete = event("system.compact_complete", {
      label: "Compaction complete",
      providerNative: true,
    });
    applyCompactionEventToStores(TAB_ID, complete);
    applyCompactionEventToStores(TAB_ID, complete);
    blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    expect(blocks).toHaveLength(1);
    expect(blocks[0].tools?.[0].status).toBe("running");

    clearCompactionBlock(TAB_ID);
    blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    expect(blocks).toHaveLength(1);
  });

  it("preserves a queued continuation when an older server omits it from completion", () => {
    applyCompactionEventToStores(TAB_ID, event("system.compacting", {
      runId: "run-handoff",
      phase: "replacing_context",
      summary: "Current session:\nContinue current work",
      queuedPrompt: true,
    }));
    applyCompactionEventToStores(TAB_ID, event("system.compact_complete", {
      runId: "run-handoff",
      phase: "complete",
      summary: "Current session:\nContinue current work",
    }));

    const blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    const tab = useTabStore.getState().tabs.find(candidate => candidate.id === TAB_ID);
    expect(blocks.at(-1)?.compaction?.queuedPrompt).toBe(true);
    expect(tab?.isStreaming).toBe(true);
  });

  it("ignores legacy provider-native completions restored from the journal", () => {
    applyCompactionEventToStores(TAB_ID, event("system.compacting", {
      label: "Compacting context",
    }));
    applyCompactionEventToStores(TAB_ID, event("system.compact_complete", {
      status: "compact_complete",
      pre_tokens: 240_000,
    }));

    const blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    expect(blocks).toHaveLength(1);
    expect(blocks[0].id).toBe("assistant-1");
  });

  it("removes a live compaction marker when the owning run is terminal", () => {
    applyCompactionEventToStores(TAB_ID, event("system.compacting", {
      runId: "run-stopped",
      label: "Defragmenting",
      summaryDelta: "Partial summary",
    }));

    clearCompactionBlock(TAB_ID);

    const blocks = useChatStore.getState().getChat(TAB_ID).blocks;
    expect(blocks).toHaveLength(1);
    expect(blocks[0].id).toBe("assistant-1");
    expect(blocks[0].statusText).toBeUndefined();
  });
});
