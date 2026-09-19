import { useState, useEffect } from 'react';
import { api } from '../../lib/api';
import type { GitIssue, GitPullRequest } from '@flamel/shared';

type Tab = 'issues' | 'prs';
type IssueState = 'open' | 'closed';
type PRState = 'open' | 'closed';

/* ─── Create PR Modal ────────────────────────────────────────────────────── */
function CreatePRModal({
  projectPath,
  currentBranch,
  onClose,
  onCreated,
}: {
  projectPath: string;
  currentBranch: string;
  onClose: () => void;
  onCreated: (pr: GitPullRequest) => void;
}) {
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [baseBranch, setBaseBranch] = useState('main');
  const [draft, setDraft] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!title.trim()) return;
    setSaving(true);
    setError('');
    try {
      const pr = await api.gitIntegration.createPull({
        projectPath,
        title: title.trim(),
        description,
        headBranch: currentBranch,
        baseBranch,
        draft,
      });
      onCreated(pr);
    } catch (err: any) {
      setError(err.message || 'Failed to create PR');
    } finally {
      setSaving(false);
    }
  }

  const inputStyle: React.CSSProperties = {
    width: '100%',
    padding: '5px 8px',
    borderRadius: 5,
    fontSize: 12,
    outline: 'none',
    background: 'var(--surface)',
    border: '1px solid var(--border)',
    color: 'var(--text)',
  };

  return (
    <div
      style={{
        position: 'fixed', inset: 0,
        background: 'rgba(0,0,0,0.55)',
        zIndex: 9999,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div style={{
        background: 'var(--surface)',
        border: '1px solid var(--border)',
        borderRadius: 10,
        width: 420,
        padding: 20,
        boxShadow: 'var(--shadow-xl)',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
          <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text)' }}>Create Pull Request</span>
          <button onClick={onClose} style={{ color: 'var(--text-muted)', background: 'none', border: 'none', cursor: 'pointer', fontSize: 14 }}>✕</button>
        </div>
        <form onSubmit={handleSubmit} style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          <div>
            <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Title</label>
            <input
              type="text"
              placeholder="PR title"
              value={title}
              onChange={e => setTitle(e.target.value)}
              style={inputStyle}
              autoFocus
            />
          </div>
          <div>
            <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Description (optional)</label>
            <textarea
              placeholder="Describe your changes..."
              value={description}
              onChange={e => setDescription(e.target.value)}
              rows={4}
              style={{ ...inputStyle, resize: 'vertical' }}
            />
          </div>
          <div style={{ display: 'flex', gap: 10 }}>
            <div style={{ flex: 1 }}>
              <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>From branch</label>
              <input type="text" value={currentBranch} readOnly style={{ ...inputStyle, opacity: 0.6 }} />
            </div>
            <div style={{ flex: 1 }}>
              <label style={{ display: 'block', marginBottom: 3, color: 'var(--text-muted)', fontSize: 11 }}>Into branch</label>
              <input
                type="text"
                value={baseBranch}
                onChange={e => setBaseBranch(e.target.value)}
                style={inputStyle}
              />
            </div>
          </div>
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}>
            <input type="checkbox" checked={draft} onChange={e => setDraft(e.target.checked)} />
            <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>Create as draft</span>
          </label>
          {error && <p style={{ color: 'var(--error)', fontSize: 12 }}>{error}</p>}
          <button
            type="submit"
            disabled={!title.trim() || saving}
            style={{
              padding: '7px 0', borderRadius: 6, fontSize: 12,
              background: 'var(--accent)', color: '#fff',
              border: 'none', cursor: !title.trim() || saving ? 'default' : 'pointer',
              opacity: !title.trim() || saving ? 0.5 : 1,
              marginTop: 4,
            }}
          >
            {saving ? 'Creating…' : 'Create PR'}
          </button>
        </form>
      </div>
    </div>
  );
}

/* ─── Issue Detail ───────────────────────────────────────────────────────── */
function IssueDetail({ issue, onBack }: { issue: GitIssue; onBack: () => void }) {
  function startSession() {
    const message = `## Issue #${issue.number}: ${issue.title}\n\n${issue.body}\n\nPlease implement a fix for this issue.`;
    window.dispatchEvent(new CustomEvent('flamel:send-message', { detail: { message } }));
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 shrink-0" style={{ borderBottom: '1px solid var(--border)' }}>
        <button onClick={onBack} style={{ color: 'var(--text-muted)', fontSize: 11, background: 'none', border: 'none', cursor: 'pointer' }}>← back</button>
        <span className="flex-1 text-xs font-medium truncate" style={{ color: 'var(--text)' }}>
          #{issue.number} {issue.title}
        </span>
      </div>
      <div className="flex-1 overflow-y-auto px-3 py-2 flex flex-col gap-3">
        <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
          <span style={{
            padding: '2px 7px', borderRadius: 10, fontSize: 10, fontWeight: 500,
            background: issue.state === 'open' ? 'var(--success-glow, rgba(34,197,94,0.15))' : 'var(--surface-2)',
            color: issue.state === 'open' ? 'var(--success, #22c55e)' : 'var(--text-muted)',
            border: `1px solid ${issue.state === 'open' ? 'var(--success, #22c55e)' : 'var(--border)'}`,
          }}>
            {issue.state}
          </span>
          {issue.labels.map(l => (
            <span key={l} style={{
              padding: '2px 7px', borderRadius: 10, fontSize: 10,
              background: 'var(--surface-2)', color: 'var(--text-muted)',
              border: '1px solid var(--border)',
            }}>{l}</span>
          ))}
        </div>
        <p style={{ color: 'var(--text-muted)', fontSize: 11 }}>
          by <strong style={{ color: 'var(--text)' }}>{issue.author}</strong> · {new Date(issue.createdAt).toLocaleDateString()}
        </p>
        {issue.body && (
          <pre style={{
            color: 'var(--text)', fontSize: 12, whiteSpace: 'pre-wrap', wordBreak: 'break-word',
            background: 'var(--surface-2)', border: '1px solid var(--border)',
            borderRadius: 6, padding: '8px 10px', margin: 0, fontFamily: 'inherit',
          }}>{issue.body}</pre>
        )}
        <div style={{ display: 'flex', gap: 8 }}>
          <button
            onClick={startSession}
            style={{
              flex: 1, padding: '7px 0', borderRadius: 6, fontSize: 12,
              background: 'var(--accent)', color: '#fff',
              border: 'none', cursor: 'pointer',
            }}
          >
            Start session from issue
          </button>
          <a
            href={issue.htmlUrl}
            target="_blank"
            rel="noreferrer"
            style={{
              padding: '7px 12px', borderRadius: 6, fontSize: 12,
              background: 'var(--surface-2)', color: 'var(--text-muted)',
              border: '1px solid var(--border)', cursor: 'pointer', textDecoration: 'none',
            }}
          >
            Open ↗
          </a>
        </div>
      </div>
    </div>
  );
}

/* ─── PR Detail ──────────────────────────────────────────────────────────── */
function PRDetail({ pr, projectPath, onBack }: { pr: GitPullRequest; projectPath: string; onBack: () => void }) {
  const [fullPr, setFullPr] = useState<GitPullRequest | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setLoading(true);
    api.gitIntegration.pull(projectPath, pr.number)
      .then(setFullPr)
      .catch(() => setFullPr(pr))
      .finally(() => setLoading(false));
  }, [pr.number, projectPath]);

  const displayed = fullPr ?? pr;

  function askClaude() {
    const diffSection = displayed.diff ? `\n\n## Diff\n\`\`\`diff\n${displayed.diff}\n\`\`\`` : '';
    const message = `## PR #${displayed.number}: ${displayed.title}\n\n${displayed.body}${diffSection}\n\nPlease review this pull request and provide feedback.`;
    window.dispatchEvent(new CustomEvent('flamel:send-message', { detail: { message } }));
  }

  const stateColor = displayed.state === 'open'
    ? 'var(--success, #22c55e)'
    : displayed.state === 'merged'
    ? 'var(--accent)'
    : 'var(--text-muted)';

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 shrink-0" style={{ borderBottom: '1px solid var(--border)' }}>
        <button onClick={onBack} style={{ color: 'var(--text-muted)', fontSize: 11, background: 'none', border: 'none', cursor: 'pointer' }}>← back</button>
        <span className="flex-1 text-xs font-medium truncate" style={{ color: 'var(--text)' }}>
          #{displayed.number} {displayed.title}
        </span>
      </div>
      <div className="flex-1 overflow-y-auto px-3 py-2 flex flex-col gap-3">
        {loading && <p style={{ color: 'var(--text-muted)', fontSize: 11 }}>Loading diff…</p>}
        <div style={{ display: 'flex', gap: 6, alignItems: 'center', flexWrap: 'wrap' }}>
          <span style={{
            padding: '2px 7px', borderRadius: 10, fontSize: 10, fontWeight: 500,
            background: 'var(--surface-2)', color: stateColor,
            border: `1px solid ${stateColor}`,
          }}>
            {displayed.state}{displayed.draft ? ' · draft' : ''}
          </span>
          <span style={{ color: 'var(--text-muted)', fontSize: 11 }}>
            {displayed.headBranch} → {displayed.baseBranch}
          </span>
        </div>
        <p style={{ color: 'var(--text-muted)', fontSize: 11 }}>
          by <strong style={{ color: 'var(--text)' }}>{displayed.author}</strong> · {new Date(displayed.createdAt).toLocaleDateString()}
        </p>
        {displayed.body && (
          <pre style={{
            color: 'var(--text)', fontSize: 12, whiteSpace: 'pre-wrap', wordBreak: 'break-word',
            background: 'var(--surface-2)', border: '1px solid var(--border)',
            borderRadius: 6, padding: '8px 10px', margin: 0, fontFamily: 'inherit',
          }}>{displayed.body}</pre>
        )}
        {displayed.diff && (
          <div>
            <p style={{ color: 'var(--text-muted)', fontSize: 11, marginBottom: 4 }}>Diff</p>
            <pre style={{
              color: 'var(--text)', fontSize: 11, whiteSpace: 'pre', overflowX: 'auto',
              background: 'var(--surface-2)', border: '1px solid var(--border)',
              borderRadius: 6, padding: '8px 10px', margin: 0,
              fontFamily: 'var(--font-mono)',
              maxHeight: 300,
            }}>{displayed.diff}</pre>
          </div>
        )}
        <div style={{ display: 'flex', gap: 8 }}>
          <button
            onClick={askClaude}
            style={{
              flex: 1, padding: '7px 0', borderRadius: 6, fontSize: 12,
              background: 'var(--accent)', color: '#fff',
              border: 'none', cursor: 'pointer',
            }}
          >
            Ask Claude about this PR
          </button>
          <a
            href={displayed.htmlUrl}
            target="_blank"
            rel="noreferrer"
            style={{
              padding: '7px 12px', borderRadius: 6, fontSize: 12,
              background: 'var(--surface-2)', color: 'var(--text-muted)',
              border: '1px solid var(--border)', cursor: 'pointer', textDecoration: 'none',
            }}
          >
            Open ↗
          </a>
        </div>
      </div>
    </div>
  );
}

