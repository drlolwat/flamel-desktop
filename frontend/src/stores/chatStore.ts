import { create } from 'zustand';
import type { DisplayBlock, ToolCall, ContentSegment } from '../lib/messageProcessor';
import type { PendingQuestion } from '@flamel/shared';
import { mergeInputDelivery, type InputDelivery } from '../lib/chatInput';
import type { ReasoningReroute } from '../lib/chatRouting';
import { captureGuidanceAnchor } from '../lib/guidanceTranscript';

// Chat blocks are NOT persisted to localStorage — tool results and file content from long
// sessions easily exceed the 5MB quota, causing QuotaExceededError on the next setItem call
// (even from unrelated stores), which silently kills executeSend. History is loaded from the
// backend on demand via /api/sessions/{id}/history when a tab is reopened.
try { localStorage.removeItem('flamel-chats'); } catch { /* ignore */ }

interface TabChatState {
  blocks: DisplayBlock[];
  sessionId?: string;
  isWaiting: boolean;
  pendingQuestion: PendingQuestion | null;
  inputDeliveries?: InputDelivery[];
  reasoningReroute?: ReasoningReroute | null;
}

interface ChatStore {
  chats: Record<string, TabChatState>;
  getChat: (tabId: string) => TabChatState;
  addBlock: (tabId: string, block: DisplayBlock) => void;
  setBlocks: (tabId: string, blocks: DisplayBlock[]) => void;
  hydrateHistory: (
    tabId: string,
    sessionId: string,
    expectedBlocks: DisplayBlock[],
    blocks: DisplayBlock[],
  ) => boolean;
  updateLastBlock: (tabId: string, updates: Partial<DisplayBlock>) => void;
  updateTool: (tabId: string, toolId: string, updates: Partial<ToolCall>) => void;
  setSessionId: (tabId: string, sessionId: string) => void;
  setWaiting: (tabId: string, waiting: boolean) => void;
  setPendingQuestion: (tabId: string, question: PendingQuestion | null) => void;
  clearChat: (tabId: string) => void;
  stopAllStreaming: (tabId: string) => void;
  truncateBlocks: (tabId: string, fromIndex: number) => void;
  recordInputDelivery: (tabId: string, receipt: InputDelivery, onlyIfMissing?: boolean) => void;
  setReasoningReroute: (tabId: string, notice: ReasoningReroute | null) => void;
}

const defaultState = (): TabChatState => ({
  blocks: [],
  isWaiting: false,
  pendingQuestion: null,
  inputDeliveries: [],
  reasoningReroute: null,
});

function shallowRecordEqual(a: Record<string, unknown>, b: Record<string, unknown>): boolean {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const key of keys) {
    if (!Object.is(a[key], b[key])) return false;
  }
  return true;
}

function shallowBlockEqual(a: DisplayBlock, b: DisplayBlock): boolean {
  return shallowRecordEqual(a as unknown as Record<string, unknown>, b as unknown as Record<string, unknown>);
}

function sameArrayItems<T>(existing?: T[], incoming?: T[]): boolean {
  if (existing === incoming) return true;
  if (!existing || !incoming) return false;
  if (existing.length !== incoming.length) return false;
  return existing.every((item, index) => Object.is(item, incoming[index]));
}

function mergeToolCalls(existing?: ToolCall[], incoming?: ToolCall[]): ToolCall[] | undefined {
  if (!incoming) return existing;
  if (!existing?.length) return [...incoming];

  let changed = false;
  const merged = [...existing];
  const indexById = new Map(merged.map((tool, index) => [tool.id, index]));
  for (const tool of incoming) {
    const existingIndex = indexById.get(tool.id);
    if (existingIndex === undefined) {
      indexById.set(tool.id, merged.length);
      merged.push(tool);
      changed = true;
      continue;
    }
    const current = merged[existingIndex];
    const terminal = current.status === 'done' || current.status === 'error';
    const replayingActive = tool.status === 'pending' || tool.status === 'running' || tool.status === 'unknown';
    const mergedTool = {
      ...current,
      ...tool,
      ...(terminal && replayingActive ? { status: current.status } : {}),
    };
    if (!shallowRecordEqual(merged[existingIndex] as unknown as Record<string, unknown>, mergedTool as unknown as Record<string, unknown>)) {
      merged[existingIndex] = mergedTool;
      changed = true;
    }
  }
  return changed ? merged : existing;
}

