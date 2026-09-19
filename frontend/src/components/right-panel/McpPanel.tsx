import { useState, useEffect, useCallback } from 'react';
import { useTabStore } from '../../stores/tabStore';
import { apiFetch, api } from '../../lib/api';
import { useIdeStore } from '../../stores/ideStore';

// Built-in servers that are always present — can't be toggled via the panel
const BUILTIN_SERVERS = new Set(['flamel']);

type ServerStatus = 'connected' | 'failed' | 'needs_auth' | 'pending' | 'running' | 'configured' | 'unknown';

interface McpServer {
  name: string;
  status: ServerStatus;
  toolCount?: number;
  scope?: string;
  transport?: string;
  instructions?: string;
  builtin: boolean;
}

function StatusDot({ status }: { status: ServerStatus }) {
  const colors: Record<ServerStatus, string> = {
    connected:  'var(--success)',
    failed:     'var(--error)',
    needs_auth: 'var(--warning)',
    pending:    'var(--warning)',
    running:    'var(--accent)',
    configured: 'var(--accent-text)',
    unknown:    'var(--text-dim)',
  };
  const labels: Record<ServerStatus, string> = {
    connected:  'connected',
    failed:     'failed',
    needs_auth: 'needs auth',
    pending:    'pending',
    running:    'running',
    configured: 'configured',
    unknown:    'unknown',
  };
  const color = colors[status];
  return (
    <span title={labels[status]} style={{
      display: 'inline-block',
      width: 7, height: 7,
      borderRadius: '50%',
      background: color,
      flexShrink: 0,
      boxShadow: status === 'connected'
        ? '0 0 6px var(--success-glow)'
        : status === 'running'
          ? '0 0 6px var(--accent-glow-strong)'
          : undefined,
    }} />
  );
}

function ScopeBadge({ scope, builtin }: { scope?: string; builtin: boolean }) {
  const label = builtin ? 'built-in' : (scope ?? 'user');
  const bg = builtin ? 'var(--accent-glow)' : 'var(--surface-2)';
  const color = builtin ? 'var(--accent)' : 'var(--text-dim)';
  return (
    <span style={{
      padding: '1px 5px', borderRadius: 4, fontSize: 9, fontWeight: 500,
      background: bg, color, border: `1px solid ${builtin ? 'var(--accent)' : 'var(--border-soft)'}`,
      flexShrink: 0,
    }}>
      {label}
    </span>
  );
}

interface ServerRowProps {
  server: McpServer;
  sessionId?: string;
  onAction: () => void;
}

function ServerRow({ server, sessionId, onAction }: ServerRowProps) {
  const [loading, setLoading] = useState(false);

  async function reconnect() {
    if (!sessionId) return;
    setLoading(true);
    try {
      await apiFetch(`/api/sessions/${sessionId}/mcp-reconnect`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ serverName: server.name }),
      });
      onAction();
    } catch {}
    setLoading(false);
  }

  const canReconnect = sessionId && (server.status === 'failed' || server.status === 'needs_auth');

  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: 8,
      padding: '7px 12px',
      borderBottom: '1px solid var(--border-soft)',
    }}>
      <StatusDot status={server.status} />

      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
          <span style={{ fontSize: 12, fontWeight: 500, color: 'var(--text)', fontFamily: 'var(--font-mono)' }}>
            {server.name}
          </span>
          {server.toolCount != null && (
            <span style={{
              fontSize: 9, padding: '1px 4px', borderRadius: 3,
              background: 'var(--surface-2)', color: 'var(--text-dim)',
              border: '1px solid var(--border-soft)',
            }}>
              {server.toolCount} tools
            </span>
          )}
        </div>
        {server.status === 'needs_auth' && (
          <span style={{ fontSize: 10, color: 'var(--warning)', marginTop: 1, display: 'block' }}>
            authentication required
          </span>
        )}
        {server.status === 'running' && (
          <span style={{ fontSize: 10, color: 'var(--accent)', marginTop: 1, display: 'block' }}>
            session active
          </span>
        )}
        {server.status === 'failed' && (
          <span style={{ fontSize: 10, color: 'var(--error)', marginTop: 1, display: 'block' }}>
            connection failed
          </span>
        )}
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 5, flexShrink: 0 }}>
        <ScopeBadge scope={server.scope} builtin={server.builtin} />
        {canReconnect && (
          <button
            onClick={reconnect}
            disabled={loading}
            style={{
              fontSize: 10, padding: '2px 7px', borderRadius: 4,
              background: 'var(--surface-2)', color: 'var(--accent)',
              border: '1px solid var(--accent)', cursor: 'pointer',
              opacity: loading ? 0.5 : 1,
            }}
          >
            {loading ? '…' : 'reconnect'}
          </button>
        )}
      </div>
    </div>
  );
}

// ─── Memory files section ────────────────────────────────────────────────────

interface MemoryFile {
  path: string;
  type: string;
  tokenCount?: number;
}

