import { lazy, Suspense, useState, useEffect, useCallback, useRef } from 'react';
import Header from './Header';
import TabBar from './TabBar';
import SettingsModal from '../ui/SettingsModal';
import Sidebar from './Sidebar';
import ChatView from '../chat/ChatView';
import SessionRuntime from '../chat/SessionRuntime';
import RightPanel from './RightPanel';
import StatusBar from './StatusBar';
import { Icon } from '../ui/Icons';
import NewSessionScreen from './NewSessionScreen';
import WorkHubView from '../work/WorkHubView';
import { isAutonomousOrchestrationTab, useTabStore } from '../../stores/tabStore';
import { useAppStore, type WorkspaceSelectedChange } from '../../stores/appStore';
import { useIdeStore } from '../../stores/ideStore';
import { api, apiFetch } from '../../lib/api';
import { useProjectFileStore } from '../../stores/projectFileStore';
import { manualModelSelection } from '../../lib/models';
import { useTerminalStore } from '../../stores/terminalStore';
import { useStartupStore } from '../../stores/startupStore';
import { canPresentSession, readinessForBinding, useSessionReadinessStore } from '../../stores/sessionReadinessStore';

const IdePanel = lazy(() => import('../ide/IdePanel'));
const IdeView = lazy(() => import('../sidebar/IdeView'));
const WorkbenchTerminal = lazy(() => import('../ide/WorkbenchTerminal'));
const ProblemsPanel = lazy(() => import('../ide/ProblemsPanel'));

function WorkbenchLoading({ label }: { label: string }) {
  return (
    <div className="h-full flex items-center justify-center" style={{ color: 'var(--text-dim)', fontSize: 11 }}>
      <span className="spinner" style={{ width: 10, height: 10, marginRight: 8 }} />
      {label}
    </div>
  );
}

function useIsMobile() {
  // Detect by touch capability + user agent, not window width.
  // Width-based detection breaks on narrow desktop windows / Tauri startup sizing.
  const isTouchDevice = () =>
    navigator.maxTouchPoints > 0 &&
    /android|iphone|ipad|ipod|mobile/i.test(navigator.userAgent);
  const [mobile, setMobile] = useState(isTouchDevice);
  return mobile;
}

