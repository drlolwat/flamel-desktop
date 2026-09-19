import { useEffect, useRef, useState } from 'react';
import { Icon } from '../ui/Icons';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { Plan, PlanItem } from '@flamel/shared';

const STATUS_ICON: Record<string, string> = { done: '✓', in_progress: '›', blocked: '⊘', todo: '○' };
const STATUS_COLOR: Record<string, string> = {
  done: 'var(--success, #2ecf73)', in_progress: 'var(--accent, #d64232)',
  blocked: 'var(--error, #ff5c75)', todo: '#888',
};

function fmt(iso: string) {
  return new Date(iso).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

function groupBySection(items: PlanItem[]): { section: string; items: PlanItem[] }[] {
  const groups: { section: string; items: PlanItem[] }[] = [];
  const seen = new Map<string, number>();
  for (const item of items) {
    const sec = item.section || '';
    const idx = seen.get(sec);
    if (idx !== undefined) {
      groups[idx].items.push(item);
    } else {
      seen.set(sec, groups.length);
      groups.push({ section: sec, items: [item] });
    }
  }
  return groups;
}

function SectionProgress({ done, total }: { done: number; total: number }) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, maxWidth: 120 }}>
      <div style={{ height: 3, borderRadius: 2, background: 'var(--surface-2, #222228)', overflow: 'hidden', flex: 1 }}>
        <div style={{ height: '100%', width: `${pct}%`, borderRadius: 2, background: done === total ? 'var(--success, #2ecf73)' : 'var(--accent, #d64232)', transition: 'width 0.4s ease' }} />
      </div>
      <span style={{ color: '#888', fontSize: 11, fontFamily: 'monospace', whiteSpace: 'nowrap' }}>{done}/{total}</span>
    </div>
  );
}

function ItemRow({ item, index }: { item: PlanItem; index: number }) {
  const [collapsed, setCollapsed] = useState(false);
  const isDone = item.status === 'done';
  const isActive = item.status === 'in_progress';
  const hasNotes = !!item.notes;

  return (
    <div style={{
      padding: '12px 16px',
      borderRadius: 8,
      background: isActive ? 'var(--accent-glow, rgba(214,66,50,0.14))' : 'var(--surface, #18181b)',
      border: `1px solid ${isActive ? 'var(--accent-border, rgba(214,66,50,0.34))' : 'var(--border, #2a2a30)'}`,
      opacity: isDone ? 0.6 : 1,
      transition: 'opacity 0.2s',
    }}>
      <div
        style={{ display: 'flex', alignItems: 'flex-start', gap: 12, cursor: hasNotes ? 'pointer' : 'default' }}
        onClick={() => hasNotes && setCollapsed(c => !c)}
      >
        <span style={{ fontSize: 11, color: '#555', minWidth: 18, textAlign: 'right', lineHeight: '22px', flexShrink: 0 }}>
          {index + 1}
        </span>
        <span style={{ fontSize: 14, fontWeight: 600, color: STATUS_COLOR[item.status] || '#888', lineHeight: '22px', flexShrink: 0, width: 16, textAlign: 'center' }}>
          {STATUS_ICON[item.status] || '○'}
        </span>
        <div style={{ flex: 1, minWidth: 0 }}>
          <span style={{
            fontSize: 14, fontWeight: 600,
            color: 'var(--text, #e8e8ea)',
            textDecoration: isDone ? 'line-through' : 'none',
            lineHeight: 1.5,
          }}>
            {item.text}
          </span>
        </div>
        {hasNotes && (
          <span style={{
            color: '#555', fontSize: 10, marginTop: 6, flexShrink: 0,
            transform: collapsed ? 'none' : 'rotate(90deg)',
            transition: 'transform 0.15s',
          }}>
            ▶
          </span>
        )}
      </div>
      {hasNotes && !collapsed && (
        <div className="plan-notes-md" style={{ marginLeft: 46, marginTop: 10, fontSize: 13, color: '#aaa', lineHeight: 1.7 }}>
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{item.notes!}</ReactMarkdown>
        </div>
      )}
    </div>
  );
}

