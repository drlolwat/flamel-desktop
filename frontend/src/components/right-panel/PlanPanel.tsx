import { supportsSharing } from '../../lib/workspaceHost';
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { usePlanStore } from '../../stores/planStore';
import { useTabStore } from '../../stores/tabStore';
import { useAppStore } from '../../stores/appStore';
import { showContextMenu } from '../ui/ContextMenu';
import { Icon } from '../ui/Icons';
import { TypedRevealMarkdown, TypedRevealText } from '../ui/TypedReveal';
import { apiFetch } from '../../lib/api';
import type { Plan, PlanItem, PlanVersion } from '@flamel/shared';

function StatusIcon({ status }: { status: PlanItem['status'] }) {
  if (status === 'done') {
    return (
      <span style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', width: 14, height: 14, borderRadius: 3, background: 'var(--success-glow)', border: '1px solid var(--success)', flexShrink: 0 }}>
        <Icon.Check size={9} style={{ color: 'var(--success)' }} />
      </span>
    );
  }
  if (status === 'in_progress') {
    return (
      <span className="spinner" style={{ display: 'inline-block', width: 12, height: 12, border: '1.5px solid var(--accent)', borderTopColor: 'transparent', borderRadius: '50%', flexShrink: 0 }} />
    );
  }
  if (status === 'blocked') {
    return (
      <span style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', width: 14, height: 14, borderRadius: 3, background: 'var(--error-glow)', border: '1px solid var(--error)', flexShrink: 0 }}>
        <Icon.Blocked size={9} style={{ color: 'var(--error)' }} />
      </span>
    );
  }
  return (
    <span style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', width: 14, height: 14, borderRadius: 3, border: '1px solid var(--border)', flexShrink: 0 }} />
  );
}

function SectionProgressBar({ done, total }: { done: number; total: number }) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  const allDone = total > 0 && done === total;
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1 }}>
      <div style={{ height: 2, borderRadius: 1, background: 'var(--surface-3)', overflow: 'hidden', flex: 1 }}>
        <div style={{
          height: '100%', width: `${pct}%`, borderRadius: 1,
          background: allDone ? 'var(--success)' : 'var(--accent)',
          transition: 'width 0.4s ease',
        }} />
      </div>
      <span style={{ color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)', whiteSpace: 'nowrap' }}>
        {done}/{total}
      </span>
    </div>
  );
}

function OverallProgress({ done, total }: { done: number; total: number }) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  const allDone = total > 0 && done === total;

  return (
    <div style={{ marginBottom: 16 }}>
      <div style={{ height: 4, borderRadius: 2, background: 'var(--surface-3)', overflow: 'hidden', marginBottom: 5 }}>
        <div style={{
          height: '100%', width: `${pct}%`, borderRadius: 2,
          background: allDone ? 'var(--success)' : 'var(--accent)',
          transition: 'width 0.4s ease, background 0.3s ease',
        }} />
      </div>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)' }}>
          {done}/{total} done
        </span>
        {allDone && (
          <span style={{ color: 'var(--success)', fontSize: 10, fontWeight: 500 }}>Complete</span>
        )}
        {!allDone && total > 0 && (
          <span style={{ color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)' }}>{pct}%</span>
        )}
      </div>
    </div>
  );
}

function VersionDropdown({ versions, onClose }: { versions: PlanVersion[]; onClose: () => void }) {
  const list = [...versions].reverse();
  return (
    <div style={{
      position: 'absolute', top: '100%', right: 0, zIndex: 50,
      background: 'var(--surface)', border: '1px solid var(--border)',
      borderRadius: 6, boxShadow: '0 4px 16px rgba(0,0,0,0.2)',
      minWidth: 220, maxHeight: 280, overflowY: 'auto', padding: '4px 0',
    }}>
      {list.map((v, i) => (
        <button
          key={v.id}
          onClick={onClose}
          style={{
            display: 'block', width: '100%', textAlign: 'left',
            padding: '6px 12px', background: 'transparent', border: 'none',
            color: 'var(--text-muted)', fontSize: 11, cursor: 'pointer',
          }}
          onMouseEnter={e => ((e.target as HTMLElement).style.background = 'var(--surface-2)')}
          onMouseLeave={e => ((e.target as HTMLElement).style.background = 'transparent')}
        >
          <span style={{ color: 'var(--accent)', marginRight: 6 }}>v{list.length - i}</span>
          {new Date(v.timestamp).toLocaleTimeString()} — {v.items.length} items
          <span style={{ marginLeft: 6, color: 'var(--success)' }}>
            ({v.items.filter(x => x.status === 'done').length} done)
          </span>
        </button>
      ))}
    </div>
  );
}

