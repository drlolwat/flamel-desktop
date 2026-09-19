import { isAutonomousOrchestrationTab, useTabStore } from '../../stores/tabStore';
import { useChatStore } from '../../stores/chatStore';
import { useAutoScroll } from '../../hooks/useAutoScroll';
import BlockList from './BlockList';
import InputBar from './InputBar';
import BtwPanel from './BtwPanel';
import { AskQuestionBlock } from './AskQuestionBlock';
import { BlockErrorBoundary } from '../ui/BlockErrorBoundary';
import { useEffect, useCallback, useState, useRef, useLayoutEffect } from 'react';
import { api } from '../../lib/api';
import { useAppStore } from '../../stores/appStore';
import { useChat } from '../../hooks/useChat';
import { usePlanStore } from '../../stores/planStore';
import { useProjectFileStore } from '../../stores/projectFileStore';
import { apiFetch } from '../../lib/api';
import { parseInputDelivery } from '../../lib/chatInput';
import { InputDeliveryList } from './InputDeliveryList';
import WorkspaceNotice from './WorkspaceNotice';
import { canPresentSession, readinessForBinding, useSessionReadinessStore, type SessionReadiness } from '../../stores/sessionReadinessStore';

const ACCEPTED_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];


interface Props { tabId: string }

export default function ChatView({ tabId }: Props) {
  const tab = useTabStore(state => state.tabs.find(candidate => candidate.id === tabId));
  const entry = useSessionReadinessStore(state => state.entries[tabId]);
  const hasStreamingBlocks = useChatStore(state => {
    const chat = state.chats[tabId];
    return chat?.sessionId === tab?.sessionId && chat?.blocks.some(block => block.isStreaming === true);
  });
  const readiness = readinessForBinding(entry, tab?.sessionId, tab?.bindingGeneration);
  const hasLocalTurn = readiness ? readiness.allowLocalPresentation : hasStreamingBlocks;
  // A prompt sent from a new tab is already visible and must not disappear
  // while its newly created server session catches up. Restored history can
  // be read while live state catches up; sending still waits for live readiness.
  if (tab?.sessionId && !isAutonomousOrchestrationTab(tab) && readiness?.phase !== 'ready' && !hasLocalTurn && !canPresentSession(readiness)) {
    return (
      <div className="h-full flex flex-col items-center justify-center" role="status" aria-live="polite" aria-busy={readiness?.phase !== 'error'} style={{ gap: 10, color: 'var(--text-muted)', fontSize: 12 }}>
        {readiness?.phase === 'error' ? (
          <>
            <span>{readiness.error || 'Could not load this chat. Your saved messages are still safe.'}</span>
            <button type="button" onClick={() => useSessionReadinessStore.getState().retry(tabId)} style={{ padding: '6px 12px', color: 'var(--text)', background: 'var(--surface)', border: '1px solid var(--border)', borderRadius: 4, cursor: 'pointer' }}>Try again</button>
          </>
        ) : <><span className="spinner" style={{ width: 10, height: 10 }} /><span>Loading chat…</span></>}
      </div>
    );
  }
  return <ChatContent tabId={tabId} syncing={!hasLocalTurn && readiness?.phase !== 'ready' ? readiness : undefined} />;
}

