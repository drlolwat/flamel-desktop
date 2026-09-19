import React, { useEffect, useRef, useState } from 'react';
import { useTabStore } from '../../stores/tabStore';
import { useChatStore } from '../../stores/chatStore';
import { Icon } from '../ui/Icons';
import type { ToolCall } from '../../lib/messageProcessor';
import { apiFetch } from '../../lib/api';
import { useAgentStream, type AgentRunWithOutput } from '../../hooks/useAgentStream';

function useElapsedSeconds(startedAt: number | string | undefined): number {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (!startedAt) { setElapsed(0); return; }
    const ts = typeof startedAt === 'number' ? startedAt : new Date(startedAt).getTime();
    const update = () => setElapsed(Math.floor((Date.now() - ts) / 1000));
    update();
    const id = setInterval(update, 1000);
    return () => clearInterval(id);
  }, [startedAt]);
  return elapsed;
}

function formatElapsed(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return s > 0 ? `${m}m ${s}s` : `${m}m`;
}

// Parse mcp__{server}__{action} → friendly label
function toolLabel(name: string): string {
  if (!name) return '';
  const parts = name.split('__');
  if (parts[0] === 'mcp' && parts.length >= 3) {
    const server = parts[1].replace(/_/g, ' ');
    const action = parts.slice(2).join(' ').replace(/_/g, ' ');
    return `${server} · ${action}`;
  }
  return name;
}

function SectionHeader({ label, count }: { label: string; count?: number }) {
  return (
    <div style={{
      fontSize: 10,
      fontWeight: 600,
      color: 'var(--text-dim)',
      textTransform: 'uppercase',
      letterSpacing: '0.06em',
      marginBottom: 8,
    }}>
      {label}{count !== undefined ? ` · ${count}` : ''}
    </div>
  );
}

interface SubagentInfo {
  tabId: string;
  tabTitle: string;
  tool: ToolCall;
  startedAt?: number;
}

