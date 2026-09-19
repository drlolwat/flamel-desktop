import { useState, useEffect } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { api, apiFetch } from '../../lib/api';
import { usePlanStore } from '../../stores/planStore';
import { useTabStore } from '../../stores/tabStore';
import { useAppStore } from '../../stores/appStore';
import { showContextMenu } from '../ui/ContextMenu';
import type { PlanMeta, Plan } from '@flamel/shared';

function statusDot(status: string) {
  if (status === 'done') return { color: 'var(--success)', char: '✓' };
  if (status === 'in_progress') return { color: 'var(--accent)', char: '›' };
  if (status === 'blocked') return { color: 'var(--error)', char: '!' };
  return { color: 'var(--text-muted)', char: '·' };
}

function ProgressBar({ done, total }: { done: number; total: number }) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  const color = pct === 100 ? 'var(--success)' : pct > 50 ? 'var(--accent)' : 'var(--text-muted)';
  return (
    <div className="flex items-center gap-1.5">
      <div className="flex-1 h-1 rounded-full overflow-hidden" style={{ background: 'var(--surface-2)' }}>
        <div className="h-full rounded-full transition-all" style={{ width: `${pct}%`, background: color }} />
      </div>
      <span style={{ color: 'var(--text-muted)', fontSize: 10, fontFamily: 'var(--font-mono)' }}>
        {done}/{total}
      </span>
    </div>
  );
}

function PlanRow({ plan, onLoad, onDelete, onRestore }: { plan: PlanMeta; onLoad: () => void; onDelete: () => void; onRestore?: () => void }) {
  const pct = plan.itemCount > 0 ? Math.round((plan.doneCount / plan.itemCount) * 100) : 0;

  return (
    <div
      className="rounded p-2 cursor-pointer group"
      style={{ border: '1px solid var(--border)', background: 'var(--surface-2)', position: 'relative' }}
      onClick={onLoad}
      onMouseEnter={e => (e.currentTarget.style.borderColor = 'var(--accent)')}
      onMouseLeave={e => (e.currentTarget.style.borderColor = 'var(--border)')}
    >
      <div className="flex items-start justify-between gap-2 mb-1.5">
        <span className="text-xs font-medium truncate" style={{ color: 'var(--text)' }}>{plan.title}</span>
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-xs" style={{ color: pct === 100 ? 'var(--success)' : 'var(--text-muted)' }}>{pct}%</span>
          {onRestore ? (
            <button
              title="Restore plan"
              onClick={e => { e.stopPropagation(); onRestore(); }}
              className="opacity-0 group-hover:opacity-100 transition-opacity"
              style={{ color: 'var(--accent)', fontSize: 11, lineHeight: 1, background: 'none', border: 'none', cursor: 'pointer', padding: '0 2px' }}
            >↺</button>
          ) : (
            <button
              title="Archive plan"
              onClick={e => { e.stopPropagation(); onDelete(); }}
              className="opacity-0 group-hover:opacity-100 transition-opacity"
              style={{ color: 'var(--text-muted)', fontSize: 13, lineHeight: 1, background: 'none', border: 'none', cursor: 'pointer', padding: '0 2px' }}
            >⊘</button>
          )}
        </div>
      </div>
      <ProgressBar done={plan.doneCount} total={plan.itemCount} />
      <div className="mt-1.5" style={{ color: 'var(--text-muted)', fontSize: 10 }}>
        {new Date(plan.updatedAt).toLocaleDateString()}
      </div>
    </div>
  );
}