function mergeTextValue(existing?: string, incoming?: string): string | undefined {
  if (incoming === undefined) return existing;
  return incoming;
}

function mergeSegments(existing?: ContentSegment[], incoming?: ContentSegment[]): ContentSegment[] | undefined {
  if (!incoming) return existing;
  if (sameArrayItems(existing, incoming)) return existing;
  return [...incoming];
}

function mergeBlock(existing: DisplayBlock, updates: Partial<DisplayBlock>): DisplayBlock {
  const next = existing.type !== 'assistant'
    ? { ...existing, ...updates }
    : {
        ...existing,
        ...updates,
        content: mergeTextValue(existing.content, updates.content),
        thinking: mergeTextValue(existing.thinking, updates.thinking),
        tools: mergeToolCalls(existing.tools, updates.tools),
        segments: mergeSegments(existing.segments, updates.segments),
      };
  return shallowBlockEqual(existing, next) ? existing : next;
}

export const useChatStore = create<ChatStore>()(
  (set, get) => ({
  chats: {},

  getChat: (tabId) => get().chats[tabId] || defaultState(),

  addBlock: (tabId, block) => {
    set(s => ({
      chats: {
        ...s.chats,
        [tabId]: {
          ...(s.chats[tabId] || defaultState()),
          blocks: [...(s.chats[tabId]?.blocks || []), block],
        },
      },
    }));
  },

  setBlocks: (tabId, blocks) => {
    set(s => ({
      chats: {
        ...s.chats,
        [tabId]: { ...(s.chats[tabId] || defaultState()), blocks },
      },
    }));
  },

  hydrateHistory: (tabId, sessionId, expectedBlocks, blocks) => {
    let applied = false;
    set(s => {
      const chat = s.chats[tabId];
      // Bootstrap can finish after a send or stream update, or begin while
      // an optimistic opening prompt is still being saved by the server.
      // Neither snapshot may replace the newer transcript. Compare references
      // instead of message text: identical prompts can be separate real turns.
      if (
        !chat || chat.sessionId !== sessionId
        || chat.blocks !== expectedBlocks
        || expectedBlocks.some(block => block.isStreaming)
      ) return s;
      applied = true;
      return { chats: { ...s.chats, [tabId]: { ...chat, blocks } } };
    });
    return applied;
  },

  updateLastBlock: (tabId, updates) => {
    set(s => {
      const chat = s.chats[tabId] || defaultState();
      const lastBlock = chat.blocks[chat.blocks.length - 1];
      if (!lastBlock) return s;
      const nextBlock = mergeBlock(lastBlock, updates);
      if (nextBlock === lastBlock) return s;
      const blocks = [...chat.blocks];
      blocks[blocks.length - 1] = nextBlock;
      return { chats: { ...s.chats, [tabId]: { ...chat, blocks } } };
    });
  },

  updateTool: (tabId, toolId, updates) => {
    set(s => {
      const chat = s.chats[tabId] || defaultState();
      const blocks = [...chat.blocks];
      if (blocks.length === 0) return s;

      for (let blockIndex = blocks.length - 1; blockIndex >= 0; blockIndex -= 1) {
        const block = blocks[blockIndex];
        const toolIndex = block.tools?.findIndex(tool => tool.id === toolId) ?? -1;
        if (toolIndex < 0 || !block.tools) continue;

        const currentTool = block.tools[toolIndex];
        const terminal = currentTool.status === 'done' || currentTool.status === 'error';
        const replayingActive = updates.status === 'pending' || updates.status === 'running' || updates.status === 'unknown';
        const nextTool = {
          ...currentTool,
          ...updates,
          ...(terminal && replayingActive ? { status: currentTool.status } : {}),
        };
        if (shallowRecordEqual(
          currentTool as unknown as Record<string, unknown>,
          nextTool as unknown as Record<string, unknown>,
        )) return s;

        const tools = [...block.tools];
        tools[toolIndex] = nextTool;
        blocks[blockIndex] = { ...block, tools };
        return { chats: { ...s.chats, [tabId]: { ...chat, blocks } } };
      }

      return s;
    });
  },

  setSessionId: (tabId, sessionId) => {
    set(s => {
      const current = s.chats[tabId] || defaultState();
      if (current.sessionId === sessionId) return s;
      return {
        chats: {
          ...s.chats,
          [tabId]: {
            ...current,
            sessionId,
            // A tab without a session id can still contain stale presentation
            // state after hydration or a failed bootstrap. Session identity is
            // authoritative, so every new binding starts from a clean slate.
            blocks: [],
            isWaiting: false,
            pendingQuestion: null,
            inputDeliveries: [],
            reasoningReroute: null,
          },
        },
      };
    });
  },

  setWaiting: (tabId, waiting) => {
    set(s => ({
      chats: {
        ...s.chats,
        [tabId]: { ...(s.chats[tabId] || defaultState()), isWaiting: waiting },
      },
    }));
  },

  setPendingQuestion: (tabId, question) => {
    set(s => {
      const chat = s.chats[tabId] || defaultState();
      const current = chat.pendingQuestion;
      if (current === question) return s;
      if (
        current &&
        question &&
        current.requestId === question.requestId
      ) {
        return s;
      }
      return {
        chats: {
          ...s.chats,
          [tabId]: { ...chat, pendingQuestion: question },
        },
      };
    });
  },

  clearChat: (tabId) => {
    set(s => ({ chats: { ...s.chats, [tabId]: defaultState() } }));
  },

  recordInputDelivery: (tabId, receipt, onlyIfMissing = false) => {
    set(s => {
      const chat = s.chats[tabId];
      if (!chat || chat.sessionId !== receipt.sessionId) return s;
      const inputDeliveries = mergeInputDelivery(chat.inputDeliveries || [], {
        ...receipt,
        transcriptAnchor: chat.inputDeliveries?.find(item => item.requestId === receipt.requestId)?.transcriptAnchor
          ?? captureGuidanceAnchor(chat.blocks),
      }, onlyIfMissing);
      if (inputDeliveries === chat.inputDeliveries) return s;
      return { chats: { ...s.chats, [tabId]: { ...chat, inputDeliveries } } };
    });
  },

  setReasoningReroute: (tabId, notice) => {
    set(s => {
      const chat = s.chats[tabId];
      if (!chat || (notice && chat.sessionId !== notice.sessionId)) return s;
      if (JSON.stringify(chat.reasoningReroute ?? null) === JSON.stringify(notice)) return s;
      return { chats: { ...s.chats, [tabId]: { ...chat, reasoningReroute: notice } } };
    });
  },

  truncateBlocks: (tabId, fromIndex) => {
    set(s => {
      const chat = s.chats[tabId];
      if (!chat) return s;
      return { chats: { ...s.chats, [tabId]: { ...chat, blocks: chat.blocks.slice(0, fromIndex) } } };
    });
  },

  stopAllStreaming: (tabId) => {
    set(s => {
      const chat = s.chats[tabId];
      if (!chat) return s;
      return {
        chats: {
          ...s.chats,
          [tabId]: {
            ...chat,
            blocks: chat.blocks.map(b => {
              const hasStuckTools = b.tools?.some(t => t.status === 'running' || t.status === 'pending');
              if (!b.isStreaming && !hasStuckTools) return b;
              const tools = b.tools?.map(t =>
                // Ending a browser stream is not evidence that the tool succeeded.
                (t.status === 'running' || t.status === 'pending') ? { ...t, status: 'unknown' as const } : t
              );
              return { ...b, isStreaming: false, tools };
            }),
          },
        },
      };
    });
  },
  }),
);
