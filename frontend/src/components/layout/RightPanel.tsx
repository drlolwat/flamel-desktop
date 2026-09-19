import { supportsSharing } from '../../lib/workspaceHost';
import { useState } from 'react';
import { useAppStore } from '../../stores/appStore';
import { useTabStore } from '../../stores/tabStore';
import { usePlanStore } from '../../stores/planStore';
import PlanPanel from '../right-panel/PlanPanel';
import PlansHistoryPanel from '../right-panel/PlansHistoryPanel';
import { Icon } from '../ui/Icons';
import { apiFetch } from '../../lib/api';

export default function RightPanel({ mobile = false }: { mobile?: boolean }) {
  const { rightPanel, setRightPanel, setRightPanelOpen, setMobilePanel } = useAppStore();
  const { tabs, activeTabId } = useTabStore();
  const activeTab = tabs.find(t => t.id === activeTabId);
  const plan = usePlanStore(s => activeTabId ? s.plans[activeTabId] : undefined);
  const [opening, setOpening] = useState(false);
  const isHistory = rightPanel === 'plan-history';

  async function openInTab() {
    if (!plan?.planId || plan.planId === 'pending') return;
    setOpening(true);
    try {
      const r = await apiFetch(`/api/plans/${plan.planId}/share`, { method: 'POST' });
      if (r.ok) {
        const { shareUrl } = await r.json();
        if (shareUrl) await navigator.clipboard.writeText(shareUrl);
      }
    } catch {}
    setOpening(false);
  }

  const hasPlan = plan && plan.items && plan.items.length > 0;
  const canOpen = hasPlan && plan.planId && plan.planId !== 'pending';

  const iconBtn = (onClick: () => void, title: string, children: React.ReactNode, active?: boolean) => (
    <button
      onClick={onClick}
      title={title}
      style={{
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        width: mobile ? 36 : 22, height: mobile ? 36 : 22,
        color: active ? 'var(--accent)' : 'var(--text-dim)',
        background: active ? 'var(--accent-glow)' : 'transparent',
        border: 'none', cursor: 'pointer',
        borderRadius: 'var(--radius-sm)',
        transition: 'color var(--transition-fast), background var(--transition-fast)',
      }}
      onMouseEnter={e => { e.currentTarget.style.color = 'var(--text)'; e.currentTarget.style.background = 'var(--surface-2)'; }}
      onMouseLeave={e => { e.currentTarget.style.color = active ? 'var(--accent)' : 'var(--text-dim)'; e.currentTarget.style.background = active ? 'var(--accent-glow)' : 'transparent'; }}
    >
      {children}
    </button>
  );

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div
        className="flex items-center justify-between shrink-0"
        style={{ height: 'var(--header-h)', borderBottom: '1px solid var(--border-soft)', padding: '0 10px' }}
      >
        <div className="flex items-center gap-1.5">
          <Icon.Plan size={12} />
          <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text)', letterSpacing: '-0.01em' }}>
            {isHistory ? 'Plan History' : 'Plan'}
          </span>
        </div>
        <div className="flex items-center gap-1">
          {supportsSharing() && !isHistory && canOpen && iconBtn(openInTab, 'Share plan', opening
            ? <span className="spinner" style={{ width: 10, height: 10, border: '1.5px solid var(--text-dim)', borderTopColor: 'transparent', borderRadius: '50%', display: 'inline-block' }} />
            : <Icon.ExternalLink size={11} />
          )}
          {iconBtn(
            () => setRightPanel(isHistory ? 'plan' : 'plan-history'),
            isHistory ? 'Back to plan' : 'Plan history',
            <Icon.History size={11} />,
            isHistory,
          )}
          {iconBtn(
            () => mobile ? setMobilePanel('chat') : setRightPanelOpen(false),
            'Close panel',
            <Icon.Close size={mobile ? 15 : 11} />,
          )}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-hidden">
        {isHistory ? (
          <PlansHistoryPanel />
        ) : activeTab ? (
          <PlanPanel tabId={activeTab.id} mobile={mobile} />
        ) : (
          <div className="flex items-center justify-center h-full">
            <p style={{ color: 'var(--text-dim)', fontSize: 12 }}>No active session</p>
          </div>
        )}
      </div>
    </div>
  );
}