function ChatContent({ tabId, syncing }: Props & { syncing?: SessionReadiness }) {
  const tab = useTabStore(state => state.tabs.find(candidate => candidate.id === tabId));
  const updateTab = useTabStore(state => state.updateTab);
  const chat = useChatStore(state => state.chats[tabId]);
  const pendingQuestion = !tab?.sessionId || chat?.sessionId === tab.sessionId
    ? chat?.pendingQuestion
    : null;
  const visibleBlocks = !tab?.sessionId || chat?.sessionId === tab.sessionId
    ? chat?.blocks || []
    : [];
  const lastBlock = visibleBlocks[visibleBlocks.length - 1];
  const lastBlockContentLen = lastBlock?.content?.length ?? 0;
  const lastBlockThinkingLen = lastBlock?.thinking?.length ?? 0;
  const tabIsStreaming = useTabStore(s => s.tabs.find(t => t.id === tabId)?.isStreaming ?? false);
  const isActiveTab = useTabStore(s => s.activeTabId === tabId);
  const { editAndRerun, isActive: isChatActive } = useChat(tabId);
  const isStreaming = tabIsStreaming || isChatActive;
  const { ref: scrollRef, anchorRef, scrollToBottom } = useAutoScroll([visibleBlocks.length, lastBlockContentLen, lastBlockThinkingLen], isStreaming);
  const setLeftPanel = useAppStore(state => state.setLeftPanel);
  const setLeftPanelOpen = useAppStore(state => state.setLeftPanelOpen);
  const openPlanPanel = useAppStore(state => state.openPlanPanel);
  const setPlan = usePlanStore(state => state.setPlan);
  const refreshGitStatus = useProjectFileStore(s => s.refreshGitStatus);
  const isOrchestrationOwned = isAutonomousOrchestrationTab(tab);
  const prevIsStreaming = useRef<boolean | null>(null);
  const initiallyScrolledSession = useRef<string | null>(null);
  const needsActivationScroll = useRef(true);
  const [dragCount, setDragCount] = useState(0);
  const [btw, setBtw] = useState<{ question: string; sessionId?: string; workingDirectory?: string } | null>(null);
  const receiptsBinding = useRef<string | null>(null);
  const hasQueuedInputs = chat?.sessionId === tab?.sessionId && chat?.inputDeliveries?.some(
    receipt => receipt.delivery === 'queued' && !receipt.runStartedAt,
  );

  useEffect(() => {
    const sessionId = tab?.sessionId;
    if (!sessionId) return;
    const controller = new AbortController();
    let refreshing = false;
    const refreshInputs = () => {
      if (refreshing) return;
      refreshing = true;
      void apiFetch(`/api/chat/${encodeURIComponent(sessionId)}/inputs`, { signal: controller.signal })
      .then(response => response.ok ? response.json() : null)
      .then(payload => {
        if (controller.signal.aborted || !Array.isArray(payload?.inputs)) return;
        for (const value of payload.inputs) {
          const receipt = parseInputDelivery(value, sessionId);
          if (receipt) useChatStore.getState().recordInputDelivery(tabId, receipt, true);
        }
      }).catch(() => {}).finally(() => { refreshing = false; }); };
    // SessionRuntime already fetched these before the transcript was revealed.
    // Refresh on later run transitions and reconnect/focus, not once again at
    // mount (which used to make guidance appear after the loading screen).
    if (receiptsBinding.current === sessionId) refreshInputs();
    receiptsBinding.current = sessionId;
    const refreshTimer = hasQueuedInputs && isActiveTab ? window.setInterval(() => {
      if (document.visibilityState === 'visible') refreshInputs();
    }, 5000) : undefined;
    window.addEventListener('online', refreshInputs);
    window.addEventListener('focus', refreshInputs);
    return () => {
      controller.abort();
      window.clearInterval(refreshTimer);
      window.removeEventListener('online', refreshInputs);
      window.removeEventListener('focus', refreshInputs);
    };
  }, [tabId, tab?.sessionId, tab?.requestId, isStreaming, hasQueuedInputs, isActiveTab]);

  // Tool-result events refresh git status live; this final refresh is a cheap fallback for
  // providers or flows that finish without emitting a file mutation event.
  useEffect(() => {
    if (prevIsStreaming.current === true && !isStreaming && tab?.projectPath) {
      refreshGitStatus(tab.projectPath);
    }
    prevIsStreaming.current = isStreaming;
  }, [isStreaming, tab?.projectPath, refreshGitStatus]);

  // History may already be in Zustand before the realtime snapshot arrives.
  // Anchor the first rendered frame for each opened session independently of
  // network timing, then leave subsequent scroll position under user control.
  useLayoutEffect(() => {
    if (!isActiveTab) {
      needsActivationScroll.current = true;
      return;
    }
    const sessionId = tab?.sessionId;
    if (!sessionId || visibleBlocks.length === 0) return;
    if (
      !needsActivationScroll.current &&
      initiallyScrolledSession.current === sessionId
    ) return;
    let secondFrame = 0;
    const firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => {
        needsActivationScroll.current = false;
        initiallyScrolledSession.current = sessionId;
        scrollToBottom();
      });
    });
    return () => {
      cancelAnimationFrame(firstFrame);
      if (secondFrame) cancelAnimationFrame(secondFrame);
    };
  }, [isActiveTab, tab?.sessionId, visibleBlocks.length, scrollToBottom]);

  useEffect(() => {
    function onBtw(e: Event) {
      const detail = (e as CustomEvent).detail;
      if (detail?.tabId !== tabId) return;
      setBtw({ question: detail.question, sessionId: detail.sessionId, workingDirectory: detail.workingDirectory });
    }
    window.addEventListener('flamel:btw', onBtw);
    return () => window.removeEventListener('flamel:btw', onBtw);
  }, [tabId]);

  const clearResolvedQuestion = useCallback((requestId: string) => {
    const store = useChatStore.getState();
    if (store.getChat(tabId).pendingQuestion?.requestId === requestId) {
      store.setPendingQuestion(tabId, null);
    }
  }, [tabId]);

  const onOpenPlan = useCallback(async () => {
    const sessionId = tab?.sessionId;
    if (!sessionId) return;
    const generation = tab.bindingGeneration || 0;
    const bootstrap = await api.sessions.bootstrap(sessionId).catch(() => null);
    const current = useTabStore.getState().tabs.find(candidate => candidate.id === tabId);
    if (
      !bootstrap
      || current?.sessionId !== sessionId
      || (current.bindingGeneration || 0) !== generation
    ) return;
    const plan = bootstrap.activePlan;
    if (!plan) return;
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
    updateTab(tabId, { activePlanId: plan.id });
    openPlanPanel();
  }, [tab?.bindingGeneration, tab?.sessionId, tabId, openPlanPanel, setPlan, updateTab]);

  if (!tab?.projectPath) return null;

  const isDragging = dragCount > 0;

  function onDragEnter(e: React.DragEvent) {
    if (isOrchestrationOwned) return;
    if (!e.dataTransfer.types.includes('Files')) return;
    e.preventDefault();
    setDragCount(c => c + 1);
  }
  function onDragLeave() {
    setDragCount(c => Math.max(0, c - 1));
  }
  function onDragOver(e: React.DragEvent) {
    if (isOrchestrationOwned) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
  }
  function onDrop(e: React.DragEvent) {
    if (isOrchestrationOwned) return;
    e.preventDefault();
    setDragCount(0);
    const files = Array.from(e.dataTransfer.files).filter(f => ACCEPTED_IMAGE_TYPES.includes(f.type));
    if (files.length) {
      window.dispatchEvent(new CustomEvent('flamel:attach-images', { detail: { files, tabId } }));
    }
  }

  return (
    <div
      className="flex flex-col h-full overflow-hidden"
      style={{ position: 'relative' }}
      onDragEnter={onDragEnter}
      onDragLeave={onDragLeave}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      {isDragging && (
        <div style={{
          position: 'absolute', inset: 0, zIndex: 100,
          background: 'rgba(0,0,0,0.45)',
          border: '2px dashed var(--accent)',
          borderRadius: 'var(--radius-lg)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          pointerEvents: 'none',
        }}>
          <span style={{
            color: 'var(--accent)',
            fontSize: 13,
            fontFamily: 'var(--font-mono)',
            letterSpacing: '0.04em',
          }}>Drop image to attach</span>
        </div>
      )}

      {tab.sessionId && tab.workspace?.isolated && <WorkspaceNotice key={tab.sessionId} sessionId={tab.sessionId} workspace={tab.workspace} />}
      <div ref={scrollRef} className="flex-1 overflow-y-auto">
        <BlockList tabId={tabId} blocks={visibleBlocks} inputDeliveries={chat?.sessionId === tab?.sessionId ? chat?.inputDeliveries : undefined} onOpenPlan={onOpenPlan} readOnly={!!syncing} onEditRerun={isOrchestrationOwned || syncing ? undefined : editAndRerun} />
        <div ref={anchorRef} style={{ height: 1 }} />
      </div>

      <div className="shrink-0" style={{ borderTop: '1px solid var(--border-soft)' }}>
        <InputDeliveryList
          receipts={chat?.sessionId === tab?.sessionId ? chat?.inputDeliveries || [] : []}
          deliveredRequestIds={chat?.sessionId === tab?.sessionId ? chat?.blocks.filter(block => block.type === 'user' && block.requestId).map(block => block.requestId!) : []}
        />
        {chat?.sessionId === tab?.sessionId && chat?.reasoningReroute && (
          <div className="chat-routing-notice" role="status">{chat.reasoningReroute.note}</div>
        )}
        {btw && !isOrchestrationOwned && (
          <div style={{ padding: '8px 12px 0' }}>
            <BtwPanel
              question={btw.question}
              sessionId={btw.sessionId}
              workingDirectory={btw.workingDirectory}
              onClose={() => setBtw(null)}
            />
          </div>
        )}
        {pendingQuestion && !isOrchestrationOwned && !syncing && (
          <div style={{ maxHeight: 'min(52vh, 520px)', overflowY: 'auto', padding: '8px 12px 0' }}>
            <BlockErrorBoundary label="Question">
              <AskQuestionBlock
                key={pendingQuestion.requestId}
                question={pendingQuestion}
                onResolved={() => clearResolvedQuestion(pendingQuestion.requestId)}
              />
            </BlockErrorBoundary>
          </div>
        )}
        {syncing ? (
          <div role="status" style={{ padding: '10px 14px', color: 'var(--text-muted)', fontSize: 12 }}>
            {syncing.phase === 'error' ? <>
              <span>{syncing.error}</span>{' '}
              <button type="button" onClick={() => useSessionReadinessStore.getState().retry(tabId)}>Reconnect</button>
            </> : 'Connecting to live status…'}
          </div>
        ) : isOrchestrationOwned ? (
          <div style={{ padding: '10px 14px', color: 'var(--text-muted)', fontSize: 12 }}>
            Autonomous orchestration transcript. Send guidance from Orchestration.
          </div>
        ) : (
          <InputBar
            tabId={tabId}
            projectPath={tab.projectPath}
            onViewChanges={() => {
              setLeftPanel('changes');
              setLeftPanelOpen(true);
            }}
          />
        )}
      </div>
    </div>
  );
}
