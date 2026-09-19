import { useEffect, useRef } from 'react';
import { useChat } from '../../hooks/useChat';
import { api, apiFetch } from '../../lib/api';
import { parseInputDelivery } from '../../lib/chatInput';
import { processHistoryMessages } from '../../lib/historyProcessor';
import { useChatStore } from '../../stores/chatStore';
import { usePlanStore } from '../../stores/planStore';
import { useTabStore } from '../../stores/tabStore';
import { useSessionReadinessStore } from '../../stores/sessionReadinessStore';
import { manualModelSelection } from '../../lib/models';

interface Props {
  tabId: string;
}

async function loadInputReceipts(sessionId: string, signal: AbortSignal) {
  const response = await apiFetch(`/api/chat/${encodeURIComponent(sessionId)}/inputs`, { signal });
  if (!response.ok) throw new Error(`${response.status}: Could not load saved session inputs.`);
  const payload = await response.json();
  if (!Array.isArray(payload?.inputs)) throw new Error('The server returned invalid session inputs.');
  return payload.inputs.map((value: unknown) => {
    const receipt = parseInputDelivery(value, sessionId);
    if (!receipt) throw new Error('The server returned inputs for an invalid session.');
    return receipt;
  });
}

function isMissingSession(error: unknown): boolean {
  return error instanceof Error && /^(?:404(?::|$)|Request failed \(404\))/.test(error.message);
}

function waitForAdmission(signal: AbortSignal, delay: number): Promise<void> {
  return new Promise(resolve => {
    const finish = () => {
      clearTimeout(timer);
      signal.removeEventListener('abort', finish);
      resolve();
    };
    const timer = setTimeout(finish, delay);
    signal.addEventListener('abort', finish, { once: true });
    if (signal.aborted) finish();
  });
}

/**
 * Owns the durable session binding without keeping an inactive ChatView tree
 * mounted. A tab's persisted history and realtime stream always share the
 * same exact session identity and binding generation.
 */
