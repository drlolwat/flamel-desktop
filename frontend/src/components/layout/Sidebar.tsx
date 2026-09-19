import { hasHostPanel, renderHostPanel } from '../../lib/workspaceHost';
import { useState, useEffect, useRef } from 'react';
import { useAppStore } from '../../stores/appStore';
import { api } from '../../lib/api';
import { isAutonomousOrchestrationTab, useTabStore } from '../../stores/tabStore';
import { useIdeStore } from '../../stores/ideStore';
import { getProjectFileRuntime, getProjectFileView, useProjectFileStore } from '../../stores/projectFileStore';
import FileTree from '../sidebar/FileTree';
import LiveDiffViewer from '../sidebar/LiveDiffViewer';
import { Icon } from '../ui/Icons';
import type { SessionMeta } from '@flamel/shared';
import { apiFetch } from '../../lib/api';
import { manualModelSelection } from '../../lib/models';
import { useConnectionStore } from '../../stores/connectionStore';

function groupSessions(sessions: SessionMeta[]) {
  const now = new Date();
  const today: SessionMeta[] = [];
  const yesterday: SessionMeta[] = [];
  const thisWeek: SessionMeta[] = [];
  const pinned: SessionMeta[] = [];

  for (const s of sessions) {
    if (s.pinned) { pinned.push(s); continue; }
    const d = new Date(s.updatedAt);
    const diffDays = Math.floor((now.getTime() - d.getTime()) / (1000 * 60 * 60 * 24));
    if (diffDays === 0) today.push(s);
    else if (diffDays === 1) yesterday.push(s);
    else thisWeek.push(s);
  }
  return { pinned, today, yesterday, thisWeek };
}

