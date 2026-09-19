import { useState, type ReactNode } from 'react';
import { useTabStore } from '../../stores/tabStore';
import { useChatStore } from '../../stores/chatStore';
import { Icon } from '../ui/Icons';
import { renderHostPanel } from '../../lib/workspaceHost';
import { useConnectionStore } from '../../stores/connectionStore';
import { applyCompactionEventToStores, clearCompactionBlock } from '../../lib/compactionEvents';
import { streamCompactSession } from '../../lib/sessionCompaction';

export default function StatusBar() {
  const connectionMode = useConnectionStore(s => s.mode);
  const isLocalConnection = import.meta.env.VITE_MODE === 'local' && connectionMode !== 'remote';
  const { tabs, activeTabId, updateTab } = useTabStore();
  const tab = tabs.find(t => t.id === activeTabId);
  const chat = useChatStore(s => activeTabId ? s.chats[activeTabId] : undefined);
  const [isCompacting, setIsCompacting] = useState(false);

  const isStreaming = tab?.isStreaming ?? false;
  const model = tab?.model ?? null;

  const blocks = chat?.blocks ?? [];
  const lastUsage = [...blocks].reverse().find(b => b.role === 'assistant' && b.usage)?.usage;
  const contextWindow = 200_000;
  const inputTok = lastUsage?.inputTokens ?? 0;
  const fillPct = inputTok > 0 ? (inputTok / contextWindow) * 100 : 0;
  const fillColor = fillPct >= 90 ? 'var(--error)' : fillPct >= 50 ? 'var(--warning)' : 'var(--accent)';

  function fmt(n: number) {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1000) return `${Math.round(n / 1000)}K`;
    return String(n);
  }

  async function handleCompact() {
    if (!tab?.sessionId || !activeTabId || isStreaming || isCompacting) return;
    const tabId = activeTabId;
    setIsCompacting(true);
    updateTab(tabId, { isStreaming: true, currentTool: 'Collecting context...' });
    try {
      await streamCompactSession(tab.sessionId, {
        onEvent: event => applyCompactionEventToStores(tabId, event),
      });
    } catch (error) {
      clearCompactionBlock(tabId);
      updateTab(tabId, { isStreaming: false, currentTool: undefined });
      console.warn('manual compaction failed', error);
    } finally {
      setIsCompacting(false);
    }
  }

  const canCompact = !!tab?.sessionId && !isStreaming && !isCompacting;

  const cell = (content: ReactNode, title?: string, accent?: boolean, onClick?: () => void) => (
    <div
      title={title}
      onClick={onClick}
      style={{
        display: 'flex', alignItems: 'center', gap: 4,
        padding: '0 8px', height: '100%',
        color: accent ? 'var(--accent)' : 'var(--text-dim)',
        fontSize: 11, fontFamily: 'var(--font-mono)',
        cursor: onClick ? 'pointer' : title ? 'default' : undefined,
        borderRight: '1px solid var(--border-soft)',
        transition: 'background var(--transition-fast)',
        whiteSpace: 'nowrap',
      }}
      onMouseEnter={onClick ? e => { e.currentTarget.style.background = 'var(--surface-2)'; } : undefined}
      onMouseLeave={onClick ? e => { e.currentTarget.style.background = 'transparent'; } : undefined}
    >
      {content}
    </div>
  );

  return (
    <div style={{ position: 'relative' }}>
      <div style={{
        height: 'var(--statusbar-h)',
        background: 'var(--surface)',
        borderTop: '1px solid var(--border-soft)',
        display: 'flex', alignItems: 'stretch',
        flexShrink: 0, overflow: 'hidden',
        position: 'relative',
      }}>
        {false && fillPct > 0 && (
          <div style={{
            position: 'absolute', top: 0, left: 0,
            width: `${Math.min(fillPct, 100)}%`, height: 2,
            background: fillColor,
            transition: 'width 0.4s ease, background 0.3s ease',
            zIndex: 1,
          }} />
        )}
        {/* Left side */}
        <div style={{ display: 'flex', alignItems: 'stretch', flex: 1, minWidth: 0 }}>
          {isStreaming && cell(
            <>
              <span className="spinner" style={{ width: 7, height: 7, border: '1.5px solid var(--accent)', borderTopColor: 'transparent', borderRadius: '50%', display: 'inline-block' }} />
              <span>{isCompacting ? 'compacting' : 'streaming'}</span>
            </>,
            undefined, true
          )}

          {!isStreaming && tab?.isStreaming === false && tab && cell(
            <span style={{ color: 'var(--success)', opacity: 0.7 }}>ready</span>
          )}

          {model && cell(
            model.replace('claude-', '').replace(/-\d+$/, ''),
            `Model: ${model}`
          )}

          {tab?.sessionId && cell(
            <>
              <Icon.Prompts size={10} style={{ flexShrink: 0 }} />
              <span>compact</span>
            </>,
            canCompact ? 'Compact this session now' : isCompacting ? 'Compaction is running' : 'Stop the active stream before compacting',
            isCompacting,
            canCompact ? handleCompact : undefined
          )}

          {false && inputTok > 0 && cell(
            <span style={{ color: fillPct >= 90 ? 'var(--error)' : fillPct >= 75 ? 'var(--warning)' : 'var(--text-dim)' }}>
              {fmt(inputTok)}/{fmt(contextWindow)} ctx
            </span>,
            `Context window: ${fmt(inputTok)} / ${fmt(contextWindow)} tokens (${fillPct.toFixed(1)}% full)${fillPct >= 75 ? ' - compaction recommended soon' : ''}`
          )}
        </div>

        {/* Right side */}
        <div style={{ display: 'flex', alignItems: 'stretch', justifyContent: 'flex-end' }}>
          {!isLocalConnection && renderHostPanel('runnerPicker')}

          {tab?.projectPath && cell(
            <>
              <Icon.Folder size={10} style={{ flexShrink: 0 }} />
              <span style={{ maxWidth: 160, overflow: 'hidden', textOverflow: 'ellipsis' }}>
                {tab.projectPath.split('/').pop()}
              </span>
            </>,
            tab.projectPath
          )}

          {tab?.permissionMode && cell(
            tab.permissionMode === 'bypassPermissions'
              ? <span style={{ color: 'var(--warning)' }}>auto</span>
              : <span>ask</span>,
            `Permission mode: ${tab.permissionMode}`
          )}
        </div>
      </div>
    </div>
  );
}
