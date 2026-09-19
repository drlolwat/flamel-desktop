import { useChatStore } from "../stores/chatStore";
import { useTabStore } from "../stores/tabStore";
import type { CompactionDetails, DisplayBlock } from "./messageProcessor";
import type { WorkflowEvent } from "./workflowEvents";

export const COMPACTING_STATUS_PREFIX = "__compacting__";

function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function asNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function asBoolean(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

export function compactionStatusLabel(payload: any): string {
  return asString(payload?.label) ?? "Compacting context";
}

export function compactionStatusText(payload: any): string {
  return `${COMPACTING_STATUS_PREFIX}:${compactionStatusLabel(payload)}`;
}

export function compactionDetailsFromPayload(
  payload: any,
  existing?: CompactionDetails,
): CompactionDetails {
  const resetSummary = payload?.summaryReset === true;
  const summaryDelta = asString(payload?.summaryDelta);
  const explicitSummary = asString(payload?.summary);
  const baseSummary = resetSummary ? "" : (existing?.summary ?? "");
  const incomingSummaryChars = asNumber(payload?.summaryChars);
  const replayedDelta =
    !!summaryDelta &&
    incomingSummaryChars !== undefined &&
    existing?.summaryChars !== undefined &&
    incomingSummaryChars <= existing.summaryChars;
  const summary =
    explicitSummary ??
    (summaryDelta && !replayedDelta
      ? `${baseSummary}${summaryDelta}`
      : resetSummary
        ? ""
        : existing?.summary);
  const preTokens = asNumber(payload?.preTokens) ?? asNumber(payload?.pre_tokens) ?? existing?.preTokens;
  const estimatedSummaryTokens = asNumber(payload?.estimatedSummaryTokens) ?? existing?.estimatedSummaryTokens;
  const estimatedRecoveredTokens = asNumber(payload?.estimatedRecoveredTokens) ?? existing?.estimatedRecoveredTokens;
  const summaryChars = incomingSummaryChars ?? (summary ? Array.from(summary).length : existing?.summaryChars);

  return {
    ...existing,
    runId: asString(payload?.runId) ?? existing?.runId,
    providerNative: asBoolean(payload?.providerNative) ?? existing?.providerNative,
    phase: asString(payload?.phase) ?? existing?.phase,
    label: compactionStatusLabel(payload),
    summary,
    summaryChars,
    preTokens,
    model: asString(payload?.model) ?? existing?.model,
    reasoningEffort: asString(payload?.reasoningEffort) ?? existing?.reasoningEffort,
    modelRole: asString(payload?.modelRole) ?? existing?.modelRole,
    fallbackUsed: asBoolean(payload?.fallbackUsed) ?? existing?.fallbackUsed,
    fallbackEnabled: asBoolean(payload?.fallbackEnabled) ?? existing?.fallbackEnabled,
    summarizedTurns: asNumber(payload?.summarizedTurns) ?? existing?.summarizedTurns,
    preservedRecentMessages: asNumber(payload?.preservedRecentMessages) ?? existing?.preservedRecentMessages,
    estimatedSummaryTokens,
    estimatedRecoveredTokens,
    recoveryStrategy: asString(payload?.recoveryStrategy) ?? existing?.recoveryStrategy,
    queuedPrompt: asBoolean(payload?.queuedPrompt) ?? existing?.queuedPrompt,
  };
}

export function compactionStreamingUpdate(
  payload: any,
  existingBlock?: DisplayBlock,
): Partial<DisplayBlock> {
  const compaction = compactionDetailsFromPayload(payload, existingBlock?.compaction);
  return {
    statusText: compactionStatusText(payload),
    compaction,
    preTokens: compaction.preTokens ?? existingBlock?.preTokens,
  };
}

export function compactBlockFromPayload(
  payload: any,
  existing?: CompactionDetails,
): DisplayBlock {
  const compaction = compactionDetailsFromPayload(payload, existing);
  return {
    id: compaction.runId ? `compaction:${compaction.runId}` : crypto.randomUUID(),
    role: "system",
    type: "compact",
    preTokens: compaction.preTokens,
    compaction,
  };
}

function findStreamingCompaction(blocks: DisplayBlock[], runId?: string): number {
  if (runId) {
    const exact = blocks.findIndex(
      block => block.type === "compact" && block.compaction?.runId === runId,
    );
    if (exact >= 0) return exact;
  }
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    const block = blocks[index];
    if (
      block.type === "compact" &&
      block.isStreaming &&
      (!runId || !block.compaction?.runId || block.compaction.runId === runId)
    ) {
      return index;
    }
  }
  return -1;
}

function isEmptyAssistant(block: DisplayBlock): boolean {
  return (
    block.type === "assistant" &&
    !block.content &&
    !block.thinking &&
    !block.tools?.length &&
    !block.segments?.length
  );
}