export default function Sidebar({ onOpenFile, onOpenDiff, onOpenSession, onOpenWork }: { onOpenFile?: (path: string) => void; onOpenDiff?: (file: any, projectPath: string) => void; onOpenSession?: () => void; onOpenWork?: () => void }) {
  const { leftPanel, setLeftPanel, setRightPanel, setRightPanelOpen, sessionsVersion, refreshSessions } = useAppStore();
  const connectionMode = useConnectionStore(s => s.mode);
  const isLocalConnection = import.meta.env.VITE_MODE === 'local' && connectionMode !== 'remote';
  const [sessions, setSessions] = useState<SessionMeta[]>([]);
  const [search, setSearch] = useState('');
  const [isAdmin, setIsAdmin] = useState(false);
  const { addTab, removeTab, tabs, setActiveTab, activeTabId, updateTab, reconcileSessions } = useTabStore();
  const activeSessionId = tabs.find(t => t.id === activeTabId)?.sessionId;

  useEffect(() => {
    let current = true;
    api.sessions.list(search || undefined).then(items => {
      if (!current) return;
      setSessions(items);
      // The server owns session identity and titles. Reconcile persisted tabs so
      // reconnects and multi-device updates cannot highlight a stale duplicate.
      reconcileSessions(items);
    }).catch(() => {});
    return () => { current = false; };
  }, [search, sessionsVersion, reconcileSessions]);

  // Check admin status once
  useEffect(() => {
    if (isLocalConnection) return;
    apiFetch('/api/me').then(r => r.json()).then(d => { if (d.isAdmin) setIsAdmin(true); }).catch(() => {});
  }, [isLocalConnection]);

  useEffect(() => {
    if ((leftPanel as string) === 'plans') {
      setLeftPanel('drive');
      setRightPanel('plan-history');
      setRightPanelOpen(true);
      return;
    }
    if (['runners', 'admin'].includes(leftPanel) && (isLocalConnection || !hasHostPanel(leftPanel as 'runners'|'admin'))) {
      setLeftPanel('drive');
    }
  }, [isLocalConnection, leftPanel, setLeftPanel, setRightPanel, setRightPanelOpen]);

  function openSession(session: SessionMeta) {
    const existing = tabs.find(t => t.sessionId === session.id);
    if (existing) {
      setActiveTab(existing.id);
    } else {
      addTab({
        sessionId: session.id,
        projectPath: session.projectPath,
        title: session.title || 'Session',
        model: manualModelSelection(session.requestedModel, session.model),
        reasoningEffort: (session.requestedReasoningEffort as any) || 'auto',
        resolvedModel: session.model as any,
        resolvedReasoningEffort: session.resolvedReasoningEffort as any,
        routingReason: session.routingReason ?? undefined,
        activePlanId: session.activePlanId ?? undefined,
        sessionOwnerKind: session.sessionOwnerKind,
        orchestrationId: session.orchestrationId ?? undefined,
        orchestrationRunId: session.orchestrationRunId ?? undefined,
        orchestrationRole: session.orchestrationRole ?? undefined,
      });
    }
    onOpenSession?.();
  }

  async function deleteSession(session: SessionMeta) {
    // Close any open tab for this session first. Internal orchestration sessions are closable even while their team keeps running.
    const tab = tabs.find(t => t.sessionId === session.id);
    if (tab && (!tab.isStreaming || isAutonomousOrchestrationTab(tab))) removeTab(tab.id);
    await api.sessions.delete(session.id).catch(() => {});
    refreshSessions();
  }

  async function renameSession(sessionId: string, title: string) {
    await api.sessions.update(sessionId, { title }).catch(() => {});
    const tab = tabs.find(t => t.sessionId === sessionId);
    if (tab) updateTab(tab.id, { title });
    refreshSessions();
  }

  const grouped = groupSessions(sessions.filter(session => !isAutonomousOrchestrationTab(session)));

  const activeProjectPath = tabs.find(t => t.id === activeTabId)?.projectPath;
  const activeProjectRuntime = useProjectFileStore(s => getProjectFileRuntime(s.projects, activeProjectPath));
  const activeChangeCount = activeProjectRuntime.gitStatus?.files?.length ?? 0;
  const activeGitLoading = activeProjectRuntime.gitLoading;

  const PANELS = [
    { key: 'drive', label: 'Sessions', icon: <Icon.Chat size={12} /> },
    { key: 'files', label: 'Files', icon: <Icon.Folder size={12} /> },
    {
      key: 'changes',
      label: 'Changes',
      icon: <Icon.Branch size={12} />,
      badge: activeChangeCount > 0 ? String(activeChangeCount) : undefined,
      loading: activeGitLoading,
    },
    ...(!isLocalConnection ? [
      ...(hasHostPanel('runners') ? [{ key: 'runners' as const, label: 'Workers', icon: <Icon.Server size={12} /> }] : []),
      ...(isAdmin && hasHostPanel('admin') ? [{ key: 'admin' as const, label: 'Admin', icon: <Icon.Shield size={12} /> }] : []),
    ] : []),
  ] as { key: Parameters<typeof setLeftPanel>[0]; label: string; icon: React.ReactNode; badge?: string; loading?: boolean }[];

  return (
    <div className="flex h-full overflow-hidden">
      {/* Activity bar */}
      <div className="flex flex-col shrink-0 items-center py-1" style={{ borderRight: '1px solid var(--border-soft)', width: 36 }}>
        {PANELS.map(p => (
          <button
            key={p.key}
            onClick={() => setLeftPanel(p.key)}
            title={p.label}
            style={{
              position: 'relative',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              width: 28, height: 28, borderRadius: 'var(--radius-sm)',
              color: leftPanel === p.key ? 'var(--text)' : 'var(--text-dim)',
              background: leftPanel === p.key ? 'var(--surface-2)' : 'transparent',
              border: 'none', cursor: 'pointer',
              borderLeft: leftPanel === p.key ? '2px solid var(--accent)' : '2px solid transparent',
              transition: 'color var(--transition-fast), background var(--transition-fast)',
            }}
            onMouseEnter={e => { if (leftPanel !== p.key) e.currentTarget.style.color = 'var(--text-muted)'; }}
            onMouseLeave={e => { if (leftPanel !== p.key) e.currentTarget.style.color = 'var(--text-dim)'; }}
          >
            {p.icon}
            {p.loading ? (
              <span
                className="spinner"
                style={{
                  position: 'absolute', top: 2, right: 2,
                  width: 9, height: 9,
                  borderWidth: 1,
                  borderColor: 'var(--accent) transparent var(--accent) var(--accent)',
                }}
              />
            ) : p.badge ? (
              <span
                style={{
                  position: 'absolute', top: 1, right: 1,
                  minWidth: 12, height: 12, padding: '0 3px',
                  borderRadius: 6,
                  background: 'var(--accent)',
                  border: '1px solid var(--surface)',
                  color: '#fff',
                  fontSize: 8,
                  fontFamily: 'var(--font-mono)',
                  fontWeight: 700,
                  lineHeight: '11px',
                  textAlign: 'center',
                }}
              >
                {p.badge}
              </span>
            ) : null}
          </button>
        ))}
      </div>
      {/* Panel content */}
      <div className="flex flex-col flex-1 overflow-hidden">

      {leftPanel === 'drive' && (
        <div className="flex flex-col flex-1 overflow-hidden">
          <div className="px-2 py-2">
            <input
              placeholder="Search sessions…"
              value={search}
              onChange={e => setSearch(e.target.value)}
              className="w-full outline-none"
              style={{
                background: 'var(--surface-2)',
                border: '1px solid var(--border-soft)',
                borderRadius: 'var(--radius-md)',
                color: 'var(--text)',
                fontSize: 12, padding: '5px 9px',
                transition: 'border-color var(--transition-fast)',
              }}
              onFocus={e => (e.currentTarget.style.borderColor = 'var(--border-focus)')}
              onBlur={e => (e.currentTarget.style.borderColor = 'var(--border-soft)')}
            />
          </div>

          <div className="flex-1 overflow-y-auto px-2 pb-4">
            {grouped.pinned.length > 0 && (
              <Section label="Pinned" sessions={grouped.pinned} onOpen={openSession} onDelete={deleteSession} onRename={renameSession} activeSessionId={activeSessionId} />
            )}
            {grouped.today.length > 0 && (
              <Section label="Today" sessions={grouped.today} onOpen={openSession} onDelete={deleteSession} onRename={renameSession} activeSessionId={activeSessionId} />
            )}
            {grouped.yesterday.length > 0 && (
              <Section label="Yesterday" sessions={grouped.yesterday} onOpen={openSession} onDelete={deleteSession} onRename={renameSession} activeSessionId={activeSessionId} />
            )}
            {grouped.thisWeek.length > 0 && (
              <Section label="This week" sessions={grouped.thisWeek} onOpen={openSession} onDelete={deleteSession} onRename={renameSession} activeSessionId={activeSessionId} />
            )}
          </div>

          <div className="mx-2 mb-2 flex flex-col gap-1.5">
            <button
              onClick={() => {
                addTab();
                onOpenSession?.();
              }}
              className="px-3 py-1.5 rounded text-xs flex items-center gap-1.5"
              style={{
                background: 'var(--surface-2)', color: 'var(--text-muted)',
                border: '1px solid var(--border-soft)',
                borderRadius: 'var(--radius-md)',
                transition: 'background var(--transition-fast), color var(--transition-fast)',
              }}
              onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-3)'; e.currentTarget.style.color = 'var(--text)'; }}
              onMouseLeave={e => { e.currentTarget.style.background = 'var(--surface-2)'; e.currentTarget.style.color = 'var(--text-muted)'; }}
            >
              <Icon.Plus size={11} /> New session
            </button>
            <button onClick={onOpenWork} className="px-3 py-1.5 rounded text-xs flex items-center gap-1.5" style={{ background: 'var(--surface-2)', color: 'var(--text-muted)', border: '1px solid var(--border-soft)', borderRadius: 'var(--radius-md)', fontWeight: 600 }}><Icon.Plan size={11} /> Open Work</button>
          </div>
        </div>
      )}

      {leftPanel === 'files' && <FilesTab onOpenFile={onOpenFile} />}
      {leftPanel === 'changes' && <ChangesTab onOpenFile={onOpenFile} onOpenDiff={onOpenDiff} onOpenChat={onOpenSession} />}
      {!isLocalConnection && leftPanel === 'runners' && renderHostPanel('runners')}
      {!isLocalConnection && leftPanel === 'admin' && renderHostPanel('admin')}
      </div>
    </div>
  );
}

