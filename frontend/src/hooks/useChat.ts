import { useCallback, useEffect, useRef, useState } from "react";
import { useChatStore } from "../stores/chatStore";
import { usePlanStore } from "../stores/planStore";
import {
  isAutonomousOrchestrationTab,
  useTabStore,
  type ModelId,
  type ReasoningEffort,
  type ReasoningSelection,
} from "../stores/tabStore";
import {
  appendThinkingSegment,
  extractHiddenControlSignal,
  extractPlanItems,
  extractPlanFromWrite,
  stripHiddenControlMarkers,
} from "../lib/messageProcessor";
import { processHistoryMessages } from "../lib/historyProcessor";
import { isCommandEventPayload, attachPendingCommandEvents, attachCommandEventToTool, applyStreamingToolResult, shouldStartAssistantBlockAfterToolResult, toolFromAssistantSnapshot } from "../lib/chatToolEvents";
import type { ToolResultEventLike } from "../lib/toolResultReducer";
import { isAcceptedDuplicate, parseInputDelivery } from "../lib/chatInput";
import { parseReasoningReroute, routingTabUpdates } from "../lib/chatRouting";
import { manualModelSelection } from "../lib/models";
import {
  completedQuestionRequestId,
  normalizeWorkflowEvent,
  normalizePlanInputItems,
  pendingQuestionFromWorkflowEvent,
  parsePartialPlanInput,
  parsePlanMarkerOutput,
  planMetaFromMarker,
  planToolAction,
  type WorkflowEvent,
} from "../lib/workflowEvents";
import {
  applyCompactionEventToStores,
  clearCompactionBlock,
} from "../lib/compactionEvents";
import { resolveExecutionSessionId } from "../lib/sessionIdentity";
import {
  hasQueuedUserBlock,
  reconcileQueuedOptimisticBlocks,
  visibleQueuedMessages,
  type VisibleQueuedMessage,
} from "../lib/queuedTranscript";
import {
  maybeRefreshProjectFilesFromTool,
  maybeRefreshProjectFilesFromWorkflowEvent,
} from "../lib/projectFileEvents";
import { useAppStore } from "../stores/appStore";
import { api, apiFetch, errorMessageFromUnknown } from "../lib/api";
import { openRealtimeSessionStream, REALTIME_SNAPSHOT_COMPLETE, REALTIME_REPLAY_COMPLETE } from "../lib/realtimeClient";
import {
  shouldApplyRealtimeEventToRun,
  shouldApplyRealtimeEventToSession,
  shouldRunRealtimeSideEffects,
} from "../lib/realtimeReplay";
import {
  applyRealtimeContinuityEvent,
  applyRealtimeQueueEvent,
  chatRuntime,
  countUserMessagesBefore,
  notifySessionSnapshotReady,
  type QueuedMessage,
  type SendOptions,
  type SessionStreamOwner,
} from "../lib/chatRuntime";
import type {
  DisplayBlock,
  HiddenControlDirective,
  ToolCall,
  ContentSegment,
} from "../lib/messageProcessor";
import type { CommandEvent } from "../lib/consoleTypes";
import type { PlanItem } from "@flamel/shared";


function makeTitle(message: string): string {
  const t = message.trim().replace(/\s+/g, " ");
  return t.length > 50 ? t.slice(0, 47) + "..." : t;
}

const AUTO_CONTINUE_STOP_REASONS = new Set([
  "max_tokens",
  "process_died",
  "max_turns",
  "turn_limit",
  "turn_limit_reached",
  "context_window_exceeded",
  "length",
  "incomplete",
]);

// Hard safety cap for hidden follow-up turns so bad prompts cannot loop forever.
const MAX_AUTO_CONTINUATIONS = 3;

function isCodexModel(model?: string): boolean {
  return !!model && (/^gpt-/i.test(model) || /^o[1-9]/i.test(model));
}

function looksLikeQuestionForUser(content: string): boolean {
  const trimmed = content.trim();
  return (
    trimmed.endsWith("?") ||
    /\b(should i|would you like|do you want|let me know|please confirm)\b/i.test(
      trimmed,
    )
  );
}