/* ─── Issues Tab ─────────────────────────────────────────────────────────── */
function IssuesTab({ projectPath }: { projectPath: string }) {
  const [issues, setIssues] = useState<GitIssue[]>([]);
  const [state, setState] = useState<IssueState>('open');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [selected, setSelected] = useState<GitIssue | null>(null);

  useEffect(() => {
    setLoading(true);
    setError('');
    api.gitIntegration.issues(projectPath, state)
      .then(setIssues)
      .catch(err => setError(err.message || 'Failed to load issues'))
      .finally(() => setLoading(false));
  }, [projectPath, state]);

  if (selected) {
    return <IssueDetail issue={selected} onBack={() => setSelected(null)} />;
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="px-2 py-2 shrink-0 flex gap-1">
        {(['open', 'closed'] as IssueState[]).map(s => (
          <button
            key={s}
            onClick={() => setState(s)}
            style={{
              padding: '3px 10px', borderRadius: 5, fontSize: 11,
              background: state === s ? 'var(--accent)' : 'var(--surface-2)',
              color: state === s ? '#fff' : 'var(--text-muted)',
              border: `1px solid ${state === s ? 'var(--accent)' : 'var(--border)'}`,
              cursor: 'pointer',
            }}
          >
            {s}
          </button>
        ))}
      </div>
      <div className="flex-1 overflow-y-auto px-2 pb-2 flex flex-col gap-1.5">
        {loading ? (
          <p className="text-center py-4 text-xs" style={{ color: 'var(--text-muted)' }}>Loading…</p>
        ) : error ? (
          <p className="text-center py-4 text-xs" style={{ color: 'var(--error)' }}>{error}</p>
        ) : issues.length === 0 ? (
          <p className="text-center py-4 text-xs" style={{ color: 'var(--text-muted)' }}>No {state} issues</p>
        ) : issues.map(issue => (
          <div
            key={issue.number}
            className="rounded p-2 cursor-pointer"
            style={{ border: '1px solid var(--border)', background: 'var(--surface-2)' }}
            onClick={() => setSelected(issue)}
            onMouseEnter={e => (e.currentTarget.style.borderColor = 'var(--accent)')}
            onMouseLeave={e => (e.currentTarget.style.borderColor = 'var(--border)')}
          >
            <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 6 }}>
              <span style={{ color: 'var(--text)', fontSize: 12, fontWeight: 500, flex: 1 }}>
                #{issue.number} {issue.title}
              </span>
            </div>
            <div style={{ display: 'flex', gap: 6, marginTop: 5, flexWrap: 'wrap', alignItems: 'center' }}>
              {issue.labels.slice(0, 3).map(l => (
                <span key={l} style={{
                  padding: '1px 5px', borderRadius: 8, fontSize: 10,
                  background: 'var(--surface)', border: '1px solid var(--border)',
                  color: 'var(--text-muted)',
                }}>{l}</span>
              ))}
              <span style={{ color: 'var(--text-muted)', fontSize: 10, marginLeft: 'auto' }}>
                {new Date(issue.createdAt).toLocaleDateString()}
              </span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

/* ─── PRs Tab ────────────────────────────────────────────────────────────── */
function PRsTab({ projectPath, currentBranch }: { projectPath: string; currentBranch: string }) {
  const [pulls, setPulls] = useState<GitPullRequest[]>([]);
  const [state, setState] = useState<PRState>('open');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [selected, setSelected] = useState<GitPullRequest | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [createdPr, setCreatedPr] = useState<GitPullRequest | null>(null);

  useEffect(() => {
    setLoading(true);
    setError('');
    api.gitIntegration.pulls(projectPath, state)
      .then(setPulls)
      .catch(err => setError(err.message || 'Failed to load PRs'))
      .finally(() => setLoading(false));
  }, [projectPath, state]);

  if (selected) {
    return <PRDetail pr={selected} projectPath={projectPath} onBack={() => setSelected(null)} />;
  }

  function handleCreated(pr: GitPullRequest) {
    setShowCreate(false);
    setCreatedPr(pr);
    setPulls(prev => [pr, ...prev]);
  }

  const stateColor = (s: string) => s === 'open'
    ? 'var(--success, #22c55e)'
    : s === 'merged'
    ? 'var(--accent)'
    : 'var(--text-muted)';

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="px-2 py-2 shrink-0 flex gap-1 items-center">
        {(['open', 'closed'] as PRState[]).map(s => (
          <button
            key={s}
            onClick={() => setState(s)}
            style={{
              padding: '3px 10px', borderRadius: 5, fontSize: 11,
              background: state === s ? 'var(--accent)' : 'var(--surface-2)',
              color: state === s ? '#fff' : 'var(--text-muted)',
              border: `1px solid ${state === s ? 'var(--accent)' : 'var(--border)'}`,
              cursor: 'pointer',
            }}
          >
            {s}
          </button>
        ))}
        <button
          onClick={() => setShowCreate(true)}
          style={{
            marginLeft: 'auto', padding: '3px 10px', borderRadius: 5, fontSize: 11,
            background: 'var(--accent)', color: '#fff',
            border: 'none', cursor: 'pointer',
          }}
        >
          + Create PR
        </button>
      </div>

      {createdPr && (
        <div style={{
          margin: '0 8px 6px',
          padding: '8px 10px',
          background: 'var(--surface-2)',
          border: '1px solid var(--success, #22c55e)',
          borderRadius: 6,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 8,
        }}>
          <span style={{ color: 'var(--success, #22c55e)', fontSize: 12 }}>
            PR created: <strong>#{createdPr.number}</strong>
          </span>
          <div style={{ display: 'flex', gap: 6 }}>
            <button
              onClick={() => { navigator.clipboard.writeText(createdPr.htmlUrl); }}
              style={{ fontSize: 11, color: 'var(--text-muted)', background: 'none', border: 'none', cursor: 'pointer' }}
            >
              Copy link
            </button>
            <a
              href={createdPr.htmlUrl}
              target="_blank"
              rel="noreferrer"
              style={{ fontSize: 11, color: 'var(--accent)', textDecoration: 'none' }}
            >
              Open ↗
            </a>
            <button
              onClick={() => setCreatedPr(null)}
              style={{ fontSize: 11, color: 'var(--text-muted)', background: 'none', border: 'none', cursor: 'pointer' }}
            >
              ✕
            </button>
          </div>
        </div>
      )}

      <div className="flex-1 overflow-y-auto px-2 pb-2 flex flex-col gap-1.5">
        {loading ? (
          <p className="text-center py-4 text-xs" style={{ color: 'var(--text-muted)' }}>Loading…</p>
        ) : error ? (
          <p className="text-center py-4 text-xs" style={{ color: 'var(--error)' }}>{error}</p>
        ) : pulls.length === 0 ? (
          <p className="text-center py-4 text-xs" style={{ color: 'var(--text-muted)' }}>No {state} PRs</p>
        ) : pulls.map(pr => (
          <div
            key={pr.number}
            className="rounded p-2 cursor-pointer"
            style={{ border: '1px solid var(--border)', background: 'var(--surface-2)' }}
            onClick={() => setSelected(pr)}
            onMouseEnter={e => (e.currentTarget.style.borderColor = 'var(--accent)')}
            onMouseLeave={e => (e.currentTarget.style.borderColor = 'var(--border)')}
          >
            <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 6 }}>
              <span style={{ color: 'var(--text)', fontSize: 12, fontWeight: 500, flex: 1 }}>
                #{pr.number} {pr.title}
              </span>
              <span style={{
                fontSize: 10, padding: '1px 6px', borderRadius: 8,
                background: 'var(--surface)',
                border: `1px solid ${stateColor(pr.state)}`,
                color: stateColor(pr.state),
                flexShrink: 0,
              }}>
                {pr.state}{pr.draft ? ' draft' : ''}
              </span>
            </div>
            <div style={{ display: 'flex', gap: 4, marginTop: 4, alignItems: 'center' }}>
              <span style={{ color: 'var(--text-muted)', fontSize: 10, fontFamily: 'var(--font-mono)' }}>
                {pr.headBranch} → {pr.baseBranch}
              </span>
              <span style={{ color: 'var(--text-muted)', fontSize: 10, marginLeft: 'auto' }}>
                {new Date(pr.createdAt).toLocaleDateString()}
              </span>
            </div>
          </div>
        ))}
      </div>

      {showCreate && (
        <CreatePRModal
          projectPath={projectPath}
          currentBranch={currentBranch}
          onClose={() => setShowCreate(false)}
          onCreated={handleCreated}
        />
      )}
    </div>
  );
}

