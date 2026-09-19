import { useEffect, useState } from 'react';
import { navigate } from '../../lib/router';
import type { SessionMeta } from '@flamel/shared';
import { apiFetch } from '../../lib/api';
import { Icon } from '../ui/Icons';

export default function ShareView({ token }: { token: string }) {
  const [data, setData] = useState<{ session: SessionMeta } | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!token) return;
    apiFetch(`/share/${token}`)
      .then(r => r.ok ? r.json() : Promise.reject(r))
      .then(setData)
      .catch(() => setError('Share link not found or expired'));
  }, [token]);

  if (error) return (
    <div className="flex items-center justify-center min-h-screen" style={{ background: 'var(--bg)' }}>
      <p style={{ color: 'var(--error)' }}>{error}</p>
    </div>
  );

  if (!data) return (
    <div className="flex items-center justify-center min-h-screen" style={{ background: 'var(--bg)' }}>
      <div className="spinner" style={{ width: 24, height: 24, border: '2px solid var(--border)', borderTopColor: 'var(--accent)', borderRadius: '50%' }} />
    </div>
  );

  return (
    <div className="min-h-screen" style={{ background: 'var(--bg)' }}>
      <header className="flex items-center justify-between px-6 h-12 sticky top-0 z-10"
        style={{ background: 'var(--surface)', borderBottom: '1px solid var(--border)' }}>
        <div className="flex items-center gap-3">
          <Icon.Logo size={16} style={{ color: 'var(--accent)' }} />
          <span style={{ color: 'var(--text-muted)', fontSize: 13 }}>
            Shared session · {data.session.title || 'Untitled'}
          </span>
        </div>
        <button
          onClick={() => navigate('/')}
          className="px-3 py-1.5 rounded text-xs"
          style={{ background: 'var(--accent)', color: '#fff' }}
        >
          Open copy
        </button>
      </header>

      <div className="max-w-3xl mx-auto py-8 px-4">
        <div className="rounded-lg p-4 mb-4" style={{ background: 'var(--surface)', border: '1px solid var(--border)' }}>
          <p style={{ color: 'var(--text-muted)', fontSize: 12 }}>
            Read-only session · {data.session.projectPath}
          </p>
          <p style={{ color: 'var(--text)', fontSize: 14, marginTop: 4 }}>
            {data.session.messageCount} messages · {data.session.model || 'Unknown model'}
          </p>
        </div>
        <p style={{ color: 'var(--text-muted)', fontSize: 12, textAlign: 'center' }}>
          [Session blocks would render here — load from /api/sessions/:id]
        </p>
      </div>
    </div>
  );
}