function SubagentRow({ info }: { info: SubagentInfo }) {
  const elapsed = useElapsedSeconds(info.startedAt);
  const subtype = (info.tool.input?.subagent_type as string) || 'general-purpose';
  const desc = (info.tool.input?.description as string) || '';
  const isDone = info.tool.status === 'done';
  const isError = info.tool.status === 'error';

  return (
    <div style={{
      borderRadius: 'var(--radius-sm)',
      border: `1px solid ${isError ? 'var(--error)' : isDone ? 'var(--border-soft)' : 'var(--accent)'}`,
      background: isDone ? 'var(--surface-2)' : 'var(--accent-glow)',
      padding: '7px 10px',
      marginBottom: 4,
      opacity: isDone ? 0.7 : 1,
    }}>
      <div className="flex items-center gap-2" style={{ marginBottom: desc ? 3 : 0 }}>
        <span style={{
          width: 6, height: 6, borderRadius: '50%', flexShrink: 0,
          background: isError ? 'var(--error)' : isDone ? 'var(--success)' : 'var(--accent)',
          animation: (!isDone && !isError) ? 'pulse 1.8s ease-in-out infinite' : undefined,
        }} />
        <span style={{ color: 'var(--text)', fontSize: 11, fontWeight: 500, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {subtype}
        </span>
        <span style={{ color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)', flexShrink: 0 }}>
          {isDone ? 'done' : isError ? 'error' : formatElapsed(elapsed)}
        </span>
      </div>
      {desc && (
        <div style={{ color: 'var(--text-muted)', fontSize: 10, paddingLeft: 14, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {desc}
        </div>
      )}
    </div>
  );
}

interface RecentAgent {
  tabId: string;
  title: string;
  projectPath?: string;
  finishedAt: number;
}

function AgentRow({ tabId, isActive }: { tabId: string; isActive: boolean }) {
  const tab = useTabStore(s => s.tabs.find(t => t.id === tabId));
  const { setActiveTab } = useTabStore();
  const elapsed = useElapsedSeconds(tab?.streamingStartedAt);

  if (!tab) return null;

  async function abort() {
    if (!tab?.requestId) return;
    await apiFetch(`/api/abort/${tab.requestId}`, { method: 'POST', credentials: 'include' }).catch(() => {});
  }

  const projectName = tab.projectPath?.split('/').pop() || tab.projectPath || '—';
  const statusText = tab.currentTool ? toolLabel(tab.currentTool) : 'Working…';

  return (
    <div style={{
      borderRadius: 'var(--radius-md)',
      border: isActive ? '1px solid var(--accent)' : '1px solid var(--border-soft)',
      background: isActive ? 'var(--accent-glow)' : 'var(--surface-2)',
      padding: '10px 12px',
      marginBottom: 6,
    }}>
      <div className="flex items-center gap-2" style={{ marginBottom: 5 }}>
        <span style={{
          width: 7, height: 7, borderRadius: '50%',
          background: 'var(--accent)',
          flexShrink: 0,
          animation: 'pulse 1.8s ease-in-out infinite',
        }} />
        <span style={{
          color: 'var(--text)',
          fontSize: 12,
          fontWeight: 500,
          flex: 1,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}>
          {tab.title}
        </span>
        {isActive && (
          <span style={{ fontSize: 10, color: 'var(--accent)', fontWeight: 500 }}>active</span>
        )}
      </div>

      <div style={{ color: 'var(--text-dim)', fontSize: 11, marginBottom: 4, fontFamily: 'var(--font-mono)' }}>
        {projectName}
      </div>

      <div style={{
        color: 'var(--text-muted)',
        fontSize: 11,
        marginBottom: 8,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}>
        {statusText}
      </div>

      <div className="flex items-center gap-2">
        <span style={{ color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)', flex: 1 }}>
          {formatElapsed(elapsed)}
        </span>

        {!isActive && (
          <button
            onClick={() => setActiveTab(tabId)}
            style={{
              padding: '2px 8px',
              borderRadius: 'var(--radius-sm)',
              fontSize: 11,
              background: 'var(--accent)',
              color: '#fff',
              border: 'none',
              cursor: 'pointer',
            }}
          >
            Switch
          </button>
        )}

        <button
          onClick={abort}
          title="Abort this agent"
          style={{
            padding: '2px 8px',
            borderRadius: 'var(--radius-sm)',
            fontSize: 11,
            background: 'transparent',
            color: 'var(--text-dim)',
            border: '1px solid var(--border-soft)',
            cursor: 'pointer',
          }}
          onMouseEnter={e => { e.currentTarget.style.color = 'var(--error)'; e.currentTarget.style.borderColor = 'var(--error)'; }}
          onMouseLeave={e => { e.currentTarget.style.color = 'var(--text-dim)'; e.currentTarget.style.borderColor = 'var(--border-soft)'; }}
        >
          Abort
        </button>
      </div>
    </div>
  );
}

function statusDotColor(status: AgentRunWithOutput['status']): string {
  switch (status) {
    case 'running': return 'var(--accent)';
    case 'done': return 'var(--success)';
    case 'failed': return 'var(--error)';
    default: return 'var(--text-dim)';
  }
}

function statusLabel(status: AgentRunWithOutput['status']): string {
  switch (status) {
    case 'running': return 'running';
    case 'pending': return 'queued';
    case 'done': return 'done';
    case 'failed': return 'failed';
    default: return status;
  }
}

function GithubAgentRow({ run }: { run: AgentRunWithOutput }) {
  const [expanded, setExpanded] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);
  const elapsed = useElapsedSeconds(run.startedAt);
  const isActive = run.status === 'running' || run.status === 'pending';
  const isDone = run.status === 'done';
  const isFailed = run.status === 'failed';

  // Auto-scroll log to bottom when new lines arrive
  useEffect(() => {
    if (expanded && logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [run.outputLines.length, expanded]);

  async function kill() {
    await apiFetch(`/api/agent-runs/${run.id}/kill`, { method: 'POST', credentials: 'include' }).catch(() => {});
  }

  // Extract a display name from trigger payload
  const repo = (run.triggerPayload?.repository as string) || '';
  const prNum = (run.triggerPayload?.pr_number as number | string) || '';
  const title = repo ? (prNum ? `${repo} #${prNum}` : repo) : run.id.slice(0, 8);

  return (
    <div style={{
      borderRadius: 'var(--radius-md)',
      border: `1px solid ${isFailed ? 'var(--error-dim, #6b2020)' : isActive ? 'var(--accent)' : 'var(--border-soft)'}`,
      background: isActive ? 'var(--accent-glow)' : 'var(--surface-2)',
      marginBottom: 6,
      opacity: isDone ? 0.8 : 1,
    }}>
      {/* Header row */}
      <div
        className="flex items-center gap-2"
        style={{ padding: '9px 10px', cursor: 'pointer', userSelect: 'none' }}
        onClick={() => setExpanded(e => !e)}
      >
        <span style={{
          width: 6, height: 6, borderRadius: '50%', flexShrink: 0,
          background: statusDotColor(run.status),
          animation: run.status === 'running' ? 'pulse 1.8s ease-in-out infinite' : undefined,
        }} />
        <span style={{ color: 'var(--text)', fontSize: 11, fontWeight: 500, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {title}
        </span>
        <span style={{ color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)', flexShrink: 0, marginRight: 4 }}>
          {isDone || isFailed ? statusLabel(run.status) : run.status === 'pending' ? 'queued' : formatElapsed(elapsed)}
        </span>
        <span style={{ color: 'var(--text-dim)', fontSize: 9, flexShrink: 0 }}>
          {expanded ? '▲' : '▼'}
        </span>
      </div>

      {/* Current tool / status line */}
      {(run.currentTool || run.error) && !expanded && (
        <div style={{
          padding: '0 10px 7px 22px',
          color: isFailed ? 'var(--error)' : 'var(--text-muted)',
          fontSize: 10,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}>
          {run.error || toolLabel(run.currentTool || '')}
        </div>
      )}

      {/* Expanded: output log + actions */}
      {expanded && (
        <div style={{ borderTop: '1px solid var(--border-soft)', padding: '6px 10px 8px' }}>
          {/* PR URL if available */}
          {run.prUrl && (
            <div style={{ marginBottom: 6, fontSize: 10 }}>
              <a href={run.prUrl} target="_blank" rel="noopener noreferrer"
                style={{ color: 'var(--accent)', textDecoration: 'none' }}
                onClick={e => e.stopPropagation()}
              >
                {run.prUrl}
              </a>
            </div>
          )}

          {/* Output log */}
          <div
            ref={logRef}
            style={{
              background: 'var(--surface-1)',
              borderRadius: 'var(--radius-sm)',
              padding: '6px 8px',
              maxHeight: 180,
              overflowY: 'auto',
              fontFamily: 'var(--font-mono)',
              fontSize: 10,
              lineHeight: 1.5,
              color: 'var(--text-muted)',
              marginBottom: 6,
            }}
          >
            {run.outputLines.length === 0 ? (
              <span style={{ color: 'var(--text-dim)', fontStyle: 'italic' }}>
                {run.status === 'pending' ? 'Waiting for slot…' : 'No output yet…'}
              </span>
            ) : (
              run.outputLines.map((line, i) => (
                <div key={i} style={{
                  color: line.startsWith('[tool]') ? 'var(--accent)' :
                         line.startsWith('[error]') || line.toLowerCase().includes('error') ? 'var(--error)' :
                         'var(--text-muted)',
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-all',
                }}>
                  {line}
                </div>
              ))
            )}
          </div>

          {/* Actions */}
          <div className="flex items-center gap-2">
            {run.currentTool && (
              <span style={{ color: 'var(--text-muted)', fontSize: 10, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {toolLabel(run.currentTool)}
              </span>
            )}
            {run.error && (
              <span style={{ color: 'var(--error)', fontSize: 10, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {run.error}
              </span>
            )}
            {isActive && (
              <button
                onClick={e => { e.stopPropagation(); kill(); }}
                style={{
                  padding: '2px 8px',
                  borderRadius: 'var(--radius-sm)',
                  fontSize: 10,
                  background: 'transparent',
                  color: 'var(--text-dim)',
                  border: '1px solid var(--border-soft)',
                  cursor: 'pointer',
                  flexShrink: 0,
                }}
                onMouseEnter={e => { e.currentTarget.style.color = 'var(--error)'; e.currentTarget.style.borderColor = 'var(--error)'; }}
                onMouseLeave={e => { e.currentTarget.style.color = 'var(--text-dim)'; e.currentTarget.style.borderColor = 'var(--border-soft)'; }}
              >
                Kill
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

export default function BackgroundAgentsPanel() {
  const { tabs, activeTabId } = useTabStore();
  const [recentAgents, setRecentAgents] = useState<RecentAgent[]>([]);
  const chats = useChatStore(s => s.chats);
  const githubRuns = useAgentStream();

  const streamingTabs = tabs.filter(t => t.isStreaming);

  // Collect running Agent tool subagents from ALL streaming tabs
  const runningSubagents: SubagentInfo[] = [];
  for (const tab of streamingTabs) {
    const chat = chats[tab.id];
    if (chat) {
      for (const block of chat.blocks) {
        if (!block.tools) continue;
        for (const tool of block.tools) {
          if (tool.name !== 'Agent') continue;
          if (tool.status === 'running') {
            runningSubagents.push({ tabId: tab.id, tabTitle: tab.title, tool, startedAt: tab.streamingStartedAt });
          }
        }
      }
    }
  }

  // Track recently completed session agents
  useEffect(() => {
    function onDone(e: Event) {
      const { tabId } = (e as CustomEvent).detail;
      const tab = useTabStore.getState().tabs.find(t => t.id === tabId);
      if (!tab) return;
      setRecentAgents(prev => [
        { tabId, title: tab.title, projectPath: tab.projectPath, finishedAt: Date.now() },
        ...prev.filter(r => r.tabId !== tabId),
      ].slice(0, 10));
    }
    window.addEventListener('flamel:agent-done', onDone);
    return () => window.removeEventListener('flamel:agent-done', onDone);
  }, []);

  useEffect(() => {
    const id = setInterval(() => {
      const cutoff = Date.now() - 5 * 60 * 1000;
      setRecentAgents(prev => prev.filter(r => r.finishedAt > cutoff));
    }, 30_000);
    return () => clearInterval(id);
  }, []);

  const githubRunList = Object.values(githubRuns);
  const activeGithubRuns = githubRunList.filter(r => r.status === 'running' || r.status === 'pending');
  const finishedGithubRuns = githubRunList.filter(r => r.status === 'done' || r.status === 'failed');

  const isEmpty =
    streamingTabs.length === 0 &&
    recentAgents.length === 0 &&
    runningSubagents.length === 0 &&
    githubRunList.length === 0;

  return (
    <div style={{ padding: '12px 10px', overflowY: 'auto', height: '100%' }}>
      {/* Running streaming tabs */}
      {streamingTabs.length > 0 && (
        <div style={{ marginBottom: 12 }}>
          <SectionHeader label="Running" count={streamingTabs.length} />
          {streamingTabs.map(tab => (
            <AgentRow key={tab.id} tabId={tab.id} isActive={tab.id === activeTabId} />
          ))}
        </div>
      )}

      {/* Running Agent tool subagents */}
      {runningSubagents.length > 0 && (
        <div style={{ marginBottom: 12 }}>
          <SectionHeader label="Subagents" count={runningSubagents.length} />
          {runningSubagents.map(info => (
            <SubagentRow key={`${info.tabId}-${info.tool.id}`} info={info} />
          ))}
        </div>
      )}

      {/* GitHub-triggered background agents (active) */}
      {activeGithubRuns.length > 0 && (
        <div style={{ marginBottom: 12 }}>
          <SectionHeader label="Background" count={activeGithubRuns.length} />
          {activeGithubRuns.map(run => (
            <GithubAgentRow key={run.id} run={run} />
          ))}
        </div>
      )}

      {/* Recently finished session agents */}
      {recentAgents.length > 0 && (
        <div style={{ marginBottom: 12 }}>
          <SectionHeader label="Recently finished" />
          {recentAgents.map(r => {
            const secsAgo = Math.floor((Date.now() - r.finishedAt) / 1000);
            const projectName = r.projectPath?.split('/').pop() || r.projectPath || '—';
            const isCurrentlyRunning = tabs.find(t => t.id === r.tabId)?.isStreaming;
            if (isCurrentlyRunning) return null;
            return (
              <div
                key={r.tabId}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  padding: '7px 10px',
                  borderRadius: 'var(--radius-md)',
                  background: 'var(--surface-2)',
                  marginBottom: 4,
                  cursor: 'pointer',
                }}
                onClick={() => useTabStore.getState().setActiveTab(r.tabId)}
                onMouseEnter={e => (e.currentTarget.style.background = 'var(--surface-3)')}
                onMouseLeave={e => (e.currentTarget.style.background = 'var(--surface-2)')}
              >
                <Icon.Dot size={7} style={{ color: 'var(--success)', flexShrink: 0 }} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ color: 'var(--text-muted)', fontSize: 11, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {r.title}
                  </div>
                  <div style={{ color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)' }}>
                    {projectName}
                  </div>
                </div>
                <span style={{ color: 'var(--text-dim)', fontSize: 10, flexShrink: 0 }}>
                  {formatElapsed(secsAgo)} ago
                </span>
              </div>
            );
          })}
        </div>
      )}

      {/* Finished GitHub background runs */}
      {finishedGithubRuns.length > 0 && (
        <div>
          <SectionHeader label="Finished background" />
          {finishedGithubRuns.map(run => (
            <GithubAgentRow key={run.id} run={run} />
          ))}
        </div>
      )}

      {isEmpty && (
        <div className="flex flex-col items-center justify-center h-full" style={{ paddingTop: 60 }}>
          <div style={{ color: 'var(--text-dim)', fontSize: 12, textAlign: 'center', lineHeight: 1.6 }}>
            No agents running.
            <br />
            <span style={{ color: 'var(--text-dim)', fontSize: 11, opacity: 0.7 }}>
              Start a session to see it here.
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