/* ─── IssuesPRPanel ──────────────────────────────────────────────────────── */
export default function IssuesPRPanel({ projectPath }: { projectPath: string }) {
  const [tab, setTab] = useState<Tab>('issues');
  const [repoInfo, setRepoInfo] = useState<{ branch: string; owner?: string; repo?: string; hasAccount: boolean } | null>(null);

  useEffect(() => {
    api.gitIntegration.repoInfo(projectPath)
      .then(setRepoInfo)
      .catch(() => setRepoInfo({ branch: 'unknown', hasAccount: false }));
  }, [projectPath]);

  if (!repoInfo) {
    return <p className="text-center py-8 text-xs" style={{ color: 'var(--text-muted)' }}>Loading…</p>;
  }

  if (!repoInfo.hasAccount) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-2 px-4">
        <p className="text-xs text-center" style={{ color: 'var(--text-muted)' }}>
          No git account configured for this repo.
        </p>
        <p className="text-xs text-center" style={{ color: 'var(--text-muted)' }}>
          Add a GitHub or Gitea account in Settings → Git Accounts.
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Tab bar */}
      <div className="flex items-center shrink-0 px-2 pt-2 gap-1">
        {(['issues', 'prs'] as Tab[]).map(t => (
          <button
            key={t}
            onClick={() => setTab(t)}
            style={{
              padding: '4px 10px', borderRadius: 5, fontSize: 11,
              background: tab === t ? 'var(--surface-2)' : 'transparent',
              color: tab === t ? 'var(--text)' : 'var(--text-dim)',
              border: 'none', cursor: 'pointer',
              fontWeight: tab === t ? 500 : 400,
            }}
          >
            {t === 'issues' ? 'Issues' : 'Pull Requests'}
          </button>
        ))}
        {repoInfo.owner && repoInfo.repo && (
          <span style={{ marginLeft: 'auto', fontSize: 10, color: 'var(--text-muted)', fontFamily: 'var(--font-mono)' }}>
            {repoInfo.owner}/{repoInfo.repo}
          </span>
        )}
      </div>

      <div style={{ borderBottom: '1px solid var(--border-soft)', margin: '6px 0 0' }} />

      {/* Panel content */}
      <div className="flex-1 overflow-hidden">
        {tab === 'issues' && <IssuesTab projectPath={projectPath} />}
        {tab === 'prs' && <PRsTab projectPath={projectPath} currentBranch={repoInfo.branch} />}
      </div>
    </div>
  );
}
