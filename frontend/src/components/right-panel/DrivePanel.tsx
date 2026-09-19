import { useState, useEffect } from 'react';
import { api } from '../../lib/api';
import { isAutonomousOrchestrationTab, useTabStore } from '../../stores/tabStore';
import { useAppStore } from '../../stores/appStore';
import { manualModelSelection } from '../../lib/models';
import type { SessionMeta } from '@flamel/shared';

function DeleteButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      onClick={e => { e.stopPropagation(); onClick(); }}
      className="opacity-0 group-hover:opacity-100 px-1.5 shrink-0 text-xs"
      style={{ color: 'var(--text-muted)' }}
      title="Delete"
    >
      ×
    </button>
  );
}

function groupSessions(sessions: SessionMeta[]) {
  const now = new Date();
  const today: SessionMeta[] = [];
  const yesterday: SessionMeta[] = [];
  const older: SessionMeta[] = [];
  const pinned: SessionMeta[] = [];

  for (const s of sessions) {
    if (s.pinned) { pinned.push(s); continue; }
    const diff = Math.floor((now.getTime() - new Date(s.updatedAt).getTime()) / 86400000);
    if (diff === 0) today.push(s);
    else if (diff === 1) yesterday.push(s);
    else older.push(s);
  }
  return { pinned, today, yesterday, older };
}

function formatTokens(n: number) {
  if (n >= 1000) return `${Math.round(n / 1000)}K`;
  return n ? String(n) : '';
}

function SessionRow({ session, onOpen, onDelete }: { session: SessionMeta; onOpen: () => void; onDelete: () => void }) {
  const tokens = formatTokens(session.inputTokens + session.outputTokens);
  return (
    <div
      className="flex items-center rounded group"
      style={{ background: 'transparent' }}
      onMouseEnter={e => (e.currentTarget.style.background = 'var(--surface-2)')}
      onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
    >
      <button
        onClick={onOpen}
        className="flex-1 text-left px-2 py-1.5 flex items-center gap-2 overflow-hidden"
        style={{ color: 'var(--text)' }}
      >
        <span className="truncate" style={{ fontSize: 12 }}>
          {session.pinned && '📌 '}{session.title || 'Untitled session'}
        </span>
        {tokens && (
          <span className="shrink-0 opacity-0 group-hover:opacity-100" style={{ color: 'var(--text-muted)', fontSize: 10, fontFamily: 'var(--font-mono)' }}>
            {tokens}
          </span>
        )}
      </button>
      <DeleteButton onClick={onDelete} />
    </div>
  );
}

function Section({ label, sessions, onOpen, onDelete }: { label: string; sessions: SessionMeta[]; onOpen: (s: SessionMeta) => void; onDelete: (s: SessionMeta) => void }) {
  const [open, setOpen] = useState(true);
  if (!sessions.length) return null;
  return (
    <div className="mb-2">
      <button onClick={() => setOpen(o => !o)} className="flex items-center gap-1 w-full py-0.5 px-2">
        <span style={{ color: 'var(--text-muted)', fontSize: 9 }}>{open ? '▾' : '▸'}</span>
        <span style={{ color: 'var(--text-muted)', fontSize: 10, fontWeight: 500, textTransform: 'uppercase', letterSpacing: '0.05em' }}>{label}</span>
      </button>
      {open && sessions.map(s => (
        <SessionRow key={s.id} session={s} onOpen={() => onOpen(s)} onDelete={() => onDelete(s)} />
      ))}
    </div>
  );
}

export default function DrivePanel() {
  const [sessions, setSessions] = useState<SessionMeta[]>([]);
  const [search, setSearch] = useState('');
  const [loading, setLoading] = useState(false);
  const { addTab, tabs, setActiveTab } = useTabStore();
  const { sessionsVersion, refreshSessions } = useAppStore();

  useEffect(() => {
    setLoading(true);
    api.sessions.list(search || undefined)
      .then(setSessions)
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [search, sessionsVersion]);

  function openSession(session: SessionMeta) {
    const existing = tabs.find(t => t.sessionId === session.id);
    if (existing) {
      setActiveTab(existing.id);
    } else {
      addTab({
        sessionId: session.id,
        projectPath: session.projectPath,
        title: session.title || 'Session',
        model: manualModelSelection(session.requestedModel, session.model),
        reasoningEffort: (session.requestedReasoningEffort as any) || 'auto',
        resolvedModel: session.model as any,
        resolvedReasoningEffort: session.resolvedReasoningEffort as any,
        routingReason: session.routingReason ?? undefined,
        activePlanId: session.activePlanId ?? undefined,
        sessionOwnerKind: session.sessionOwnerKind,
        orchestrationId: session.orchestrationId ?? undefined,
        orchestrationRunId: session.orchestrationRunId ?? undefined,
        orchestrationRole: session.orchestrationRole ?? undefined,
      });
    }
  }

  async function deleteSession(session: SessionMeta) {
    await api.sessions.delete(session.id).catch(() => {});
    refreshSessions();
  }

  const operatorSessions = sessions.filter(session => !isAutonomousOrchestrationTab(session));
  const grouped = groupSessions(operatorSessions);

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="px-2 py-2 shrink-0">
        <input
          placeholder="Search sessions..."
          value={search}
          onChange={e => setSearch(e.target.value)}
          className="w-full px-2 py-1.5 rounded text-xs outline-none"
          style={{ background: 'var(--surface-2)', border: '1px solid var(--border)', color: 'var(--text)' }}
        />
      </div>

      <div className="flex-1 overflow-y-auto px-1 pb-2">
        {loading && !sessions.length ? (
          <p className="text-center py-4" style={{ color: 'var(--text-muted)', fontSize: 11 }}>Loading...</p>
        ) : operatorSessions.length === 0 ? (
          <p className="text-center py-4" style={{ color: 'var(--text-muted)', fontSize: 11 }}>No chat sessions yet</p>
        ) : (
          <>
            <Section label="Pinned" sessions={grouped.pinned} onOpen={openSession} onDelete={deleteSession} />
            <Section label="Today" sessions={grouped.today} onOpen={openSession} onDelete={deleteSession} />
            <Section label="Yesterday" sessions={grouped.yesterday} onOpen={openSession} onDelete={deleteSession} />
            <Section label="Earlier" sessions={grouped.older} onOpen={openSession} onDelete={deleteSession} />
          </>
        )}
      </div>
    </div>
  );
}
