import { isAutonomousOrchestrationTab, useTabStore, type Tab } from '../../stores/tabStore';
import { useAppStore } from '../../stores/appStore';
import { api } from '../../lib/api';
import { useEffect, useMemo, useRef, useState } from 'react';
import { DndContext, PointerSensor, useSensor, useSensors, closestCenter } from '@dnd-kit/core';
import { SortableContext, useSortable, horizontalListSortingStrategy } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { Icon } from '../ui/Icons';
import { useConnectionStore } from '../../stores/connectionStore';
import { useChatStore } from '../../stores/chatStore';

const WARM_SESSION_TIMEOUT_MS = 10 * 60 * 1000;

type ChatMap = ReturnType<typeof useChatStore.getState>['chats'];

export function tabRequiresInput(chats: ChatMap, tabId: string): boolean {
  return Boolean(chats[tabId]?.pendingQuestion);
}

function TabItem({ tab, active, inputRequired, onClose, onClick, onRename }: {
  tab: Pick<Tab, 'id' | 'kind' | 'title' | 'isStreaming' | 'sessionId' | 'isSessionWarm' | 'requiresResume' | 'sessionWarmSince' | 'runnerId' | 'runnerName' | 'sessionOwnerKind' | 'orchestrationRunId' | 'orchestrationRole'>;
  active: boolean;
  inputRequired: boolean;
  onClose: () => void;
  onClick: () => void;
  onRename: (title: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: tab.id });
  const connectionMode = useConnectionStore(s => s.mode);
  const isLocalConnection = import.meta.env.VITE_MODE === 'local' && connectionMode !== 'remote';
  const sessionNeedsResume = tab.kind !== 'orchestration' && !!tab.sessionId && !!tab.requiresResume;
  const isInternalOrchestration = tab.kind !== 'orchestration' && isAutonomousOrchestrationTab(tab);
  const canClose = !tab.isStreaming || isAutonomousOrchestrationTab(tab);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  function startEdit(e: React.MouseEvent) {
    e.stopPropagation();
    setDraft(tab.title);
    setEditing(true);
  }

  function commit() {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== tab.title) onRename(trimmed);
    setEditing(false);
  }

  return (
    <div
      ref={setNodeRef}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
        opacity: isDragging ? 0.4 : 1,
        borderTop: active ? '2px solid var(--accent)' : '2px solid transparent',
        borderRight: '1px solid var(--border-soft)',
        background: active ? 'var(--surface)' : 'transparent',
        height: 'var(--tabbar-h)',
        paddingTop: active ? 0 : 2,
      }}
      className="flex items-center gap-1.5 px-3 cursor-pointer shrink-0 relative select-none group"
      onClick={editing ? undefined : onClick}
      {...attributes}
      {...(editing ? {} : listeners)}
    >
      {inputRequired && (
        <span
          role="img"
          aria-label="Input required"
          title="Input required"
          style={{
            width: 7, height: 7, borderRadius: '50%',
            background: 'var(--warning, #f59e0b)',
            boxShadow: '0 0 0 2px color-mix(in srgb, var(--warning, #f59e0b) 20%, transparent)',
            flexShrink: 0,
          }}
        />
      )}
      {/* Running dot (pulsing), warm dot, or cold/resume-needed dot. */}
      {tab.isStreaming ? (
        <span
          title="Session running"
          style={{
            width: 6, height: 6, borderRadius: '50%',
            background: 'var(--accent)',
            flexShrink: 0,
            animation: 'pulse 1.8s ease-in-out infinite',
          }}
        />
      ) : tab.isSessionWarm ? (
        <span
          title="Session warm — instant response"
          style={{
            width: 5, height: 5, borderRadius: '50%',
            background: 'var(--success, #22c55e)',
            flexShrink: 0,
            opacity: 0.7,
          }}
        />
      ) : sessionNeedsResume ? (
        <span
          title="Session cold — will resume on next send"
          style={{
            width: 5, height: 5, borderRadius: '50%',
            background: 'var(--error, #ef4444)',
            flexShrink: 0,
            opacity: 0.85,
          }}
        />
      ) : null}

      {(tab.kind === 'orchestration' || isInternalOrchestration) && !editing && (
        <Icon.Deploy size={10} style={{ color: active ? 'var(--accent)' : 'var(--text-dim)', flexShrink: 0 }} />
      )}

      {editing ? (
        <input
          ref={inputRef}
          value={draft}
          onChange={e => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={e => {
            if (e.key === 'Enter') { e.preventDefault(); commit(); }
            if (e.key === 'Escape') setEditing(false);
          }}
          onPointerDown={e => e.stopPropagation()}
          onClick={e => e.stopPropagation()}
          style={{
            fontSize: 12, width: 90, padding: '1px 3px',
            background: 'var(--surface-3)',
            border: '1px solid var(--accent)',
            borderRadius: 'var(--radius-sm)',
            color: 'var(--text)',
            outline: 'none',
          }}
        />
      ) : (
        <span
          onDoubleClick={startEdit}
          style={{
            color: active ? 'var(--text)' : 'var(--text-muted)',
            fontSize: 12,
            maxWidth: 100,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            transition: 'color var(--transition-fast)',
          }}
        >
          {tab.title}
        </span>
      )}

      {isInternalOrchestration && !editing && (
        <span title="Internal orchestration session" style={{
          fontSize: 9, padding: '0 4px', borderRadius: 3, flexShrink: 0,
          background: 'var(--surface-2)', color: 'var(--text-dim)', border: '1px solid var(--border-soft)',
          lineHeight: '16px',
        }}>
          internal
        </span>
      )}

      {!isLocalConnection && tab.runnerId && (
        <span title={tab.runnerId === 'auto' ? 'Auto-select runner' : `Running on ${tab.runnerName || 'runner'}`} style={{
          fontSize: 9, padding: '0 4px', borderRadius: 3, flexShrink: 0,
          background: tab.runnerId === 'auto' ? 'var(--accent-glow)' : 'rgba(34,197,94,0.1)',
          color: tab.runnerId === 'auto' ? 'var(--accent)' : 'var(--success, #22c55e)',
          border: `1px solid ${tab.runnerId === 'auto' ? 'var(--accent-border, rgba(214,66,50,0.34))' : 'rgba(34,197,94,0.2)'}`,
          lineHeight: '16px',
        }}>
          <Icon.Server size={8} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 2 }} />
          {tab.runnerId === 'auto' ? 'auto' : tab.runnerName || 'remote'}
        </span>
      )}

      <button
        onClick={e => { e.stopPropagation(); if (canClose) onClose(); }}
        aria-label="Close tab"
        disabled={!canClose}
        title={canClose ? 'Close tab' : 'Session is running'}
        style={{
          color: 'var(--text-dim)',
          background: 'none', border: 'none', cursor: canClose ? 'pointer' : 'not-allowed',
          padding: 2, borderRadius: 'var(--radius-sm)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          transition: 'opacity var(--transition-fast), color var(--transition-fast)',
          flexShrink: 0,
        }}
        className="opacity-0 group-hover:opacity-100 tab-close"
        onMouseEnter={e => { e.currentTarget.style.color = 'var(--text)'; }}
        onMouseLeave={e => { e.currentTarget.style.color = 'var(--text-dim)'; }}
      >
        <Icon.Close size={10} />
      </button>

      <style>{`.group:hover .tab-close { opacity: 1; }`}</style>
    </div>
  );
}

