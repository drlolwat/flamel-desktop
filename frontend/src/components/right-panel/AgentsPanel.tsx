import { useState, useEffect, useRef } from 'react';
import { api } from '../../lib/api';
import type { AgentConfig, AgentRun, GitAccount } from '@flamel/shared';

/* ─── Status badge ───────────────────────────────────────────────────────── */
function StatusBadge({ status }: { status: AgentRun['status'] }) {
  const map: Record<AgentRun['status'], { color: string; bg: string; label: string }> = {
    pending:           { color: 'var(--text-muted)',          bg: 'var(--surface-2)',                            label: 'pending' },
    running:           { color: 'var(--accent)',              bg: 'var(--accent-glow)',                          label: 'running' },
    awaiting_approval: { color: 'var(--warning)',             bg: 'var(--warning-glow)',                         label: 'needs approval' },
    done:              { color: 'var(--success)',             bg: 'var(--success-glow)',                         label: 'done' },
    failed:            { color: 'var(--error)',               bg: 'var(--error-glow)',                           label: 'failed' },
  };
  const s = map[status];
  return (
    <span style={{
      padding: '2px 7px', borderRadius: 8, fontSize: 10, fontWeight: 500,
      color: s.color, background: s.bg,
      border: `1px solid ${s.color}`,
      flexShrink: 0,
    }}>
      {s.label}
    </span>
  );
}

/* ─── Add agent form ─────────────────────────────────────────────────────── */
function AddAgentForm({
  accounts,
  initialProjectPath,
  onCreated,
  onCancel,
}: {
  accounts: GitAccount[];
  initialProjectPath?: string;
  onCreated: (config: AgentConfig) => void;
  onCancel: () => void;
}) {
  const [gitAccountId, setGitAccountId] = useState(accounts[0]?.id ?? '');
  const [repoOwner, setRepoOwner] = useState('');
  const [repoName, setRepoName] = useState('');
  const [projectPath, setProjectPath] = useState(initialProjectPath ?? '');
  const [triggerEvent, setTriggerEvent] = useState('issues');
  const [triggerLabel, setTriggerLabel] = useState('');
  const [model, setModel] = useState('claude-opus-4-7');
  const [requireApproval, setRequireApproval] = useState(true);
  const [autoPr, setAutoPr] = useState(true);
  const [autoClose, setAutoClose] = useState(true);
  const [commentOnStart, setCommentOnStart] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  const inputStyle: React.CSSProperties = {
    width: '100%', padding: '5px 8px', borderRadius: 5, fontSize: 12,
    outline: 'none', background: 'var(--surface)', border: '1px solid var(--border)', color: 'var(--text)',
  };

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!gitAccountId || !repoOwner || !repoName || !projectPath) return;
    setSaving(true);
    setError('');
    try {
      const config = await api.agents.create({
        gitAccountId, repoOwner, repoName, projectPath,
        triggerEvent, triggerLabel: triggerLabel || undefined,
        model, requireApproval, autoPr, autoClose, commentOnStart,
      });
      onCreated(config);
    } catch (err: any) {
      setError(err.message || 'Failed to create agent');
    } finally {
      setSaving(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} style={{
      margin: '8px', padding: '12px',
      background: 'var(--surface-2)', border: '1px solid var(--border)',
      borderRadius: 8, display: 'flex', flexDirection: 'column', gap: 8,
    }}>
      <p style={{ color: 'var(--text)', fontSize: 12, fontWeight: 500, margin: 0 }}>New agent</p>

      <div>
        <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Git account</label>
        <select value={gitAccountId} onChange={e => setGitAccountId(e.target.value)} style={inputStyle}>
          {accounts.map(a => (
            <option key={a.id} value={a.id}>{a.label} ({a.type})</option>
          ))}
        </select>
      </div>

      <div style={{ display: 'flex', gap: 8 }}>
        <div style={{ flex: 1 }}>
          <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Owner</label>
          <input type="text" placeholder="octocat" value={repoOwner} onChange={e => setRepoOwner(e.target.value)} style={inputStyle} />
        </div>
        <div style={{ flex: 1 }}>
          <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Repo</label>
          <input type="text" placeholder="my-repo" value={repoName} onChange={e => setRepoName(e.target.value)} style={inputStyle} />
        </div>
      </div>

      <div>
        <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Project path</label>
        <input type="text" placeholder="/home/user/my-repo" value={projectPath} onChange={e => setProjectPath(e.target.value)} style={inputStyle} />
      </div>

      <div style={{ display: 'flex', gap: 8 }}>
        <div style={{ flex: 1 }}>
          <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Trigger event</label>
          <select value={triggerEvent} onChange={e => setTriggerEvent(e.target.value)} style={inputStyle}>
            <option value="issues">issues</option>
            <option value="issue_comment">issue_comment</option>
            <option value="pull_request">pull_request</option>
          </select>
        </div>
        <div style={{ flex: 1 }}>
          <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Label filter (opt.)</label>
          <input type="text" placeholder="claude" value={triggerLabel} onChange={e => setTriggerLabel(e.target.value)} style={inputStyle} />
        </div>
      </div>

      <div>
        <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Model</label>
        <select value={model} onChange={e => setModel(e.target.value)} style={inputStyle}>
          <option value="claude-opus-4-7">claude-opus-4-7</option>
          <option value="claude-opus-4-6">claude-opus-4-6</option>
          <option value="claude-sonnet-4-6">claude-sonnet-4-6</option>
          <option value="claude-haiku-4-5">claude-haiku-4-5</option>
        </select>
      </div>

      <div style={{ display: 'flex', flexWrap: 'wrap', gap: '8px 16px' }}>
        <label style={{ display: 'flex', alignItems: 'center', gap: 5, cursor: 'pointer' }}>
          <input type="checkbox" checked={requireApproval} onChange={e => setRequireApproval(e.target.checked)} />
          <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>Require approval</span>
        </label>
        <label style={{ display: 'flex', alignItems: 'center', gap: 5, cursor: 'pointer' }}>
          <input type="checkbox" checked={autoPr} onChange={e => setAutoPr(e.target.checked)} />
          <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>Auto PR</span>
        </label>
        <label style={{ display: 'flex', alignItems: 'center', gap: 5, cursor: 'pointer' }}>
          <input type="checkbox" checked={autoClose} onChange={e => setAutoClose(e.target.checked)} />
          <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>Auto close issue</span>
        </label>
        <label style={{ display: 'flex', alignItems: 'center', gap: 5, cursor: 'pointer' }}>
          <input type="checkbox" checked={commentOnStart} onChange={e => setCommentOnStart(e.target.checked)} />
          <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>Comment on issue</span>
        </label>
      </div>

      {error && <p style={{ color: 'var(--error)', fontSize: 12 }}>{error}</p>}

      <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
        <button
          type="submit"
          disabled={saving || !gitAccountId || !repoOwner || !repoName || !projectPath}
          style={{
            flex: 1, padding: '6px 0', borderRadius: 5, fontSize: 12,
            background: 'var(--accent)', color: '#fff',
            border: 'none', cursor: 'pointer',
            opacity: saving || !gitAccountId || !repoOwner || !repoName || !projectPath ? 0.5 : 1,
          }}
        >
          {saving ? 'Creating…' : 'Create agent'}
        </button>
        <button
          type="button"
          onClick={onCancel}
          style={{
            padding: '6px 12px', borderRadius: 5, fontSize: 12,
            background: 'var(--surface)', color: 'var(--text-muted)',
            border: '1px solid var(--border)', cursor: 'pointer',
          }}
        >
          Cancel
        </button>
      </div>
    </form>
  );
}