function Section({ label, sessions, onOpen, onDelete, onRename, activeSessionId }: { label: string; sessions: SessionMeta[]; onOpen: (s: SessionMeta) => void; onDelete: (s: SessionMeta) => void; onRename: (id: string, title: string) => void; activeSessionId?: string }) {
  const [open, setOpen] = useState(true);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editingId) inputRef.current?.select();
  }, [editingId]);

  function startEdit(s: SessionMeta, e: React.MouseEvent) {
    e.stopPropagation();
    setDraft(s.title || '');
    setEditingId(s.id);
  }

  function commit(s: SessionMeta) {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== s.title) onRename(s.id, trimmed);
    setEditingId(null);
  }

  return (
    <div style={{ marginBottom: 4 }}>
      <button
        onClick={() => setOpen(o => !o)}
        className="w-full flex items-center gap-1 text-left"
        style={{ padding: '3px 6px', border: 'none', background: 'transparent', cursor: 'pointer' }}
      >
        <span style={{ color: 'var(--text-dim)', display: 'flex' }}>
          {open ? <Icon.ChevronDown size={11} /> : <Icon.ChevronRight size={11} />}
        </span>
        <span className="panel-section-title" style={{ padding: 0 }}>{label}</span>
      </button>
      {open && sessions.map(s => {
        const isActive = s.id === activeSessionId;
        const isEditing = s.id === editingId;
        return (
          <div
            key={s.id}
            className="group flex items-center"
            style={{
              height: 26,
              background: isActive ? 'var(--surface-2)' : 'transparent',
              borderLeft: isActive ? '2px solid var(--accent)' : '2px solid transparent',
              transition: 'background var(--transition-fast)',
            }}
            onMouseEnter={e => { if (!isActive) e.currentTarget.style.background = 'var(--surface-2)'; }}
            onMouseLeave={e => { e.currentTarget.style.background = isActive ? 'var(--surface-2)' : 'transparent'; }}
          >
            {isEditing ? (
              <input
                ref={inputRef}
                value={draft}
                onChange={e => setDraft(e.target.value)}
                onBlur={() => commit(s)}
                onKeyDown={e => {
                  if (e.key === 'Enter') { e.preventDefault(); commit(s); }
                  if (e.key === 'Escape') setEditingId(null);
                }}
                onClick={e => e.stopPropagation()}
                style={{
                  flex: 1, padding: '0 8px', fontSize: 12,
                  background: 'var(--surface-3)',
                  border: '1px solid var(--accent)',
                  borderRadius: 'var(--radius-sm)',
                  color: 'var(--text)',
                  outline: 'none', height: '100%',
                }}
              />
            ) : (
              <button
                onClick={() => onOpen(s)}
                onDoubleClick={e => startEdit(s, e)}
                className="flex-1 text-left truncate"
                style={{
                  padding: '0 8px', fontSize: 12,
                  color: isActive ? 'var(--text)' : 'var(--text-muted)',
                  fontWeight: isActive ? 500 : 400,
                  background: 'none', border: 'none', cursor: 'pointer',
                }}
              >
                {s.title || 'Untitled session'}
              </button>
            )}
            {!isEditing && (
              <button
                onClick={e => { e.stopPropagation(); onDelete(s); }}
                className="opacity-0 group-hover:opacity-100 shrink-0"
                style={{
                  padding: '2px 6px', color: 'var(--text-dim)',
                  background: 'none', border: 'none', cursor: 'pointer',
                  display: 'flex', alignItems: 'center',
                  transition: 'color var(--transition-fast)',
                }}
                title="Delete"
                onMouseEnter={e => (e.currentTarget.style.color = 'var(--error)')}
                onMouseLeave={e => (e.currentTarget.style.color = 'var(--text-dim)')}
              >
                <Icon.Close size={10} />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

function FilesTab({ onOpenFile }: { onOpenFile?: (path: string) => void }) {
  const { tabs, activeTabId } = useTabStore();
  const activeTab = tabs.find(t => t.id === activeTabId);
  const projectPath = activeTab?.projectPath ?? null;
  const { activeFilePath } = useIdeStore();
  const runtime = useProjectFileStore(s => getProjectFileRuntime(s.projects, projectPath));
  const view = useProjectFileStore(s => getProjectFileView(s.views, projectPath));
  const loadProject = useProjectFileStore(s => s.loadProject);
  const refreshGitStatus = useProjectFileStore(s => s.refreshGitStatus);
  const toggleExpanded = useProjectFileStore(s => s.toggleExpanded);
  const setExpanded = useProjectFileStore(s => s.setExpanded);
  const selectPath = useProjectFileStore(s => s.selectPath);
  const setSearch = useProjectFileStore(s => s.setSearch);
  const setShowHidden = useProjectFileStore(s => s.setShowHidden);
  const setSortMode = useProjectFileStore(s => s.setSortMode);
  const setScrollTop = useProjectFileStore(s => s.setScrollTop);
  const recordOpen = useProjectFileStore(s => s.recordOpen);

  useEffect(() => {
    if (!projectPath) return;
    loadProject(projectPath);
    refreshGitStatus(projectPath);
  }, [projectPath, view.showHidden, loadProject, refreshGitStatus]);

  if (!projectPath) {
    return <div className="p-3 text-xs" style={{ color: 'var(--text-muted)' }}>No project selected</div>;
  }

  const currentProjectPath = projectPath;

  function refresh() {
    loadProject(currentProjectPath, { force: true });
    refreshGitStatus(currentProjectPath);
  }

  async function openFile(path: string) {
    recordOpen(currentProjectPath, path);
    try {
      const { content, revision } = await api.fileContent.get(path);
      useIdeStore.getState().openFile(path, content, revision);
      onOpenFile?.(path);
    } catch {}
  }

  return (
    <FileTree
      projectPath={currentProjectPath}
      tree={runtime.tree}
      activeFilePath={activeFilePath}
      selectedPath={view.selectedPath}
      expandedPaths={view.expandedPaths}
      search={view.search}
      showHidden={view.showHidden}
      sortMode={view.sortMode}
      scrollTop={view.scrollTop}
      gitStatus={runtime.gitStatus}
      loading={runtime.loading}
      error={runtime.error}
      onRefresh={refresh}
      onOpenFile={openFile}
      onToggleExpanded={(path) => toggleExpanded(currentProjectPath, path)}
      onSetExpanded={(path, expanded) => setExpanded(currentProjectPath, path, expanded)}
      onSelectPath={(path) => selectPath(currentProjectPath, path)}
      onSearchChange={(search) => setSearch(currentProjectPath, search)}
      onShowHiddenChange={(showHidden) => setShowHidden(currentProjectPath, showHidden)}
      onSortModeChange={(mode) => setSortMode(currentProjectPath, mode)}
      onScrollChange={(scrollTop) => setScrollTop(currentProjectPath, scrollTop)}
    />
  );
}

export function ChangesTab({ onOpenFile, onOpenDiff, onOpenChat }: { onOpenFile?: (path: string) => void; onOpenDiff?: (file: any, projectPath: string) => void; onOpenChat?: () => void }) {
  const { tabs, activeTabId } = useTabStore();
  const activeTab = tabs.find(t => t.id === activeTabId);
  const projectPath = activeTab?.projectPath ?? null;

  if (!projectPath) return (
    <div className="p-3 text-xs" style={{ color: 'var(--text-muted)' }}>No project selected</div>
  );

  return (
    <LiveDiffViewer
      projectPath={projectPath}
      onOpenFile={onOpenFile}
      onOpenDiff={onOpenDiff}
      onOpenChat={onOpenChat}
    />
  );
}
