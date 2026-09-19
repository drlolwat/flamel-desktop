import { useState } from 'react';
import type { SessionWorkspace } from '@flamel/shared';
import { apiFetch, errorMessageFromUnknown } from '../../lib/api';

interface Review {
  summary: string;
  diff: string;
  hasUncommittedChanges: boolean;
}

export default function WorkspaceNotice({ sessionId, workspace }: { sessionId: string; workspace: SessionWorkspace }) {
  const [review, setReview] = useState<Review | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(false);
  if (!workspace.isolated) return null;

  async function loadReview() {
    setOpen(true);
    setLoading(true);
    setError('');
    try {
      const response = await apiFetch(`/api/sessions/${encodeURIComponent(sessionId)}/workspace/review`);
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.error || 'Could not load the comparison');
      setReview(payload);
    } catch (cause) {
      setError(errorMessageFromUnknown(cause, 'Could not load the comparison'));
    } finally {
      setLoading(false);
    }
  }

  return <section aria-label="Separate workspace" style={{ flexShrink: 0, padding: '6px 12px', borderBottom: '1px solid var(--border-soft)', fontSize: 11, color: 'var(--text-muted)' }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
      <span title={workspace.projectPath}>Separate workspace · <code>{workspace.branch}</code></span>
      <button type="button" disabled={loading} onClick={() => open ? setOpen(false) : void loadReview()} style={{ marginLeft: 'auto', color: 'var(--accent)' }}>{loading ? 'Loading…' : open ? 'Close review' : 'Review to bring back'}</button>
    </div>
    {open && <div style={{ paddingTop: 6, maxHeight: '35vh', overflow: 'auto' }}>
      <p>Started from {workspace.baseBranch || 'detached HEAD'} at <code>{workspace.baseSha?.slice(0, 8)}</code>. The original folder’s uncommitted changes and ignored files, including local .env files, were not copied.</p>
      <p title={workspace.sourcePath}>Review saved commits below, then publish this branch and open a pull request to bring them back. Nothing is merged automatically.</p>
      {error && <p role="alert" style={{ color: 'var(--error)' }}>{error}</p>}
      {review && <>
        {review.hasUncommittedChanges && <p role="status">There are unsaved changes. Use Review → Save version before bringing those back.</p>}
        <pre style={{ fontSize: 10, whiteSpace: 'pre-wrap', marginTop: 6 }}>{review.summary || 'No saved changes since this workspace started.'}</pre>
        {review.diff && <pre style={{ fontSize: 10, whiteSpace: 'pre', overflowX: 'auto', marginTop: 6 }}>{review.diff}</pre>}
        <button type="button" disabled={loading} onClick={() => void loadReview()} style={{ color: 'var(--accent)', marginTop: 6 }}>Refresh comparison</button>
      </>}
    </div>}
  </section>;
}