/* ─── AgentsPanel ────────────────────────────────────────────────────────── */
export default function AgentsPanel({ projectPath }: { projectPath?: string }) {
  const [allConfigs, setAllConfigs] = useState<AgentConfig[]>([]);
  const [runs, setRuns] = useState<AgentRun[]>([]);
  const [accounts, setAccounts] = useState<GitAccount[]>([]);
  const [loading, setLoading] = useState(false);
  const [showAddForm, setShowAddForm] = useState(false);
  const [expandedConfig, setExpandedConfig] = useState<string | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Only show configs for the active project
  const configs = projectPath
    ? allConfigs.filter(c => c.projectPath === projectPath)
    : allConfigs;

  async function loadData() {
    try {
      const [c, r, a] = await Promise.all([
        api.agents.list(),
        api.agents.runs(),
        api.gitAccounts.list(),
      ]);
      setAllConfigs(c);
      setRuns(r);
      setAccounts(a);
    } catch { /* ignore */ }
  }

  useEffect(() => {
    // Set interval before async work so cleanup always clears it even if loadData throws
    intervalRef.current = setInterval(loadData, 30_000);
    setLoading(true);
    loadData().finally(() => setLoading(false));
    return () => { if (intervalRef.current) clearInterval(intervalRef.current); };
  }, []);

  async function toggleEnabled(config: AgentConfig) {
    try {
      const updated = await api.agents.update(config.id, { enabled: !config.enabled });
      setAllConfigs(prev => prev.map(c => c.id === config.id ? updated : c));
    } catch { /* ignore */ }
  }

  async function deleteConfig(config: AgentConfig) {
    if (!confirm(`Delete agent for ${config.repoOwner}/${config.repoName}?`)) return;
    try {
      await api.agents.delete(config.id);
      setAllConfigs(prev => prev.filter(c => c.id !== config.id));
      setRuns(prev => prev.filter(r => r.agentConfigId !== config.id));
    } catch { /* ignore */ }
  }

  async function approveRun(run: AgentRun) {
    try {
      await api.agents.approve(run.id);
      setRuns(prev => prev.map(r => r.id === run.id ? { ...r, status: 'running' } : r));
    } catch { /* ignore */ }
  }

  async function rejectRun(run: AgentRun) {
    try {
      await api.agents.reject(run.id);
      setRuns(prev => prev.map(r => r.id === run.id ? { ...r, status: 'failed', error: 'Rejected by user' } : r));
    } catch { /* ignore */ }
  }

  async function killRun(run: AgentRun) {
    try {
      await api.agents.kill(run.id);
      setRuns(prev => prev.map(r => r.id === run.id ? { ...r, status: 'failed', error: 'Killed by user' } : r));
    } catch { /* ignore */ }
  }

  function getRunsForConfig(configId: string) {
    return runs.filter(r => r.agentConfigId === configId);
  }

  if (loading) {
    return <p className="text-center py-8 text-xs" style={{ color: 'var(--text-muted)' }}>Loading…</p>;
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 shrink-0">
        <span style={{ fontSize: 12, color: 'var(--text-muted)', fontWeight: 500 }}>Agent configurations</span>
        <button
          onClick={() => setShowAddForm(s => !s)}
          style={{
            padding: '3px 10px', borderRadius: 5, fontSize: 11,
            background: showAddForm ? 'var(--surface-2)' : 'var(--accent)',
            color: showAddForm ? 'var(--text-muted)' : '#fff',
            border: showAddForm ? '1px solid var(--border)' : 'none',
            cursor: 'pointer',
          }}
        >
          {showAddForm ? 'Cancel' : '+ Add agent'}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto pb-2">
        {showAddForm && accounts.length === 0 && (
          <div style={{ margin: '8px', padding: '12px', background: 'var(--surface-2)', border: '1px solid var(--border)', borderRadius: 8 }}>
            <p style={{ color: 'var(--text-muted)', fontSize: 12 }}>
              No git accounts configured. Add a GitHub or Gitea account in Settings → Git Accounts first.
            </p>
          </div>
        )}

        {showAddForm && accounts.length > 0 && (
          <AddAgentForm
            accounts={accounts}
            initialProjectPath={projectPath}
            onCreated={config => {
              setAllConfigs(prev => [config, ...prev]);
              setShowAddForm(false);
            }}
            onCancel={() => setShowAddForm(false)}
          />
        )}

        {configs.length === 0 && !showAddForm ? (
          <p className="text-center py-8 text-xs" style={{ color: 'var(--text-muted)' }}>
            No agents configured
          </p>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, padding: '4px 8px' }}>
            {configs.map(config => {
              const configRuns = getRunsForConfig(config.id);
              const isExpanded = expandedConfig === config.id;

              return (
                <div
                  key={config.id}
                  style={{
                    background: 'var(--surface-2)', border: '1px solid var(--border)',
                    borderRadius: 8, overflow: 'hidden',
                  }}
                >
                  {/* Config header */}
                  <div style={{ padding: '8px 10px' }}>
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 6 }}>
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <span style={{
                          color: 'var(--text)', fontSize: 12, fontWeight: 500,
                          fontFamily: 'var(--font-mono)',
                        }}>
                          {config.repoOwner}/{config.repoName}
                        </span>
                        <div style={{ display: 'flex', gap: 6, marginTop: 3, flexWrap: 'wrap' }}>
                          <span style={{ color: 'var(--text-muted)', fontSize: 10 }}>on: {config.triggerEvent}</span>
                          {config.triggerLabel && (
                            <span style={{ color: 'var(--text-muted)', fontSize: 10 }}>label: {config.triggerLabel}</span>
                          )}
                          <span style={{ color: 'var(--text-muted)', fontSize: 10 }}>{config.model}</span>
                        </div>
                      </div>

                      <div style={{ display: 'flex', gap: 6, alignItems: 'center', flexShrink: 0 }}>
                        {/* Enable/disable toggle */}
                        <button
                          onClick={() => toggleEnabled(config)}
                          title={config.enabled ? 'Disable' : 'Enable'}
                          style={{
                            padding: '3px 8px', borderRadius: 4, fontSize: 11,
                            background: config.enabled ? 'var(--success-glow, rgba(34,197,94,0.15))' : 'var(--surface)',
                            color: config.enabled ? 'var(--success, #22c55e)' : 'var(--text-muted)',
                            border: `1px solid ${config.enabled ? 'var(--success, #22c55e)' : 'var(--border)'}`,
                            cursor: 'pointer',
                          }}
                        >
                          {config.enabled ? 'on' : 'off'}
                        </button>

                        {/* Expand runs */}
                        <button
                          onClick={() => setExpandedConfig(isExpanded ? null : config.id)}
                          style={{
                            padding: '3px 8px', borderRadius: 4, fontSize: 11,
                            background: 'var(--surface)', color: 'var(--text-muted)',
                            border: '1px solid var(--border)', cursor: 'pointer',
                          }}
                        >
                          {configRuns.length} runs {isExpanded ? '▴' : '▾'}
                        </button>

                        {/* Delete */}
                        <button
                          onClick={() => deleteConfig(config)}
                          title="Delete agent"
                          style={{
                            padding: '3px 6px', borderRadius: 4, fontSize: 11,
                            background: 'none', color: 'var(--text-muted)',
                            border: '1px solid var(--border)', cursor: 'pointer',
                          }}
                          onMouseEnter={e => { e.currentTarget.style.color = 'var(--error)'; e.currentTarget.style.borderColor = 'var(--error)'; }}
                          onMouseLeave={e => { e.currentTarget.style.color = 'var(--text-muted)'; e.currentTarget.style.borderColor = 'var(--border)'; }}
                        >
                          ✕
                        </button>
                      </div>
                    </div>
                  </div>

                  {/* Runs list */}
                  {isExpanded && (
                    <div style={{ borderTop: '1px solid var(--border)' }}>
                      {configRuns.length === 0 ? (
                        <p style={{ color: 'var(--text-muted)', fontSize: 11, padding: '8px 10px' }}>No runs yet</p>
                      ) : (
                        configRuns.map(run => (
                          <div
                            key={run.id}
                            style={{
                              padding: '8px 10px',
                              borderBottom: '1px solid var(--border-soft)',
                              display: 'flex',
                              flexDirection: 'column',
                              gap: 5,
                            }}
                          >
                            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 6 }}>
                              <span style={{ color: 'var(--text-muted)', fontSize: 10, fontFamily: 'var(--font-mono)' }}>
                                {new Date(run.createdAt).toLocaleString()}
                              </span>
                              <StatusBadge status={run.status} />
                            </div>

                            {run.error && (
                              <p style={{ color: 'var(--error)', fontSize: 11 }}>{run.error}</p>
                            )}

                            {run.prUrl && (
                              <a
                                href={run.prUrl}
                                target="_blank"
                                rel="noreferrer"
                                style={{ color: 'var(--accent)', fontSize: 11 }}
                              >
                                View PR ↗
                              </a>
                            )}

                            {/* Action buttons */}
                            <div style={{ display: 'flex', gap: 6 }}>
                              {run.status === 'awaiting_approval' && (
                                <>
                                  <button
                                    onClick={() => approveRun(run)}
                                    style={{
                                      padding: '3px 10px', borderRadius: 4, fontSize: 11,
                                      background: 'var(--success-glow, rgba(34,197,94,0.15))',
                                      color: 'var(--success, #22c55e)',
                                      border: '1px solid var(--success, #22c55e)',
                                      cursor: 'pointer',
                                    }}
                                  >
                                    Approve
                                  </button>
                                  <button
                                    onClick={() => rejectRun(run)}
                                    style={{
                                      padding: '3px 10px', borderRadius: 4, fontSize: 11,
                                      background: 'rgba(239,68,68,0.1)',
                                      color: 'var(--error, #ef4444)',
                                      border: '1px solid var(--error, #ef4444)',
                                      cursor: 'pointer',
                                    }}
                                  >
                                    Reject
                                  </button>
                                </>
                              )}
                              {run.status === 'running' && (
                                <button
                                  onClick={() => killRun(run)}
                                  style={{
                                    padding: '3px 10px', borderRadius: 4, fontSize: 11,
                                    background: 'rgba(239,68,68,0.1)',
                                    color: 'var(--error, #ef4444)',
                                    border: '1px solid var(--error, #ef4444)',
                                    cursor: 'pointer',
                                  }}
                                >
                                  Kill
                                </button>
                              )}
                            </div>
                          </div>
                        ))
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