export default function SharedPlanView({ token, server }: { token: string; server: string | null }) {
  const [plan, setPlan] = useState<Plan | null>(null);
  const [error, setError] = useState('');
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  async function fetchPlan() {
    if (!token) return;
    try {
      // If a server param is present (web.flamel.dev viewing a remote server's plan),
      // fetch cross-origin directly from that server. Otherwise use relative URL.
      const url = server
        ? `${server.replace(/\/$/, '')}/api/shared-plans/${token}`
        : `/api/shared-plans/${token}`;
      const r = await fetch(url);
      if (!r.ok) { setError('Plan not found or link expired'); return; }
      setPlan(await r.json());
      setLastUpdated(new Date());
    } catch { setError('Unable to reach server'); }
  }

  useEffect(() => {
    if (!token) return;
    fetchPlan();
    intervalRef.current = setInterval(fetchPlan, 3000);
    return () => { if (intervalRef.current) clearInterval(intervalRef.current); };
  }, [token]);

  if (error) return (
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', minHeight: '100vh', background: 'var(--bg, #0f0f11)' }}>
      <div style={{ textAlign: 'center' }}>
        <div style={{ fontSize: 32, marginBottom: 12 }}>⊘</div>
        <p style={{ color: '#ef4444', fontSize: 14 }}>{error}</p>
      </div>
    </div>
  );

  if (!plan) return (
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', minHeight: '100vh', background: 'var(--bg, #0f0f11)' }}>
      <div style={{ width: 24, height: 24, border: '2px solid #2a2a30', borderTopColor: 'var(--accent, #d64232)', borderRadius: '50%', animation: 'spin 0.8s linear infinite' }} />
    </div>
  );

  const sorted = [...plan.items].sort((a, b) => a.position - b.position);
  const sections = groupBySection(sorted);
  const hasSections = sections.some(s => s.section !== '');
  const done = sorted.filter(i => i.status === 'done').length;
  const inProgress = sorted.filter(i => i.status === 'in_progress').length;
  const blocked = sorted.filter(i => i.status === 'blocked').length;
  const todo = sorted.filter(i => i.status === 'todo').length;
  const total = sorted.length;
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;

  let globalIndex = 0;

  return (
    <div style={{ minHeight: '100vh', background: 'var(--bg, #0f0f11)', fontFamily: 'var(--font-sans, system-ui, sans-serif)' }}>
      {/* Header */}
      <header style={{
        position: 'sticky', top: 0, zIndex: 10,
        background: 'var(--surface, #18181b)', borderBottom: '1px solid var(--border, #2a2a30)',
        padding: '0 24px', height: 48, display: 'flex', alignItems: 'center', justifyContent: 'space-between',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <Icon.Logo size={16} style={{ color: 'var(--accent, #d64232)' }} />
          <span style={{ color: '#888', fontSize: 12, fontWeight: 500, letterSpacing: '0.04em' }}>flamel</span>
          <span style={{ color: '#2a2a30' }}>·</span>
          <span style={{ color: 'var(--text, #e8e8ea)', fontSize: 13, fontWeight: 500 }}>{plan.title}</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          {plan.finished && (
            <span style={{ padding: '2px 8px', borderRadius: 20, fontSize: 11, fontWeight: 500, background: 'rgba(34,197,94,0.12)', color: '#22c55e', border: '1px solid rgba(34,197,94,0.25)' }}>
              Completed
            </span>
          )}
          {lastUpdated && (
            <span style={{ color: '#888', fontSize: 11 }}>
              {lastUpdated.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' })}
            </span>
          )}
        </div>
      </header>

      <div style={{ maxWidth: 800, margin: '0 auto', padding: '32px 24px' }}>
        {/* Title + description */}
        <div style={{ marginBottom: 28 }}>
          <h1 style={{ fontSize: 24, fontWeight: 700, color: 'var(--text, #e8e8ea)', marginBottom: 10, letterSpacing: '-0.02em' }}>
            {plan.title}
          </h1>
          {plan.description && (
            <div className="plan-notes-md" style={{ color: '#aaa', fontSize: 14, lineHeight: 1.7, marginBottom: 12 }}>
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{plan.description}</ReactMarkdown>
            </div>
          )}
          <div style={{ display: 'flex', gap: 16, flexWrap: 'wrap' }}>

            <span style={{ color: '#888', fontSize: 12 }}>Created {fmt(plan.createdAt)}</span>
            <span style={{ color: '#888', fontSize: 12 }}>Updated {fmt(plan.updatedAt)}</span>
          </div>
        </div>

        {/* Progress */}
        <div style={{ marginBottom: 28 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
            <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text, #e8e8ea)' }}>{pct}% complete</span>
            <span style={{ fontSize: 12, color: '#888' }}>{done}/{total} done</span>
          </div>
          <div style={{ height: 6, borderRadius: 3, background: 'var(--surface-2, #222228)', overflow: 'hidden' }}>
            <div style={{ height: '100%', width: `${pct}%`, borderRadius: 3, background: pct === 100 ? 'var(--success, #2ecf73)' : 'var(--accent, #d64232)', transition: 'width 0.4s ease' }} />
          </div>
        </div>

        {/* Stats chips */}
        <div style={{ display: 'flex', gap: 8, marginBottom: 32, flexWrap: 'wrap' }}>
          {[
            { label: 'Done', count: done, color: 'var(--success, #2ecf73)', bg: 'var(--success-glow, rgba(46,207,115,0.12))' },
            { label: 'In progress', count: inProgress, color: 'var(--accent, #d64232)', bg: 'var(--accent-glow, rgba(214,66,50,0.14))' },
            { label: 'Blocked', count: blocked, color: 'var(--error, #ff5c75)', bg: 'var(--error-glow, rgba(255,92,117,0.13))' },
            { label: 'To do', count: todo, color: '#888', bg: 'var(--surface-2, #222228)' },
          ].map(s => (
            <div key={s.label} style={{ padding: '5px 12px', borderRadius: 8, background: s.bg, display: 'flex', alignItems: 'center', gap: 6 }}>
              <span style={{ fontSize: 15, fontWeight: 700, color: s.color, lineHeight: 1 }}>{s.count}</span>
              <span style={{ fontSize: 11, color: '#888' }}>{s.label}</span>
            </div>
          ))}
        </div>

        {/* Items — grouped by section or flat */}
        {hasSections ? (
          sections.map((group) => {
            const startIdx = globalIndex;
            globalIndex += group.items.length;
            const sectionDone = group.items.filter(i => i.status === 'done').length;

            if (group.section) {
              return (
                <div key={group.section} style={{ marginBottom: 28 }}>
                  <div style={{
                    display: 'flex', alignItems: 'center', gap: 12,
                    padding: '10px 0',
                    borderBottom: '1px solid var(--border, #2a2a30)',
                    marginBottom: 8,
                  }}>
                    <h2 style={{ fontSize: 16, fontWeight: 700, color: 'var(--text, #e8e8ea)', flex: 1, margin: 0, letterSpacing: '-0.01em' }}>
                      {group.section}
                    </h2>
                    <SectionProgress done={sectionDone} total={group.items.length} />
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                    {group.items.map((item, i) => (
                      <ItemRow key={item.id} item={item} index={startIdx + i} />
                    ))}
                  </div>
                </div>
              );
            }

            return (
              <div key="__ungrouped" style={{ display: 'flex', flexDirection: 'column', gap: 4, marginBottom: 28 }}>
                {group.items.map((item, i) => (
                  <ItemRow key={item.id} item={item} index={startIdx + i} />
                ))}
              </div>
            );
          })
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {sorted.map((item, idx) => (
              <ItemRow key={item.id} item={item} index={idx} />
            ))}
          </div>
        )}

        {/* Footer */}
        <div style={{ marginTop: 40, paddingTop: 20, borderTop: '1px solid var(--border, #2a2a30)', textAlign: 'center' }}>
          <span style={{ fontSize: 12, color: '#888' }}>
            Shared via <span style={{ color: 'var(--accent, #d64232)', fontWeight: 500 }}>flamel</span> · live updates every 3s
          </span>
        </div>
      </div>
    </div>
  );
}
