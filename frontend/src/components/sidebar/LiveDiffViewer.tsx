import { useEffect, useMemo, useState } from 'react';
import type { GitBranchSummary, GitCommitSummary, GitFileStatus, GitOperationKind } from '@flamel/shared';
import { api, errorMessageFromUnknown } from '../../lib/api';
import { getProjectFileRuntime, useProjectFileStore } from '../../stores/projectFileStore';
import { useIdeStore } from '../../stores/ideStore';
import { Icon } from '../ui/Icons';

interface Props {
  projectPath: string;
  onOpenFile?: (path: string) => void;
  onOpenDiff?: (file: GitFileStatus, projectPath: string) => void;
  onOpenChat?: () => void;
}

function absoluteFilePath(projectPath: string, relativePath: string): string {
  return `${projectPath.replace(/\/$/, '')}/${relativePath}`;
}

function statusColor(status: GitFileStatus['status']) {
  if (status === 'A' || status === '?') return 'var(--success)';
  if (status === 'D') return 'var(--error)';
  return 'var(--warning)';
}

function formatAge(value: number | string | null) {
  if (!value) return 'now';
  const timestamp = typeof value === 'number' ? value : Date.parse(value);
  const seconds = Math.max(0, Math.round((Date.now() - timestamp) / 1000));
  if (seconds < 5) return 'now';
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.round(seconds / 3600)}h`;
  return `${Math.round(seconds / 86400)}d`;
}

export default function LiveDiffViewer({ projectPath, onOpenFile, onOpenDiff, onOpenChat }: Props) {
  const runtime = useProjectFileStore(state => getProjectFileRuntime(state.projects, projectPath));
  const refreshGitStatus = useProjectFileStore(state => state.refreshGitStatus);
  const status = runtime.gitStatus;
  const files = useMemo(() => status?.files ?? [], [status]);
  const [view, setView] = useState<'changes' | 'history'>('changes');
  const [history, setHistory] = useState<GitCommitSummary[]>([]);
  const [branches, setBranches] = useState<GitBranchSummary[]>([]);
  const [commitMessage, setCommitMessage] = useState('');
  const [working, setWorking] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => { void refreshGitStatus(projectPath); }, [projectPath, refreshGitStatus]);

  useEffect(() => {
    let cancelled = false;
    Promise.all([api.git.history(projectPath, { limit: 40 }), api.git.branches(projectPath)])
      .then(([historyResult, branchResult]) => {
        if (!cancelled) {
          setHistory(historyResult.commits);
          setBranches(branchResult.branches);
        }
      })
      .catch(cause => { if (!cancelled) setError(errorMessageFromUnknown(cause, 'Could not load repository history')); });
    return () => { cancelled = true; };
  }, [projectPath, runtime.gitLoadedAt]);

  async function operate(operation: GitOperationKind, options: { files?: string[]; message?: string; target?: string } = {}) {
    setWorking(operation);
    setError(null);
    setNotice(null);
    try {
      const result = await api.git.operate({ path: projectPath, operation, expectedRevision: status?.revision, ...options });
      if (!result.ok) throw new Error(result.stderr || `Git ${operation} failed`);
      if (operation === 'commit' || operation === 'amend') setCommitMessage('');
      if (operation === 'commit') setNotice('Version saved locally. Publish when you’re ready to share it.');
      if (operation === 'push') setNotice('Published to your remote repository.');
      await refreshGitStatus(projectPath);
    } catch (cause) {
      setError(errorMessageFromUnknown(cause, `Git ${operation.replaceAll('_', ' ')} failed`));
    } finally {
      setWorking(null);
    }
  }

  async function openFile(file: GitFileStatus) {
    if (file.status === 'D') return;
    const path = absoluteFilePath(projectPath, file.path);
    try {
      const { content, revision } = await api.fileContent.get(path);
      useIdeStore.getState().openFile(path, content, revision);
      onOpenFile?.(path);
    } catch (cause) {
      setError(errorMessageFromUnknown(cause, 'Could not open file'));
    }
  }

  if (!status && runtime.gitLoading) {
    return <div className="p-3 text-xs" style={{ color: 'var(--text-muted)' }}>Loading repository...</div>;
  }
  if (!status && runtime.gitError) {
    return <div className="p-3 text-xs" role="alert" style={{ color: 'var(--error)' }}>{runtime.gitError}<button type="button" onClick={() => void refreshGitStatus(projectPath)} className="px-2 py-1">Retry</button></div>;
  }
  if (!status?.isRepo) {
    return <div className="p-3 text-xs" style={{ color: 'var(--text-muted)' }}>This folder is not a Git repository.</div>;
  }

  const stagedFiles = files.filter(file => file.staged);
  const currentBranch = branches.find(branch => branch.current)?.name || status.branch || 'detached';

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="px-2 py-2 shrink-0" style={{ borderBottom: '1px solid var(--border-soft)' }}>
        <div className="flex items-center gap-1.5">
          <Icon.Branch size={12} style={{ color: 'var(--accent)' }} />
          <select
            aria-label="Current branch"
            value={currentBranch}
            disabled={Boolean(working)}
            onChange={event => void operate('branch_checkout', { target: event.target.value })}
            style={{ minWidth: 0, flex: 1, height: 26, padding: '0 24px 0 7px', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', background: 'var(--surface-2)', color: 'var(--text)', fontSize: 11 }}
          >
            {branches.filter(branch => !branch.remote).map(branch => <option key={branch.name} value={branch.name}>{branch.name}</option>)}
          </select>
          <button type="button" title="Pull" disabled={Boolean(working)} onClick={() => void operate('pull')} className="icon-btn" style={{ width: 26, height: 26 }}><Icon.Download size={12} /></button>
          <button type="button" title="Publish commits to the remote repository (git push)" disabled={Boolean(working) || runtime.gitLoading || Boolean(runtime.gitError)} onClick={() => void operate('push')} className="icon-btn" style={{ height: 26, padding: '0 6px', gap: 4, fontSize: 10 }}><Icon.Upload size={12} />Publish</button>
          <button type="button" title="Refresh" disabled={runtime.gitLoading} onClick={() => void refreshGitStatus(projectPath)} className="icon-btn" style={{ width: 26, height: 26 }}><Icon.Refresh size={12} /></button>
        </div>
        <div className="flex items-center gap-2" style={{ marginTop: 7, color: 'var(--text-muted)', fontSize: 10 }}>
          <span>{files.length} changed</span>
          <span style={{ color: 'var(--success)', fontFamily: 'var(--font-mono)' }}>+{status.added}</span>
          <span style={{ color: 'var(--error)', fontFamily: 'var(--font-mono)' }}>-{status.removed}</span>
          <span style={{ marginLeft: 'auto' }}>{runtime.gitLoading ? 'updating' : formatAge(runtime.gitLoadedAt)}</span>
        </div>
      </div>

      <div className="flex items-center shrink-0" style={{ height: 34, borderBottom: '1px solid var(--border-soft)' }}>
        <button type="button" onClick={() => setView('changes')} style={{ height: '100%', padding: '0 10px', color: view === 'changes' ? 'var(--text)' : 'var(--text-muted)', borderBottom: `2px solid ${view === 'changes' ? 'var(--accent)' : 'transparent'}`, fontSize: 11 }}>Review <span style={{ color: 'var(--text-dim)' }}>{files.length}</span></button>
        <button type="button" onClick={() => setView('history')} style={{ height: '100%', padding: '0 10px', color: view === 'history' ? 'var(--text)' : 'var(--text-muted)', borderBottom: `2px solid ${view === 'history' ? 'var(--accent)' : 'transparent'}`, fontSize: 11 }}>History</button>
        {status.repositoryUrl && <a href={`${status.repositoryUrl}/pulls`} target="_blank" rel="noopener noreferrer" title="Open pull requests in your repository" style={{ padding: '0 8px', fontSize: 10, color: 'var(--text-muted)' }}>Pull requests ↗</a>}
        {onOpenChat && <button type="button" title="Open chat" onClick={onOpenChat} className="icon-btn" style={{ marginLeft: 'auto', marginRight: 5, width: 25, height: 25 }}><Icon.Chat size={12} /></button>}
      </div>

      {(error || runtime.gitError) && <div role="alert" className="px-2 py-2 shrink-0" style={{ color: 'var(--error)', background: 'var(--error-bg)', borderBottom: '1px solid var(--error-border)', fontSize: 10 }}>{error || runtime.gitError}</div>}
      {notice && <div role="status" className="px-2 py-2 shrink-0" style={{ color: 'var(--text-muted)', fontSize: 10 }}>{notice}</div>}

      {view === 'changes' ? (
        <>
          <div className="flex-1 overflow-y-auto">
            {files.length === 0 ? (
              <div className="p-3 text-xs" style={{ color: 'var(--text-muted)' }}>Working tree clean</div>
            ) : files.map(file => (
              <div key={file.path} className="group" style={{ display: 'grid', gridTemplateColumns: '18px minmax(0, 1fr) auto', gap: 5, alignItems: 'center', minHeight: 35, padding: '4px 6px', borderBottom: '1px solid var(--border-soft)' }}>
                <span style={{ color: statusColor(file.status), fontSize: 10, fontFamily: 'var(--font-mono)', fontWeight: 700 }}>{file.status}</span>
                <button type="button" title={`Review ${file.path}`} onClick={() => onOpenDiff ? onOpenDiff(file, projectPath) : void openFile(file)} style={{ minWidth: 0, textAlign: 'left', color: 'var(--text-muted)', fontSize: 11, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{file.path}</button>
                <div className="flex items-center gap-1">
                  <span style={{ fontSize: 9, fontFamily: 'var(--font-mono)', whiteSpace: 'nowrap' }}><span style={{ color: 'var(--success)' }}>+{file.added ?? 0}</span> <span style={{ color: 'var(--error)' }}>-{file.removed ?? 0}</span></span>
                  {onOpenDiff && <button type="button" title="Open diff" onClick={() => onOpenDiff(file, projectPath)} className="icon-btn" style={{ width: 23, height: 23 }}><Icon.Diff size={11} /></button>}
                  {file.unstaged && <button type="button" title="Stage file" disabled={Boolean(working)} onClick={() => void operate('stage', { files: [file.path] })} className="icon-btn" style={{ width: 23, height: 23 }}><span style={{ fontSize: 14 }}>+</span></button>}
                  {file.staged && <button type="button" title="Unstage file" disabled={Boolean(working)} onClick={() => void operate('unstage', { files: [file.path] })} className="icon-btn" style={{ width: 23, height: 23 }}><span style={{ fontSize: 14 }}>−</span></button>}
                </div>
              </div>
            ))}
          </div>
          <div className="shrink-0 p-2" style={{ borderTop: '1px solid var(--border-soft)' }}>
            <textarea
              aria-label="Commit message"
              value={commitMessage}
              onChange={event => setCommitMessage(event.target.value)}
              placeholder="Commit message"
              rows={2}
              style={{ width: '100%', resize: 'none', padding: '7px 8px', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', background: 'var(--surface-1)', color: 'var(--text)', fontSize: 11, fontWeight: 400 }}
            />
            <div className="flex items-center gap-1.5" style={{ marginTop: 6 }}>
              <button type="button" disabled={Boolean(working) || files.length === 0} onClick={() => void operate(stagedFiles.length ? 'unstage' : 'stage', { files: files.map(file => file.path) })} className="px-2 py-1 text-xs" style={{ border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', background: 'var(--surface-2)', color: 'var(--text-muted)' }}>{stagedFiles.length ? 'Unstage all' : 'Stage all'}</button>
              <button type="button" title="Commit the staged files locally; this does not push or deploy" disabled={Boolean(working) || runtime.gitLoading || Boolean(runtime.gitError) || !commitMessage.trim() || stagedFiles.length === 0} onClick={() => void operate('commit', { message: commitMessage.trim() })} className="px-2 py-1 text-xs" style={{ marginLeft: 'auto', borderRadius: 'var(--radius-sm)', background: 'var(--accent)', color: '#fff', opacity: working || runtime.gitLoading || runtime.gitError || !commitMessage.trim() || stagedFiles.length === 0 ? 0.45 : 1 }}>{working === 'commit' ? 'Saving...' : `Save version ${stagedFiles.length || ''}`}</button>
            </div>
          </div>
        </>
      ) : (
        <div className="flex-1 overflow-y-auto">
          {history.map(commit => (
            <div key={commit.id} style={{ padding: '8px 9px', borderBottom: '1px solid var(--border-soft)' }}>
              <div title={commit.subject} style={{ color: 'var(--text)', fontSize: 11, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{commit.subject}</div>
              <div className="flex items-center gap-1.5" style={{ marginTop: 4, color: 'var(--text-dim)', fontSize: 9 }}>
                <code style={{ color: 'var(--accent)' }}>{commit.shortId}</code>
                <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{commit.authorName}</span>
                <span style={{ marginLeft: 'auto' }}>{formatAge(commit.authoredAt)}</span>
              </div>
            </div>
          ))}
          {!history.length && <div className="p-3 text-xs" style={{ color: 'var(--text-muted)' }}>No commits found</div>}
        </div>
      )}
    </div>
  );
}
