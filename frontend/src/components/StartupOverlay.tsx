import { useEffect, useRef, useState } from 'react';
import { useStartupStore, type StartupPhase } from '../stores/startupStore';
import { Icon } from './ui/Icons';

const PHASE_LABEL: Record<StartupPhase, string> = {
  starting: 'Starting Flamel',
  connecting: 'Connecting',
  authenticating: 'Connected, securing your session',
  workspace: 'Opening your workspace',
  chats: 'Loading chats and live state',
  welcome: 'Welcome back',
};

export default function StartupOverlay() {
  const active = useStartupStore((state) => state.active);
  const phase = useStartupStore((state) => state.phase);
  const targetProgress = useStartupStore((state) => state.targetProgress);
  const run = useStartupStore((state) => state.run);
  const hide = useStartupStore((state) => state.hide);
  const error = useStartupStore((state) => state.error);
  const retry = useStartupStore((state) => state.retry);
  const openWorkspace = useStartupStore((state) => state.openWorkspace);
  const [progress, setProgress] = useState(3);
  const [leaving, setLeaving] = useState(false);
  const completionTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (completionTimer.current) {
      clearTimeout(completionTimer.current);
      completionTimer.current = null;
    }
    setProgress(3);
    setLeaving(false);
  }, [run]);

  useEffect(() => {
    if (!active) return;
    const interval = window.setInterval(() => {
      setProgress((current) => {
        if (current >= targetProgress) return current;
        const distance = targetProgress - current;
        return Math.min(targetProgress, current + Math.max(0.35, distance * 0.1));
      });
    }, 32);
    return () => window.clearInterval(interval);
  }, [active, targetProgress]);

  useEffect(() => {
    if (error || !active || phase !== 'welcome') {
      if (completionTimer.current) clearTimeout(completionTimer.current);
      completionTimer.current = null;
      setLeaving(false);
      return;
    }
    if (progress < 99.5 || completionTimer.current) return;
    completionTimer.current = setTimeout(() => {
      setLeaving(true);
      completionTimer.current = setTimeout(() => {
        completionTimer.current = null;
        hide();
      }, 220);
    }, 360);
  }, [active, error, hide, phase, progress]);

  useEffect(() => () => {
    if (completionTimer.current) clearTimeout(completionTimer.current);
  }, []);

  if (!active) return null;

  const appName = import.meta.env.VITE_MODE === 'local' ? 'Flamel Desktop' : 'flamel';

  return (
    <div
      className={`startup-overlay${leaving ? ' startup-overlay--leaving' : ''}`}
      data-tauri-drag-region
      role="status"
      aria-live="polite"
      aria-label={error || `${PHASE_LABEL[phase]}, ${Math.round(progress)} percent`}
    >
      <div className="startup-overlay__content" data-tauri-drag-region>
        <div className="startup-overlay__brand" data-tauri-drag-region>
          <span className="startup-overlay__mark"><Icon.Logo size={23} /></span>
          <span>{appName}</span>
        </div>
        <div className="startup-overlay__status">
          <span key={phase} className="startup-overlay__label">{PHASE_LABEL[phase]}</span>
          <span className="startup-overlay__percent">{Math.round(progress)}%</span>
        </div>
        <div className="startup-overlay__track" aria-hidden="true">
          <span className="startup-overlay__fill" style={{ width: `${progress}%` }}>
            <span className="startup-overlay__ember" />
          </span>
        </div>
        {error && (
          <div role="alert" style={{ marginTop: 16, color: 'var(--text-muted)', fontSize: 12 }}>
            <p style={{ margin: '0 0 10px' }}>{error}</p>
            <button type="button" onClick={() => retry?.()} style={{ padding: '6px 12px', color: 'var(--text)', background: 'var(--surface)', border: '1px solid var(--border)', borderRadius: 4, cursor: 'pointer' }}>Try again</button>
            {openWorkspace && <button type="button" onClick={openWorkspace} title="Unloaded chats will stay on a retry screen" style={{ marginLeft: 10, padding: '6px 0', color: 'var(--text-muted)', background: 'transparent', border: 0, cursor: 'pointer' }}>Open workspace</button>}
          </div>
        )}
      </div>
    </div>
  );
}