export default function TabBar() {
  const { tabs, activeTabId, addTab, removeTab, setActiveTab, reorderTabs, updateTab } = useTabStore();
  const { rightPanelOpen, setRightPanelOpen, refreshSessions } = useAppStore();
  const chats = useChatStore(state => state.chats);
  const visibleTabs = useMemo(
    () => tabs.filter(tab => tab.kind !== 'orchestration' && !isAutonomousOrchestrationTab(tab)),
    [tabs]
  );

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }));

  useEffect(() => {
    const timers: number[] = [];
    for (const tab of tabs) {
      if (!tab.sessionId || !tab.isSessionWarm || !tab.sessionWarmSince || tab.isStreaming) continue;
      const delay = Math.max(0, tab.sessionWarmSince + WARM_SESSION_TIMEOUT_MS - Date.now());
      timers.push(window.setTimeout(() => {
        const current = useTabStore.getState().tabs.find(t => t.id === tab.id);
        if (!current?.sessionId || !current.isSessionWarm || current.isStreaming) return;
        if (current.sessionWarmSince !== tab.sessionWarmSince) return;
        updateTab(tab.id, { isSessionWarm: false, requiresResume: true, sessionWarmSince: undefined });
      }, delay));
    }
    return () => timers.forEach(timer => window.clearTimeout(timer));
  }, [tabs, updateTab]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 't') {
        e.preventDefault();
        addTab();
      }
      if ((e.metaKey || e.ctrlKey) && e.key === 'w') {
        e.preventDefault();
        if (activeTabId) {
          const activeTab = tabs.find(t => t.id === activeTabId);
          if (activeTab && (!activeTab.isStreaming || isAutonomousOrchestrationTab(activeTab))) removeTab(activeTabId);
        }
      }
      if ((e.metaKey || e.ctrlKey) && /^[1-9]$/.test(e.key)) {
        e.preventDefault();
        const idx = parseInt(e.key) - 1;
        if (visibleTabs[idx]) setActiveTab(visibleTabs[idx].id);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [tabs, visibleTabs, activeTabId]);

  useEffect(() => {
    if (visibleTabs.length > 0 && (!activeTabId || !visibleTabs.some(tab => tab.id === activeTabId))) {
      setActiveTab(visibleTabs[0].id);
    }
  }, [visibleTabs, activeTabId, setActiveTab]);

  async function renameTab(tabId: string, title: string) {
    updateTab(tabId, { title });
    const sessionId = tabs.find(t => t.id === tabId)?.sessionId;
    if (sessionId) {
      await api.sessions.update(sessionId, { title }).catch(() => {});
      refreshSessions();
    }
  }

  function handleDragEnd(event: any) {
    const { active, over } = event;
    if (over && active.id !== over.id) {
      const oldIndex = tabs.findIndex(t => t.id === active.id);
      const newIndex = tabs.findIndex(t => t.id === over.id);
      reorderTabs(oldIndex, newIndex);
    }
  }

  return (
    <div
      className="flex items-stretch shrink-0"
      style={{
        height: 'var(--tabbar-h)',
        borderBottom: '1px solid var(--border-soft)',
        background: 'var(--bg)',
      }}
    >
      {/* Tab strip — clips overflow, no scrollbar */}
      <div className="flex items-stretch overflow-hidden" style={{ flex: 1, minWidth: 0 }}>
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
          <SortableContext items={visibleTabs.map(t => t.id)} strategy={horizontalListSortingStrategy}>
            {visibleTabs.map(tab => (
              <TabItem
                key={tab.id}
                tab={tab}
                active={tab.id === activeTabId}
                inputRequired={tabRequiresInput(chats, tab.id)}
                onClick={() => setActiveTab(tab.id)}
                onClose={() => removeTab(tab.id)}
                onRename={title => renameTab(tab.id, title)}
              />
            ))}
          </SortableContext>
        </DndContext>

        {/* New tab button */}
        <button
          onClick={() => addTab()}
          title="New tab (⌘T)"
          style={{
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            width: 32, flexShrink: 0,
            color: 'var(--text-dim)',
            background: 'transparent', border: 'none', cursor: 'pointer',
            borderTop: '2px solid transparent',
            transition: 'color var(--transition-fast)',
          }}
          onMouseEnter={e => (e.currentTarget.style.color = 'var(--text-muted)')}
          onMouseLeave={e => (e.currentTarget.style.color = 'var(--text-dim)')}
        >
          <Icon.Plus size={13} />
        </button>
      </div>

      {/* Right panel toggle — pinned, never scrolls */}
      <div style={{ display: 'flex', alignItems: 'center', padding: '0 4px', gap: 2, flexShrink: 0 }}>
        <button
          onClick={() => setRightPanelOpen(!rightPanelOpen)}
          title="Toggle right panel"
          style={{
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            width: 22, height: 22,
            color: rightPanelOpen ? 'var(--accent)' : 'var(--text-dim)',
            background: rightPanelOpen ? 'var(--accent-glow)' : 'transparent',
            border: 'none', cursor: 'pointer',
            borderRadius: 'var(--radius-sm)',
            transition: 'color var(--transition-fast), background var(--transition-fast)',
          }}
          onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-2)'; e.currentTarget.style.color = 'var(--text-muted)'; }}
          onMouseLeave={e => { e.currentTarget.style.background = rightPanelOpen ? 'var(--accent-glow)' : 'transparent'; e.currentTarget.style.color = rightPanelOpen ? 'var(--accent)' : 'var(--text-dim)'; }}
        >
          <Icon.PanelRight size={13} />
        </button>
      </div>
    </div>
  );
}