export default function SessionRuntime({ tabId }: Props) {
  const tab = useTabStore(state => state.tabs.find(candidate => candidate.id === tabId));
  const updateTab = useTabStore(state => state.updateTab);
  const hydrateHistory = useChatStore(state => state.hydrateHistory);
  const setSessionId = useChatStore(state => state.setSessionId);
  const setPlan = usePlanStore(state => state.setPlan);
  const clearPlan = usePlanStore(state => state.clearPlan);
  const retry = useSessionReadinessStore(state => state.retries[tabId] || 0);
  const { subscribeToLiveStream } = useChat(tabId);
  // Effect cleanup removes its readiness entry before a retry starts. Keep
  // this binding's original local-turn decision across that cleanup, so live
  // replay cannot turn itself into a newly submitted local prompt.
  const localPresentation = useRef<{ sessionId: string; generation: number; allowed: boolean } | undefined>(undefined);

  useEffect(() => {
    if (!tab?.sessionId) return;

    const expectedSessionId = tab.sessionId;
    const expectedGeneration = tab.bindingGeneration || 0;
    const controller = new AbortController();
    const isCurrentBinding = () => {
      const current = useTabStore.getState().tabs.find(candidate => candidate.id === tabId);
      return current?.sessionId === expectedSessionId
        && (current.bindingGeneration || 0) === expectedGeneration;
    };
    if (!isCurrentBinding()) return;
    setSessionId(tabId, expectedSessionId);
    const expectedBlocks = useChatStore.getState().getChat(tabId).blocks;
    const readiness = useSessionReadinessStore.getState();
    if (localPresentation.current?.sessionId !== expectedSessionId
      || localPresentation.current.generation !== expectedGeneration) {
      const previous = readiness.entries[tabId];
      localPresentation.current = {
        sessionId: expectedSessionId,
        generation: expectedGeneration,
        allowed: previous?.sessionId === expectedSessionId && previous.generation === expectedGeneration
          ? previous.allowLocalPresentation
          : expectedBlocks.some(block => block.isStreaming),
      };
    }
    const pendingLocalAdmission = localPresentation.current.allowed;
    const attempt = readiness.begin(tabId, expectedSessionId, expectedGeneration, pendingLocalAdmission);
    const isCurrent = () => !controller.signal.aborted && isCurrentBinding()
      && useSessionReadinessStore.getState().entries[tabId]?.attempt === attempt;
    let snapshotReady = false;
    const fail = (message: string) => {
      if (!isCurrent()) return;
      readiness.transition(tabId, attempt, 'error', message);
      clearTimeout(watchdog);
      controller.abort();
    };
    const watchdog = setTimeout(() => {
      const phase = useSessionReadinessStore.getState().entries[tabId]?.phase;
      fail(phase === 'state'
        ? 'Chat history loaded, but the live connection did not finish syncing. Try again.'
        : 'Chat history took too long to download. Try again.');
    }, 30_000);

    void (async () => {
      // A brand-new local send binds its session before the POST persists it.
      // Retry only that brief 404 admission race, never treat missing data as ready.
      let loaded;
      for (let readAttempt = 0; isCurrent(); readAttempt++) {
        const results = await Promise.allSettled([
          api.sessions.bootstrap(expectedSessionId, { signal: controller.signal }),
          loadInputReceipts(expectedSessionId, controller.signal),
        ]);
        if (!isCurrent()) return;
        const failures = results.filter(result => result.status === 'rejected');
        if (failures.length) {
          if (pendingLocalAdmission && failures.every(result => isMissingSession(result.reason))) {
            // Admission can be slower than a round trip on a busy server. The
            // enclosing watchdog still aborts both reads and this wait at 30s.
            await waitForAdmission(controller.signal, Math.min(500 * 2 ** readAttempt, 2_000));
            continue;
          }
          throw failures[0].reason;
        }
        if (results[0].status === 'fulfilled' && results[1].status === 'fulfilled') {
          loaded = [results[0].value, results[1].value] as const;
          break;
        }
      }
      if (!loaded || !isCurrent()) return;
      const [bootstrap, receipts] = loaded;
      if (
        bootstrap.binding.sessionId !== expectedSessionId
        || bootstrap.binding.streamSessionId !== expectedSessionId
        || bootstrap.session.id !== expectedSessionId
      ) throw new Error('The server returned a different session. Try again.');

      const session = bootstrap.session;
      const hydrated = hydrateHistory(
        tabId,
        expectedSessionId,
        expectedBlocks,
        processHistoryMessages(bootstrap.messages || [], session.model || tab.model),
      );
      // History, request identity and plan metadata are one snapshot. When a
      // local turn has advanced, keep all of them and still attach the stream.
      if (hydrated) {
        updateTab(tabId, {
          title: session.title || tab.title,
          projectPath: session.projectPath,
          model: manualModelSelection(session.requestedModel, session.model),
          reasoningEffort:
            (session.requestedReasoningEffort as typeof tab.reasoningEffort)
            || tab.reasoningEffort,
          resolvedModel: session.model as typeof tab.resolvedModel,
          resolvedReasoningEffort:
            session.resolvedReasoningEffort as typeof tab.resolvedReasoningEffort,
          routingReason: session.routingReason || undefined,
          activePlanId: bootstrap.binding.activePlanId || undefined,
          requestId: bootstrap.binding.activeRequestId || undefined,
          ...(typeof session.isSessionWarm === 'boolean'
            ? {
                isSessionWarm: session.isSessionWarm,
                sessionWarmSince: session.isSessionWarm ? Date.now() : undefined,
              }
            : {}),
          ...(typeof session.requiresResume === 'boolean'
            ? { requiresResume: session.requiresResume }
            : {}),
        });
        if (!isCurrentBinding()) return;

        const plan = bootstrap.activePlan;
        if (plan && plan.id === bootstrap.binding.activePlanId) {
          setPlan(tabId, plan.id, plan.title, plan.description, plan.items, false, {
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
          });
        } else {
          clearPlan(tabId);
        }
      }
      if (!isCurrent()) return;
      for (const receipt of receipts) useChatStore.getState().recordInputDelivery(tabId, receipt, true);
      readiness.transition(tabId, attempt, 'state');

      await subscribeToLiveStream(expectedSessionId, {
        signal: controller.signal,
        bindingGeneration: expectedGeneration,
        historyLoaded: true,
        persistent: true,
        onHistoryReady: () => {
          if (!isCurrent()) return;
          snapshotReady = true;
          clearTimeout(watchdog);
          readiness.transition(tabId, attempt, 'ready');
        },
      });
      if (!snapshotReady) fail('The live session state could not be loaded. Try again.');
    })().catch(error => {
      fail(error instanceof Error ? error.message : 'The session could not be loaded. Try again.');
    });

    return () => {
      clearTimeout(watchdog);
      controller.abort();
      readiness.remove(tabId, attempt);
    };
  }, [
    clearPlan,
    hydrateHistory,
    retry,
    setPlan,
    setSessionId,
    subscribeToLiveStream,
    tab?.bindingGeneration,
    tab?.sessionId,
    tabId,
    updateTab,
  ]);

  return null;
}