export default function Shell() {
  const { tabs, activeTabId, addTab } = useTabStore();
  const {
    leftPanelOpen,
    rightPanelOpen,
    leftPanelWidth,
    setLeftPanelWidth,
    rightPanelWidth,
    setRightPanelWidth,
    centerView,
    selectedChange,
    mobilePanel,
    setCenterView,
    setSelectedChange,
    setMobilePanel,
  } = useAppStore();
  const { files, activeFilePath, diagnostics, setActiveFile } = useIdeStore();
  const problems = Object.values(diagnostics).flat();
  const problemCount = problems.length;
  const errorCount = problems.filter(problem => problem.severity === 'error').length;
  const terminals = useTerminalStore((state) => state.terminals);
  const activeTerminalId = useTerminalStore((state) => state.activeTerminalId);
  const openTerminal = useTerminalStore((state) => state.openTerminal);
  const setActiveTerminal = useTerminalStore((state) => state.setActiveTerminal);
  const closeTerminal = useTerminalStore((state) => state.closeTerminal);
  const selectedChangeGitLoadedAt = useProjectFileStore(s => selectedChange?.projectPath ? s.projects[selectedChange.projectPath]?.gitLoadedAt ?? null : null);
  const [mobileShowSettings, setMobileShowSettings] = useState(false);
  const [showWork, setShowWork] = useState(false);
  const isMobile = useIsMobile();
  const [ready, setReady] = useState(false);
  const [listReady, setListReady] = useState(false);
  const [listAttempt, setListAttempt] = useState(0);
  const sessionReadiness = useSessionReadinessStore(state => state.entries);
  const [fadeIn, setFadeIn] = useState(false);
  // Keep visited sessions subscribed, but do not download every restored
  // transcript before the user has opened it.
  const [visitedTabs, setVisitedTabs] = useState(() => new Set([activeTabId]));
  useEffect(() => {
    setVisitedTabs(previous => previous.has(activeTabId) ? previous : new Set([...previous, activeTabId]));
  }, [activeTabId]);
  const sessionRuntimes = listReady && tabs
    .filter(tab => tab.id === activeTabId || visitedTabs.has(tab.id))
    .filter(tab => tab.sessionId && tab.kind !== 'orchestration' && !isAutonomousOrchestrationTab(tab))
    .map(tab => <SessionRuntime key={tab.id} tabId={tab.id} />);

  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();
    useStartupStore.getState().advance('workspace');
    // A slow or failed request is not a completed workspace. Keep the overlay
    // and offer retry instead of exposing empty history after a fixed delay.
    const timeout = setTimeout(() => controller.abort(), 30_000);

    const { tabs: initTabs } = useTabStore.getState();

    async function load() {
      // Discover only this signed-in account's surviving PTYs after a refresh.
      void useTerminalStore.getState().restoreTerminals().catch(() => {});
      // Reconcile persisted tabs with server-owned session identities. Each
      // SessionRuntime performs one authoritative bootstrap for its own data.
      const tabsWithSession = initTabs.filter(t => t.sessionId && t.kind !== 'orchestration' && !isAutonomousOrchestrationTab(t));
      const sessionListTask = apiFetch('/api/sessions', { signal: controller.signal }).then(async (res) => {
        if (cancelled) return;
        if (!res.ok) throw new Error('Session list unavailable');
        const sessions = await res.json();
        if (cancelled) return;
        if (controller.signal.aborted) throw new Error('Session list timed out');
        if (!Array.isArray(sessions)) throw new Error('Invalid session list');
        useTabStore.getState().reconcileSessions(sessions);
        const sessionsById = new Map(sessions.map((session: any) => [session.id, session]));
        for (const tab of tabsWithSession) {
          const session = sessionsById.get(tab.sessionId);
          if (!session) continue;
          const tabUpdates = {
            ...(typeof session.title === 'string' && session.title ? { title: session.title } : {}),
            ...((typeof session.requestedModel === 'string' && session.requestedModel)
              || (typeof session.model === 'string' && session.model)
              ? { model: manualModelSelection(session.requestedModel, session.model) }
              : {}),
            ...(typeof session.requestedReasoningEffort === 'string' && session.requestedReasoningEffort
              ? { reasoningEffort: session.requestedReasoningEffort }
              : {}),
            ...(typeof session.model === 'string' && session.model ? { resolvedModel: session.model } : {}),
            ...(typeof session.resolvedReasoningEffort === 'string' && session.resolvedReasoningEffort
              ? { resolvedReasoningEffort: session.resolvedReasoningEffort }
              : {}),
            ...(typeof session.isSessionWarm === 'boolean'
              ? { isSessionWarm: session.isSessionWarm, sessionWarmSince: session.isSessionWarm ? Date.now() : undefined }
              : {}),
            ...(typeof session.requiresResume === 'boolean' ? { requiresResume: session.requiresResume } : {}),
            activePlanId: typeof session.activePlanId === 'string' ? session.activePlanId : undefined,
            sessionOwnerKind: typeof session.sessionOwnerKind === 'string' ? session.sessionOwnerKind : null,
            orchestrationId: typeof session.orchestrationId === 'string' ? session.orchestrationId : undefined,
            orchestrationRunId: typeof session.orchestrationRunId === 'string' ? session.orchestrationRunId : undefined,
            orchestrationRole: typeof session.orchestrationRole === 'string' ? session.orchestrationRole : undefined,
          };
          if (Object.keys(tabUpdates).length > 0) {
            useTabStore.getState().updateTab(tab.id, tabUpdates);
          }
        }
      });
      await sessionListTask;
    }

    void load().then(() => {
      if (!cancelled && !controller.signal.aborted) setListReady(true);
    }).catch(() => {
      if (!cancelled) useStartupStore.getState().fail(
        'Could not load your workspace. Check your connection and try again.',
        () => setListAttempt(attempt => attempt + 1),
      );
    }).finally(() => clearTimeout(timeout));

    return () => { cancelled = true; controller.abort(); clearTimeout(timeout); };
  }, [listAttempt]);

  useEffect(() => {
    if (!listReady || ready) return;
    const boundTabs = tabs.filter(tab => tab.id === activeTabId && tab.sessionId && tab.kind !== 'orchestration' && !isAutonomousOrchestrationTab(tab));
    const failed = boundTabs.filter(tab => {
      const entry = readinessForBinding(sessionReadiness[tab.id], tab.sessionId, tab.bindingGeneration);
      return entry?.phase === 'error' && !canPresentSession(entry);
    });
    if (failed.length) {
      const first = failed[0];
      const detail = sessionReadiness[first.id]?.error || 'This chat could not finish loading.';
      useStartupStore.getState().fail(
        `${first.title || 'Chat'}: ${detail}${failed.length > 1 ? ` ${failed.length - 1} other chats also need a retry.` : ''} Your saved messages have not been changed.`,
        () => {
          useStartupStore.getState().advance('chats');
          for (const tab of failed) useSessionReadinessStore.getState().retry(tab.id);
        },
        // A deleted or unavailable inactive tab must not lock the whole app.
        // This is an explicit escape: failed chats retain their retry screen.
        () => setReady(true),
      );
    } else if (boundTabs.every(tab => canPresentSession(readinessForBinding(sessionReadiness[tab.id], tab.sessionId, tab.bindingGeneration)))) {
      setReady(true);
    } else {
      useStartupStore.getState().advance('chats');
    }
  }, [listReady, ready, sessionReadiness, tabs, activeTabId]);

  useEffect(() => {
    if (ready) {
      // Mount the prepared transcript and settle its initial scroll position
      // underneath the overlay before announcing completion.
      let secondFrame = 0;
      const firstFrame = requestAnimationFrame(() => {
        secondFrame = requestAnimationFrame(() => {
          setFadeIn(true);
          useStartupStore.getState().complete();
        });
      });
      return () => { cancelAnimationFrame(firstFrame); cancelAnimationFrame(secondFrame); };
    }
  }, [ready]);

  const activeTab = tabs.find(t => t.id === activeTabId);
  const hasIdeFiles = files.length > 0;

  useEffect(() => {
    if (centerView === 'ide' && !hasIdeFiles) setCenterView('chat');
  }, [centerView, hasIdeFiles, setCenterView]);

  useEffect(() => {
    if (centerView === 'changes' && !selectedChange) setCenterView('chat');
  }, [centerView, selectedChange, setCenterView]);

  useEffect(() => {
    if (centerView === 'terminal' && !activeTerminalId) setCenterView('chat');
  }, [activeTerminalId, centerView, setCenterView]);

  useEffect(() => {
    if (!selectedChange || !selectedChangeGitLoadedAt) return;
    const status = useProjectFileStore.getState().projects[selectedChange.projectPath]?.gitStatus;
    if (!status?.files) return;
    const nextFile = status.files.find(file => file.path === selectedChange.file.path);
    if (!nextFile) {
      setSelectedChange(null);
      if (centerView === 'changes') setCenterView('chat');
      return;
    }
    if (JSON.stringify(nextFile) !== JSON.stringify(selectedChange.file)) {
      setSelectedChange({ projectPath: selectedChange.projectPath, file: nextFile });
    }
  }, [centerView, selectedChange, selectedChangeGitLoadedAt, setCenterView, setSelectedChange]);

  useEffect(() => {
    const handler = () => {
      setCenterView('ide');
      setMobilePanel('chat');
    };
    window.addEventListener('flamel:open-ide', handler);
    return () => window.removeEventListener('flamel:open-ide', handler);
  }, [setCenterView, setMobilePanel]);

  // Right panel resize
  const resizing = useRef(false);
  const startX = useRef(0);
  const startW = useRef(0);

  const onResizeMove = useCallback((e: MouseEvent) => {
    if (!resizing.current) return;
    const delta = startX.current - e.clientX;
    setRightPanelWidth(startW.current + delta);
  }, [setRightPanelWidth]);

  const onResizeUp = useCallback(() => {
    resizing.current = false;
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
    window.removeEventListener('mousemove', onResizeMove);
    window.removeEventListener('mouseup', onResizeUp);
  }, [onResizeMove]);

  const onResizeDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    resizing.current = true;
    startX.current = e.clientX;
    startW.current = rightPanelWidth;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    window.addEventListener('mousemove', onResizeMove);
    window.addEventListener('mouseup', onResizeUp);
  }, [rightPanelWidth, onResizeMove, onResizeUp]);

  // Left panel resize
  const leftResizing = useRef(false);
  const leftStartX = useRef(0);
  const leftStartW = useRef(0);

  const onLeftResizeMove = useCallback((e: MouseEvent) => {
    if (!leftResizing.current) return;
    const delta = e.clientX - leftStartX.current;
    setLeftPanelWidth(leftStartW.current + delta);
  }, [setLeftPanelWidth]);

  const onLeftResizeUp = useCallback(() => {
    leftResizing.current = false;
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
    window.removeEventListener('mousemove', onLeftResizeMove);
    window.removeEventListener('mouseup', onLeftResizeUp);
  }, [onLeftResizeMove]);

  const onLeftResizeDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    leftResizing.current = true;
    leftStartX.current = e.clientX;
    leftStartW.current = leftPanelWidth;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    window.addEventListener('mousemove', onLeftResizeMove);
    window.addEventListener('mouseup', onLeftResizeUp);
  }, [leftPanelWidth, onLeftResizeMove, onLeftResizeUp]);

  const switchToIde = (path: string) => {
    setActiveFile(path);
    setCenterView('ide');
  };

  const switchToDiff = (file: WorkspaceSelectedChange['file'], projectPath: string) => {
    setSelectedChange({ file, projectPath });
    setCenterView('changes');
  };

  const createTerminal = () => {
    if (!activeTab?.projectPath) return;
    openTerminal(activeTab.projectPath);
    setCenterView('terminal');
    setMobilePanel('chat');
  };

  const removeTerminal = async (terminalId: string) => {
    const record = useTerminalStore.getState().terminals.find((terminal) => terminal.id === terminalId);
    if (record?.sessionId) await api.terminal.close(record.sessionId).catch(() => {});
    closeTerminal(terminalId);
    if (centerView === 'terminal' && activeTerminalId === terminalId) setCenterView('chat');
  };

  // ── Mobile layout ────────────────────────────────────────────────────────
  // Auto-switch to chat panel when a message is dispatched from the plan/drive panel
  useEffect(() => {
    if (!isMobile) return;
    const handler = () => setMobilePanel('chat');
    window.addEventListener('flamel:send-message', handler);
    return () => window.removeEventListener('flamel:send-message', handler);
  }, [isMobile]);

  if (isMobile) {
    return (
      <div className="flex flex-col h-screen overflow-hidden" inert={!ready} style={{ visibility: ready ? 'visible' : 'hidden', background: 'var(--bg)', opacity: fadeIn ? 1 : 0, transition: 'opacity 0.2s ease' }}>
        {sessionRuntimes}
        {/* Slim header */}
        <header className="flex items-center gap-2 px-4 shrink-0"
          style={{ height: 'var(--header-h)', borderBottom: '1px solid var(--border-soft)', background: 'var(--bg)' }}>
          <Icon.Logo size={14} style={{ color: 'var(--accent)' }} />
          <span style={{ color: 'var(--text)', fontWeight: 600, fontSize: 13, letterSpacing: '-0.01em' }}>flamel</span>
          <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 8 }}>
            {tabs.find(t => t.id === activeTabId)?.isStreaming && (
              <span style={{ color: 'var(--accent)', fontSize: 11, display: 'flex', alignItems: 'center', gap: 4 }}>
                <span className="spinner" style={{ width: 7, height: 7, border: '1.5px solid var(--accent)', borderTopColor: 'transparent', borderRadius: '50%', display: 'inline-block' }} />
                streaming
              </span>
            )}
            <button
              onClick={() => setMobileShowSettings(true)}
              style={{ background: 'none', border: 'none', padding: 6, cursor: 'pointer', color: 'var(--text-muted)', display: 'flex', alignItems: 'center' }}
            >
              <Icon.Settings size={16} />
            </button>
          </div>
        </header>
        {mobileShowSettings && <SettingsModal me={null} onClose={() => setMobileShowSettings(false)} />}

        {/* Content area */}
        <div className="flex-1 overflow-hidden relative">
          {/* Main work surface for chat, files, and diffs */}
          <div
            className="h-full relative"
            style={{ display: mobilePanel === 'chat' ? 'block' : 'none' }}
          >
            <div
              className="absolute inset-0"
              aria-hidden={centerView !== 'chat'}
              style={{ display: centerView === 'chat' ? 'block' : 'none' }}
            >
              {activeTab?.projectPath && activeTab.kind !== 'orchestration' && !isAutonomousOrchestrationTab(activeTab) && (
                <div className="h-full flex-col absolute inset-0" style={{ display: 'flex' }}>
                  <ChatView key={`${activeTab.id}:${activeTab.bindingGeneration}`} tabId={activeTab.id} />
                </div>
              )}
            </div>
            {centerView === 'chat' && activeTab && !activeTab.projectPath && (
              <div className="h-full flex-col absolute inset-0" style={{ display: 'flex' }}>
                <NewSessionScreen tabId={activeTab.id} />
              </div>
            )}
            {centerView === 'ide' && hasIdeFiles && (
              <div className="h-full">
                <Suspense fallback={<WorkbenchLoading label="Loading editor" />}><IdePanel /></Suspense>
              </div>
            )}
            {centerView === 'changes' && selectedChange && (
              <div className="h-full overflow-hidden flex flex-col">
              <Suspense fallback={<WorkbenchLoading label="Loading diff" />}>
                <IdeView
                  projectPath={selectedChange.projectPath}
                  file={selectedChange.file}
                  refreshAt={selectedChangeGitLoadedAt}
                  onClose={() => { setSelectedChange(null); setCenterView('chat'); }}
                />
              </Suspense>
              </div>
            )}
            {centerView === 'problems' && (
              <div className="h-full overflow-hidden">
                <Suspense fallback={<WorkbenchLoading label="Loading problems" />}><ProblemsPanel /></Suspense>
              </div>
            )}
            <Suspense fallback={centerView === 'terminal' ? <WorkbenchLoading label="Loading terminal" /> : null}>
              {terminals.map((terminal) => (
                <WorkbenchTerminal key={terminal.id} terminalId={terminal.id} visible={centerView === 'terminal' && activeTerminalId === terminal.id} />
              ))}
            </Suspense>
            {centerView === 'chat' && !activeTab && (
              <div className="flex items-center justify-center h-full flex-col gap-3">
                <Icon.Logo size={32} style={{ color: 'var(--accent)', opacity: 0.5 }} />
                <button
                  onClick={() => addTab()}
                  className="px-4 py-2 text-sm"
                  style={{ background: 'var(--accent)', color: '#fff', borderRadius: 'var(--radius-md)' }}
                >
                  New session
                </button>
              </div>
            )}
          </div>
          {mobilePanel === 'drive' && (
            <div className="absolute inset-0 overflow-hidden" style={{ background: 'var(--surface)' }}>
              <Sidebar
                onOpenFile={(path) => { switchToIde(path); setMobilePanel('chat'); }}
                onOpenDiff={(file, projectPath) => { switchToDiff(file, projectPath); setMobilePanel('chat'); }}
                onOpenSession={() => { setCenterView('chat'); setMobilePanel('chat'); }}
                onOpenWork={() => setShowWork(true)}
              />
            </div>
          )}
          {mobilePanel === 'plan' && (
            <div className="absolute inset-0 overflow-hidden" style={{ background: 'var(--surface)' }}>
              <RightPanel mobile />
            </div>
          )}
        </div>

        {/* Bottom nav */}
        <nav className="flex shrink-0" style={{ borderTop: '1px solid var(--border-soft)', background: 'var(--surface)' }}>
          {([
            { key: 'drive', label: 'Sessions', icon: <Icon.Chat size={16} /> },
            { key: 'chat',  label: 'Chat',     icon: <Icon.Logo size={16} /> },
            { key: 'plan',  label: 'Plan',      icon: <Icon.Plan size={16} /> },
          ] as const).map(({ key, label, icon }) => {
            const active = mobilePanel === key;
            return (
              <button
                key={key}
                onClick={() => setMobilePanel(key)}
                className="flex-1 flex flex-col items-center gap-1 py-2"
                style={{
                  color: active ? 'var(--accent)' : 'var(--text-dim)',
                  fontSize: 10, fontWeight: active ? 500 : 400,
                  border: 'none', background: 'transparent', cursor: 'pointer',
                  borderTop: `2px solid ${active ? 'var(--accent)' : 'transparent'}`,
                  transition: 'color var(--transition-fast)',
                }}
              >
                {icon}
                {label}
              </button>
            );
          })}
        </nav>
        {showWork && <WorkHubView projectPath={activeTab?.projectPath} onClose={() => setShowWork(false)} />}
      </div>
    );
  }

  // ── Desktop layout ───────────────────────────────────────────────────────
  return (
    <div className="flex flex-col h-screen overflow-hidden" inert={!ready} style={{ visibility: ready ? 'visible' : 'hidden', background: 'var(--bg)', opacity: fadeIn ? 1 : 0, transition: 'opacity 0.2s ease' }}>
      {sessionRuntimes}
      <Header />
      <TabBar />
      <div className="flex flex-1 overflow-hidden">
        {leftPanelOpen && (
          <div style={{ width: leftPanelWidth, position: 'relative', background: 'var(--surface)', borderRight: '1px solid var(--border-soft)' }} className="flex flex-col shrink-0">
            <Sidebar onOpenFile={switchToIde} onOpenDiff={switchToDiff} onOpenSession={() => setCenterView('chat')} onOpenWork={() => setShowWork(true)} />
            {/* Resize handle */}
            <div
              onMouseDown={onLeftResizeDown}
              style={{
                position: 'absolute', right: -3, top: 0, bottom: 0, width: 6,
                cursor: 'col-resize', zIndex: 10,
              }}
              onMouseEnter={e => { (e.currentTarget.firstElementChild as HTMLElement).style.opacity = '1'; }}
              onMouseLeave={e => { if (!leftResizing.current) (e.currentTarget.firstElementChild as HTMLElement).style.opacity = '0'; }}
            >
              <div style={{
                position: 'absolute', right: 2, top: 0, bottom: 0, width: 2,
                background: 'var(--accent)', borderRadius: 1, opacity: 0,
                transition: 'opacity 0.15s',
              }} />
            </div>
          </div>
        )}

        <div className="flex-1 overflow-hidden flex flex-col relative">
          {(hasIdeFiles || selectedChange || terminals.length > 0 || problemCount > 0) && (
            <div className="flex shrink-0" style={{ borderBottom: '1px solid var(--border-soft)', background: 'var(--bg)', overflowX: 'auto' }}>
              <button
                type="button"
                onClick={() => setCenterView('chat')}
                className="inline-flex items-center gap-1.5 shrink-0"
                style={{ minHeight: 30, padding: '0 10px', borderRight: '1px solid var(--border-soft)', color: centerView === 'chat' ? 'var(--text)' : 'var(--text-muted)', fontSize: 11, fontWeight: centerView === 'chat' ? 700 : 600, background: centerView === 'chat' ? 'var(--bg)' : 'transparent' }}
              >
                <Icon.Chat size={12} /> Chat
              </button>
              {files.map((file) => {
                const active = centerView === 'ide' && activeFilePath === file.path;
                return (
                  <div key={file.path} className="flex items-center shrink-0" style={{ borderRight: '1px solid var(--border-soft)', background: active ? 'var(--bg)' : 'transparent' }}>
                    <button type="button" onClick={() => { setActiveFile(file.path); setCenterView('ide'); }} className="inline-flex items-center gap-1.5" style={{ minHeight: 30, maxWidth: 210, padding: '0 8px', color: active ? 'var(--text)' : 'var(--text-muted)', fontSize: 11, fontWeight: active ? 700 : 600 }} title={file.path}>
                      <Icon.File size={12} />
                      <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{file.path.split('/').pop()}{file.content !== file.savedContent ? ' *' : ''}</span>
                    </button>
                    <button type="button" onClick={() => { useIdeStore.getState().closeFile(file.path); if (active) setCenterView('chat'); }} style={{ color: 'var(--text-dim)', padding: '0 7px 0 0' }} title="Close file"><Icon.Close size={11} /></button>
                  </div>
                );
              })}
              {selectedChange && (
                <div className="flex items-center shrink-0" style={{ borderRight: '1px solid var(--border-soft)', background: centerView === 'changes' ? 'var(--bg)' : 'transparent' }}>
                  <button type="button" onClick={() => setCenterView('changes')} className="inline-flex items-center gap-1.5" style={{ minHeight: 30, maxWidth: 210, padding: '0 8px', color: centerView === 'changes' ? 'var(--text)' : 'var(--text-muted)', fontSize: 11, fontWeight: centerView === 'changes' ? 700 : 600 }} title={selectedChange.file.path}>
                    <Icon.Branch size={12} /><span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{selectedChange.file.path.split('/').pop()} (diff)</span>
                  </button>
                  <button type="button" onClick={() => { setSelectedChange(null); if (centerView === 'changes') setCenterView('chat'); }} style={{ color: 'var(--text-dim)', padding: '0 7px 0 0' }} title="Close diff"><Icon.Close size={11} /></button>
                </div>
              )}
              {terminals.map((terminal) => {
                const active = centerView === 'terminal' && activeTerminalId === terminal.id;
                return (
                  <div key={terminal.id} className="flex items-center shrink-0" style={{ borderRight: '1px solid var(--border-soft)', background: active ? 'var(--bg)' : 'transparent' }}>
                    <button type="button" onClick={() => { setActiveTerminal(terminal.id); setCenterView('terminal'); }} className="inline-flex items-center gap-1.5" style={{ minHeight: 30, padding: '0 8px', color: active ? 'var(--text)' : 'var(--text-muted)', fontSize: 11, fontWeight: active ? 700 : 600 }}>
                      <span style={{ color: 'var(--accent)', fontFamily: 'var(--font-mono)' }}>&gt;_</span> {terminal.title}
                    </button>
                    <button type="button" onClick={() => void removeTerminal(terminal.id)} style={{ color: 'var(--text-dim)', padding: '0 7px 0 0' }} title="Close terminal"><Icon.Close size={11} /></button>
                  </div>
                );
              })}
              {problemCount > 0 && (
                <button
                  type="button"
                  onClick={() => setCenterView('problems')}
                  className="inline-flex items-center gap-1.5 shrink-0"
                  style={{ minHeight: 30, padding: '0 9px', borderRight: '1px solid var(--border-soft)', color: centerView === 'problems' ? 'var(--text)' : 'var(--text-muted)', background: centerView === 'problems' ? 'var(--bg)' : 'transparent', fontSize: 11, fontWeight: centerView === 'problems' ? 700 : 600 }}
                  title="Open problems"
                >
                  <Icon.Circle size={11} style={{ color: errorCount ? 'var(--error)' : 'var(--warning)' }} />
                  Problems
                  <span style={{ minWidth: 17, padding: '1px 4px', borderRadius: 7, textAlign: 'center', color: errorCount ? 'var(--error)' : 'var(--warning)', background: errorCount ? 'var(--error-bg)' : 'var(--surface-2)', fontFamily: 'var(--font-mono)', fontSize: 9 }}>{problemCount}</span>
                </button>
              )}
              {activeTab?.projectPath && (
                <button type="button" onClick={createTerminal} title="New terminal" style={{ minWidth: 30, minHeight: 30, color: 'var(--text-dim)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center' }}><Icon.Plus size={12} /></button>
              )}
            </div>
          )}

          <div className="flex-1 overflow-hidden relative">
            <div
              className="absolute inset-0"
              aria-hidden={centerView !== 'chat'}
              style={{ display: centerView === 'chat' ? 'block' : 'none' }}
            >
              {activeTab?.projectPath && activeTab.kind !== 'orchestration' && !isAutonomousOrchestrationTab(activeTab) && (
                <div className="h-full flex-col absolute inset-0" style={{ display: 'flex' }}>
                  <ChatView key={`${activeTab.id}:${activeTab.bindingGeneration}`} tabId={activeTab.id} />
                </div>
              )}
            </div>
            {/* New session screen: only render when active (it fetches repos etc.) */}
            {centerView === 'chat' && activeTab && !activeTab.projectPath && (
              <div className="h-full flex-col absolute inset-0" style={{ display: 'flex' }}>
                <NewSessionScreen tabId={activeTab.id} />
              </div>
            )}
            {centerView === 'ide' && hasIdeFiles && (
              <div className="h-full">
                <Suspense fallback={<WorkbenchLoading label="Loading editor" />}><IdePanel /></Suspense>
              </div>
            )}
            {centerView === 'changes' && selectedChange && (
              <div className="h-full overflow-hidden flex flex-col">
                <Suspense fallback={<WorkbenchLoading label="Loading diff" />}>
                  <IdeView
                    projectPath={selectedChange.projectPath}
                    file={selectedChange.file}
                    refreshAt={selectedChangeGitLoadedAt}
                    onClose={() => { setSelectedChange(null); setCenterView('chat'); }}
                  />
                </Suspense>
              </div>
            )}
            {centerView === 'problems' && (
              <div className="h-full overflow-hidden">
                <Suspense fallback={<WorkbenchLoading label="Loading problems" />}><ProblemsPanel /></Suspense>
              </div>
            )}
            <Suspense fallback={centerView === 'terminal' ? <WorkbenchLoading label="Loading terminal" /> : null}>
              {terminals.map((terminal) => (
                <WorkbenchTerminal key={terminal.id} terminalId={terminal.id} visible={centerView === 'terminal' && activeTerminalId === terminal.id} />
              ))}
            </Suspense>
          </div>
        </div>

        {rightPanelOpen && (
          <div style={{ width: rightPanelWidth, position: 'relative', background: 'var(--surface)', borderLeft: '1px solid var(--border-soft)' }} className="flex flex-col shrink-0">
            {/* Resize handle */}
            <div
              onMouseDown={onResizeDown}
              style={{
                position: 'absolute', left: -3, top: 0, bottom: 0, width: 6,
                cursor: 'col-resize', zIndex: 10,
              }}
              onMouseEnter={e => { (e.currentTarget.firstElementChild as HTMLElement).style.opacity = '1'; }}
              onMouseLeave={e => { if (!resizing.current) (e.currentTarget.firstElementChild as HTMLElement).style.opacity = '0'; }}
            >
              <div style={{
                position: 'absolute', left: 2, top: 0, bottom: 0, width: 2,
                background: 'var(--accent)', borderRadius: 1, opacity: 0,
                transition: 'opacity 0.15s',
              }} />
            </div>
            <RightPanel />
          </div>
        )}
      </div>
      <StatusBar />
      {showWork && <WorkHubView projectPath={activeTab?.projectPath} onClose={() => setShowWork(false)} />}
    </div>
  );
}