const SCOPE_COLORS: Record<string, { bg: string; color: string }> = {
  user:     { bg: 'var(--surface-2)',       color: 'var(--text-muted)' },
  project:  { bg: 'var(--success-glow)',    color: 'var(--success)' },
  local:    { bg: 'var(--warning-glow)',    color: 'var(--warning)' },
  managed:  { bg: 'var(--surface-3)',       color: 'var(--text-dim)' },
};

function MemoryFilesSection({ sessionId }: { sessionId?: string }) {
  const [files, setFiles] = useState<MemoryFile[]>([]);
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(true);

  const load = useCallback(async () => {
    if (!sessionId) return;
    setLoading(true);
    try {
      const r = await apiFetch(`/api/sessions/${sessionId}/memory-files`);
      if (r.ok) {
        const data = await r.json();
        setFiles(Array.isArray(data.memoryFiles) ? data.memoryFiles : []);
      }
    } catch {}
    setLoading(false);
  }, [sessionId]);

  useEffect(() => { load(); }, [load]);

  async function openInIde(path: string) {
    try {
      const { content, revision } = await api.fileContent.get(path);
      useIdeStore.getState().openFile(path, content, revision);
      window.dispatchEvent(new CustomEvent('flamel:open-ide'));
    } catch {}
  }

  function fmt(n: number) {
    if (n >= 1000) return `${Math.round(n / 1000)}K`;
    return String(n);
  }

  const totalTokens = files.reduce((sum, f) => sum + (f.tokenCount ?? 0), 0);

  return (
    <div style={{ borderTop: '1px solid var(--border-soft)' }}>
      {/* Section header */}
      <div
        onClick={() => setOpen(o => !o)}
        style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '6px 12px', cursor: 'pointer',
          background: 'var(--surface)',
        }}
        onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'var(--surface-2)'; }}
        onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'var(--surface)'; }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
          <span style={{ fontSize: 10, transform: open ? 'rotate(90deg)' : 'rotate(0)', display: 'inline-block', transition: 'transform 0.15s', color: 'var(--text-dim)' }}>▶</span>
          <span style={{ fontSize: 11, fontWeight: 600, color: 'var(--text)' }}>Memory Files</span>
          {files.length > 0 && (
            <span style={{ fontSize: 9, padding: '1px 5px', borderRadius: 4, background: 'var(--surface-2)', color: 'var(--text-dim)', border: '1px solid var(--border-soft)' }}>
              {files.length}
            </span>
          )}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          {totalTokens > 0 && (
            <span style={{ fontSize: 9, color: 'var(--text-dim)', fontFamily: 'var(--font-mono)' }}>
              {fmt(totalTokens)} tok
            </span>
          )}
          <button
            onClick={e => { e.stopPropagation(); load(); }}
            disabled={loading || !sessionId}
            title="Refresh"
            style={{
              fontSize: 11, background: 'transparent', border: 'none',
              color: 'var(--text-dim)', cursor: 'pointer', padding: 0,
              opacity: loading ? 0.4 : 1,
            }}
          >↻</button>
        </div>
      </div>

      {open && (
        <div>
          {!sessionId ? (
            <div style={{ padding: '8px 12px', fontSize: 10, color: 'var(--text-dim)' }}>
              no active session
            </div>
          ) : loading && files.length === 0 ? (
            <div style={{ padding: '8px 12px', fontSize: 10, color: 'var(--text-dim)' }}>loading…</div>
          ) : files.length === 0 ? (
            <div style={{ padding: '8px 12px', fontSize: 10, color: 'var(--text-dim)' }}>
              no memory files loaded (start a session to fetch)
            </div>
          ) : (
            files.map((file) => {
              const typeKey = (file.type ?? '').toLowerCase().split('_')[0];
              const colors = SCOPE_COLORS[typeKey] ?? SCOPE_COLORS.managed;
              const shortPath = file.path.replace(/^\/home\/[^/]+\//, '~/').replace(/^\/Users\/[^/]+\//, '~/');
              return (
                <div
                  key={file.path}
                  onClick={() => openInIde(file.path)}
                  title={`${file.path}\nClick to open in editor`}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 8,
                    padding: '5px 12px',
                    borderBottom: '1px solid var(--border-soft)',
                    cursor: 'pointer',
                  }}
                  onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'var(--surface-2)'; }}
                  onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
                >
                  <span style={{
                    padding: '1px 5px', borderRadius: 4, fontSize: 9, fontWeight: 500,
                    background: colors.bg, color: colors.color,
                    border: `1px solid ${colors.color}40`,
                    flexShrink: 0,
                  }}>
                    {typeKey || 'file'}
                  </span>
                  <span style={{
                    flex: 1, fontSize: 10, color: 'var(--text)',
                    fontFamily: 'var(--font-mono)',
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }}>
                    {shortPath.split('/').pop()}
                  </span>
                  {file.tokenCount != null && (
                    <span style={{ fontSize: 9, color: 'var(--text-dim)', fontFamily: 'var(--font-mono)', flexShrink: 0 }}>
                      {fmt(file.tokenCount)}
                    </span>
                  )}
                </div>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}

// ─── Main panel ──────────────────────────────────────────────────────────────

export default function McpPanel({ tabId }: { tabId: string }) {
  const tab = useTabStore(s => s.tabs.find(t => t.id === tabId));
  const sessionId = tab?.sessionId;
  const projectPath = tab?.projectPath;

  const [servers, setServers] = useState<McpServer[]>([]);
  const [loading, setLoading] = useState(false);
  const [liveStatus, setLiveStatus] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      // Try live status first if session is active
      if (sessionId) {
        const r = await apiFetch(`/api/sessions/${sessionId}/mcp-status`);
        if (r.ok) {
          const data = await r.json();
          if (data.servers && Array.isArray(data.servers)) {
            setServers(data.servers.map((s: Record<string, unknown>) => ({
              name: s.name as string,
              status: (s.status as ServerStatus) ?? 'unknown',
              toolCount: s.toolCount as number | undefined,
              scope: s.scope as string | undefined,
              transport: s.transport as string | undefined,
              instructions: s.instructions as string | undefined,
              builtin: BUILTIN_SERVERS.has(s.name as string),
            })));
            setLiveStatus(true);
            setLoading(false);
            return;
          }
        }
      }
      // Fall back to static list: builtins + user-configured
      const builtins: McpServer[] = Array.from(BUILTIN_SERVERS).map(name => ({
        name, status: 'configured', builtin: true,
      }));
      if (projectPath) {
        const r2 = await apiFetch(`/api/mcp?projectPath=${encodeURIComponent(projectPath)}`);
        if (r2.ok) {
          const configs = await r2.json();
          const userServers: McpServer[] = (Array.isArray(configs) ? configs : []).map(
            (c: { name: string; enabled: boolean }) => ({
              name: c.name,
              status: c.enabled ? 'configured' : 'unknown',
              builtin: BUILTIN_SERVERS.has(c.name),
              scope: 'user',
            })
          );
          setServers([...builtins, ...userServers.filter(s => !BUILTIN_SERVERS.has(s.name))]);
        } else {
          setServers(builtins);
        }
      } else {
        setServers(builtins);
      }
      setLiveStatus(false);
    } catch {
      setLiveStatus(false);
    }
    setLoading(false);
  }, [sessionId, projectPath]);

  useEffect(() => { load(); }, [load]);

  const connectedCount = servers.filter(s => s.status === 'connected' || s.status === 'running').length;
  const failedCount = servers.filter(s => s.status === 'failed' || s.status === 'needs_auth').length;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      {/* Sub-header */}
      <div style={{
        padding: '8px 12px 7px',
        borderBottom: '1px solid var(--border-soft)',
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        flexShrink: 0,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          {liveStatus ? (
            <>
              <span style={{ fontSize: 10, color: '#22c55e' }}>{connectedCount} connected</span>
              {failedCount > 0 && (
                <span style={{ fontSize: 10, color: '#ef4444' }}>{failedCount} failed</span>
              )}
            </>
          ) : (
            <span style={{ fontSize: 10, color: 'var(--text-dim)' }}>
              {sessionId ? 'no live status' : 'no active session'}
            </span>
          )}
        </div>
        <button
          onClick={load}
          disabled={loading}
          title="Refresh"
          style={{
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            width: 20, height: 20, borderRadius: 'var(--radius-sm)',
            background: 'transparent', border: 'none', cursor: 'pointer',
            color: 'var(--text-dim)', fontSize: 11, opacity: loading ? 0.4 : 1,
          }}
          onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'var(--surface-2)'; (e.currentTarget as HTMLElement).style.color = 'var(--text)'; }}
          onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent'; (e.currentTarget as HTMLElement).style.color = 'var(--text-dim)'; }}
        >
          ↻
        </button>
      </div>

      {/* Server list */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {loading && servers.length === 0 ? (
          <div style={{ padding: '24px 12px', textAlign: 'center', color: 'var(--text-dim)', fontSize: 11 }}>
            loading…
          </div>
        ) : servers.length === 0 ? (
          <div style={{ padding: '24px 12px', textAlign: 'center', color: 'var(--text-dim)', fontSize: 11 }}>
            no servers configured
          </div>
        ) : (
          servers.map(server => (
            <ServerRow
              key={server.name}
              server={server}
              sessionId={sessionId}
              onAction={load}
            />
          ))
        )}
      </div>

      <MemoryFilesSection sessionId={sessionId} />

      {!liveStatus && sessionId && (
        <div style={{
          padding: '6px 12px',
          borderTop: '1px solid var(--border-soft)',
          fontSize: 10, color: 'var(--text-dim)',
          flexShrink: 0,
        }}>
          Live status requires an active Claude session
        </div>
      )}
    </div>
  );
}