function looksLikePrematureCodexHandoff(content: string): boolean {
  const tail = content.trim().slice(-1500);
  return [
    /\btell me (?:if|when) you(?:'d)? want me to (?:continue|keep going|proceed)\b/i,
    /\blet me know (?:if|when) you(?:'d)? want me to (?:continue|keep going|proceed)\b/i,
    /\bif you(?:'d)? like me to (?:continue|keep going|proceed)\b/i,
    /\bif you want me to (?:continue|keep going|proceed)\b/i,
    /\bwhen you(?:'re| are) ready,? I can (?:continue|keep going|proceed)\b/i,
    /\b(?:say|reply with)\s+["']?continue["']?\b/i,
    /\bI can continue from here\b/i,
    /\bI can keep going\b/i,
  ].some((pattern) => pattern.test(tail));
}

function looksLikeStallRecap(content: string): boolean {
  const normalized = content.trim().toLowerCase();
  if (!normalized || normalized.endsWith("?")) return false;

  const stallActions = [
    "i'm checking",
    "i am checking",
    "checking the",
    "i'm loading",
    "i am loading",
    "loading the",
    "i'm reading",
    "i am reading",
    "reading the",
    "i'm tracing",
    "i am tracing",
    "tracing the",
    "i'm looking at",
    "i am looking at",
    "re-reading the",
    "reopening the",
    "pulling the",
    "reviewing the",
    "scanning the",
  ];
  const stallTargets = [
    "active plan",
    "plan panel",
    "plan items",
    "repo state",
    "current repo",
    "repo contents",
    "existing code",
    "codebase",
    "entrypoints",
    "project state",
  ];

  return (
    stallActions.some((marker) => normalized.includes(marker)) &&
    stallTargets.some((marker) => normalized.includes(marker))
  );
}

function looksLikePartialProgressRecap(content: string): boolean {
  const trimmed = content.trim();
  if (!trimmed || trimmed.endsWith("?")) return false;

  const normalized = trimmed.toLowerCase();
  if (looksLikeStallRecap(trimmed)) return false;

  const blockingMarkers = [
    "full plan is in the plan panel",
    "wait for explicit user approval",
    "need your approval",
    "please choose",
    "what would you like",
    "do you want",
    "would you like",
    "can you confirm",
    "which option",
  ];
  if (blockingMarkers.some((marker) => normalized.includes(marker)))
    return false;

  const terminalMarkers = [
    "all done",
    "done.",
    "done and",
    "completed the task",
    "finished the task",
    "implemented everything",
    "nothing else left",
    "all plan items are done",
    "committed and pushed",
    "pushed the change",
    "pushed to ",
  ];
  if (terminalMarkers.some((marker) => normalized.includes(marker)))
    return false;

  const progressMarkers = [
    "implemented ",
    "completed ",
    "finished ",
    "verified ",
    "fixed ",
    "added ",
    "updated ",
    "wrote ",
    "ran ",
    "passes",
    "passed",
    "are in place",
    "i also fixed",
  ];
  const recapMarkers = [
    "plan item",
    "test suite",
    "integrations",
    "coverage",
    "unittest",
    "vitest",
    "sdk",
    "verified them",
  ];
  const unfinishedMarkers = [
    "remaining",
    "still need",
    "next i",
    "next up",
    "moving on to",
    "continuing with",
    "resume with",
    "left to do",
  ];

  const hasProgressMarker = progressMarkers.some((marker) =>
    normalized.includes(marker),
  );
  const hasRecapMarker = recapMarkers.some((marker) =>
    normalized.includes(marker),
  );
  const hasUnfinishedMarker = unfinishedMarkers.some((marker) =>
    normalized.includes(marker),
  );

  return hasProgressMarker && (hasRecapMarker || hasUnfinishedMarker);
}

function shouldAutoContinueResponse(
  stopReason: string | undefined,
  content: string,
  model: string | undefined,
  continuationCount: number,
): boolean {
  if (continuationCount >= MAX_AUTO_CONTINUATIONS) return false;
  if (AUTO_CONTINUE_STOP_REASONS.has(stopReason || "")) return true;
  if (!isCodexModel(model)) return false;
  if (looksLikeQuestionForUser(content)) return false;
  return looksLikePrematureCodexHandoff(content);
}

function shouldAutoContinuePlanRecap(
  content: string,
  model: string | undefined,
  hasUnfinishedPlan: boolean,
  continuationCount: number,
): boolean {
  if (continuationCount >= MAX_AUTO_CONTINUATIONS) return false;
  if (!hasUnfinishedPlan || !isCodexModel(model)) return false;
  if (looksLikeQuestionForUser(content)) return false;
  return looksLikePartialProgressRecap(content);
}

function continuationLabel(
  stopReason: string | undefined,
  continuationCount: number,
): string {
  if (stopReason === "process_died") {
    return `Process died — restarting (${continuationCount})…`;
  }
  if (stopReason === "partial_progress_recap") {
    return `Continuing unfinished work (${continuationCount})…`;
  }
  if (stopReason === "native_plan_rejected") {
    return `Retrying with Flamel Plan (${continuationCount})…`;
  }
  return `Continuing response (${continuationCount})…`;
}

function continuationPrompt(
  stopReason: string | undefined,
  model: string | undefined,
): string {
  if (stopReason === "partial_progress_recap") {
    return "Continue from where you left off. For an active plan, mark the current item done immediately when its own acceptance checks pass, before any work on the next item; then mark the next item in_progress. Do not defer completed statuses until the end, restate progress, or restart from an earlier user prompt.";
  }
  if (stopReason === "native_plan_rejected") {
    return 'Your Codex-native update_plan call was rejected because it does not persist to Flamel. Use the available Flamel Plan MCP now: call plan(action="list"), then create or update the correct session plan. Do not claim success until the Flamel plan tool returns a non-error PLAN_CREATED or PLAN_UPDATED marker.';
  }
  if (
    isCodexModel(model) &&
    !AUTO_CONTINUE_STOP_REASONS.has(stopReason || "")
  ) {
    return "Continue from where you left off. Do not repeat what you already wrote or restart from an earlier user prompt.";
  }
  return "Continue from where you left off.";
}

function coerceRecoveryNote(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed || undefined;
}

function coerceRecoverySummary(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed || undefined;
}

export function useChat(tabId: string) {
  const abortRef = chatRuntime.abortRef(tabId);
  const runtime = chatRuntime.forTab(tabId);
  const [queuedMessages, setQueuedMessages] = useState<VisibleQueuedMessage[]>(
    () => visibleQueuedMessages(runtime.queue),
  );
  const [queuedCount, setQueuedCount] = useState(
    () => visibleQueuedMessages(runtime.queue).length,
  );
  const [runtimeActive, setRuntimeActive] = useState(() => runtime.active);

  const syncQueueState = useCallback(() => {
    const visible = visibleQueuedMessages(runtime.queue);
    setQueuedMessages(visible);
    setQueuedCount(visible.length);
    setRuntimeActive(runtime.active);
  }, [runtime]);

  useEffect(() => {
    runtime.subscribers.add(syncQueueState);
    syncQueueState();
    return () => {
      runtime.subscribers.delete(syncQueueState);
      if (
        runtime.subscribers.size === 0 &&
        !runtime.active &&
        runtime.queue.length === 0
      ) {
        chatRuntime.cleanupTab(tabId, runtime);
      }
    };
  }, [runtime, syncQueueState, tabId]);
  // Ref to subscribeToLiveStream — avoids circular dep between executeSend and subscribeToLiveStream
  const subscribeToLiveStreamRef = useRef<
    | ((
        sessionId: string,
        opts?: {
          onHistoryReady?: () => void;
          historyLoaded?: boolean;
          signal?: AbortSignal;
          followQueuedRuns?: boolean;
          requestId?: string;
          bindingGeneration?: number;
          persistent?: boolean;
        },
      ) => Promise<boolean>)
    | null
  >(null);
  const planRefreshGenerationRef = useRef(0);
  const addBlock = useChatStore((state) => state.addBlock);
  const setBlocks = useChatStore((state) => state.setBlocks);
  const updateLastBlock = useChatStore((state) => state.updateLastBlock);
  const updateTool = useChatStore((state) => state.updateTool);
  const setSessionId = useChatStore((state) => state.setSessionId);
  const setWaiting = useChatStore((state) => state.setWaiting);
  const setPendingQuestion = useChatStore((state) => state.setPendingQuestion);
  const stopAllStreaming = useChatStore((state) => state.stopAllStreaming);
  const getChat = useChatStore((state) => state.getChat);
  const updatePlan = usePlanStore((state) => state.updatePlan);
  const setPlan = usePlanStore((state) => state.setPlan);
  const updateItem = usePlanStore((state) => state.updateItem);
  const addItems = usePlanStore((state) => state.addItems);
  const removeItem = usePlanStore((state) => state.removeItem);
  const getPlan = usePlanStore((state) => state.getPlan);
  const setFinished = usePlanStore((state) => state.setFinished);
  const setExecutionMeta = usePlanStore((state) => state.setExecutionMeta);
  const setStreamingIds = usePlanStore((state) => state.setStreamingIds);
  const clearStreamingIds = usePlanStore((state) => state.clearStreamingIds);
  const clearPlan = usePlanStore((state) => state.clearPlan);
  const updateTab = useTabStore((state) => state.updateTab);
  const openPlanPanel = useAppStore((state) => state.openPlanPanel);
  const refreshSessions = useAppStore((state) => state.refreshSessions);

  function applyInputDelivery(value: unknown, sessionId: string): boolean {
    const receipt = parseInputDelivery(value, sessionId);
    if (!receipt) return false;
    useChatStore.getState().recordInputDelivery(tabId, receipt);
    return true;
  }

  function applyReasoningReroute(value: unknown, sessionId: string): boolean {
    const notice = parseReasoningReroute(value, sessionId);
    if (!notice) return false;
    useChatStore.getState().setReasoningReroute(tabId, notice);
    return true;
  }

  function applyModelRouting(routing: unknown, requestId: string | undefined) {
    updateTab(tabId, routingTabUpdates(routing));
    const store = useChatStore.getState();
    if (store.getChat(tabId).reasoningReroute?.requestId === requestId) {
      store.setReasoningReroute(tabId, null);
    }
  }

  function clearPendingQuestionRequest(requestId: unknown) {
    if (typeof requestId !== "string" || !requestId) return;
    const current = useChatStore.getState().getChat(tabId).pendingQuestion;
    if (current?.requestId === requestId) setPendingQuestion(tabId, null);
  }

  function applyQuestionWorkflowEvent(event: WorkflowEvent | null): boolean {
    const pending = pendingQuestionFromWorkflowEvent(event);
    if (pending) {
      setPendingQuestion(tabId, pending);
      return true;
    }
    const completedRequestId = completedQuestionRequestId(event);
    if (completedRequestId) {
      clearPendingQuestionRequest(completedRequestId);
      return true;
    }
    return false;
  }

  function persistActivePlan(planId: string) {
    const sessionId = useTabStore
      .getState()
      .tabs.find((tab) => tab.id === tabId)?.sessionId;
    if (sessionId)
      void api.sessions.setActivePlan(sessionId, planId).catch(() => null);
  }

  function refreshAuthoritativePlan(planId: string) {
    const generation = ++planRefreshGenerationRef.current;
    void api.plans
      .get(planId)
      .then((plan) => {
        if (generation !== planRefreshGenerationRef.current) return;
        const activePlanId = useTabStore
          .getState()
          .tabs.find((tab) => tab.id === tabId)?.activePlanId;
        if (activePlanId && activePlanId !== planId) return;
        setPlan(
          tabId,
          plan.id,
          plan.title,
          plan.description,
          plan.items,
          false,
          {
            artifact: plan.artifact,
            versions: plan.versions,
            finished: plan.finished,
            executionStatus: plan.executionStatus,
            approvalStatus: plan.approvalStatus,
            activeItemId: plan.activeItemId,
            approvalRequestedAt: plan.approvalRequestedAt,
            approvedAt: plan.approvedAt,
            executionStartedAt: plan.executionStartedAt,
            executionUpdatedAt: plan.executionUpdatedAt,
          },
        );
      })
      .catch(() => null);
  }

  function applyPlanWorkflowEvent(
    event: WorkflowEvent | null,
    toolPlanAct: string | null,
    toolInput: any,
    options?: { dispatchCreatedEvent?: boolean },
  ): boolean {
    if (!event || !event.kind.startsWith("plan.")) return false;
    const json = event.payload || {};

    if (event.kind === "plan.created" && json?.planId) {
      const currentPlan = getPlan(tabId);
      setPlan(
        tabId,
        json.planId,
        json.title,
        json.description,
        Array.isArray(json.items) ? json.items : currentPlan.items,
        true,
        planMetaFromMarker(json),
      );
      clearStreamingIds(tabId);
      updateTab(tabId, { activePlanId: json.planId });
      openPlanPanel();
      refreshAuthoritativePlan(json.planId);
      if (options?.dispatchCreatedEvent) {
        window.dispatchEvent(new CustomEvent("flamel:plan-created"));
      }
      return true;
    }

    if (event.kind === "plan.updated" && json?.planId) {
      const currentPlan = getPlan(tabId);
      setPlan(
        tabId,
        json.planId,
        json.title || currentPlan.title || "",
        json.description || currentPlan.description,
        Array.isArray(json.items) ? json.items : currentPlan.items,
        true,
        planMetaFromMarker(json),
      );
      clearStreamingIds(tabId);
      updateTab(tabId, { activePlanId: json.planId });
      refreshAuthoritativePlan(json.planId);
      return true;
    }

    if (event.kind === "plan.renamed" && json?.planId && json.title) {
      const currentPlan = getPlan(tabId);
      setPlan(
        tabId,
        json.planId,
        json.title,
        currentPlan.description,
        currentPlan.items,
      );
      refreshAuthoritativePlan(json.planId);
      return true;
    }

    if (event.kind === "plan.loaded" && json?.planId && json.items) {
      setPlan(
        tabId,
        json.planId,
        json.title,
        json.description,
        json.items,
        false,
        planMetaFromMarker(json),
      );
      updateTab(tabId, { activePlanId: json.planId });
      persistActivePlan(json.planId);
      refreshAuthoritativePlan(json.planId);
      return true;
    }

    if (event.kind === "plan.execution_updated" && json?.planId) {
      const currentPlan = getPlan(tabId);
      setPlan(
        tabId,
        json.planId,
        json.title || currentPlan.title || "",
        json.description || currentPlan.description,
        Array.isArray(json.items) ? json.items : currentPlan.items,
        false,
        planMetaFromMarker(json),
      );
      updateTab(tabId, { activePlanId: json.planId });
      refreshAuthoritativePlan(json.planId);
      return true;
    }

    if (event.kind === "plan.item_updated" && json?.itemId) {
      const updates: Partial<PlanItem> = {};
      if (json.status) updates.status = json.status;
      if (toolInput?.text !== undefined) updates.text = json.text;
      if (json.notes !== undefined) updates.notes = json.notes;
      if (json.section !== undefined) updates.section = json.section;
      if (json.reasoningFloor !== undefined)
        updates.reasoningFloor = json.reasoningFloor;
      updateItem(tabId, json.itemId, updates);
      setExecutionMeta(tabId, planMetaFromMarker(json));
      if (json.planId) refreshAuthoritativePlan(json.planId);
      return true;
    }

    if (event.kind === "plan.finished") {
      setFinished(tabId, true);
      if (json.planId) refreshAuthoritativePlan(json.planId);
      return true;
    }

    if (event.kind === "plan.item_deleted" && json?.itemId) {
      removeItem(tabId, json.itemId);
      if (json.planId) refreshAuthoritativePlan(json.planId);
      return true;
    }

    if (
      event.kind === "plan.items_added" &&
      Array.isArray(json.items) &&
      json.items.length > 0
    ) {
      const currentPlan = getPlan(tabId);
      const realItems: PlanItem[] = json.items.map((item: any) => ({
        id: item.id,
        text: item.text,
        status: item.status as PlanItem["status"],
        position: item.position,
        notes: item.notes,
        section: item.section,
        reasoningFloor: item.reasoningFloor || "medium",
      }));
      const withoutPending = currentPlan.items.filter(
        (item) => !item.id.startsWith("pending_"),
      );
      setPlan(
        tabId,
        json.planId || currentPlan.planId || "pending",
        json.title || currentPlan.title || "",
        json.description || currentPlan.description,
        [...withoutPending, ...realItems],
        true,
        planMetaFromMarker(json),
      );
      clearStreamingIds(tabId);
      if (json.planId) refreshAuthoritativePlan(json.planId);
      return true;
    }

    if (toolPlanAct === "mark_finished") {
      setFinished(tabId, true);
      return true;
    }
    return false;
  }

  function syncStreamingPlanHeader(
    parsed: ReturnType<typeof parsePartialPlanInput>,
  ) {
    if (
      !parsed ||
      (parsed.action !== "create" && parsed.action !== "replace_items")
    )
      return;
    if (!parsed.title && parsed.description === undefined) return;

    const currentPlan = getPlan(tabId);
    const planId =
      parsed.action === "create" ? "pending" : currentPlan.planId || "pending";
    const title =
      parsed.action === "create"
        ? parsed.title || currentPlan.title || "Preparing plan..."
        : currentPlan.title || parsed.title || "";
    const description = parsed.description ?? currentPlan.description;

    if (
      currentPlan.planId === planId &&
      currentPlan.title === title &&
      currentPlan.description === description
    )
      return;

    setPlan(tabId, planId, title, description, currentPlan.items);
    openPlanPanel();
  }

  const executeSend = useCallback(
    async (message: string, workingDirectory: string, options: SendOptions) => {
      // Outer guard: any error thrown BEFORE the inner try/finally would leave runtime.active=true forever.
      // This ensures cleanup always runs.
      try {
        const currentTab = useTabStore
          .getState()
          .tabs.find((tab) => tab.id === tabId);
        const currentChat = useChatStore.getState().chats[tabId];
        options = {
          ...options,
          sessionId: resolveExecutionSessionId(
            options.sessionId,
            currentTab?.sessionId,
            currentChat?.sessionId,
            options.__forceNewSession,
          ),
        };
        // A durable identity must exist before the request leaves the browser.
        // If the POST response is lost, every device can still attach to the
        // accepted run by this session id without submitting the prompt again.
        if (!options.sessionId) {
          options.sessionId = crypto.randomUUID();
          setSessionId(tabId, options.sessionId);
          updateTab(tabId, { sessionId: options.sessionId });
        }
        const requestId = crypto.randomUUID();
        const isFirstMessage = getChat(tabId).blocks.length === 0;
        const isContinuation = !!options.__isContinuation;
        const shouldReuseAssistantBlock =
          isContinuation && !!options.__reuseAssistantBlock;
        const optimisticBlockIds = new Set<string>();
        let pendingRecoveryNote = coerceRecoveryNote(options.__recoveryNote);
        let pendingRecoverySummary = coerceRecoverySummary(
          options.__recoverySummary,
        );
        const title = makeTitle(message);

        // Auto-title the tab from the first message
        if (isFirstMessage) {
          updateTab(tabId, { title });
        }

        if (!isContinuation) {
          // Add user block (skip for auto-continuations — they're invisible)
          const userBlock: DisplayBlock = {
            id: crypto.randomUUID(),
            requestId,
            role: "user",
            type: "user",
            content: message,
            images: options.images,
          };
          optimisticBlockIds.add(userBlock.id);
          addBlock(tabId, userBlock);
        }

        let reusedAssistantBlock: DisplayBlock | undefined;
        if (shouldReuseAssistantBlock) {
          const existingBlocks = getChat(tabId).blocks;
          const lastBlock = existingBlocks[existingBlocks.length - 1];
          if (lastBlock?.role === "assistant") {
            reusedAssistantBlock = lastBlock;
          }
          // Same-turn/manual continuation: resume the current assistant block in place.
          updateLastBlock(tabId, {
            isStreaming: true,
            ...(pendingRecoveryNote
              ? { recoveryNote: pendingRecoveryNote }
              : {}),
            ...(pendingRecoverySummary
              ? { recoverySummary: pendingRecoverySummary }
              : {}),
          });
        } else {
          // Fresh turns, including queued auto-continuations, get their own assistant block.
          const assistantBlockId = crypto.randomUUID();
          optimisticBlockIds.add(assistantBlockId);
          addBlock(tabId, {
            id: assistantBlockId,
            role: "assistant",
            type: "assistant",
            content: "",
            tools: [],
            isStreaming: true,
            isStarted: false,
            model: options.model,
            recoveryNote: pendingRecoveryNote,
            recoverySummary: pendingRecoverySummary,
          });
        }

        function applyRecoveryNotice(note: unknown, summary: unknown) {
          const nextNote = coerceRecoveryNote(note);
          const nextSummary = coerceRecoverySummary(summary);
          if (!nextNote && !nextSummary) return;
          if (nextNote) pendingRecoveryNote = nextNote;
          if (nextSummary) pendingRecoverySummary = nextSummary;
          const blocks = getChat(tabId).blocks;
          if (blocks[blocks.length - 1]?.role === "assistant") {
            updateLastBlock(tabId, {
              ...(nextNote ? { recoveryNote: nextNote } : {}),
              ...(nextSummary ? { recoverySummary: nextSummary } : {}),
            });
          }
        }

        updateTab(tabId, {
          isStreaming: true,
          streamingStartedAt: Date.now(),
          requestId,
          currentTool: undefined,
          isSessionWarm: false,
          requiresResume: false,
          sessionWarmSince: undefined,
        });
        setWaiting(tabId, false);
        setPendingQuestion(tabId, null);

        let receivedDone = false;
        let serverQueuePending = false;
        let admittedAsQueued = false;
        let userAborted = false;
        let handoffToRealtime = false;
        let lastStopReason: string | null = null;
        let flushTimer: ReturnType<typeof setTimeout> | null = null;
        // Declared outside try so finally can cancel — prevents browser HTTP connection slot leak.
        // Without this, the ReadableStreamDefaultReader holds the connection slot open after the
        // stream ends, exhausting the 6-connection browser limit on the same origin.
        let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
        try {
          const controller = new AbortController();

          // Set abort handler BEFORE the fetch so the stop button works even when
          // the request is stuck in the browser's connection queue (abortRef would
          // otherwise be null until the response headers arrive, making stop a no-op).
          abortRef.current = () => {
            userAborted = true;
            // Cancel reader directly — controller.abort() alone doesn't interrupt
            // an in-progress ReadableStream read in WebKitGTK.
            reader?.cancel().catch(() => {});
            controller.abort();
            // /api/abort sends SIGINT to Claude — it finishes gracefully, keeps session warm.
            apiFetch(`/api/abort/${requestId}`, { method: "POST" }).catch(
              () => {},
            );
          };

          const res = await apiFetch("/api/chat", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              message,
              requestId,
              workingDirectory,
              sessionId: options.sessionId,
              activePlanId: useTabStore
                .getState()
                .tabs.find((tab) => tab.id === tabId)?.activePlanId,
              title: isFirstMessage ? title : undefined,
              model: options.model,
              reasoningEffort: options.reasoningEffort,
              ponytailEnabled:
                options.ponytailEnabled ??
                useTabStore.getState().tabs.find((tab) => tab.id === tabId)
                  ?.ponytailEnabled ??
                false,
              permissionMode: options.permissionMode,
              images: options.images,
              planningMode: options.planningMode,
              runnerId: options.runnerId || undefined,
              hiddenContinuation: isContinuation || undefined,
              interruptActive: options.__interruptActive || undefined,
            }),
            signal: controller.signal,
          });

          const duplicateReceipt = res.status === 409 ? await res.clone().json().catch(() => null) : null;
          const acceptedDuplicate = isAcceptedDuplicate(duplicateReceipt, requestId, options.sessionId!);
          if (!res.ok && !acceptedDuplicate) {
            const body = await res.text().catch(() => "");
            const detail = body.trim() || res.statusText || "Request failed";
            throw new Error(`HTTP ${res.status}: ${detail}`);
          }
          if (res.status === 202 || acceptedDuplicate) {
            const queuedResponse = duplicateReceipt || await res.json().catch(() => ({}));
            if (typeof queuedResponse.projectPath === "string") {
              workingDirectory = queuedResponse.projectPath;
              updateTab(tabId, { projectPath: workingDirectory, workspace: queuedResponse.workspace || undefined });
            }
            const queuedRequestId =
              typeof queuedResponse.requestId === "string"
                ? queuedResponse.requestId
                : requestId;
            // Ordinary queued prompts remain editable in the queue. Interrupts
            // have already been accepted as the user's visible next turn, so
            // retain that user row while removing the empty assistant placeholder.
            setBlocks(
              tabId,
              reconcileQueuedOptimisticBlocks(
                getChat(tabId).blocks,
                optimisticBlockIds,
                options.__interruptActive === true,
                queuedRequestId,
              ),
            );
            if (!runtime.queue.some((item) => item.id === queuedRequestId)) {
              runtime.queue.push({
                id: queuedRequestId,
                message,
                workingDirectory,
                options,
                createdAt: Date.now(),
                serverOwned: true,
                accepted: Promise.resolve(true),
              });
            }
            admittedAsQueued = true;
            serverQueuePending = true;
            chatRuntime.notify(runtime);
            return;
          }
          if (!res.body)
            throw new Error("Chat stream did not include a response body");

          reader = res.body.getReader();
          const decoder = new TextDecoder();
          let buffer = "";

          let currentThinking = reusedAssistantBlock?.thinking || "";
          let currentContent = reusedAssistantBlock?.content || "";
          const currentTools: ToolCall[] = reusedAssistantBlock?.tools
            ? [...reusedAssistantBlock.tools]
            : [];
          const currentSegments: ContentSegment[] =
            reusedAssistantBlock?.segments
              ? [...reusedAssistantBlock.segments]
              : [];
          const pendingShellCommandEvents: CommandEvent[] = [];
          const pendingToolResults = new Map<string, ToolResultEventLike>();

          function ensureAssistantBlock() {
            const blocks = getChat(tabId).blocks;
            if (blocks[blocks.length - 1]?.role === "assistant") return;

            currentContent = "";
            currentThinking = "";
            currentTools.length = 0;
            currentSegments.length = 0;
            pendingShellCommandEvents.length = 0;
            seenToolIds = new Set();
            streamingPlanState = new Map();
            sawToolResult = false;
            addBlock(tabId, {
              id: crypto.randomUUID(),
              role: "assistant",
              type: "assistant",
              content: "",
              tools: [],
              isStreaming: true,
              isStarted: false,
              model: options.model,
            });
          }

          function applyControlWorkflowEvent(
            event: WorkflowEvent | null,
          ): boolean {
            if (!event) return false;
            const payload = event.payload || {};
            if (
              maybeRefreshProjectFilesFromWorkflowEvent(event, workingDirectory)
            ) {
              return true;
            }
            if (
              event.kind === "command.event" &&
              isCommandEventPayload(payload)
            ) {
              return attachCommandEventToTool(
                payload,
                currentTools,
                pendingShellCommandEvents,
                (toolId, updates) => updateTool(tabId, toolId, updates),
              );
            }
            if (applyQuestionWorkflowEvent(event)) return true;
            if (event.kind === "recovery.notice") {
              applyRecoveryNotice(payload.note, payload.summary);
              if (payload.nativePlanRejected === true) {
                if (payload.clearStalePlan === true) {
                  clearPlan(tabId);
                  updateTab(tabId, { activePlanId: undefined });
                }
                if (payload.discardAssistantText === true) {
                  currentContent = "";
                  for (let i = currentSegments.length - 1; i >= 0; i--) {
                    if (currentSegments[i].type === "text")
                      currentSegments.splice(i, 1);
                  }
                  updateLastBlock(tabId, {
                    content: "",
                    tools: [...currentTools],
                    segments: [...currentSegments],
                  });
                }
              }
              return true;
            }
            if (event.kind === "system.thinking") {
              ensureAssistantBlock();
              const lastSeg = currentSegments[currentSegments.length - 1];
              if (
                lastSeg?.type !== "thinking" ||
                !lastSeg.redacted ||
                lastSeg.text
              ) {
                currentSegments.push({
                  type: "thinking",
                  text: "",
                  redacted: true,
                });
              }
              updateLastBlock(tabId, {
                content: currentContent,
                thinking: currentThinking || undefined,
                tools: [...currentTools],
                segments: [...currentSegments],
                statusText: undefined,
              });
              updateTab(tabId, { currentTool: "Thinking…" });
              return true;
            }
            if (event.kind === "system.compacting") {
              applyCompactionEventToStores(tabId, event);
              return true;
            }
            if (event.kind === "system.compact_complete") {
              if (flushTimer) {
                clearTimeout(flushTimer);
                flushContent();
              }
              applyCompactionEventToStores(tabId, event);
              beginPostCompactionContinuation(payload);
              return true;
            }
            if (event.kind === "system.compact_failed") {
              applyCompactionEventToStores(tabId, event);
              return true;
            }
            return false;
          }

          // Tool IDs added to currentTools (prevents duplicate rows within a turn)
          let seenToolIds = new Set<string>(currentTools.map((t) => t.id));
          // Track streaming plan tools: toolId → last item count we rendered
          let streamingPlanState = new Map<string, number>();
          // Flag: a tool_result was received, so next assistant message is a new turn → new block
          let sawToolResult = false;

          function beginPostCompactionContinuation(payload: any) {
            const blocks = getChat(tabId).blocks;
            const last = blocks[blocks.length - 1];
            const hasQueuedPrompt =
              payload?.queuedPrompt === true ||
              last?.compaction?.queuedPrompt === true;
            if (!hasQueuedPrompt) return;
            if (
              last?.type !== "compact" ||
              (payload.runId && last.compaction?.runId !== payload.runId)
            ) {
              return;
            }
            currentContent = "";
            currentThinking = "";
            currentTools.splice(0);
            currentSegments.splice(0);
            seenToolIds.clear();
            sawToolResult = false;
            addBlock(tabId, {
              id: crypto.randomUUID(),
              role: "assistant",
              type: "assistant",
              content: "",
              tools: [],
              isStreaming: true,
              isStarted: false,
              model: options.model,
            });
          }

          // Number of content blocks to skip in the next assistant snapshot (from previous blocks)
          let contentBlockOffset = 0;
          // Sub-agent nested events: buffered by runId, flushed onto the owning Agent
          // tool either live (via nested_start description match) or on tool_result
          // (via the AGENT_RUN:{uuid} marker on the output's first line).
          const nestedEventBuffers = new Map<
            string,
            Array<Record<string, unknown>>
          >();
          const toolIdByRunId = new Map<string, string>();
          const parentRunIdByRunId = new Map<string, string>();
          const nestedRunIdsByToolId = new Map<string, Set<string>>();
          const bindNestedRunToTool = (runId: string, toolId: string) => {
            toolIdByRunId.set(runId, toolId);
            const runIds =
              nestedRunIdsByToolId.get(toolId) || new Set<string>();
            runIds.add(runId);
            nestedRunIdsByToolId.set(toolId, runIds);
            for (const [childRunId, parentRunId] of parentRunIdByRunId) {
              if (parentRunId === runId && !toolIdByRunId.has(childRunId)) {
                bindNestedRunToTool(childRunId, toolId);
              }
            }
          };
          const nestedEventsForTool = (toolId: string) => {
            const runIds = nestedRunIdsByToolId.get(toolId);
            if (!runIds) return [];
            return Array.from(runIds).flatMap(
              (id) => nestedEventBuffers.get(id) || [],
            );
          };
          const mergedNestedEventsForTool = (
            toolId: string,
            existing: Array<Record<string, unknown>> = [],
          ) => {
            const seen = new Set<string>();
            return [...existing, ...nestedEventsForTool(toolId)].filter(
              (ev) => {
                const key = JSON.stringify(ev);
                if (seen.has(key)) return false;
                seen.add(key);
                return true;
              },
            );
          };

          // Throttle content updates to ~30fps so text streams in smoothly
          let contentDirty = false;
          function flushContent() {
            flushTimer = null;
            if (!contentDirty) return;
            contentDirty = false;
            updateLastBlock(tabId, {
              thinking: currentThinking || undefined,
              content: currentContent,
              tools: [...currentTools],
              segments: [...currentSegments],
              statusText: undefined,
            });
          }
          function scheduleFlush() {
            contentDirty = true;
            if (!flushTimer) flushTimer = setTimeout(flushContent, 33);
          }

          streamLoop: while (true) {
            const { done, value } = await reader.read();
            if (done) break;

            buffer += decoder.decode(value, { stream: true });
            const lines = buffer.split("\n");
            buffer = lines.pop() || "";

            for (const line of lines) {
              if (!line.trim()) continue;
              let parsed: any;
              try {
                parsed = JSON.parse(line);
              } catch {
                continue;
              }
              if (!parsed || typeof parsed !== "object") continue;

              if (applyInputDelivery(parsed, options.sessionId!)) continue;
              if (applyReasoningReroute(parsed, getChat(tabId).sessionId || options.sessionId || "")) continue;

              if (parsed.type === "ping") continue;

              if (parsed.type === "nested_agent_event") {
                const runId = parsed.runId as string | undefined;
                const parentRunId = parsed.parentRunId as string | undefined;
                const event = (parsed.event || {}) as Record<
                  string,
                  unknown
                > & { type?: string };
                if (!runId) continue;
                const eventWithRunId = {
                  ...event,
                  run_id: runId,
                  ...(parentRunId ? { parent_run_id: parentRunId } : {}),
                };
                const buf = nestedEventBuffers.get(runId) || [];
                buf.push(eventWithRunId);
                nestedEventBuffers.set(runId, buf);

                if (parentRunId) {
                  parentRunIdByRunId.set(runId, parentRunId);
                  const parentToolId = toolIdByRunId.get(parentRunId);
                  if (parentToolId && !toolIdByRunId.has(runId)) {
                    bindNestedRunToTool(runId, parentToolId);
                  }
                }

                if (!toolIdByRunId.has(runId)) {
                  const existingIdx = currentTools.findIndex(
                    (t) => t.runId === runId,
                  );
                  if (existingIdx >= 0) {
                    bindNestedRunToTool(runId, currentTools[existingIdx].id);
                  }
                }

                // If we haven't bound this runId to a tool yet, try to match via the
                // nested_start event's description + subagent_type on the most recent
                // unbound mcp__flamel__Agent tool_use.
                // For AgentBatch: batch_start events bind both the batch id and child
                // run ids to the same batch tool so each child's history stays visible.
                if (
                  !toolIdByRunId.has(runId) &&
                  event?.type === "batch_start"
                ) {
                  for (let ti = currentTools.length - 1; ti >= 0; ti--) {
                    const t = currentTools[ti];
                    if (
                      (t.name !== "AgentBatch" &&
                        t.name !== "mcp__flamel__AgentBatch") ||
                      t.runId
                    )
                      continue;
                    bindNestedRunToTool(runId, t.id);
                    const agents = Array.isArray(event.agents)
                      ? (event.agents as Array<Record<string, unknown>>)
                      : [];
                    for (const agent of agents) {
                      const childRunId = agent.run_id;
                      if (typeof childRunId === "string" && childRunId) {
                        bindNestedRunToTool(childRunId, t.id);
                      }
                    }
                    const nestedEvents = mergedNestedEventsForTool(
                      t.id,
                      t.nestedEvents || [],
                    );
                    currentTools[ti] = { ...t, runId, nestedEvents };
                    updateTool(tabId, t.id, {
                      runId,
                      nestedEvents,
                    });
                    break;
                  }
                } else if (
                  !toolIdByRunId.has(runId) &&
                  event?.type === "nested_start"
                ) {
                  const desc = event.description;
                  const subtype = event.subagent_type || "general-purpose";
                  const batchIdx = currentTools.findIndex(
                    (t) =>
                      (t.name === "AgentBatch" ||
                        t.name === "mcp__flamel__AgentBatch") &&
                      Array.isArray(t.nestedEvents) &&
                      t.nestedEvents.some((ev) => {
                        if (
                          ev.type !== "batch_start" ||
                          !Array.isArray(ev.agents)
                        ) {
                          return false;
                        }
                        return (
                          ev.agents as Array<Record<string, unknown>>
                        ).some((agent) => agent.run_id === runId);
                      }),
                  );
                  if (batchIdx >= 0) {
                    const batchTool = currentTools[batchIdx];
                    bindNestedRunToTool(runId, batchTool.id);
                    const nestedEvents = mergedNestedEventsForTool(
                      batchTool.id,
                      batchTool.nestedEvents || [],
                    );
                    currentTools[batchIdx] = { ...batchTool, nestedEvents };
                    updateTool(tabId, batchTool.id, { nestedEvents });
                  } else {
                    for (let ti = currentTools.length - 1; ti >= 0; ti--) {
                      const t = currentTools[ti];
                      if (
                        (t.name !== "Agent" &&
                          t.name !== "mcp__flamel__Agent") ||
                        t.runId
                      )
                        continue;
                      const inp = t.input as Record<string, unknown>;
                      const tdesc = inp?.description;
                      const tsub =
                        (inp?.subagent_type as string) || "general-purpose";
                      if (tdesc === desc && tsub === subtype) {
                        bindNestedRunToTool(runId, t.id);
                        const nestedEvents = mergedNestedEventsForTool(
                          t.id,
                          t.nestedEvents || [],
                        );
                        currentTools[ti] = {
                          ...t,
                          runId,
                          nestedEvents,
                        };
                        updateTool(tabId, t.id, {
                          runId,
                          nestedEvents,
                        });
                        break;
                      }
                    }
                  }
                } else if (toolIdByRunId.has(runId)) {
                  const toolId = toolIdByRunId.get(runId)!;
                  const idx = currentTools.findIndex((t) => t.id === toolId);
                  if (idx >= 0) {
                    const nestedEvents = mergedNestedEventsForTool(
                      toolId,
                      currentTools[idx].nestedEvents || [],
                    );
                    currentTools[idx] = {
                      ...currentTools[idx],
                      nestedEvents,
                    };
                    updateTool(tabId, toolId, {
                      nestedEvents,
                    });
                  }
                }
                continue;
              }

              if (parsed.type === "workflow_event") {
                if (applyControlWorkflowEvent(normalizeWorkflowEvent(parsed)))
                  continue;
              }

              if (parsed.type === "model_routed" && parsed.routing) {
                applyModelRouting(parsed.routing, parsed.requestId || requestId);
                continue;
              }

              if (parsed.type === "connected" && parsed.sessionId) {
                setSessionId(tabId, parsed.sessionId);
                if (typeof parsed.projectPath === "string") workingDirectory = parsed.projectPath;
                updateTab(tabId, { sessionId: parsed.sessionId, projectPath: workingDirectory, workspace: parsed.workspace || undefined });
                if (subscribeToLiveStreamRef.current) {
                  // The POST owns admission only. All presentation follows the
                  // durable journal so compaction, tab switches, and reconnects
                  // use one resumable transport instead of two competing parsers.
                  handoffToRealtime = true;
                  await reader.cancel();
                  break streamLoop;
                }
              }

              if (parsed.type === "started") {
                const mcpNames = Array.isArray(parsed.mcpServers)
                  ? parsed.mcpServers
                  : undefined;
                updateLastBlock(tabId, {
                  isStarted: true,
                  mcpServerNames: mcpNames,
                });
                if (parsed.runner) {
                  updateTab(tabId, { runnerName: parsed.runner });
                }
              }

              if (parsed.type === "status") {
                // Lightweight breadcrumb — which tool Claude is currently using
                ensureAssistantBlock();
                updateLastBlock(tabId, { statusText: parsed.text });
                updateTab(tabId, { currentTool: parsed.text });
              }

              if (parsed.type === "system_status") {
                if (applyControlWorkflowEvent(normalizeWorkflowEvent(parsed)))
                  continue;
              }

              if (parsed.type === "message") {
                const msg = parsed.message;
                ensureAssistantBlock();

                if (msg.type === "assistant") {
                  const content = Array.isArray(msg.message?.content) ? msg.message.content : [];
                  const startsNewBlock = shouldStartAssistantBlockAfterToolResult(
                    sawToolResult, content, currentTools,
                  );
                  sawToolResult = false;
                  if (startsNewBlock) {
                    // Flush any pending content before switching blocks
                    if (flushTimer) {
                      clearTimeout(flushTimer);
                      flushContent();
                    }
                    // Backend resets stream_content per-turn, so offset is always 0
                    contentBlockOffset = 0;
                    // Stop streaming indicator on the previous block before appending a new one
                    updateLastBlock(tabId, { isStreaming: false });
                    stopAllStreaming(tabId); // clear any stuck 'running' tools from the previous turn
                    currentContent = "";
                    currentThinking = "";
                    currentTools.length = 0;
                    currentSegments.length = 0;
                    pendingShellCommandEvents.length = 0;
                    seenToolIds = new Set();
                    streamingPlanState = new Map();
                    addBlock(tabId, {
                      id: crypto.randomUUID(),
                      role: "assistant",
                      type: "assistant",
                      content: "",
                      tools: [],
                      isStreaming: true,
                      model: options.model,
                    });
                  }

                  // Rebuild content/segments from scratch each time (snapshots, not deltas).
                  // Skip content blocks from previous turns (contentBlockOffset).
                  let newText = "";
                  let newThinking = "";
                  const newSegments: ContentSegment[] = [];

                  for (let ci = contentBlockOffset; ci < content.length; ci++) {
                    const block = content[ci];
                    if (!block) continue;
                    if (block.type === "thinking") {
                      const text = block.thinking || "";
                      const redacted =
                        block.redacted === true || (!text && !!block.signature);
                      if (text) {
                        newThinking += newThinking ? `\n\n${text}` : text;
                      }
                      if (text || redacted) {
                        appendThinkingSegment(newSegments, text, redacted);
                      }
                    } else if (block.type === "text") {
                      const text = block.text || "";
                      newText += text;
                      const lastSeg = newSegments[newSegments.length - 1];
                      if (lastSeg?.type === "text") {
                        lastSeg.text += text;
                      } else if (text) {
                        newSegments.push({ type: "text", text });
                      }
                    } else if (block.type === "tool_use") {
                      // Add tool row only once
                      if (!seenToolIds.has(block.id)) {
                        seenToolIds.add(block.id);
                        const tool: ToolCall = attachPendingCommandEvents(
                          toolFromAssistantSnapshot({
                            id: block.id,
                            name: block.name,
                            input: block.input || {},
                          }, getChat(tabId).blocks.flatMap((entry) => entry.tools || []).reverse().find((tool) => tool.id === block.id)),
                          pendingShellCommandEvents,
                        );
                        currentTools.push(tool);

                        // Handle TodoWrite (input arrives immediately)
                        if (
                          block.name === "TodoWrite" ||
                          block.name === "todo_write"
                        ) {
                          const planItems = extractPlanItems([tool]);
                          if (planItems) {
                            updatePlan(tabId, planItems, undefined);
                            openPlanPanel();
                          }
                        }

                        // Handle old flamel_write_plan (legacy compat)
                        if (block.name === "flamel_write_plan") {
                          const items = (block.input?.items as any[]) || [];
                          const title =
                            (block.input?.title as string) || undefined;
                          if (items.length > 0) {
                            updatePlan(
                              tabId,
                              items.map((item: any, i: number) => ({
                                id: String(i),
                                text: item.text || String(item),
                                status:
                                  item.status === "done"
                                    ? "done"
                                    : item.status === "in_progress"
                                      ? "in_progress"
                                      : "todo",
                                position: i,
                              })),
                              title,
                            );
                            openPlanPanel();
                          }
                        }

                        // Handle Write to .claude/plans/*.md (fallback)
                        const writeItems = extractPlanFromWrite(tool);
                        if (writeItems) {
                          updatePlan(tabId, writeItems, undefined);
                          openPlanPanel();
                        }

                        // Mark plan MCP tools as pending until input is populated
                        const isPlanTool =
                          block.name === "plan" ||
                          block.name.endsWith("__plan") ||
                          block.name.includes("flamel_plan_");
                        if (isPlanTool) {
                          streamingPlanState.set(block.id, -1);
                          const currentPlan = getPlan(tabId);
                          if (
                            !currentPlan.planId &&
                            currentPlan.items.length === 0
                          ) {
                            setPlan(
                              tabId,
                              "pending",
                              "Preparing plan...",
                              undefined,
                              [],
                            );
                            setStreamingIds(tabId, []);
                          }
                          openPlanPanel();
                        }
                      }

                      // Stream plan items incrementally — re-fire on every snapshot where items grow
                      if (
                        streamingPlanState.has(block.id) &&
                        block.input &&
                        Object.keys(block.input).length > 0
                      ) {
                        const prevCount = streamingPlanState.get(block.id)!;
                        const planAct = planToolAction(block.name, block.input);

                        if (planAct === "create") {
                          const rawItems = normalizePlanInputItems(
                            block.input?.items,
                          );
                          if (
                            block.input?.title &&
                            rawItems.length > prevCount
                          ) {
                            const newIds = rawItems
                              .slice(prevCount)
                              .map(
                                (_: any, i: number) =>
                                  `pending_${prevCount + i}`,
                              );
                            streamingPlanState.set(block.id, rawItems.length);
                            const items: PlanItem[] = rawItems.map(
                              (item: any, i: number) => ({
                                id: `pending_${i}`,
                                text: item.text,
                                status: (item.status ||
                                  "todo") as PlanItem["status"],
                                position: i,
                                notes: item.notes || undefined,
                                section: item.section || undefined,
                                reasoningFloor:
                                  item.reasoning_floor || "medium",
                              }),
                            );
                            setPlan(
                              tabId,
                              "pending",
                              block.input.title as string,
                              block.input?.description as string | undefined,
                              items,
                            );
                            setStreamingIds(tabId, newIds);
                          }
                        }

                        if (planAct === "update_item") {
                          if (prevCount < 0) {
                            streamingPlanState.set(block.id, 0);
                            const itemId = block.input?.item_id as string;
                            const status = block.input?.status as
                              PlanItem["status"] | undefined;
                            const notes = block.input?.notes as
                              string | undefined;
                            const text = block.input?.text as
                              string | undefined;
                            if (itemId) {
                              const updates: Partial<PlanItem> = {};
                              if (status) updates.status = status;
                              if (notes !== undefined) updates.notes = notes;
                              if (text !== undefined) updates.text = text;
                              if (Object.keys(updates).length > 0)
                                updateItem(tabId, itemId, updates);
                            }
                          }
                        }

                        if (planAct === "replace_items") {
                          const rawItems = normalizePlanInputItems(
                            block.input?.items,
                          );
                          if (rawItems.length > prevCount) {
                            const newIds = rawItems
                              .slice(prevCount)
                              .map(
                                (_: any, i: number) =>
                                  `pending_${prevCount + i}`,
                              );
                            streamingPlanState.set(block.id, rawItems.length);
                            const currentPlan = getPlan(tabId);
                            const items: PlanItem[] = rawItems.map(
                              (item: any, i: number) => ({
                                id: `pending_${i}`,
                                text: item.text,
                                status: (item.status ||
                                  "todo") as PlanItem["status"],
                                position: i,
                                notes: item.notes || undefined,
                                section: item.section || undefined,
                                reasoningFloor:
                                  item.reasoning_floor || "medium",
                              }),
                            );
                            setPlan(
                              tabId,
                              currentPlan.planId || "pending",
                              currentPlan.title || "",
                              currentPlan.description,
                              items,
                            );
                            setStreamingIds(tabId, newIds);
                          }
                        }

                        if (planAct === "add_items") {
                          const rawItems = normalizePlanInputItems(
                            block.input?.items,
                          );
                          if (rawItems.length > prevCount) {
                            const currentPlan = getPlan(tabId);
                            const startPos = currentPlan.items.length;
                            const newIds = rawItems
                              .slice(prevCount)
                              .map(
                                (_: any, i: number) =>
                                  `pending_${startPos + prevCount + i}`,
                              );
                            streamingPlanState.set(block.id, rawItems.length);
                            const items: PlanItem[] = rawItems.map(
                              (item: any, i: number) => ({
                                id: `pending_${startPos + i}`,
                                text: item.text,
                                status: (item.status ||
                                  "todo") as PlanItem["status"],
                                position: startPos + i,
                                notes: item.notes || undefined,
                                section: item.section || undefined,
                                reasoningFloor:
                                  item.reasoning_floor || "medium",
                              }),
                            );
                            addItems(tabId, items);
                            setStreamingIds(tabId, newIds);
                          }
                        }
                      }

                      // Already seen — update partialInput or finalize input from streaming snapshots.
                      if (seenToolIds.has(block.id)) {
                        const idx = currentTools.findIndex(
                          (t) => t.id === block.id,
                        );
                        if (idx >= 0) {
                          if ((block as any)._inputStr !== undefined) {
                            // Tool input still building — show raw partial JSON
                            const partial = (block as any)._inputStr as string;
                            currentTools[idx] = {
                              ...currentTools[idx],
                              partialInput: partial,
                            };
                            updateTool(tabId, block.id, {
                              partialInput: partial,
                            });

                            // Stream plan items from partial JSON as they complete
                            if (streamingPlanState.has(block.id)) {
                              const parsed = parsePartialPlanInput(partial);
                              syncStreamingPlanHeader(parsed);
                              if (
                                parsed &&
                                (parsed.action === "create" ||
                                  parsed.action === "replace_items" ||
                                  parsed.action === "add_items")
                              ) {
                                const prevCount = streamingPlanState.get(
                                  block.id,
                                )!;
                                if (
                                  parsed.items.length > Math.max(prevCount, 0)
                                ) {
                                  streamingPlanState.set(
                                    block.id,
                                    parsed.items.length,
                                  );
                                  if (parsed.action === "add_items") {
                                    const currentPlan = getPlan(tabId);
                                    const startPos = currentPlan.items.filter(
                                      (i) => !i.id.startsWith("pending_"),
                                    ).length;
                                    const items: PlanItem[] = parsed.items.map(
                                      (item: any, i: number) => ({
                                        id: `pending_${startPos + i}`,
                                        text: item.text || "",
                                        status: (item.status ||
                                          "todo") as PlanItem["status"],
                                        position: startPos + i,
                                        notes: item.notes || undefined,
                                        section: item.section || undefined,
                                        reasoningFloor:
                                          item.reasoning_floor || "medium",
                                      }),
                                    );
                                    // Replace all pending items (don't keep stacking)
                                    const withoutPending =
                                      currentPlan.items.filter(
                                        (i) => !i.id.startsWith("pending_"),
                                      );
                                    setPlan(
                                      tabId,
                                      currentPlan.planId || "pending",
                                      currentPlan.title || "",
                                      currentPlan.description,
                                      [...withoutPending, ...items],
                                    );
                                  } else {
                                    const items: PlanItem[] = parsed.items.map(
                                      (item: any, i: number) => ({
                                        id: `pending_${i}`,
                                        text: item.text || "",
                                        status: (item.status ||
                                          "todo") as PlanItem["status"],
                                        position: i,
                                        notes: item.notes || undefined,
                                        section: item.section || undefined,
                                        reasoningFloor:
                                          item.reasoning_floor || "medium",
                                      }),
                                    );
                                    const planId =
                                      parsed.action === "create"
                                        ? "pending"
                                        : getPlan(tabId).planId || "pending";
                                    const title =
                                      parsed.title ||
                                      getPlan(tabId).title ||
                                      "";
                                    const desc =
                                      parsed.description ||
                                      getPlan(tabId).description;
                                    setPlan(tabId, planId, title, desc, items);
                                  }
                                  openPlanPanel();
                                }
                              }
                            }
                          } else if (
                            block.input &&
                            Object.keys(block.input).length > 0
                          ) {
                            // Input finalized — always update (dropped snapshots may have skipped earlier updates)
                            currentTools[idx] = {
                              ...currentTools[idx],
                              input: block.input,
                              partialInput: undefined,
                            };
                            updateTool(tabId, block.id, {
                              input: block.input,
                              partialInput: undefined,
                            });
                          }
                        }
                      }

                      newSegments.push({ type: "tool_ref", toolId: block.id });
                    }
                  }

                  currentContent = stripHiddenControlMarkers(newText);
                  currentThinking = newThinking;
                  currentSegments.length = 0;
                  for (const seg of newSegments) {
                    if (seg.type === "text") {
                      currentSegments.push({
                        ...seg,
                        text: stripHiddenControlMarkers(seg.text),
                      });
                    } else {
                      currentSegments.push(seg);
                    }
                  }

                  for (const result of [...pendingToolResults.values()]) {
                    if (!currentTools.some((tool) => tool.id === result.tool_use_id)) continue;
                    applyStreamingToolResult(
                      currentTools, result,
                      (id, updates) => updateTool(tabId, id, updates),
                      bindNestedRunToTool, mergedNestedEventsForTool,
                      () => undefined, pendingToolResults,
                    );
                  }
                  scheduleFlush();
                }

                if (msg.type === "tool_result") {
                  if (!msg.same_turn) sawToolResult = true;
                  const application = applyStreamingToolResult(
                    currentTools,
                    msg,
                    (toolId, updates) => updateTool(tabId, toolId, updates),
                    bindNestedRunToTool,
                    mergedNestedEventsForTool,
                    (id) => getChat(tabId).blocks.flatMap((block) => block.tools || []).reverse().find((tool) => tool.id === id),
                    pendingToolResults,
                  );
                  if (application) {
                    const {
                      tool,
                      isDelta,
                      output: mergedOutput,
                      changed,
                    } = application;
                    if (!isDelta) {
                      updateLastBlock(tabId, { statusText: undefined });
                      updateTab(tabId, { currentTool: undefined });
                    }
                    if (!isDelta && changed) {
                      maybeRefreshProjectFilesFromTool(
                        workingDirectory,
                        tool.name,
                        tool.input,
                        msg.is_error,
                      );
                    }

                    const toolPlanAct = planToolAction(tool.name, tool.input);
                    if (!isDelta && !msg.is_error && changed) {
                      applyPlanWorkflowEvent(
                        parsePlanMarkerOutput(mergedOutput),
                        toolPlanAct,
                        tool.input,
                        { dispatchCreatedEvent: true },
                      );
                    }
                  }
                }
              }

              if (
                parsed.type === "ask_question" ||
                parsed.type === "question_resolved" ||
                parsed.type === "question_cancelled"
              ) {
                applyQuestionWorkflowEvent(normalizeWorkflowEvent(parsed));
              }

              if (parsed.type === "recovery") {
                applyRecoveryNotice(parsed.note, parsed.summary);
              }

              if (parsed.type === "done") {
                clearCompactionBlock(tabId);
                // Cancel throttle — final update happens below with isStreaming: false
                if (flushTimer) {
                  clearTimeout(flushTimer);
                  flushTimer = null;
                }
                contentDirty = false;
                receivedDone = true;
                serverQueuePending = parsed.queuePending === true;
                if (!serverQueuePending) {
                  window.dispatchEvent(
                    new CustomEvent("flamel:agent-done", { detail: { tabId } }),
                  );
                }

                {
                  // Persist title to backend on first message
                  const sid = getChat(tabId).sessionId;
                  if (isFirstMessage && sid) {
                    api.sessions.update(sid, { title }).catch(() => {});
                  }
                  refreshSessions();
                }
                const continuationSignal =
                  extractHiddenControlSignal(currentContent);
                currentContent = continuationSignal.content;
                const continuationDirective =
                  (parsed.continuationDirective as
                    HiddenControlDirective | undefined) ||
                  continuationSignal.continuationDirective;
                // Detect waiting state — question-shaped text
                const isQuestion = looksLikeQuestionForUser(currentContent);
                if (isQuestion) {
                  setWaiting(tabId, true);
                }

                const continuationCount =
                  (options as any).__continuationCount ?? 0;
                const activePlan = getPlan(tabId);
                const hasUnfinishedPlan =
                  !!activePlan.planId &&
                  activePlan.items.some((item) => item.status !== "done");
                const shouldAutoContinue =
                  continuationDirective === "complete"
                    ? false
                    : continuationDirective === "continue"
                      ? continuationCount < MAX_AUTO_CONTINUATIONS
                      : continuationSignal.shouldAutoContinue ||
                        (!isQuestion &&
                          shouldAutoContinueResponse(
                            parsed.stopReason,
                            currentContent,
                            options.model,
                            continuationCount,
                          )) ||
                        shouldAutoContinuePlanRecap(
                          currentContent,
                          options.model,
                          hasUnfinishedPlan,
                          continuationCount,
                        );

                if (
                  shouldAutoContinue &&
                  parsed.completionManaged !== "server" &&
                  !serverQueuePending &&
                  !runtime.queue.some((item) => item.serverOwned)
                ) {
                  // Flush current content to the block but keep streaming appearance
                  updateLastBlock(tabId, {
                    isStreaming: false,
                    content: currentContent,
                    thinking: currentThinking || undefined,
                    tools: [...currentTools],
                    segments: [...currentSegments],
                    statusText: continuationLabel(
                      parsed.stopReason,
                      continuationCount + 1,
                    ),
                  });
                  // Queue a transparent continuation — no user block, just keep going
                  runtime.queue.unshift({
                    id: crypto.randomUUID(),
                    message: continuationPrompt(
                      parsed.stopReason,
                      options.model,
                    ),
                    workingDirectory,
                    options: {
                      ...options,
                      sessionId: getChat(tabId).sessionId || options.sessionId,
                      __continuationCount: continuationCount + 1,
                      __isContinuation: true,
                      __reuseAssistantBlock: false,
                      __recoveryNote: pendingRecoveryNote,
                      __recoverySummary: pendingRecoverySummary,
                    },
                    createdAt: Date.now(),
                  });
                  chatRuntime.notify(runtime);
                  // Don't finalize — let the finally block pick up the queued continuation
                  receivedDone = true;
                  break;
                }

                if (parsed.stopReason === "max_tokens") {
                  currentContent +=
                    "\n\n---\n*Response truncated after multiple continuations.*";
                } else if (parsed.stopReason === "process_died") {
                  currentContent +=
                    "\n\n---\n*Claude process exited after multiple restart attempts. Send another message to restart.*";
                }

                // Set isStreaming: false in the same call as content — prevents cursor flash
                // (two separate Zustand set() calls = two renders = visible blink)
                lastStopReason = parsed.stopReason ?? null;
                updateLastBlock(tabId, {
                  isStreaming: false,
                  content: currentContent,
                  thinking: currentThinking || undefined,
                  tools: [...currentTools],
                  segments: [...currentSegments],
                  statusText: undefined,
                  stopReason: parsed.stopReason,
                  usage: parsed.usage
                    ? {
                        inputTokens: parsed.usage.input_tokens ?? 0,
                        outputTokens: parsed.usage.output_tokens,
                        cacheReadTokens:
                          parsed.usage.cache_read_input_tokens ?? 0,
                        cacheCreationTokens:
                          parsed.usage.cache_creation_input_tokens ?? 0,
                      }
                    : undefined,
                });
                stopAllStreaming(tabId); // clears any leftover isStreaming on intermediate blocks
              }

              if (parsed.type === "error") {
                ensureAssistantBlock();
                updateLastBlock(tabId, {
                  isStreaming: false,
                  content: `Error: ${parsed.error}`,
                });
              }
            }
          }
        } catch (err: any) {
          if (err.name !== "AbortError") {
            // Don't overwrite existing content — it may be the only copy of a
            // completed response if the 'done' event was lost. The reconnect
            // path in finally will restore live content if the session is active.
            const chat = getChat(tabId);
            const lastBlock = chat.blocks[chat.blocks.length - 1];
            const hasContent = !!(
              lastBlock?.content ||
              lastBlock?.thinking ||
              (lastBlock?.tools && lastBlock.tools.length > 0)
            );
            if (!hasContent) {
              updateLastBlock(tabId, {
                isStreaming: false,
                content: `Connection error: ${errorMessageFromUnknown(err, "Flamel Desktop stream failed")}`,
              });
            }
          }
          refreshSessions();
        } finally {
          // Release the reader so the browser can reuse/close the HTTP connection.
          // Without this, the connection slot stays occupied and the next chat fetch queues forever.
          reader?.cancel().catch(() => {});
          reader = null;
          // Clean up throttle timer
          if (flushTimer) {
            clearTimeout(flushTimer);
            flushTimer = null;
          }
          if (admittedAsQueued) {
            const currentSessionId =
              getChat(tabId).sessionId || options.sessionId;
            runtime.active = true;
            updateTab(tabId, {
              isStreaming: true,
              requestId: undefined,
              currentTool: undefined,
              requiresResume: false,
              sessionWarmSince: undefined,
            });
            abortRef.current = null;
            chatRuntime.notify(runtime);
            if (currentSessionId && subscribeToLiveStreamRef.current) {
              await subscribeToLiveStreamRef.current(currentSessionId);
            }
            return;
          }
          const willContinueWithQueue =
            serverQueuePending || runtime.queue.length > 0;
          if (!willContinueWithQueue) stopAllStreaming(tabId);
          // Session is warm if the provider finished a turn cleanly (process still alive).
          const sessionIsWarm =
            receivedDone &&
            !willContinueWithQueue &&
            lastStopReason !== "process_died" &&
            !userAborted;
          const currentSessionId = getChat(tabId).sessionId;
          // Keep requestId alive if we're about to reconnect — subscribeToLiveStream's
          // abort handler needs it to call /api/abort on the backend.
          const willReconnect = !!(
            (!receivedDone || handoffToRealtime) &&
            !userAborted &&
            currentSessionId &&
            subscribeToLiveStreamRef.current
          );
          updateTab(tabId, {
            currentTool: undefined,
            ...(!willReconnect && { requestId: undefined }),
            isSessionWarm: sessionIsWarm,
            requiresResume:
              !!currentSessionId && !sessionIsWarm && !willReconnect,
            sessionWarmSince: sessionIsWarm ? Date.now() : undefined,
          });
          abortRef.current = null;

          if ((!receivedDone || handoffToRealtime) && !userAborted) {
            const sessionId = getChat(tabId).sessionId;
            if (sessionId && subscribeToLiveStreamRef.current) {
              // Backend runner keeps going after client disconnect — reconnect to pick it back up.
              updateTab(tabId, {
                isStreaming: true,
                requiresResume: false,
                sessionWarmSince: undefined,
              });
              const hasKnownDurableQueue = runtime.queue.some(
                (item) => item.serverOwned,
              );
              const joinedPersistentOwner =
                await subscribeToLiveStreamRef.current(sessionId, {
                  followQueuedRuns: !hasKnownDurableQueue,
                  // A stop followed by an immediate prompt can briefly leave the
                  // interrupted run active. Only rebuild the run this POST owns.
                  requestId,
                });
              if (joinedPersistentOwner) return;
              // subscribeToLiveStream returns when stream ends (done/error/inactive)
              // Now safe to clear requestId
              updateTab(tabId, { requestId: undefined });
            } else {
              // No session to reconnect to — show disconnected
              const chat = getChat(tabId);
              const lastBlock = chat.blocks[chat.blocks.length - 1];
              const hasAnyContent = !!(
                lastBlock?.content ||
                lastBlock?.thinking ||
                (lastBlock?.tools && lastBlock.tools.length > 0)
              );
              if (!hasAnyContent)
                updateLastBlock(tabId, { content: "— disconnected —" });
            }
          }

          // Preserve the active indicator across a durable queue handoff. The
          // server is the sole dispatcher for accepted turns; the local queue is
          // only a projection used for editing and display.
          runtime.active = willContinueWithQueue;
          updateTab(tabId, { isStreaming: willContinueWithQueue });
          const shouldDrainAfterAbort = runtime.drainAfterAbort;
          runtime.drainAfterAbort = false;
          chatRuntime.notify(runtime);
          if (!userAborted || shouldDrainAfterAbort) {
            const durableItems = runtime.queue.filter(
              (item) => item.serverOwned,
            );
            if (
              (serverQueuePending || durableItems.length > 0) &&
              currentSessionId &&
              subscribeToLiveStreamRef.current
            ) {
              const admissions = await Promise.all(
                durableItems.map(async (item) => ({
                  item,
                  accepted: (await item.accepted) !== false,
                })),
              );
              for (const { item, accepted } of admissions) {
                item.admissionState = accepted ? "accepted" : "failed";
                if (!accepted) item.serverOwned = false;
              }
              runtime.active = true;
              serverQueuePending = false;
              updateTab(tabId, {
                isStreaming: true,
                requestId: undefined,
                requiresResume: false,
                sessionWarmSince: undefined,
              });
              chatRuntime.notify(runtime);
              const joinedPersistentOwner =
                await subscribeToLiveStreamRef.current(currentSessionId, {
                  followQueuedRuns: true,
                  requestId: undefined,
                });
              if (joinedPersistentOwner) return;
              // Every accepted server-backed turn has now reached a terminal
              // state. Continuity events normally remove them as they start;
              // this final sweep handles reconnects that missed that projection.
              runtime.queue = runtime.queue.filter((item) => !item.serverOwned);
              runtime.active = false;
              chatRuntime.notify(runtime);
            }

            const drain = chatRuntime.drain(runtime);
            if (drain.status === "next") {
              executeSend(
                drain.item.message,
                drain.item.workingDirectory,
                drain.item.options,
              );
            } else if (drain.status === "idle") {
              updateTab(tabId, { isStreaming: false, requestId: undefined });
            }
          }
        }
      } catch (outerErr: any) {
        // Caught an error thrown BEFORE the inner try block (e.g. in setup code).
        // runtime.active would otherwise be stuck true forever.
        runtime.active = false;
        chatRuntime.notify(runtime);
        updateTab(tabId, {
          isStreaming: false,
          currentTool: undefined,
          requestId: undefined,
        });
        stopAllStreaming(tabId);
      }
    },
    [runtime, tabId],
  );

  const sendMessage = useCallback(
    (message: string, workingDirectory: string, options: SendOptions) => {
      const tab = useTabStore.getState().tabs.find((t) => t.id === tabId);
      if (isAutonomousOrchestrationTab(tab)) return undefined;
      const chat = useChatStore.getState().chats[tabId];
      const sessionId = resolveExecutionSessionId(
        options.sessionId,
        tab?.sessionId,
        chat?.sessionId,
        options.__forceNewSession,
      );
      const id = crypto.randomUUID();
      const queued: QueuedMessage = {
        id,
        message,
        workingDirectory,
        options: sessionId ? { ...options, sessionId } : options,
        createdAt: Date.now(),
      };

      // Once a turn is running, transfer ownership of every following turn to
      // the server immediately. Closing the tab or switching devices cannot
      // discard work that the UI previously showed as queued.
      if (runtime.active && sessionId) {
        queued.serverOwned = true;
        queued.admissionState = "pending";
        const durableBody = JSON.stringify({
          message,
          requestId: id,
          workingDirectory,
          sessionId,
          activePlanId: tab?.activePlanId,
          model: options.model,
          reasoningEffort: options.reasoningEffort,
          ponytailEnabled:
            options.ponytailEnabled ?? tab?.ponytailEnabled ?? false,
          permissionMode: options.permissionMode,
          images: options.images,
          planningMode: options.planningMode,
          runnerId: options.runnerId || undefined,
          hiddenContinuation: options.__isContinuation || undefined,
          interruptActive: options.__interruptActive || undefined,
        });
        queued.accepted = (async () => {
          for (let attempt = 0; attempt < 2; attempt++) {
            try {
              const response = await apiFetch("/api/chat", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: durableBody,
              });
              const duplicate = response.status === 409 ? await response.json().catch(() => null) : null;
              if (response.ok || isAcceptedDuplicate(duplicate, id, sessionId)) {
                queued.admissionState = "accepted";
                chatRuntime.notify(runtime);
                return true;
              }
            } catch {
              // Retrying the same request id is idempotent on the server.
            }
          }
          queued.admissionState = "failed";
          queued.serverOwned = false;
          chatRuntime.notify(runtime);
          return false;
        })();
      }
      // Always enqueue — drain immediately if idle, queue if busy.
      // This makes the race structurally impossible: there is no bypass path.
      const next = chatRuntime.admit(runtime, queued);
      if (next) executeSend(next.message, next.workingDirectory, next.options);
      return queued;
    },
    [executeSend, runtime, tabId],
  );

  const steerMessage = useCallback(async (message: string, workingDirectory: string, options: SendOptions) => {
    const tab = useTabStore.getState().tabs.find((candidate) => candidate.id === tabId);
    if (isAutonomousOrchestrationTab(tab)) return;
    // Image input needs a full next turn; the steering protocol accepts text only.
    if (options.images?.length) return sendMessage(message, workingDirectory, options);
    const sessionId = options.sessionId || tab?.sessionId || getChat(tabId).sessionId;
    if (!sessionId) {
      sendMessage(message, workingDirectory, options);
      return;
    }
    const requestId = crypto.randomUUID();
    const record = useChatStore.getState().recordInputDelivery;
    record(tabId, { requestId, sessionId, message, delivery: "saving" });
    let response: Response;
    try {
      response = await apiFetch("/api/chat/steer", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          message, requestId, sessionId, workingDirectory,
          activePlanId: tab?.activePlanId,
          model: options.model, reasoningEffort: options.reasoningEffort,
          ponytailEnabled: options.ponytailEnabled ?? tab?.ponytailEnabled ?? false,
          permissionMode: options.permissionMode, images: options.images,
          planningMode: options.planningMode, runnerId: options.runnerId || undefined,
        }),
      });
    } catch {
      record(tabId, { requestId, sessionId, message, delivery: "uncertain", note: "Connection lost while sending. Check delivery before sending this again." });
      return;
    }
    const payload = await response.json().catch(() => null);
    const receipt = parseInputDelivery(payload, sessionId);
    if (response.ok && receipt?.requestId === requestId) {
      record(tabId, receipt);
      if (receipt.delivery === "failed") throw new Error(receipt.note || "The saved message was not delivered.");
      if (receipt.delivery === "queued" && !runtime.active) {
        void subscribeToLiveStreamRef.current?.(sessionId);
      }
      return;
    }
    if (response.ok || (response.status >= 500 && payload?.accepted !== false)) {
      record(tabId, { requestId, sessionId, message, delivery: "uncertain", note: "The server did not return a delivery receipt. Check delivery before sending this again." });
      return;
    }
    const note = typeof payload?.error === "string" ? payload.error : `Message was not accepted (${response.status}).`;
    record(tabId, { requestId, sessionId, message, delivery: "failed", note });
    throw new Error(note);
  }, [tabId, getChat, sendMessage, runtime]);

  const abortServerSession = useCallback(
    async (expectedRequestId?: string) => {
      const tab = useTabStore.getState().tabs.find((t) => t.id === tabId);
      const chat = useChatStore.getState().chats[tabId];
      const sessionId = chat?.sessionId || tab?.sessionId;
      if (!sessionId) return false;
      try {
        const response = await apiFetch(
          `/api/sessions/${encodeURIComponent(sessionId)}/abort`,
          {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              expectedRequestId: expectedRequestId || undefined,
            }),
          },
        );
        return response.ok;
      } catch {
        return false;
      }
    },
    [tabId],
  );

  const abort = useCallback(() => {
    // Server-accepted turns are durable and cannot be discarded by clearing a
    // browser array. Keep them visible while the server completes the current
    // cancellation and performs the ordered handoff.
    runtime.drainAfterAbort = false;
    chatRuntime.notify(runtime);
    abortRef.current?.();
    void abortServerSession();
    // Focus the input so the user can immediately type a follow-up
    window.dispatchEvent(
      new CustomEvent("flamel:focus-input", { detail: { tabId } }),
    );
  }, [abortServerSession, runtime, tabId]);

  const updateQueuedMessage = useCallback(
    async (id: string, message: string): Promise<string | null> => {
      const nextMessage = message.trim();
      if (!nextMessage) return "Queued message cannot be empty";
      const queued = runtime.queue.find(
        (item) => item.id === id && !item.options.__isContinuation,
      );
      if (!queued) return "Queued message not found";

      if (queued.serverOwned) {
        const accepted = (await queued.accepted) !== false;
        if (!accepted) return "The server did not accept this queued message";
        try {
          const response = await apiFetch(
            `/api/chat/queue/${encodeURIComponent(id)}`,
            {
              method: "PUT",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ message: nextMessage }),
            },
          );
          if (!response.ok) {
            const payload = (await response.json().catch(() => null)) as {
              error?: string;
            } | null;
            if (response.status === 409) {
              runtime.queue = runtime.queue.filter((item) => item.id !== id);
              chatRuntime.notify(runtime);
              return null;
            }
            return (
              payload?.error ||
              `Unable to edit queued message (${response.status})`
            );
          }
        } catch (error) {
          return errorMessageFromUnknown(error);
        }
      }

      runtime.queue = runtime.queue.map((item) =>
        item.id === id && !item.options.__isContinuation
          ? { ...item, message: nextMessage }
          : item,
      );
      chatRuntime.notify(runtime);
      return null;
    },
    [runtime],
  );

  const removeQueuedMessage = useCallback(
    (id: string) => {
      runtime.queue = runtime.queue.filter(
        (item) => item.id !== id || item.serverOwned,
      );
      chatRuntime.notify(runtime);
    },
    [runtime],
  );

  const interruptWithMessage = useCallback(
    async (message: string, workingDirectory: string, options: SendOptions) => {
      const tab = useTabStore.getState().tabs.find((t) => t.id === tabId);
      if (isAutonomousOrchestrationTab(tab)) return;
      const nextMessage = message.trim();
      if (!nextMessage) return;
      const chat = useChatStore.getState().chats[tabId];
      const sessionId = resolveExecutionSessionId(
        options.sessionId,
        tab?.sessionId,
        chat?.sessionId,
        options.__forceNewSession,
      );
      const queued = sendMessage(nextMessage, workingDirectory, {
        ...options,
        ...(sessionId ? { sessionId } : {}),
        __interruptActive: true,
      });
      if (queued?.accepted && (await queued.accepted) === false) {
        return;
      }
      window.dispatchEvent(
        new CustomEvent("flamel:focus-input", { detail: { tabId } }),
      );
    },
    [sendMessage, tabId],
  );

  const loadHistory = useCallback(
    async (sessionId: string) => {
      // Reuse blocks only when they belong to this exact server session. A
      // tab can be rebound while an older request is still resolving.
      const existingChat = getChat(tabId);
      if (
        existingChat.sessionId === sessionId &&
        existingChat.blocks.length > 0
      )
        return;
      // Populate chat.sessionId immediately so sendMessage uses the right session even before
      // the first streaming response (which is the only other place setSessionId is called).
      setSessionId(tabId, sessionId);

      const expectedBlocks = useChatStore.getState().getChat(tabId).blocks;
      const expectedGeneration =
        useTabStore.getState().tabs.find((tab) => tab.id === tabId)
          ?.bindingGeneration || 0;
      const isCurrentBinding = () => {
        const current = useTabStore
          .getState()
          .tabs.find((tab) => tab.id === tabId);
        return (
          current?.sessionId === sessionId &&
          (current.bindingGeneration || 0) === expectedGeneration
        );
      };

      try {
        const bootstrap = await api.sessions.bootstrap(sessionId);
        if (!isCurrentBinding() || bootstrap.binding.sessionId !== sessionId)
          return;
        const session = bootstrap.session;
        const sessionModel = session?.model;
        const blocks = processHistoryMessages(
          bootstrap.messages || [],
          sessionModel ||
            useTabStore.getState().tabs.find((t) => t.id === tabId)
              ?.resolvedModel,
        );
        if (!useChatStore.getState().hydrateHistory(tabId, sessionId, expectedBlocks, blocks))
          return;
        if (session) {
          const tabUpdates = {
            projectPath: session.projectPath,
            workspace: session.workspace,
            ...(session?.title ? { title: session.title } : {}),
            ...(session?.requestedModel || session?.model
              ? {
                  model: manualModelSelection(
                    session.requestedModel,
                    session.model,
                  ),
                }
              : {}),
            ...(session?.requestedReasoningEffort
              ? {
                  reasoningEffort:
                    session.requestedReasoningEffort as ReasoningSelection,
                }
              : {}),
            ...(session?.model
              ? { resolvedModel: session.model as ModelId }
              : {}),
            ...(session?.resolvedReasoningEffort
              ? {
                  resolvedReasoningEffort:
                    session.resolvedReasoningEffort as ReasoningEffort,
                }
              : {}),
            ...(session?.routingReason
              ? { routingReason: session.routingReason }
              : {}),
            ...(typeof session?.isSessionWarm === "boolean"
              ? {
                  isSessionWarm: session.isSessionWarm,
                  sessionWarmSince: session.isSessionWarm
                    ? Date.now()
                    : undefined,
                }
              : {}),
            ...(typeof session?.requiresResume === "boolean"
              ? { requiresResume: session.requiresResume }
              : {}),
            activePlanId: bootstrap.binding.activePlanId || undefined,
          };
          if (Object.keys(tabUpdates).length > 0) {
            updateTab(tabId, tabUpdates);
          }
        }

        if (!isCurrentBinding()) return;
        if (bootstrap.activePlan) {
          const plan = bootstrap.activePlan;
          setPlan(
            tabId,
            plan.id,
            plan.title,
            plan.description,
            plan.items,
            false,
            {
              artifact: plan.artifact,
              versions: plan.versions,
              finished: plan.finished,
              executionStatus: plan.executionStatus,
              approvalStatus: plan.approvalStatus,
              activeItemId: plan.activeItemId,
              approvalRequestedAt: plan.approvalRequestedAt,
              approvedAt: plan.approvedAt,
              executionStartedAt: plan.executionStartedAt,
              executionUpdatedAt: plan.executionUpdatedAt,
            },
          );
        } else {
          clearPlan(tabId);
        }
      } catch {}
    },
    [tabId],
  );

  const subscribeToLiveStream = useCallback(
    async (
      sessionId: string,
      opts?: {
        onHistoryReady?: () => void;
        historyLoaded?: boolean;
        signal?: AbortSignal;
        followQueuedRuns?: boolean;
        requestId?: string;
        bindingGeneration?: number;
        persistent?: boolean;
      },
    ): Promise<boolean> => {
      if (opts?.signal?.aborted) return false;
      const expectedGeneration =
        opts?.bindingGeneration ??
        useTabStore.getState().tabs.find((tab) => tab.id === tabId)
          ?.bindingGeneration ??
        0;
      const isCurrentBinding = () => {
        const current = useTabStore
          .getState()
          .tabs.find((tab) => tab.id === tabId);
        return (
          current?.sessionId === sessionId &&
          (current.bindingGeneration || 0) === expectedGeneration
        );
      };
      if (!isCurrentBinding()) return false;
      let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
      let userAborted = false;
      let externallyCancelled = false;
      const ownerAbort = new AbortController();
      let owner!: SessionStreamOwner;
      let resolveSnapshotReady!: (ready: boolean) => void;
      let initialSnapshotReady = false;
      const snapshotReady = new Promise<boolean>((resolve) => {
        resolveSnapshotReady = resolve;
      });
      const cancelForOwner = () => {
        externallyCancelled = true;
        resolveSnapshotReady(false);
        ownerAbort.abort();
        reader?.cancel().catch(() => {});
        // Effect cleanup and replacement can run in the same React turn. Drop
        // ownership synchronously so the replacement never joins a dead reader.
        chatRuntime.releaseSessionStream(sessionId, owner);
      };
      let resolveOwner!: () => void;
      const ownerDone = new Promise<void>((resolve) => {
        resolveOwner = resolve;
      });
      owner = {
        tabId,
        bindingGeneration: expectedGeneration,
        persistent: opts?.persistent === true,
        cancel: cancelForOwner,
        done: ownerDone,
        snapshotReady,
      };
      const streamClaim = chatRuntime.claimSessionStream(sessionId, owner);
      const historyNotification = opts?.onHistoryReady
        ? notifySessionSnapshotReady(
          streamClaim.owner,
          opts.onHistoryReady,
          () => !opts.signal?.aborted && isCurrentBinding()
            && chatRuntime.ownsSessionStream(sessionId, streamClaim.owner),
          opts.signal,
        ) : Promise.resolve();
      if (streamClaim.status === "joined") {
        const previousOwner = streamClaim.owner;
        // SessionRuntime owns one durable presentation subscriber. InputBar,
        // ChatView, and reconnect callers only admit work; they must never
        // replace, reset, or wait on that long-lived cursor.
        if (previousOwner.persistent) {
          await historyNotification;
          return true;
        }
        if (!opts?.signal) await previousOwner.done;
        else if (!opts.signal.aborted) {
          await Promise.race([
            previousOwner.done,
            new Promise<void>((resolve) => {
              opts.signal!.addEventListener("abort", () => resolve(), {
                once: true,
              });
            }),
          ]);
        }
        return true;
      }
      // Only true when we enter live streaming mode — prevents finally from clobbering
      // executeSend's streaming state if both run concurrently (race on mount).
      let tookStreamingOwnership = false;
      let handoffToExecuteSend = false;
      let followQueuedRun = false;
      let expectedRequestId = opts?.requestId;

      try {
        opts?.signal?.addEventListener("abort", cancelForOwner, { once: true });
        const freshSnapshot =
          useChatStore.getState().getChat(tabId).blocks.length === 0;
        const res = openRealtimeSessionStream(
          sessionId,
          ownerAbort.signal,
          freshSnapshot,
          opts?.historyLoaded === true,
        );
        if (!res.ok) return false;

        reader = res.body!.getReader();
        const decoder = new TextDecoder();
        let buffer = "";
        const streamProjectPath =
          useTabStore.getState().tabs.find((t) => t.id === tabId)
            ?.projectPath || "";

        // Phase 1: collect history messages until 'streaming' event
        const historyMessages: Array<{ type: string; message: any }> = [];
        let historyReplayed = false;
        let snapshotConsumed = false;
        let isActive = false;
        let activeRunId = expectedRequestId;

        // Phase 2: live streaming state
        let currentContent = "";
        let currentThinking = "";
        let pendingRecoveryNote: string | undefined;
        let pendingRecoverySummary: string | undefined;
        const currentTools: ToolCall[] = [];
        const currentSegments: ContentSegment[] = [];
        const pendingShellCommandEvents: CommandEvent[] = [];
        const pendingToolResults = new Map<string, ToolResultEventLike>();
        let seenToolIds = new Set<string>();
        let streamingPlanState2 = new Map<string, number>();
        let sawToolResult = false;
        let liveContinuationCount = 0;
        let flushTimer: ReturnType<typeof setTimeout> | null = null;
        let contentDirty = false;
        // Nested sub-agent event buffers (see executeSend for semantics).
        const nestedEventBuffers = new Map<
          string,
          Array<Record<string, unknown>>
        >();
        const toolIdByRunId = new Map<string, string>();
        const parentRunIdByRunId = new Map<string, string>();
        const nestedRunIdsByToolId = new Map<string, Set<string>>();
        const bindNestedRunToTool = (runId: string, toolId: string) => {
          toolIdByRunId.set(runId, toolId);
          const runIds = nestedRunIdsByToolId.get(toolId) || new Set<string>();
          runIds.add(runId);
          nestedRunIdsByToolId.set(toolId, runIds);
          for (const [childRunId, parentRunId] of parentRunIdByRunId) {
            if (parentRunId === runId && !toolIdByRunId.has(childRunId)) {
              bindNestedRunToTool(childRunId, toolId);
            }
          }
        };
        const nestedEventsForTool = (toolId: string) => {
          const runIds = nestedRunIdsByToolId.get(toolId);
          if (!runIds) return [];
          return Array.from(runIds).flatMap(
            (id) => nestedEventBuffers.get(id) || [],
          );
        };
        const mergedNestedEventsForTool = (
          toolId: string,
          existing: Array<Record<string, unknown>> = [],
        ) => {
          const seen = new Set<string>();
          return [...existing, ...nestedEventsForTool(toolId)].filter((ev) => {
            const key = JSON.stringify(ev);
            if (seen.has(key)) return false;
            seen.add(key);
            return true;
          });
        };

        function flushContent() {
          flushTimer = null;
          if (!contentDirty) return;
          contentDirty = false;
          updateLastBlock(tabId, {
            thinking: currentThinking || undefined,
            content: currentContent,
            tools: [...currentTools],
            segments: [...currentSegments],
            statusText: undefined,
          });
        }
        function scheduleFlush() {
          contentDirty = true;
          if (!flushTimer) flushTimer = setTimeout(flushContent, 33);
        }
        function ensureLiveAssistantBlock() {
          const blocks = getChat(tabId).blocks;
          if (blocks[blocks.length - 1]?.role === "assistant") return;

          currentContent = "";
          currentThinking = "";
          currentTools.splice(0);
          currentSegments.splice(0);
          pendingShellCommandEvents.splice(0);
          seenToolIds = new Set();
          streamingPlanState2 = new Map();
          sawToolResult = false;
          const tabModel = useTabStore
            .getState()
            .tabs.find((candidate) => candidate.id === tabId)?.model;
          addBlock(tabId, {
            id: crypto.randomUUID(),
            role: "assistant",
            type: "assistant",
            content: "",
            tools: [],
            isStreaming: true,
            isStarted: false,
            model: tabModel,
          });
        }
        function applyRecoveryNotice(note: unknown, summary: unknown) {
          const nextNote = coerceRecoveryNote(note);
          const nextSummary = coerceRecoverySummary(summary);
          if (!nextNote && !nextSummary) return;
          if (nextNote) pendingRecoveryNote = nextNote;
          if (nextSummary) pendingRecoverySummary = nextSummary;
          const blocks = getChat(tabId).blocks;
          if (blocks[blocks.length - 1]?.role === "assistant") {
            updateLastBlock(tabId, {
              ...(nextNote ? { recoveryNote: nextNote } : {}),
              ...(nextSummary ? { recoverySummary: nextSummary } : {}),
            });
          }
        }

        function resetLiveRunState() {
          if (flushTimer) {
            clearTimeout(flushTimer);
            flushTimer = null;
          }
          contentDirty = false;
          currentContent = "";
          currentThinking = "";
          currentTools.splice(0);
          currentSegments.splice(0);
          pendingShellCommandEvents.splice(0);
          seenToolIds = new Set();
          streamingPlanState2 = new Map();
          sawToolResult = false;
        }

        const retiredRunIds = new Set<string>();
        function beginLiveRun(
          requestId?: string,
          queuedItem?: QueuedMessage,
          allowCompletedAssistantReuse = false,
        ) {
          const resolvedRequestId =
            requestId || activeRunId || expectedRequestId;
          const sameRun =
            isActive &&
            (!resolvedRequestId ||
              !activeRunId ||
              activeRunId === resolvedRequestId);
          if (sameRun) {
            if (resolvedRequestId) {
              activeRunId = resolvedRequestId;
              expectedRequestId = resolvedRequestId;
            }
            runtime.active = true;
            chatRuntime.notify(runtime);
            updateTab(tabId, {
              isStreaming: true,
              requestId: resolvedRequestId,
              requiresResume: false,
              isSessionWarm: false,
              sessionWarmSince: undefined,
            });
            return;
          }

          if (isActive) {
            updateLastBlock(tabId, {
              isStreaming: false,
              statusText: undefined,
            });
          }
          if (activeRunId && activeRunId !== resolvedRequestId) retiredRunIds.add(activeRunId);
          resetLiveRunState();
          isActive = true;
          activeRunId = resolvedRequestId;
          if (resolvedRequestId) expectedRequestId = resolvedRequestId;
          runtime.active = true;
          chatRuntime.notify(runtime);
          tookStreamingOwnership = true;
          abortRef.current = () => {
            userAborted = true;
            const tab = useTabStore
              .getState()
              .tabs.find((candidate) => candidate.id === tabId);
            const activeRequestId = tab?.requestId || activeRunId;
            if (activeRequestId) {
              apiFetch(`/api/abort/${activeRequestId}`, {
                method: "POST",
              }).catch(() => {});
            }
          };

          updateTab(tabId, {
            isStreaming: true,
            streamingStartedAt: Date.now(),
            requestId: resolvedRequestId,
            currentTool: undefined,
            isSessionWarm: false,
            requiresResume: false,
            sessionWarmSince: undefined,
          });

          let blocks = getChat(tabId).blocks;
          let lastBlock = blocks[blocks.length - 1];
          const previousBlock = blocks[blocks.length - 2];
          const hasOptimisticTurn =
            lastBlock?.role === "assistant" &&
            lastBlock.isStreaming === true &&
            (!queuedItem ||
              (previousBlock?.role === "user" &&
                previousBlock.content === queuedItem.message));

          if (queuedItem && !hasOptimisticTurn) {
            const lastAlreadyMatches = hasQueuedUserBlock(
              blocks,
              queuedItem.id,
              queuedItem.message,
            );
            if (!lastAlreadyMatches) {
              addBlock(tabId, {
                id: crypto.randomUUID(),
                requestId: queuedItem.id,
                role: "user",
                type: "user",
                content: queuedItem.message,
                images: queuedItem.options.images,
              });
            }
            blocks = getChat(tabId).blocks;
            lastBlock = blocks[blocks.length - 1];
          }

          const canReuseAssistant =
            lastBlock?.role === "assistant" &&
            (lastBlock.isStreaming === true || allowCompletedAssistantReuse);
          const tabModel = useTabStore
            .getState()
            .tabs.find((candidate) => candidate.id === tabId)?.model;
          if (canReuseAssistant) {
            updateLastBlock(tabId, {
              isStreaming: true,
              ...(tabModel && !lastBlock.model ? { model: tabModel } : {}),
            });
            currentContent = lastBlock.content || "";
            currentThinking = lastBlock.thinking || "";
            if (lastBlock.tools) currentTools.push(...lastBlock.tools);
            if (lastBlock.segments) currentSegments.push(...lastBlock.segments);
            for (const tool of currentTools) seenToolIds.add(tool.id);
          } else {
            addBlock(tabId, {
              id: crypto.randomUUID(),
              role: "assistant",
              type: "assistant",
              content: "",
              tools: [],
              isStreaming: true,
              isStarted: false,
              model: tabModel,
            });
          }
        }

        function beginPostCompactionSubscription(payload: any): boolean {
          const blocks = getChat(tabId).blocks;
          const last = blocks[blocks.length - 1];
          const hasQueuedPrompt =
            payload?.queuedPrompt === true ||
            last?.compaction?.queuedPrompt === true;
          if (!hasQueuedPrompt) return false;
          if (
            last?.type !== "compact" ||
            (payload.runId && last.compaction?.runId !== payload.runId)
          ) {
            return false;
          }
          currentContent = "";
          currentThinking = "";
          currentTools.splice(0);
          currentSegments.splice(0);
          seenToolIds.clear();
          sawToolResult = false;
          const tabModel = useTabStore
            .getState()
            .tabs.find((tab) => tab.id === tabId)?.model;
          addBlock(tabId, {
            id: crypto.randomUUID(),
            role: "assistant",
            type: "assistant",
            content: "",
            tools: [],
            isStreaming: true,
            isStarted: false,
            model: tabModel,
          });
          return true;
        }

        function applyControlWorkflowEvent(
          event: WorkflowEvent | null,
        ): boolean {
          if (!event) return false;
          const payload = event.payload || {};
          if (
            maybeRefreshProjectFilesFromWorkflowEvent(event, streamProjectPath)
          ) {
            return true;
          }
          if (
            event.kind === "command.event" &&
            isCommandEventPayload(payload)
          ) {
            return attachCommandEventToTool(
              payload,
              currentTools,
              pendingShellCommandEvents,
              (toolId, updates) => updateTool(tabId, toolId, updates),
            );
          }
          if (applyQuestionWorkflowEvent(event)) return true;
          if (event.kind === "recovery.notice") {
            applyRecoveryNotice(payload.note, payload.summary);
            if (payload.nativePlanRejected === true) {
              if (payload.clearStalePlan === true) {
                clearPlan(tabId);
                updateTab(tabId, { activePlanId: undefined });
              }
              if (payload.discardAssistantText === true) {
                currentContent = "";
                for (let i = currentSegments.length - 1; i >= 0; i--) {
                  if (currentSegments[i].type === "text")
                    currentSegments.splice(i, 1);
                }
                updateLastBlock(tabId, {
                  content: "",
                  tools: [...currentTools],
                  segments: [...currentSegments],
                });
              }
            }
            return true;
          }
          if (event.kind === "system.thinking") {
            ensureLiveAssistantBlock();
            const lastSeg = currentSegments[currentSegments.length - 1];
            if (
              lastSeg?.type !== "thinking" ||
              !lastSeg.redacted ||
              lastSeg.text
            ) {
              currentSegments.push({
                type: "thinking",
                text: "",
                redacted: true,
              });
            }
            updateLastBlock(tabId, {
              content: currentContent,
              thinking: currentThinking || undefined,
              tools: [...currentTools],
              segments: [...currentSegments],
              statusText: undefined,
            });
            updateTab(tabId, { currentTool: "Thinking…" });
            return true;
          }
          if (event.kind === "system.compacting") {
            applyCompactionEventToStores(tabId, event);
            return true;
          }
          if (event.kind === "system.compact_complete") {
            if (flushTimer) {
              clearTimeout(flushTimer);
              flushContent();
            }
            applyCompactionEventToStores(tabId, event);
            if (beginPostCompactionSubscription(payload)) {
              // A manual compaction can hand this session stream to a successor
              // run with a new request ID. Keep filtering the predecessor replay,
              // then follow the newly claimed run after the checkpoint boundary.
              expectedRequestId = undefined;
            }
            return true;
          }
          if (event.kind === "system.compact_failed") {
            applyCompactionEventToStores(tabId, event);
            return true;
          }
          return false;
        }

        while (true) {
          if (!isCurrentBinding()) {
            cancelForOwner();
            break;
          }
          const { done, value } = await reader.read();
          if (done) break;
          if (!isCurrentBinding()) {
            cancelForOwner();
            break;
          }

          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split("\n");
          buffer = lines.pop() || "";

          for (const line of lines) {
            if (!line.trim()) continue;
            let parsed: any;
            try {
              parsed = JSON.parse(line);
            } catch {
              continue;
            }
            if (!parsed || typeof parsed !== "object") continue;
            if (parsed.type === "ping") continue;
            if (!shouldApplyRealtimeEventToSession(parsed, sessionId)) continue;
            if (applyInputDelivery(parsed, sessionId)) continue;
            if (applyReasoningReroute(parsed, sessionId)) continue;
            // A fresh subscription replays the currently active run. During an
            // interrupt handoff that may still be the stopped predecessor, so
            // never merge another request's journal into this assistant block.
            if (!shouldApplyRealtimeEventToRun(parsed, expectedRequestId, retiredRunIds))
              continue;
            if (
              applyRealtimeContinuityEvent(parsed, {
                runtime,
                now: Date.now(),
                notify: () => chatRuntime.notify(runtime),
                beginLiveRun,
                clearLiveRun: () => {
                  isActive = false;
                  activeRunId = undefined;
                  expectedRequestId = undefined;
                  stopAllStreaming(tabId);
                },
                updateTab: (updates) => updateTab(tabId, updates),
              })
            ) {
              continue;
            }

            if (parsed.type === "recovery") {
              applyRecoveryNotice(parsed.note, parsed.summary);
              continue;
            }

            if (parsed.type === "model_routed" && parsed.routing) {
              if (typeof parsed.projectPath === "string") {
                updateTab(tabId, { projectPath: parsed.projectPath, workspace: parsed.workspace || undefined });
              }
              applyModelRouting(parsed.routing, parsed.requestId || activeRunId || expectedRequestId);
              continue;
            }

            if (parsed.type === "queue_updated") {
              const currentTab = useTabStore
                .getState()
                .tabs.find((tab) => tab.id === tabId);
              if (
                applyRealtimeQueueEvent(parsed, {
                  runtime,
                  workingDirectory: streamProjectPath,
                  sessionId,
                  defaults: {
                    model: currentTab?.model || "claude-sonnet-4-6",
                    reasoningEffort: currentTab?.reasoningEffort || "high",
                    permissionMode: currentTab?.permissionMode || "default",
                  },
                  now: Date.now(),
                  notify: () => chatRuntime.notify(runtime),
                })
              ) {
                continue;
              }
            }

            if (
              parsed.type === "ask_question" ||
              parsed.type === "question_resolved" ||
              parsed.type === "question_cancelled"
            ) {
              if (applyQuestionWorkflowEvent(normalizeWorkflowEvent(parsed))) continue;
            }

            if (parsed.type === REALTIME_SNAPSHOT_COMPLETE) {
              snapshotConsumed = historyReplayed;
              continue;
            }
            if (parsed.type === REALTIME_REPLAY_COMPLETE) {
              if (snapshotConsumed) {
                if (flushTimer) {
                  clearTimeout(flushTimer);
                  flushContent();
                }
                initialSnapshotReady = true;
                resolveSnapshotReady(true);
                if (!isActive && !owner.persistent) {
                  await historyNotification;
                  return false;
                }
              }
              continue;
            }

            if (!historyReplayed) {
              // Phase 1: accumulate history messages
              if (parsed.type === "message") {
                const msg = parsed.message;
                if (msg.type === "assistant") {
                  historyMessages.push({
                    type: "assistant",
                    message: msg.message,
                  });
                } else if (msg.type === "tool_result") {
                  // tool_result stored as user message in DB — processHistoryMessages expects 'user'
                  historyMessages.push({ type: "user", message: msg.message });
                }
              }

              if (parsed.type === "streaming") {
                historyReplayed = true;
                const snapshotActive = parsed.active === true;
                runtime.active = snapshotActive;
                chatRuntime.notify(runtime);

                // Only render from realtime history if blocks aren't already pre-populated.
                // The snapshot only replays assistant/tool_result messages (not user), so
                // pre-populated blocks from the REST history endpoint are more complete.
                if (getChat(tabId).blocks.length === 0) {
                  const tabModel = useTabStore
                    .getState()
                    .tabs.find((t) => t.id === tabId)?.model;
                  const blocks = processHistoryMessages(
                    historyMessages,
                    tabModel,
                  );
                  if (blocks.length > 0) {
                    setBlocks(tabId, blocks);
                  }
                }
                if (!snapshotActive) {
                  // A process can be stopped while its compactor is active. The
                  // inactive snapshot is authoritative even if an older server
                  // never journaled the matching compact_failed event.
                  clearCompactionBlock(tabId);
                  // History-only. A persistent owner stays attached so a later
                  // continuity(active) transition can begin without a reload.
                  runtime.active = false;
                  chatRuntime.notify(runtime);
                  updateTab(tabId, {
                    isStreaming: false,
                    requestId: undefined,
                    currentTool: undefined,
                  });
                  // Consume continuity, questions and queue before declaring
                  // this snapshot ready, then wait for journal catch-up too.
                  continue;
                }

                // The following continuity line supplies the exact request ID.
                // Reuse the assistant reconstructed from history in the meantime.
                beginLiveRun(undefined, undefined, true);
              }
            } else {
              // Phase 2: process live events
              if (parsed.type === "nested_agent_event") {
                const runId = parsed.runId as string | undefined;
                const parentRunId = parsed.parentRunId as string | undefined;
                const event = (parsed.event || {}) as Record<
                  string,
                  unknown
                > & { type?: string };
                if (!runId) continue;
                const eventWithRunId = {
                  ...event,
                  run_id: runId,
                  ...(parentRunId ? { parent_run_id: parentRunId } : {}),
                };
                const buf = nestedEventBuffers.get(runId) || [];
                buf.push(eventWithRunId);
                nestedEventBuffers.set(runId, buf);

                if (parentRunId) {
                  parentRunIdByRunId.set(runId, parentRunId);
                  const parentToolId = toolIdByRunId.get(parentRunId);
                  if (parentToolId && !toolIdByRunId.has(runId)) {
                    bindNestedRunToTool(runId, parentToolId);
                  }
                }

                if (!toolIdByRunId.has(runId)) {
                  const existingIdx = currentTools.findIndex(
                    (t) => t.runId === runId,
                  );
                  if (existingIdx >= 0) {
                    bindNestedRunToTool(runId, currentTools[existingIdx].id);
                  }
                }

                // AgentBatch: batch_start events bind both the batch id and child
                // run ids to the same batch tool so each child's history stays visible.
                if (
                  !toolIdByRunId.has(runId) &&
                  event?.type === "batch_start"
                ) {
                  for (let ti = currentTools.length - 1; ti >= 0; ti--) {
                    const t = currentTools[ti];
                    if (
                      (t.name !== "AgentBatch" &&
                        t.name !== "mcp__flamel__AgentBatch") ||
                      t.runId
                    )
                      continue;
                    bindNestedRunToTool(runId, t.id);
                    const agents = Array.isArray(event.agents)
                      ? (event.agents as Array<Record<string, unknown>>)
                      : [];
                    for (const agent of agents) {
                      const childRunId = agent.run_id;
                      if (typeof childRunId === "string" && childRunId) {
                        bindNestedRunToTool(childRunId, t.id);
                      }
                    }
                    const nestedEvents = mergedNestedEventsForTool(
                      t.id,
                      t.nestedEvents || [],
                    );
                    currentTools[ti] = { ...t, runId, nestedEvents };
                    updateTool(tabId, t.id, {
                      runId,
                      nestedEvents,
                    });
                    break;
                  }
                } else if (
                  !toolIdByRunId.has(runId) &&
                  event?.type === "nested_start"
                ) {
                  const desc = event.description;
                  const subtype = event.subagent_type || "general-purpose";
                  const batchIdx = currentTools.findIndex(
                    (t) =>
                      (t.name === "AgentBatch" ||
                        t.name === "mcp__flamel__AgentBatch") &&
                      Array.isArray(t.nestedEvents) &&
                      t.nestedEvents.some((ev) => {
                        if (
                          ev.type !== "batch_start" ||
                          !Array.isArray(ev.agents)
                        ) {
                          return false;
                        }
                        return (
                          ev.agents as Array<Record<string, unknown>>
                        ).some((agent) => agent.run_id === runId);
                      }),
                  );
                  if (batchIdx >= 0) {
                    const batchTool = currentTools[batchIdx];
                    bindNestedRunToTool(runId, batchTool.id);
                    const nestedEvents = mergedNestedEventsForTool(
                      batchTool.id,
                      batchTool.nestedEvents || [],
                    );
                    currentTools[batchIdx] = { ...batchTool, nestedEvents };
                    updateTool(tabId, batchTool.id, { nestedEvents });
                  } else {
                    for (let ti = currentTools.length - 1; ti >= 0; ti--) {
                      const t = currentTools[ti];
                      if (
                        (t.name !== "Agent" &&
                          t.name !== "mcp__flamel__Agent") ||
                        t.runId
                      )
                        continue;
                      const inp = t.input as Record<string, unknown>;
                      const tdesc = inp?.description;
                      const tsub =
                        (inp?.subagent_type as string) || "general-purpose";
                      if (tdesc === desc && tsub === subtype) {
                        bindNestedRunToTool(runId, t.id);
                        const nestedEvents = mergedNestedEventsForTool(
                          t.id,
                          t.nestedEvents || [],
                        );
                        currentTools[ti] = {
                          ...t,
                          runId,
                          nestedEvents,
                        };
                        updateTool(tabId, t.id, {
                          runId,
                          nestedEvents,
                        });
                        break;
                      }
                    }
                  }
                } else if (toolIdByRunId.has(runId)) {
                  const toolId = toolIdByRunId.get(runId)!;
                  const idx = currentTools.findIndex((t) => t.id === toolId);
                  if (idx >= 0) {
                    const nestedEvents = mergedNestedEventsForTool(
                      toolId,
                      currentTools[idx].nestedEvents || [],
                    );
                    currentTools[idx] = {
                      ...currentTools[idx],
                      nestedEvents,
                    };
                    updateTool(tabId, toolId, {
                      nestedEvents,
                    });
                  }
                }
                continue;
              }

              if (parsed.type === "workflow_event") {
                if (applyControlWorkflowEvent(normalizeWorkflowEvent(parsed)))
                  continue;
              }

              if (parsed.type === "status") {
                ensureLiveAssistantBlock();
                updateLastBlock(tabId, { statusText: parsed.text });
                updateTab(tabId, { currentTool: parsed.text });
              }

              if (
                parsed.type === "ask_question" ||
                parsed.type === "question_resolved" ||
                parsed.type === "question_cancelled"
              ) {
                applyQuestionWorkflowEvent(normalizeWorkflowEvent(parsed));
              }

              if (parsed.type === "recovery") {
                applyRecoveryNotice(parsed.note, parsed.summary);
              }

              if (parsed.type === "system_status") {
                if (applyControlWorkflowEvent(normalizeWorkflowEvent(parsed)))
                  continue;
              }

              if (parsed.type === "message") {
                const msg = parsed.message;

                if (msg.type === "assistant") {
                  ensureLiveAssistantBlock();
                  const content = Array.isArray(msg.message?.content) ? msg.message.content : [];
                  const startsNewBlock = shouldStartAssistantBlockAfterToolResult(
                    sawToolResult, content, currentTools,
                  );
                  sawToolResult = false;
                  if (startsNewBlock) {
                    if (flushTimer) {
                      clearTimeout(flushTimer);
                      flushContent();
                    }
                    updateLastBlock(tabId, { isStreaming: false });
                    stopAllStreaming(tabId); // clear any stuck 'running' tools from the previous turn
                    currentContent = "";
                    currentThinking = "";
                    currentTools.length = 0;
                    currentSegments.length = 0;
                    pendingShellCommandEvents.length = 0;
                    seenToolIds = new Set();
                    streamingPlanState2 = new Map();
                    const tabModel2 = useTabStore
                      .getState()
                      .tabs.find((t) => t.id === tabId)?.model;
                    addBlock(tabId, {
                      id: crypto.randomUUID(),
                      role: "assistant",
                      type: "assistant",
                      content: "",
                      tools: [],
                      isStreaming: true,
                      model: tabModel2,
                      recoveryNote: pendingRecoveryNote,
                      recoverySummary: pendingRecoverySummary,
                    });
                  }

                  let newText = "";
                  let newThinking = "";
                  const newSegments: ContentSegment[] = [];
                  for (const block of content) {
                    if (!block) continue;
                    if (block.type === "thinking") {
                      const text = block.thinking || "";
                      const redacted =
                        block.redacted === true || (!text && !!block.signature);
                      if (text) {
                        newThinking += newThinking ? `\n\n${text}` : text;
                      }
                      if (text || redacted) {
                        appendThinkingSegment(newSegments, text, redacted);
                      }
                    } else if (block.type === "text") {
                      newText += block.text || "";
                      const lastSeg = newSegments[newSegments.length - 1];
                      if (lastSeg?.type === "text")
                        lastSeg.text += block.text || "";
                      else if (block.text)
                        newSegments.push({ type: "text", text: block.text });
                    } else if (block.type === "tool_use") {
                      if (!seenToolIds.has(block.id)) {
                        seenToolIds.add(block.id);
                        currentTools.push(
                          attachPendingCommandEvents(
                            toolFromAssistantSnapshot({
                              id: block.id,
                              name: block.name,
                              input: block.input || {},
                            }, getChat(tabId).blocks.flatMap((entry) => entry.tools || []).reverse().find((tool) => tool.id === block.id)),
                            pendingShellCommandEvents,
                          ),
                        );
                        // Track plan tools for incremental streaming
                        const isPlanTool =
                          block.name === "plan" ||
                          block.name.endsWith("__plan") ||
                          block.name.includes("flamel_plan_");
                        if (isPlanTool) {
                          streamingPlanState2.set(block.id, -1);
                          const currentPlan = getPlan(tabId);
                          if (
                            !currentPlan.planId &&
                            currentPlan.items.length === 0
                          ) {
                            setPlan(
                              tabId,
                              "pending",
                              "Preparing plan...",
                              undefined,
                              [],
                            );
                            setStreamingIds(tabId, []);
                          }
                          openPlanPanel();
                        }
                      } else {
                        // Already seen — update partialInput or finalize input.
                        const idx = currentTools.findIndex(
                          (t) => t.id === block.id,
                        );
                        if (idx >= 0) {
                          if ((block as any)._inputStr !== undefined) {
                            const partial = (block as any)._inputStr as string;
                            currentTools[idx] = {
                              ...currentTools[idx],
                              partialInput: partial,
                            };
                            updateTool(tabId, block.id, {
                              partialInput: partial,
                            });

                            // Stream plan items from partial JSON as they complete
                            if (streamingPlanState2.has(block.id)) {
                              const parsed = parsePartialPlanInput(partial);
                              syncStreamingPlanHeader(parsed);
                              if (
                                parsed &&
                                (parsed.action === "create" ||
                                  parsed.action === "replace_items" ||
                                  parsed.action === "add_items")
                              ) {
                                const prevCount = streamingPlanState2.get(
                                  block.id,
                                )!;
                                if (
                                  parsed.items.length > Math.max(prevCount, 0)
                                ) {
                                  streamingPlanState2.set(
                                    block.id,
                                    parsed.items.length,
                                  );
                                  if (parsed.action === "add_items") {
                                    const currentPlan = getPlan(tabId);
                                    const startPos = currentPlan.items.filter(
                                      (i) => !i.id.startsWith("pending_"),
                                    ).length;
                                    const items: PlanItem[] = parsed.items.map(
                                      (item: any, i: number) => ({
                                        id: `pending_${startPos + i}`,
                                        text: item.text || "",
                                        status: (item.status ||
                                          "todo") as PlanItem["status"],
                                        position: startPos + i,
                                        notes: item.notes || undefined,
                                        section: item.section || undefined,
                                        reasoningFloor:
                                          item.reasoning_floor || "medium",
                                      }),
                                    );
                                    const withoutPending =
                                      currentPlan.items.filter(
                                        (i) => !i.id.startsWith("pending_"),
                                      );
                                    setPlan(
                                      tabId,
                                      currentPlan.planId || "pending",
                                      currentPlan.title || "",
                                      currentPlan.description,
                                      [...withoutPending, ...items],
                                    );
                                  } else {
                                    const items: PlanItem[] = parsed.items.map(
                                      (item: any, i: number) => ({
                                        id: `pending_${i}`,
                                        text: item.text || "",
                                        status: (item.status ||
                                          "todo") as PlanItem["status"],
                                        position: i,
                                        notes: item.notes || undefined,
                                        section: item.section || undefined,
                                        reasoningFloor:
                                          item.reasoning_floor || "medium",
                                      }),
                                    );
                                    const planId =
                                      parsed.action === "create"
                                        ? "pending"
                                        : getPlan(tabId).planId || "pending";
                                    const title =
                                      parsed.title ||
                                      getPlan(tabId).title ||
                                      "";
                                    const desc =
                                      parsed.description ||
                                      getPlan(tabId).description;
                                    setPlan(tabId, planId, title, desc, items);
                                  }
                                  openPlanPanel();
                                }
                              }
                            }
                          } else if (
                            block.input &&
                            Object.keys(block.input).length > 0
                          ) {
                            // Always update — dropped snapshots may have skipped earlier updates
                            currentTools[idx] = {
                              ...currentTools[idx],
                              input: block.input,
                              partialInput: undefined,
                            };
                            updateTool(tabId, block.id, {
                              input: block.input,
                              partialInput: undefined,
                            });
                          }
                        }
                      }

                      // Stream plan items incrementally
                      if (
                        streamingPlanState2.has(block.id) &&
                        block.input &&
                        Object.keys(block.input).length > 0
                      ) {
                        const prevCount = streamingPlanState2.get(block.id)!;
                        const planAct2 = planToolAction(
                          block.name,
                          block.input,
                        );

                        if (planAct2 === "create") {
                          const rawItems = normalizePlanInputItems(
                            block.input?.items,
                          );
                          if (
                            block.input?.title &&
                            rawItems.length > prevCount
                          ) {
                            const newIds = rawItems
                              .slice(prevCount)
                              .map(
                                (_: any, i: number) =>
                                  `pending_${prevCount + i}`,
                              );
                            streamingPlanState2.set(block.id, rawItems.length);
                            const items: PlanItem[] = rawItems.map(
                              (item: any, i: number) => ({
                                id: `pending_${i}`,
                                text: item.text,
                                status: (item.status ||
                                  "todo") as PlanItem["status"],
                                position: i,
                                notes: item.notes || undefined,
                                section: item.section || undefined,
                                reasoningFloor:
                                  item.reasoning_floor || "medium",
                              }),
                            );
                            setPlan(
                              tabId,
                              "pending",
                              block.input.title as string,
                              block.input?.description as string | undefined,
                              items,
                            );
                            setStreamingIds(tabId, newIds);
                          }
                        }

                        if (planAct2 === "replace_items") {
                          const rawItems = normalizePlanInputItems(
                            block.input?.items,
                          );
                          if (rawItems.length > prevCount) {
                            const newIds = rawItems
                              .slice(prevCount)
                              .map(
                                (_: any, i: number) =>
                                  `pending_${prevCount + i}`,
                              );
                            streamingPlanState2.set(block.id, rawItems.length);
                            const currentPlan = getPlan(tabId);
                            const items: PlanItem[] = rawItems.map(
                              (item: any, i: number) => ({
                                id: `pending_${i}`,
                                text: item.text,
                                status: (item.status ||
                                  "todo") as PlanItem["status"],
                                position: i,
                                notes: item.notes || undefined,
                                section: item.section || undefined,
                                reasoningFloor:
                                  item.reasoning_floor || "medium",
                              }),
                            );
                            setPlan(
                              tabId,
                              currentPlan.planId || "pending",
                              currentPlan.title || "",
                              currentPlan.description,
                              items,
                            );
                            setStreamingIds(tabId, newIds);
                          }
                        }

                        if (planAct2 === "update_item") {
                          if (prevCount < 0) {
                            streamingPlanState2.set(block.id, 0);
                            const itemId = block.input?.item_id as string;
                            const status = block.input?.status as
                              PlanItem["status"] | undefined;
                            const notes = block.input?.notes as
                              string | undefined;
                            const text = block.input?.text as
                              string | undefined;
                            if (itemId) {
                              const updates: Partial<PlanItem> = {};
                              if (status) updates.status = status;
                              if (notes !== undefined) updates.notes = notes;
                              if (text !== undefined) updates.text = text;
                              if (Object.keys(updates).length > 0)
                                updateItem(tabId, itemId, updates);
                            }
                          }
                        }

                        if (planAct2 === "add_items") {
                          const rawItems = normalizePlanInputItems(
                            block.input?.items,
                          );
                          if (rawItems.length > prevCount) {
                            const currentPlan = getPlan(tabId);
                            const startPos = currentPlan.items.length;
                            const newIds = rawItems
                              .slice(prevCount)
                              .map(
                                (_: any, i: number) =>
                                  `pending_${startPos + prevCount + i}`,
                              );
                            streamingPlanState2.set(block.id, rawItems.length);
                            const items: PlanItem[] = rawItems.map(
                              (item: any, i: number) => ({
                                id: `pending_${startPos + i}`,
                                text: item.text,
                                status: (item.status ||
                                  "todo") as PlanItem["status"],
                                position: startPos + i,
                                notes: item.notes || undefined,
                                section: item.section || undefined,
                                reasoningFloor:
                                  item.reasoning_floor || "medium",
                              }),
                            );
                            addItems(tabId, items);
                            setStreamingIds(tabId, newIds);
                          }
                        }
                      }

                      newSegments.push({ type: "tool_ref", toolId: block.id });
                    }
                  }
                  currentContent = stripHiddenControlMarkers(newText);
                  currentThinking = newThinking;
                  currentSegments.length = 0;
                  for (const seg of newSegments) {
                    if (seg.type === "text") {
                      currentSegments.push({
                        ...seg,
                        text: stripHiddenControlMarkers(seg.text),
                      });
                    } else {
                      currentSegments.push(seg);
                    }
                  }
                  for (const result of [...pendingToolResults.values()]) {
                    if (!currentTools.some((tool) => tool.id === result.tool_use_id)) continue;
                    applyStreamingToolResult(
                      currentTools, result,
                      (id, updates) => updateTool(tabId, id, updates),
                      bindNestedRunToTool, mergedNestedEventsForTool,
                      () => undefined, pendingToolResults,
                    );
                  }
                  scheduleFlush();
                }

                if (msg.type === "tool_result") {
                  if (!msg.same_turn) sawToolResult = true;
                  const application = applyStreamingToolResult(
                    currentTools,
                    msg,
                    (toolId, updates) => updateTool(tabId, toolId, updates),
                    bindNestedRunToTool,
                    mergedNestedEventsForTool,
                    (id) => getChat(tabId).blocks.flatMap((block) => block.tools || []).reverse().find((tool) => tool.id === id),
                    pendingToolResults,
                  );
                  if (application) {
                    const {
                      tool,
                      isDelta,
                      output: mergedOutput,
                      changed,
                    } = application;
                    if (!isDelta) {
                      updateLastBlock(tabId, { statusText: undefined });
                      updateTab(tabId, { currentTool: undefined });
                    }
                    if (!isDelta && changed) {
                      maybeRefreshProjectFilesFromTool(
                        streamProjectPath,
                        tool.name,
                        tool.input,
                        msg.is_error,
                      );
                    }

                    const toolPlanAct2 = planToolAction(tool.name, tool.input);
                    if (!isDelta && !msg.is_error && changed) {
                      applyPlanWorkflowEvent(
                        parsePlanMarkerOutput(mergedOutput),
                        toolPlanAct2,
                        tool.input,
                      );
                    }
                  }
                }
              }

              if (parsed.type === "done") {
                // Terminal replay reconstructs presentation state too. Clear a
                // live compaction marker before suppressing replay side effects.
                clearCompactionBlock(tabId);
                // Journal replay is reconstruction only. A replayed completion must
                // never launch another continuation or recursively follow a queued run.
                if (!shouldRunRealtimeSideEffects(parsed)) continue;
                if (flushTimer) {
                  clearTimeout(flushTimer);
                  flushTimer = null;
                }
                contentDirty = false;

                const tab = useTabStore
                  .getState()
                  .tabs.find((t) => t.id === tabId);
                const chat = useChatStore.getState().chats[tabId];
                const continuationSignal =
                  extractHiddenControlSignal(currentContent);
                currentContent = continuationSignal.content;
                const continuationDirective =
                  (parsed.continuationDirective as
                    HiddenControlDirective | undefined) ||
                  continuationSignal.continuationDirective;
                const shouldAutoContinue =
                  continuationDirective === "complete"
                    ? false
                    : continuationDirective === "continue"
                      ? liveContinuationCount < MAX_AUTO_CONTINUATIONS
                      : continuationSignal.shouldAutoContinue ||
                        shouldAutoContinueResponse(
                          parsed.stopReason,
                          currentContent,
                          tab?.model,
                          liveContinuationCount,
                        );

                if (
                  shouldAutoContinue &&
                  parsed.completionManaged !== "server" &&
                  parsed.queuePending !== true &&
                  !runtime.queue.some((item) => item.serverOwned) &&
                  tab?.projectPath
                ) {
                  liveContinuationCount++;
                  updateLastBlock(tabId, {
                    content: currentContent,
                    thinking: currentThinking || undefined,
                    tools: [...currentTools],
                    segments: [...currentSegments],
                    statusText: continuationLabel(
                      parsed.stopReason,
                      liveContinuationCount,
                    ),
                  });
                  // Queue a transparent continuation via the same mechanism as executeSend
                  runtime.queue.unshift({
                    id: crypto.randomUUID(),
                    message: continuationPrompt(parsed.stopReason, tab.model),
                    workingDirectory: tab.projectPath,
                    options: {
                      model: tab.model,
                      reasoningEffort: tab.reasoningEffort ?? "high",
                      permissionMode: tab.permissionMode,
                      sessionId: chat?.sessionId || sessionId,
                      __continuationCount: liveContinuationCount,
                      __isContinuation: true,
                      __reuseAssistantBlock: false,
                      __recoveryNote: pendingRecoveryNote,
                      __recoverySummary: pendingRecoverySummary,
                    },
                    createdAt: Date.now(),
                  });
                  chatRuntime.notify(runtime);
                  const next = runtime.queue.shift();
                  if (next) {
                    handoffToExecuteSend = !owner.persistent;
                    runtime.active = true;
                    chatRuntime.notify(runtime);
                    executeSend(
                      next.message,
                      next.workingDirectory,
                      next.options,
                    );
                  }
                  if (owner.persistent) {
                    isActive = false;
                    activeRunId = undefined;
                    expectedRequestId = undefined;
                    resetLiveRunState();
                    continue;
                  }
                  return false; // executeSend takes presentation ownership
                }

                let finalContent = currentContent;
                if (parsed.stopReason === "max_tokens") {
                  finalContent +=
                    "\n\n---\n*Response truncated after multiple continuations.*";
                } else if (parsed.stopReason === "process_died") {
                  finalContent +=
                    "\n\n---\n*Claude process exited after multiple restart attempts. Send another message to restart.*";
                }
                updateLastBlock(tabId, {
                  isStreaming: false,
                  content: finalContent,
                  thinking: currentThinking || undefined,
                  tools: [...currentTools],
                  segments: [...currentSegments],
                  statusText: undefined,
                  stopReason: parsed.stopReason,
                });
                followQueuedRun = parsed.queuePending === true;
                if (!followQueuedRun) stopAllStreaming(tabId);
                runtime.active = followQueuedRun;
                chatRuntime.notify(runtime);
                const sessionIsWarm =
                  !followQueuedRun &&
                  parsed.stopReason !== "process_died" &&
                  !userAborted;
                updateTab(tabId, {
                  isStreaming: followQueuedRun,
                  isSessionWarm: sessionIsWarm,
                  requiresResume:
                    !followQueuedRun && !sessionIsWarm && !!sessionId,
                  sessionWarmSince: sessionIsWarm ? Date.now() : undefined,
                });
                // Detect waiting state
                const isQuestion = looksLikeQuestionForUser(currentContent);
                if (isQuestion) setWaiting(tabId, true);
                if (!followQueuedRun) {
                  window.dispatchEvent(
                    new CustomEvent("flamel:agent-done", { detail: { tabId } }),
                  );
                }
                refreshSessions();
                if (!owner.persistent) return false;
                isActive = false;
                activeRunId = undefined;
                expectedRequestId = undefined;
                resetLiveRunState();
                continue;
              }

              if (parsed.type === "error") {
                updateLastBlock(tabId, {
                  isStreaming: false,
                  content: `Error: ${parsed.error}`,
                });
              }
            }
          }
        }
      } catch (err: any) {
        if (err?.name !== "AbortError") {
          // Startup must receive the real setup error instead of waiting for a
          // generic timeout. Provider error events remain normal chat content.
          if (opts?.onHistoryReady && !initialSnapshotReady) throw err;
          // Connection dropped — blocks already rendered from history, leave as-is
        }
      } finally {
        resolveSnapshotReady(false);
        opts?.signal?.removeEventListener("abort", cancelForOwner);
        const releasedOwnership = chatRuntime.releaseSessionStream(sessionId, owner);
        resolveOwner();
        reader?.cancel().catch(() => {});
        reader = null;
        // Only clean up streaming state if this stream ended naturally. A tab-switch
        // cancellation should just release the subscription so remount can replay state.
        if (
          releasedOwnership &&
          followQueuedRun &&
          opts?.followQueuedRuns !== false &&
          !externallyCancelled &&
          !handoffToExecuteSend
        ) {
          try {
            const history = await apiFetch(
              `/api/sessions/${sessionId}/history`,
            );
            if (history.ok && isCurrentBinding()) {
              const payload = await history.json();
              const model = useTabStore
                .getState()
                .tabs.find((tab) => tab.id === tabId)?.model;
              const blocks = processHistoryMessages(
                payload.messages || [],
                model,
              );
              setBlocks(tabId, blocks);
            }
          } catch {
            // The next WebSocket snapshot remains authoritative if history refresh fails.
          }
          if (!isCurrentBinding()) return false;
          runtime.active = true;
          updateTab(tabId, { isStreaming: true, requiresResume: false });
          chatRuntime.notify(runtime);
          await subscribeToLiveStreamRef.current?.(sessionId, {
            ...opts,
            // The requested run is complete; a durable successor has its own ID.
            requestId: undefined,
          });
        } else if (
          releasedOwnership &&
          tookStreamingOwnership &&
          !externallyCancelled &&
          !handoffToExecuteSend
        ) {
          stopAllStreaming(tabId);
          updateTab(tabId, { isStreaming: false, currentTool: undefined });
          abortRef.current = null;
          runtime.active = false;
          chatRuntime.notify(runtime);
        }
      }
      return false;
    },
    [
      runtime,
      tabId,
      addBlock,
      setBlocks,
      updateLastBlock,
      updateTool,
      setSessionId,
      setWaiting,
      stopAllStreaming,
      updateTab,
      refreshSessions,
    ],
  );

  // Keep ref in sync so executeSend's finally block can call it without creating a circular dep
  subscribeToLiveStreamRef.current = subscribeToLiveStream;

  // Listen for "Continue" button clicks on interrupted blocks
  // Using a ref for sendMessage to avoid stale closure in the event listener
  const sendMessageRef = useRef(sendMessage);
  sendMessageRef.current = sendMessage;
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail?.tabId !== tabId) return;
      const tab = useTabStore.getState().tabs.find((t) => t.id === tabId);
      const chat = useChatStore.getState().chats[tabId];
      if (!tab?.projectPath || isAutonomousOrchestrationTab(tab)) return;
      sendMessageRef.current(
        "Continue from where you left off. For an active plan, mark the current item done immediately when its own acceptance checks pass, before any work on the next item; then mark the next item in_progress. Do not defer completed statuses until the end.",
        tab.projectPath,
        {
          model: tab.model,
          reasoningEffort: tab.reasoningEffort ?? "high",
          permissionMode: tab.permissionMode,
          sessionId: chat?.sessionId,
          __isContinuation: true,
          __reuseAssistantBlock: true,
        },
      );
    };
    window.addEventListener("flamel:continue-response", handler);
    return () =>
      window.removeEventListener("flamel:continue-response", handler);
  }, [tabId]);

  // editAndRerun: truncate history at blockIndex, reset session, re-send edited text
  const editAndRerun = useCallback(
    async (blockIndex: number, newText: string) => {
      const tab = useTabStore.getState().tabs.find((t) => t.id === tabId);
      const chat = useChatStore.getState().chats[tabId];
      if (
        !tab?.projectPath ||
        !chat?.sessionId ||
        isAutonomousOrchestrationTab(tab)
      )
        return;

      // Count how many user blocks appear before blockIndex — that's how many to keep in DB
      const blocks = chat.blocks;
      const keepUserMessageCount = countUserMessagesBefore(blocks, blockIndex);

      // Truncate frontend blocks from blockIndex onward
      useChatStore.getState().truncateBlocks(tabId, blockIndex);
      // Clear session state
      useChatStore.getState().setPendingQuestion(tabId, null);
      useTabStore.getState().updateTab(tabId, {
        isStreaming: false,
        currentTool: undefined,
        requiresResume: false,
        sessionWarmSince: undefined,
      });

      // Truncate backend: clear DB messages from this point, kill warm session, clear claude_session_id
      try {
        await import("../lib/api").then((m) =>
          m.api.sessions.truncate(chat.sessionId!, keepUserMessageCount),
        );
      } catch {
        /* non-fatal */
      }

      // Re-send the edited message — session will start fresh
      await sendMessage(newText, tab.projectPath, {
        model: tab.model,
        reasoningEffort: tab.reasoningEffort ?? "high",
        permissionMode: tab.permissionMode,
        sessionId: undefined, // force new session after truncate cleared claude_session_id
        __forceNewSession: true,
      });
    },
    [tabId, sendMessage],
  );

  return {
    sendMessage,
    steerMessage,
    abort,
    loadHistory,
    subscribeToLiveStream,
    queuedCount,
    queuedMessages,
    updateQueuedMessage,
    removeQueuedMessage,
    interruptWithMessage,
    editAndRerun,
    isActive: runtimeActive,
  };
}