function ExpandedPlan({ plan, onClose }: { plan: Plan; onClose: () => void }) {
  const { activeTabId, tabs, updateTab } = useTabStore();
  const { setPlan } = usePlanStore();
  const { setRightPanel } = useAppStore();

  async function loadIntoPanel() {
    if (!activeTabId) return;
    const sessionId = tabs.find(tab => tab.id === activeTabId)?.sessionId;
    if (sessionId) {
      await api.sessions.setActivePlan(sessionId, plan.id).catch(() => null);
    }
    setPlan(activeTabId, plan.id, plan.title, plan.description, plan.items, false, {
      artifact: plan.artifact,
      versions: plan.versions,
      finished: plan.finished,
      executionStatus: plan.executionStatus,
      approvalStatus: plan.approvalStatus,
      activeItemId: plan.activeItemId,
      approvalRequestedAt: plan.approvalRequestedAt,
      approvedAt: plan.approvedAt,
      executionStartedAt: plan.executionStartedAt,
      executionUpdatedAt: plan.executionUpdatedAt,
    });
    updateTab(activeTabId, { activePlanId: plan.id });
    onClose();
    setRightPanel('plan');
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 shrink-0" style={{ borderBottom: '1px solid var(--border)' }}>
        <button onClick={onClose} style={{ color: 'var(--text-muted)', fontSize: 11 }}>← back</button>
        <span className="flex-1 text-xs font-medium truncate" style={{ color: 'var(--text)' }}>{plan.title}</span>
        {activeTabId && (
          <button
            onClick={loadIntoPanel}
            className="shrink-0 px-2 py-0.5 rounded text-xs"
            style={{ background: 'var(--accent)', color: '#fff' }}
            title="Load into current plan panel"
          >
            Load
          </button>
        )}
      </div>

      <div className="flex-1 overflow-y-auto px-3 py-2">
        {plan.description && (
          <div className="plan-notes-md mb-3" style={{ fontSize: 12, color: 'var(--text-muted)', lineHeight: 1.7 }}>
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{plan.description}</ReactMarkdown>
          </div>
        )}
        <div className="flex flex-col gap-1.5">
          {plan.items.map(item => {
            const dot = statusDot(item.status);
            return (
              <div key={item.id} className="flex items-start gap-2">
                <span className="mt-0.5 shrink-0 text-xs font-mono" style={{ color: dot.color }}>{dot.char}</span>
                <div className="flex-1 min-w-0">
                  <span
                    className="text-xs"
                    style={{
                      color: item.status === 'done' ? 'var(--text-muted)' : 'var(--text)',
                      textDecoration: item.status === 'done' ? 'line-through' : 'none',
                    }}
                  >
                    {item.text}
                  </span>
                  {item.notes && (
                    <div className="plan-notes-md mt-0.5" style={{ fontSize: 11, color: 'var(--text-muted)', lineHeight: 1.6 }}>
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>{item.notes}</ReactMarkdown>
                    </div>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

export default function PlansHistoryPanel() {
  const { tabs, activeTabId } = useTabStore();
  const projectPath = tabs.find(t => t.id === activeTabId)?.projectPath;

  const [plans, setPlans] = useState<PlanMeta[]>([]);
  const [archivedPlans, setArchivedPlans] = useState<PlanMeta[]>([]);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<Plan | null>(null);
  const [search, setSearch] = useState('');
  const [showArchived, setShowArchived] = useState(false);

  function fetchPlans() {
    setLoading(true);
    api.plans.list(undefined, projectPath)
      .then(setPlans)
      .catch(() => {})
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    fetchPlans();
    window.addEventListener('flamel:plan-created', fetchPlans);
    return () => window.removeEventListener('flamel:plan-created', fetchPlans);
  }, [projectPath]);

  useEffect(() => {
    if (!showArchived) return;
    const params = new URLSearchParams({ archived: '1' });
    if (projectPath) params.set('projectPath', projectPath);
    apiFetch(`/api/plans?${params}`, { credentials: 'include' })
      .then(r => r.json())
      .then(setArchivedPlans)
      .catch(() => {});
  }, [showArchived, projectPath]);

  async function openPlan(meta: PlanMeta) {
    const plan = await api.plans.get(meta.id).catch(() => null);
    if (plan) setExpanded(plan);
  }

  async function archivePlan(meta: PlanMeta) {
    await apiFetch(`/api/plans/${meta.id}`, { method: 'DELETE', credentials: 'include' }).catch(() => {});
    setPlans(prev => prev.filter(p => p.id !== meta.id));
    if (showArchived) setArchivedPlans(prev => [{ ...meta, archived: true }, ...prev]);
  }

  async function restorePlan(meta: PlanMeta) {
    await apiFetch(`/api/plans/${meta.id}`, {
      method: 'PUT',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ archived: false }),
    }).catch(() => {});
    setArchivedPlans(prev => prev.filter(p => p.id !== meta.id));
    setPlans(prev => [{ ...meta, archived: false }, ...prev]);
  }

  const filtered = search
    ? plans.filter(p => p.title.toLowerCase().includes(search.toLowerCase()))
    : plans;

  const activePlans = filtered.filter(p => !p.finished);
  const finishedPlans = filtered.filter(p => p.finished);

  if (expanded) {
    return <ExpandedPlan plan={expanded} onClose={() => setExpanded(null)} />;
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="px-2 py-2 shrink-0">
        <input
          placeholder="Search plans..."
          value={search}
          onChange={e => setSearch(e.target.value)}
          className="w-full px-2 py-1.5 rounded text-xs outline-none"
          style={{ background: 'var(--surface-2)', border: '1px solid var(--border)', color: 'var(--text)' }}
        />
      </div>

      <div className="flex-1 overflow-y-auto px-2 pb-2 flex flex-col gap-1.5">
        {loading ? (
          <p className="text-center py-4 text-xs" style={{ color: 'var(--text-muted)' }}>Loading...</p>
        ) : filtered.length === 0 ? (
          <p className="text-center py-4 text-xs" style={{ color: 'var(--text-muted)' }}>No plans yet</p>
        ) : (
          <>
            {activePlans.map(p => <PlanRow key={p.id} plan={p} onLoad={() => openPlan(p)} onDelete={() => archivePlan(p)} />)}
            {finishedPlans.length > 0 && (
              <>
                <div style={{
                  display: 'flex', alignItems: 'center', gap: 6,
                  marginTop: activePlans.length > 0 ? 8 : 0, marginBottom: 2,
                }}>
                  <span style={{ color: 'var(--success)', fontSize: 10, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.06em' }}>
                    Finished
                  </span>
                  <div style={{ flex: 1, height: 1, background: 'var(--success-glow)' }} />
                </div>
                {finishedPlans.map(p => (
                  <div key={p.id} style={{ opacity: 0.65 }}>
                    <PlanRow plan={p} onLoad={() => openPlan(p)} onDelete={() => archivePlan(p)} />
                  </div>
                ))}
              </>
            )}
            {/* Archived section toggle */}
            <button
              onClick={() => setShowArchived(v => !v)}
              style={{
                marginTop: 8, display: 'flex', alignItems: 'center', gap: 6,
                background: 'none', border: 'none', cursor: 'pointer', padding: '2px 0',
                color: 'var(--text-dim)', fontSize: 10, fontWeight: 600,
                textTransform: 'uppercase', letterSpacing: '0.06em', width: '100%',
              }}
            >
              <span>{showArchived ? '▾' : '▸'}</span>
              <span>Archived</span>
              <div style={{ flex: 1, height: 1, background: 'var(--border)' }} />
            </button>
            {showArchived && (
              archivedPlans.length === 0
                ? <p style={{ color: 'var(--text-muted)', fontSize: 11, padding: '4px 0' }}>No archived plans</p>
                : archivedPlans.map(p => (
                  <div key={p.id} style={{ opacity: 0.55 }}>
                    <PlanRow plan={p} onLoad={() => openPlan(p)} onDelete={() => {}} onRestore={() => restorePlan(p)} />
                  </div>
                ))
            )}
          </>
        )}
      </div>
    </div>
  );
}