function applyCompactingBlock(tabId: string, payload: any) {
  const store = useChatStore.getState();
  const blocks = store.getChat(tabId).blocks.flatMap(block => {
    if (!block.statusText?.startsWith(COMPACTING_STATUS_PREFIX)) return [block];
    if (isEmptyAssistant(block)) return [];
    return [{
      ...block,
      isStreaming: false,
      statusText: undefined,
      compaction: undefined,
    }];
  });
  const runId = asString(payload?.runId);
  const targetIndex = findStreamingCompaction(blocks, runId);
  const existing = targetIndex >= 0 ? blocks[targetIndex] : undefined;
  const next = {
    ...compactBlockFromPayload(payload, existing?.compaction),
    id: existing?.id ?? (runId ? `compaction:${runId}` : crypto.randomUUID()),
    isStreaming: true,
  };
  if (targetIndex >= 0) {
    blocks[targetIndex] = next;
  } else {
    blocks.push(next);
  }
  store.setBlocks(tabId, blocks);
}

export function clearCompactionBlock(tabId: string) {
  const store = useChatStore.getState();
  const blocks = store.getChat(tabId).blocks.flatMap(block => {
    if (block.type === "compact" && block.isStreaming) return [];
    if (!block.statusText?.startsWith(COMPACTING_STATUS_PREFIX)) return [block];
    if (isEmptyAssistant(block)) return [];
    return [{
      ...block,
      isStreaming: false,
      statusText: undefined,
      compaction: undefined,
      usage: undefined,
    }];
  });
  store.setBlocks(tabId, blocks);
}

function completeCompactionBlock(tabId: string, payload: any) {
  const store = useChatStore.getState();
  const blocks = [...store.getChat(tabId).blocks];
  const runId = asString(payload?.runId);
  const streamingIndex = findStreamingCompaction(blocks, runId);
  if (streamingIndex >= 0) {
    const existing = blocks[streamingIndex];
    blocks[streamingIndex] = {
      ...compactBlockFromPayload(
        {
          ...payload,
          phase: asString(payload?.phase) ?? "complete",
        },
        existing.compaction,
      ),
      id: existing.id,
      isStreaming: false,
    };
    store.setBlocks(tabId, blocks);
    return;
  }
  blocks.push({
    ...compactBlockFromPayload({
      ...payload,
      phase: asString(payload?.phase) ?? "complete",
    }),
    isStreaming: false,
  });
  store.setBlocks(tabId, blocks);
}

export function applyCompactionEventToStores(tabId: string, event: WorkflowEvent) {
  const updateTab = useTabStore.getState().updateTab;
  const payload = event.payload || {};
  const providerNative =
    payload?.providerNative === true ||
    (!asString(payload?.runId) && !asString(payload?.summary));
  if (providerNative) {
    // Older journals may contain opaque provider compact events. They carry no
    // reusable summary and must not stop tools, change run state, or leave an
    // empty card behind when replayed after a reconnect.
    const store = useChatStore.getState();
    const blocks = store.getChat(tabId).blocks.filter(block =>
      !(block.type === "compact" && block.compaction?.providerNative === true)
    );
    store.setBlocks(tabId, blocks);
    return;
  }
  if (event.kind === "system.compacting") {
    // Compaction is a provider turn boundary. Any tool still shown as running
    // belongs to a replay gap or a missed terminal event, not to the summary
    // operation itself. Close it before rendering the durable checkpoint.
    const runId = asString(payload?.runId);
    const alreadyCompacting = useChatStore.getState().getChat(tabId).blocks.some(block =>
      block.type === "compact"
      && block.isStreaming
      && (!runId || block.compaction?.runId === runId)
    );
    if (!alreadyCompacting) useChatStore.getState().stopAllStreaming(tabId);
    applyCompactingBlock(tabId, payload);
    updateTab(tabId, {
      isStreaming: true,
      currentTool: `${compactionStatusLabel(payload)}...`,
    });
    return;
  }
  if (event.kind === "system.compact_complete") {
    completeCompactionBlock(tabId, payload);
    const completedBlocks = useChatStore.getState().getChat(tabId).blocks;
    const completedRunId = asString(payload?.runId);
    const completedBlock = [...completedBlocks].reverse().find(block =>
      block.type === "compact" &&
      (!completedRunId || block.compaction?.runId === completedRunId)
    );
    const queuedPrompt =
      payload.queuedPrompt === true || completedBlock?.compaction?.queuedPrompt === true;
    updateTab(tabId, {
      isStreaming: queuedPrompt,
      currentTool: undefined,
      ...(queuedPrompt
        ? {}
        : {
            isSessionWarm: false,
            requiresResume: false,
            sessionWarmSince: undefined,
          }),
    });
    return;
  }
  if (event.kind === "system.compact_failed") {
    clearCompactionBlock(tabId);
    updateTab(tabId, { isStreaming: false, currentTool: undefined });
  }
}