const PlanItemRow = memo(function PlanItemRow({ item, index, onItemContextMenu, animateIn, animateDelay, streamingItemIds }: { item: PlanItem; index: number; onItemContextMenu: (item: PlanItem, e: React.MouseEvent) => void; animateIn?: boolean; animateDelay?: number; streamingItemIds?: Set<string> }) {
  const [collapsed, setCollapsed] = useState(false);
  const isDone = item.status === 'done';
  const isActive = item.status === 'in_progress';
  const hasNotes = !!item.notes;
  const shouldRevealText = !!animateIn || item.id.startsWith('pending_') || !!streamingItemIds?.has(item.id);
  const revealDelay = shouldRevealText ? (animateIn ? animateDelay ?? 0 : 0) : 0;
  const handleContextMenu = useCallback((e: React.MouseEvent) => {
    onItemContextMenu(item, e);
  }, [item, onItemContextMenu]);

  return (
    <div
      onContextMenu={handleContextMenu}
      style={{
        padding: '10px 12px',
        borderRadius: 'var(--radius-sm)',
        background: isActive ? 'var(--accent-glow)' : 'transparent',
        borderLeft: `2px solid ${isDone ? 'var(--success)' : isActive ? 'var(--accent)' : 'transparent'}`,
        transition: 'background var(--transition-base), opacity 0.3s ease, transform 0.3s ease',
        marginBottom: 2,
        animation: animateIn ? `plan-item-in 0.3s ease-out ${animateDelay ?? 0}ms both` : undefined,
      }}
      onMouseEnter={e => { if (!isActive) e.currentTarget.style.background = 'var(--surface-2)'; }}
      onMouseLeave={e => { e.currentTarget.style.background = isActive ? 'var(--accent-glow)' : 'transparent'; }}
    >
      {/* Item header */}
      <div
        style={{ display: 'flex', alignItems: 'flex-start', gap: 8, cursor: hasNotes ? 'pointer' : 'default' }}
        onClick={() => hasNotes && setCollapsed(c => !c)}
      >
        <span style={{ color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)', minWidth: 16, textAlign: 'right', marginTop: 2, flexShrink: 0, opacity: 0.5 }}>
          {index + 1}.
        </span>
        <span style={{ marginTop: 1, flexShrink: 0 }}>
          <StatusIcon status={item.status} />
        </span>
        <span style={{
          color: isDone ? 'var(--text-dim)' : 'var(--text)',
          fontSize: 13, lineHeight: 1.5,
          textDecoration: isDone ? 'line-through' : 'none',
          flex: 1,
          opacity: isDone ? 0.7 : 1,
          fontWeight: 600,
        }}>
          <TypedRevealText text={item.text} active={shouldRevealText} delay={revealDelay} />
        </span>
        <span style={{
          color: isActive ? 'var(--accent)' : 'var(--text-dim)',
          fontSize: 9,
          fontFamily: 'var(--font-mono)',
          border: '1px solid var(--border-soft)',
          borderRadius: 3,
          padding: '1px 4px',
          marginTop: 2,
          flexShrink: 0,
        }}>
          auto ≥ {item.reasoningFloor || 'medium'}
        </span>
        {hasNotes && (
          <span style={{
            color: 'var(--text-dim)', fontSize: 9, marginTop: 4, flexShrink: 0,
            transform: collapsed ? 'none' : 'rotate(90deg)',
            transition: 'transform 0.15s',
          }}>
            ▶
          </span>
        )}
      </div>

      {/* Notes — visible by default */}
      {hasNotes && !collapsed && (
        <div style={{ marginLeft: 40, marginTop: 8 }}>
          <div className="plan-notes-md" style={{ fontSize: 12, color: 'var(--text-muted)', lineHeight: 1.7 }}>
            <TypedRevealMarkdown text={item.notes!} active={shouldRevealText} delay={revealDelay + 120} />
          </div>
        </div>
      )}
    </div>
  );
});

/** Group items by section, preserving order */
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

function SectionGroup({ section, items, startIndex, onItemContextMenu, animateFromIndex, streamingItemIds }: {
  section: string;
  items: PlanItem[];
  startIndex: number;
  onItemContextMenu: (item: PlanItem, e: React.MouseEvent) => void;
  animateFromIndex: number;
  streamingItemIds?: Set<string>;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const done = items.filter(i => i.status === 'done').length;
  const firstItem = items[0];
  const revealSection = startIndex >= animateFromIndex || !!firstItem?.id.startsWith('pending_') || !!(firstItem && streamingItemIds?.has(firstItem.id));
  const sectionDelay = revealSection && startIndex >= animateFromIndex ? Math.max(0, (startIndex - animateFromIndex) * 60 - 80) : 0;

  return (
    <div style={{ marginBottom: 16 }}>
      {/* Section header */}
      <div
        onClick={() => setCollapsed(c => !c)}
        style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '8px 4px 8px 0', cursor: 'pointer',
          borderBottom: '1px solid var(--border-soft)',
          marginBottom: 6,
        }}
      >
        <span style={{
          color: 'var(--text-dim)', fontSize: 9, flexShrink: 0,
          transform: collapsed ? 'none' : 'rotate(90deg)',
          transition: 'transform 0.15s',
        }}>
          ▶
        </span>
        <span style={{
          fontSize: 13, fontWeight: 700, color: 'var(--text)',
          flex: 1, letterSpacing: '-0.01em',
        }}>
          <TypedRevealText text={section} active={revealSection} delay={sectionDelay} charsPerSecond={640} maxStep={36} snapBacklog={900} />
        </span>
        <SectionProgressBar done={done} total={items.length} />
      </div>

      {/* Section items */}
      {!collapsed && (
        <div style={{ display: 'flex', flexDirection: 'column' }}>
          {items.map((item, i) => {
            const globalIdx = startIndex + i;
            const shouldAnimate = globalIdx >= animateFromIndex;
            return (
              <PlanItemRow
                key={item.id}
                item={item}
                index={globalIdx}
                onItemContextMenu={onItemContextMenu}
                animateIn={shouldAnimate}
                animateDelay={(globalIdx - animateFromIndex) * 60}
                streamingItemIds={streamingItemIds}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}

export default function PlanPanel({ tabId, mobile = false }: { tabId: string; mobile?: boolean }) {
  const plan = usePlanStore(s => s.plans[tabId]) || { items: [] as PlanItem[], versions: [] as PlanVersion[], artifact: undefined as Plan['artifact'], streamingItemIds: undefined as Set<string> | undefined };
  const clearPlan = usePlanStore(s => s.clearPlan);
  const removeItem = usePlanStore(s => s.removeItem);
  const setPlan = usePlanStore(s => s.setPlan);
  const tabs = useTabStore(s => s.tabs);
  const updateTab = useTabStore(s => s.updateTab);
  const setRightPanel = useAppStore(s => s.setRightPanel);
  const tab = tabs.find(t => t.id === tabId);
  const [showVersions, setShowVersions] = useState(false);

  // Restore only the exact active plan persisted on this tab. A session can own several
  // historical plans, so selecting its newest plan here can attach unrelated work.
  useEffect(() => {
    async function loadPlan(planId: string) {
      const p: Plan | null = await apiFetch(`/api/plans/${encodeURIComponent(planId)}`, { credentials: 'include' })
        .then(r => r.ok ? r.json() : null)
        .catch(() => null);
      if (!p || p.items.length === 0) return;
      if (p.archived) {
        clearPlan(tabId);
        updateTab(tabId, { activePlanId: undefined });
        return;
      }
      setPlan(tabId, p.id, p.title, p.description, p.items, false, {
        artifact: p.artifact,
        versions: p.versions,
        finished: p.finished,
        executionStatus: p.executionStatus,
        approvalStatus: p.approvalStatus,
        activeItemId: p.activeItemId,
        approvalRequestedAt: p.approvalRequestedAt,
        approvedAt: p.approvedAt,
        executionStartedAt: p.executionStartedAt,
        executionUpdatedAt: p.executionUpdatedAt,
      });
      updateTab(tabId, { activePlanId: p.id });
    }

    const planId = tab?.activePlanId;
    if (planId) {
      loadPlan(planId);
    }
  }, [tab?.activePlanId, plan.items.length, tabId, clearPlan, setPlan, updateTab]);
  const [shareMsg, setShareMsg] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);

  const sortedItems = useMemo(() => [...plan.items].sort((a, b) => a.position - b.position), [plan.items]);
  const sections = useMemo(() => groupBySection(sortedItems), [sortedItems]);
  const hasSections = useMemo(() => sections.some(s => s.section !== ''), [sections]);
  const doneCount = useMemo(() => plan.items.filter(i => i.status === 'done').length, [plan.items]);
  const total = plan.items.length;
  const streamingItemIds = plan.streamingItemIds;
  const hasStreamingItems = (streamingItemIds?.size ?? 0) > 0;

  // Track live authoring so freshly generated plan text is revealed without animating ordinary API loads.
  const prevCountRef = useRef(total);
  const shouldAnimateRef = useRef(false);
  const [animateFromIndex, setAnimateFromIndex] = useState(Infinity);
  const [creationRevealEpoch, setCreationRevealEpoch] = useState(0);

  useEffect(() => {
    const handler = () => {
      shouldAnimateRef.current = true;
      prevCountRef.current = 0;
      setCreationRevealEpoch(epoch => epoch + 1);
    };
    window.addEventListener('flamel:plan-created', handler);
    return () => window.removeEventListener('flamel:plan-created', handler);
  }, []);

  useEffect(() => {
    const prev = prevCountRef.current;
    if (total > prev && shouldAnimateRef.current) {
      setAnimateFromIndex(prev);
      const timeout = setTimeout(() => {
        setAnimateFromIndex(Infinity);
        shouldAnimateRef.current = false;
      }, (total - prev) * 60 + 500);
      prevCountRef.current = total;
      return () => clearTimeout(timeout);
    }
    prevCountRef.current = total;
  }, [total, creationRevealEpoch]);

  const handleItemContextMenu = useCallback((item: PlanItem, e: React.MouseEvent) => {
    e.preventDefault();
    showContextMenu(e.clientX, e.clientY, [
      { label: 'Copy item text', icon: '⎘', onClick: () => navigator.clipboard.writeText(item.text) },
      { separator: true },
      {
        label: 'Delete item', icon: '✕', danger: true,
        onClick: async () => {
          removeItem(tabId, item.id);
          if (plan.planId) {
            try { await apiFetch(`/api/plans/${plan.planId}/items/${item.id}`, { method: 'DELETE', credentials: 'include' }); } catch {}
          }
        },
      },
    ]);
  }, [plan.planId, removeItem, tabId]);

  if (plan.items.length === 0) {
    const isPendingPlan = plan.planId === 'pending';
    const pendingTitle = isPendingPlan && plan.title && plan.title !== 'Preparing plan...' ? plan.title : '';
    return (
      <div className="flex flex-col items-center justify-center h-full" style={{ color: 'var(--text-muted)', padding: 16 }}>
        {pendingTitle ? (
          <div style={{ maxWidth: 320, textAlign: 'center' }}>
            <h1 style={{ color: 'var(--text)', fontSize: 16, fontWeight: 700, lineHeight: 1.4, margin: '0 0 8px' }}>
              <TypedRevealText text={pendingTitle} active charsPerSecond={680} maxStep={44} snapBacklog={1000} />
            </h1>
            {plan.description && (
              <div className="plan-notes-md" style={{ fontSize: 12, color: 'var(--text-muted)', lineHeight: 1.7, marginBottom: 8 }}>
                <TypedRevealMarkdown text={plan.description} active delay={90} charsPerSecond={820} maxStep={60} snapBacklog={1800} />
              </div>
            )}
            <p style={{ fontSize: 11, lineHeight: 1.6, margin: 0 }}>Authoring plan artifact...</p>
          </div>
        ) : (
          <p style={{ fontSize: 12, textAlign: 'center', padding: '0 16px', lineHeight: 1.6 }}>
            {isPendingPlan ? 'Preparing plan...' : 'Ask the assistant to make a plan - it will appear here'}
          </p>
        )}
      </div>
    );
  }

  const exportTitle = plan.title || 'Plan';
  const executionStatus = plan.executionStatus ?? (plan.finished ? 'completed' : 'ready');
  const approvalStatus = plan.approvalStatus ?? 'approved';
  const activeItem = sortedItems.find(i => i.id === plan.activeItemId) || sortedItems.find(i => i.status === 'in_progress');
  const revealPlanHeader = plan.planId === 'pending' || hasStreamingItems || animateFromIndex === 0;

  async function handleExport() {
    const lines: string[] = [`# ${exportTitle}`, ''];
    if (plan.description) lines.push(plan.description, '');
    let currentSection = '';
    for (const item of sortedItems) {
      if (item.section && item.section !== currentSection) {
        currentSection = item.section;
        lines.push(`## ${currentSection}`, '');
      }
      const check = item.status === 'done' ? 'x' : item.status === 'in_progress' ? '>' : ' ';
      lines.push(`- [${check}] ${item.text}`);
      if (item.notes) {
        lines.push('');
        lines.push(item.notes.split('\n').map(l => `  ${l}`).join('\n'));
        lines.push('');
      }
    }
    const blob = new Blob([lines.join('\n')], { type: 'text/markdown' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${exportTitle.toLowerCase().replace(/\s+/g, '-')}.md`;
    a.click();
    URL.revokeObjectURL(url);
  }

  function handleStartPlan() {
    if (!tab?.projectPath || !plan.planId || plan.planId === 'pending') return;
    window.dispatchEvent(new CustomEvent('flamel:send-message', {
      detail: {
        tabId,
        message: `Start the plan "${plan.title || 'Untitled'}" now. Plan ID: ${plan.planId}`,
        projectPath: tab.projectPath,
      },
    }));
  }

  async function handleDelete() {
    if (!confirmDelete) {
      setConfirmDelete(true);
      setTimeout(() => setConfirmDelete(false), 3000);
      return;
    }
    if (plan.planId) {
      try { await apiFetch(`/api/plans/${plan.planId}`, { method: 'DELETE', credentials: 'include' }); } catch {}
    }
    clearPlan(tabId);
    setConfirmDelete(false);
  }

  async function handleShare() {
    if (!plan.planId || plan.planId === 'pending') {
      setShareMsg('Wait for plan to save…');
      setTimeout(() => setShareMsg(''), 2000);
      return;
    }
    try {
      const r = await apiFetch(`/api/plans/${plan.planId}/share`, { method: 'POST' });
      if (!r.ok) { setShareMsg('Failed'); setTimeout(() => setShareMsg(''), 2000); return; }
      const { shareUrl } = await r.json();
      if (!shareUrl) { setShareMsg('Failed'); setTimeout(() => setShareMsg(''), 2000); return; }
      try {
        await navigator.clipboard.writeText(shareUrl);
        setShareMsg('Copied!');
      } catch {
        window.open(shareUrl, '_blank');
        setShareMsg('Opened');
      }
      setTimeout(() => setShareMsg(''), 2000);
    } catch {
      setShareMsg('Failed');
      setTimeout(() => setShareMsg(''), 2000);
    }
  }

  function planHeaderContextMenu(e: React.MouseEvent) {
    e.preventDefault();
    showContextMenu(e.clientX, e.clientY, [
      { label: 'Export as Markdown', icon: '↗', onClick: handleExport },
      ...(supportsSharing() ? [{ label: 'Share plan', icon: '⎋', onClick: handleShare }] : []),
      { separator: true },
      {
        label: 'Archive plan', icon: '⊘', danger: true,
        onClick: async () => {
          if (plan.planId) { try { await apiFetch(`/api/plans/${plan.planId}`, { method: 'DELETE', credentials: 'include' }); } catch {} }
          clearPlan(tabId);
        },
      },
    ]);
  }

  // Build a global item index for numbering across sections
  let globalIndex = 0;

  const startPlanButton = !plan.finished && (['ready', 'paused', 'blocked'].includes(executionStatus) || approvalStatus === 'pending') ? (
    <button
      type="button"
      onClick={handleStartPlan}
      disabled={plan.planId === 'pending'}
      style={{
        width: '100%', minHeight: mobile ? 44 : undefined,
        padding: mobile ? '10px 12px' : '6px 10px', borderRadius: 'var(--radius-md)',
        background: 'var(--accent)', border: '1px solid var(--accent)',
        color: '#fff', fontSize: mobile ? 13 : 11, fontWeight: 600,
        cursor: plan.planId === 'pending' ? 'not-allowed' : 'pointer',
        opacity: plan.planId === 'pending' ? 0.6 : 1,
        display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
      }}
    >
      <Icon.Deploy size={mobile ? 13 : 11} />
      {approvalStatus === 'pending' || executionStatus === 'ready' ? 'Start plan' : 'Resume plan'}
    </button>
  ) : null;

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {mobile && startPlanButton && (
        <div style={{ borderBottom: '1px solid var(--border-soft)', padding: '10px 12px', flexShrink: 0 }}>
          {startPlanButton}
        </div>
      )}
      <div className="flex-1 overflow-y-auto p-3" style={{ minHeight: 0 }}>
        {/* Plan header */}
        <div style={{ marginBottom: 14 }}>
          {plan.title && (
            <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 8, marginBottom: 8 }}>
              <h1
                onContextMenu={planHeaderContextMenu}
                style={{
                  color: 'var(--text)', fontSize: 16, fontWeight: 700,
                  margin: 0, flex: 1, lineHeight: 1.4,
                  cursor: 'context-menu', letterSpacing: '-0.02em',
                }}
              >
                <TypedRevealText text={plan.title} active={revealPlanHeader} charsPerSecond={680} maxStep={44} snapBacklog={1000} />
              </h1>
              {plan.versions.length > 1 && (
                <div style={{ position: 'relative', flexShrink: 0 }}>
                  <button
                    onClick={() => setShowVersions(v => !v)}
                    style={{
                      fontSize: 10, padding: '2px 6px', borderRadius: 'var(--radius-sm)',
                      background: 'var(--surface-2)', border: '1px solid var(--border-soft)',
                      color: 'var(--text-dim)', cursor: 'pointer', fontFamily: 'var(--font-mono)',
                    }}
                  >
                    v{plan.versions.length}
                  </button>
                  {showVersions && <VersionDropdown versions={plan.versions} onClose={() => setShowVersions(false)} />}
                </div>
              )}
            </div>
          )}
          {plan.description && (
            <div className="plan-notes-md" style={{ fontSize: 12, color: 'var(--text-muted)', lineHeight: 1.7, marginBottom: 4 }}>
              <TypedRevealMarkdown text={plan.description} active={revealPlanHeader} delay={90} charsPerSecond={820} maxStep={60} snapBacklog={1800} />
            </div>
          )}
          {plan.artifact && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 10, color: 'var(--text-dim)', fontFamily: 'var(--font-mono)', marginTop: 8 }}>
              <span style={{ color: 'var(--accent)' }}><TypedRevealText text={plan.artifact.kind} active={revealPlanHeader} delay={160} charsPerSecond={520} maxStep={24} snapBacklog={600} /></span>
              <span><TypedRevealText text={`schema v${plan.artifact.schemaVersion}`} active={revealPlanHeader} delay={210} charsPerSecond={520} maxStep={24} snapBacklog={600} /></span>
              <span><TypedRevealText text={`${plan.versions.length || 1} version${(plan.versions.length || 1) === 1 ? '' : 's'}`} active={revealPlanHeader} delay={260} charsPerSecond={520} maxStep={24} snapBacklog={600} /></span>
            </div>
          )}
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 10, color: 'var(--text-dim)', fontFamily: 'var(--font-mono)', marginTop: 8, flexWrap: 'wrap' }}>
            <span style={{ color: executionStatus === 'running' ? 'var(--accent)' : executionStatus === 'blocked' ? 'var(--error)' : executionStatus === 'completed' ? 'var(--success)' : 'var(--text-muted)' }}>
              {executionStatus}
            </span>
            <span>{approvalStatus === 'pending' ? 'approval pending' : 'approved'}</span>
            {activeItem && <span>active item {sortedItems.findIndex(i => i.id === activeItem.id) + 1}</span>}
          </div>
        </div>

        {/* Overall progress */}
        <OverallProgress done={doneCount} total={total} />

        {/* Items — grouped by section or flat */}
        {hasSections ? (
          sections.map((group) => {
            const startIdx = globalIndex;
            globalIndex += group.items.length;
            if (group.section) {
              return (
                <SectionGroup
                  key={group.section}
                  section={group.section}
                  items={group.items}
                  startIndex={startIdx}
                  onItemContextMenu={handleItemContextMenu}
                  animateFromIndex={animateFromIndex}
                  streamingItemIds={streamingItemIds}
                />
              );
            }
            // Ungrouped items at the top (no section header)
            return (
              <div key="__ungrouped" style={{ marginBottom: 16 }}>
                {group.items.map((item, i) => {
                  const idx = startIdx + i;
                  const shouldAnimate = idx >= animateFromIndex;
                  return (
                    <PlanItemRow key={item.id} item={item} index={idx} onItemContextMenu={handleItemContextMenu} animateIn={shouldAnimate} animateDelay={shouldAnimate ? (idx - animateFromIndex) * 60 : 0} streamingItemIds={streamingItemIds} />
                  );
                })}
              </div>
            );
          })
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column' }}>
            {sortedItems.map((item, i) => {
              const shouldAnimate = i >= animateFromIndex;
              return (
                <PlanItemRow key={item.id} item={item} index={i} onItemContextMenu={handleItemContextMenu} animateIn={shouldAnimate} animateDelay={shouldAnimate ? (i - animateFromIndex) * 60 : 0} streamingItemIds={streamingItemIds} />
              );
            })}
          </div>
        )}
      </div>

      {/* Footer actions */}
      <div style={{ borderTop: '1px solid var(--border-soft)', padding: '8px 10px', display: 'flex', flexDirection: 'column', gap: 6, flexShrink: 0 }}>
        {!mobile && startPlanButton}
        <div style={{ display: 'flex', gap: 6 }}>
          <button onClick={() => setRightPanel('plan-history')} style={{
            flex: 1, padding: '5px 0', borderRadius: 'var(--radius-md)',
            background: 'var(--surface-2)', border: '1px solid var(--border-soft)',
            color: 'var(--text-muted)', fontSize: 11, cursor: 'pointer', transition: 'all var(--transition-fast)',
          }}
            onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-3)'; e.currentTarget.style.color = 'var(--text)'; }}
            onMouseLeave={e => { e.currentTarget.style.background = 'var(--surface-2)'; e.currentTarget.style.color = 'var(--text-muted)'; }}
          >Close</button>
          {[
            { label: 'Export', action: handleExport },
            ...(supportsSharing() ? [{ label: shareMsg || 'Share', action: handleShare, active: !!shareMsg }] : []),
          ].map(btn => (
            <button key={btn.label} onClick={btn.action} style={{
              flex: 1, padding: '5px 0', borderRadius: 'var(--radius-md)',
              background: 'var(--surface-2)', border: '1px solid var(--border-soft)',
              color: btn.active ? 'var(--success)' : 'var(--text-muted)',
              fontSize: 11, cursor: 'pointer', transition: 'all var(--transition-fast)',
            }}
              onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-3)'; e.currentTarget.style.color = 'var(--text)'; }}
              onMouseLeave={e => { e.currentTarget.style.background = 'var(--surface-2)'; e.currentTarget.style.color = btn.active ? 'var(--success)' : 'var(--text-muted)'; }}
            >{btn.label}</button>
          ))}
          <button onClick={handleDelete} title={confirmDelete ? 'Click again to archive' : 'Archive plan'} style={{
            padding: '5px 10px', borderRadius: 'var(--radius-md)',
            background: confirmDelete ? 'var(--error)' : 'var(--surface-2)',
            color: confirmDelete ? '#fff' : 'var(--text-dim)',
            border: `1px solid ${confirmDelete ? 'var(--error)' : 'var(--border-soft)'}`,
            fontSize: 11, cursor: 'pointer', transition: 'all var(--transition-base)',
          }}>
            {confirmDelete ? 'Sure?' : '⊘'}
          </button>
        </div>
      </div>
    </div>
  );
}
