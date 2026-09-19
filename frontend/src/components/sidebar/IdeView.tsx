import { DiffEditor, Editor, loader } from '@monaco-editor/react';
import { useEffect, useMemo, useState } from 'react';
import type { GitDiffMode, GitFileStatus } from '@flamel/shared';
import { api, errorMessageFromUnknown } from '../../lib/api';
import { monaco } from '../../lib/monaco';
import { languageForPath, READ_ONLY_MONACO_OPTIONS } from '../../lib/monacoPresentation';
import { useProjectFileStore } from '../../stores/projectFileStore';
import { Icon } from '../ui/Icons';

loader.config({ monaco });

interface Props {
  projectPath: string;
  file: GitFileStatus;
  refreshAt?: number | null;
  onClose: () => void;
}

function absolutePath(projectPath: string, path: string) {
  return `${projectPath.replace(/\/$/, '')}/${path}`;
}

export default function IdeView({ projectPath, file, refreshAt, onClose }: Props) {
  const refreshGitStatus = useProjectFileStore(state => state.refreshGitStatus);
  const [mode, setMode] = useState<GitDiffMode>(file.staged && !file.unstaged ? 'staged' : 'worktree');
  const [view, setView] = useState<'diff' | 'file'>('diff');
  const [original, setOriginal] = useState('');
  const [modified, setModified] = useState('');
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const language = useMemo(() => languageForPath(file.path), [file.path]);

  useEffect(() => {
    if (mode === 'staged' && !file.staged) setMode('worktree');
    if (mode === 'worktree' && !file.unstaged && file.staged) setMode('staged');
  }, [file.staged, file.unstaged, mode]);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      setLoading(true);
      setError(null);
      try {
        const originalVersion = mode === 'worktree' && file.staged ? 'index' : 'head';
        const isNew = file.untracked || (file.indexStatus === 'A' && originalVersion === 'head');
        const originalPath = originalVersion === 'head' ? file.originalPath ?? file.path : file.path;
        const originalTask = isNew
          ? Promise.resolve({ content: '' })
          : api.git.file(projectPath, originalPath, originalVersion);
        const modifiedTask = mode === 'staged'
          ? file.indexStatus === 'D' ? Promise.resolve({ content: '' }) : api.git.file(projectPath, file.path, 'index')
          : file.worktreeStatus === 'D' || (file.status === 'D' && !file.staged)
            ? Promise.resolve({ content: '' })
            : api.fileContent.get(absolutePath(projectPath, file.path));
        const [before, after] = await Promise.all([originalTask, modifiedTask]);
        if (!cancelled) {
          setOriginal(before.content);
          setModified(after.content);
        }
      } catch (cause) {
        if (!cancelled) {
          setOriginal('');
          setModified('');
          setError(errorMessageFromUnknown(cause, 'Failed to load this change'));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    void load();
    return () => { cancelled = true; };
  }, [projectPath, file.path, file.originalPath, file.status, file.indexStatus, file.worktreeStatus, file.staged, file.untracked, mode, refreshAt]);

  async function operate(operation: 'stage' | 'unstage') {
    setWorking(true);
    setError(null);
    try {
      await api.git.operate({ path: projectPath, operation, files: [file.path] });
      await refreshGitStatus(projectPath);
    } catch (cause) {
      setError(errorMessageFromUnknown(cause, `Could not ${operation} file`));
    } finally {
      setWorking(false);
    }
  }

  const editorOptions = {
    ...READ_ONLY_MONACO_OPTIONS,
    renderSideBySide: true,
    minimap: { enabled: true },
    renderOverviewRuler: true,
  };

  return (
    <div className="flex flex-col h-full overflow-hidden" style={{ background: 'var(--bg)' }}>
      <header className="flex items-center gap-2 px-3 shrink-0" style={{ minHeight: 42, borderBottom: '1px solid var(--border)' }}>
        <span style={{ width: 20, color: file.status === 'D' ? 'var(--error)' : file.status === 'A' || file.status === '?' ? 'var(--success)' : 'var(--warning)', fontFamily: 'var(--font-mono)', fontWeight: 700 }}>
          {file.status}
        </span>
        <span title={file.path} style={{ color: 'var(--text)', fontSize: 12, fontFamily: 'var(--font-mono)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1 }}>
          {file.path}
        </span>
        <span style={{ color: 'var(--success)', fontSize: 11, fontFamily: 'var(--font-mono)' }}>+{file.added ?? 0}</span>
        <span style={{ color: 'var(--error)', fontSize: 11, fontFamily: 'var(--font-mono)' }}>-{file.removed ?? 0}</span>
        <div className="flex items-center" style={{ border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', overflow: 'hidden' }}>
          <button type="button" onClick={() => setView('diff')} className="px-2 py-1 text-xs" style={{ background: view === 'diff' ? 'var(--surface-3)' : 'var(--surface-1)', color: view === 'diff' ? 'var(--text)' : 'var(--text-muted)' }}>Diff</button>
          <button type="button" onClick={() => setView('file')} className="px-2 py-1 text-xs" style={{ borderLeft: '1px solid var(--border)', background: view === 'file' ? 'var(--surface-3)' : 'var(--surface-1)', color: view === 'file' ? 'var(--text)' : 'var(--text-muted)' }}>File</button>
        </div>
        {file.unstaged && <button type="button" disabled={working} onClick={() => void operate('stage')} className="px-2 py-1 text-xs" style={{ color: 'var(--text)', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', background: 'var(--surface-2)' }}>Stage</button>}
        {file.staged && <button type="button" disabled={working} onClick={() => void operate('unstage')} className="px-2 py-1 text-xs" style={{ color: 'var(--text)', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', background: 'var(--surface-2)' }}>Unstage</button>}
        <button type="button" title="Close diff" onClick={onClose} className="icon-btn" style={{ width: 26, height: 26 }}><Icon.Close size={13} /></button>
      </header>

      {file.staged && file.unstaged && (
        <div className="flex items-center gap-1 px-3 py-1.5 shrink-0" style={{ borderBottom: '1px solid var(--border-soft)', background: 'var(--surface-1)' }}>
          <span style={{ color: 'var(--text-dim)', fontSize: 10, marginRight: 4 }}>Compare</span>
          <button type="button" onClick={() => setMode('worktree')} style={{ color: mode === 'worktree' ? 'var(--accent)' : 'var(--text-muted)', background: mode === 'worktree' ? 'var(--accent-glow)' : 'transparent', border: `1px solid ${mode === 'worktree' ? 'var(--accent-dim)' : 'transparent'}`, borderRadius: 'var(--radius-sm)', padding: '3px 7px', fontSize: 10 }}>Worktree</button>
          <button type="button" onClick={() => setMode('staged')} style={{ color: mode === 'staged' ? 'var(--accent)' : 'var(--text-muted)', background: mode === 'staged' ? 'var(--accent-glow)' : 'transparent', border: `1px solid ${mode === 'staged' ? 'var(--accent-dim)' : 'transparent'}`, borderRadius: 'var(--radius-sm)', padding: '3px 7px', fontSize: 10 }}>Staged</button>
        </div>
      )}

      {error && <div className="px-3 py-2 text-xs shrink-0" style={{ color: 'var(--error)', borderBottom: '1px solid var(--error-border)', background: 'var(--error-bg)' }}>{error}</div>}
      <div className="flex-1 min-h-0 relative">
        {loading ? (
          <div className="absolute inset-0 flex items-center justify-center" style={{ color: 'var(--text-muted)', fontSize: 11 }}><span className="spinner" style={{ width: 10, height: 10, marginRight: 8 }} />Loading revisions</div>
        ) : error ? (
          <div className="p-3 text-xs" role="status" style={{ color: 'var(--text-muted)' }}>The comparison could not be loaded. Refresh the repository and try again.</div>
        ) : view === 'diff' ? (
          <DiffEditor
            original={original}
            modified={modified}
            language={language}
            theme="vs-dark"
            options={editorOptions}
            keepCurrentOriginalModel
            keepCurrentModifiedModel
          />
        ) : (
          <Editor value={modified} language={language} theme="vs-dark" options={editorOptions} />
        )}
      </div>
    </div>
  );
}
