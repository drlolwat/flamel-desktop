import React from 'react';
import LoginPage from './components/LoginPage';
import SplashScreen from './components/SplashScreen';
import StartupOverlay from './components/StartupOverlay';
import Shell from './components/layout/Shell';
import ShareView from './components/share/ShareView';
import SharedPlanView from './components/share/SharedPlanView';
import { ContextMenuPortal } from './components/ui/ContextMenu';
import { Icon } from './components/ui/Icons';
import { apiFetch } from './lib/api';
import { useConnectionStore } from './stores/connectionStore';
import { getAuthToken, setAuthToken } from './stores/authTokenStore';
import { useEffect, useState } from 'react';
import { useTabStore } from './stores/tabStore';
import { useStartupStore } from './stores/startupStore';
import { matchRoute, useBrowserLocation } from './lib/router';

const ToolWidgetGallery = import.meta.env.DEV ? React.lazy(
  () => import('./components/tool-widgets/ToolWidgetGallery'),
) : null;

type AuthState = 'loading' | 'reconnecting' | 'login' | 'authed';

interface SetupInfo {
  flamelRegistered: boolean;
}

interface DesktopUpdateInfo {
  currentVersion: string;
  version: string;
  body?: string;
  date?: string;
}

function DesktopUpdateNotice() {
  const [update, setUpdate] = useState<DesktopUpdateInfo | null>(null);
  const [state, setState] = useState<'checking' | 'ready' | 'installing' | 'error'>('checking');

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const check = () => {
      timer = setTimeout(async () => {
        try {
          const available = await window.electronAPI?.checkForUpdate?.();
          if (!cancelled && available && localStorage.getItem('flamel-dismissed-update') !== available.version) {
            setUpdate(available);
            setState('ready');
          }
        } catch {
          // Update checks are opportunistic and should never interrupt startup.
        }
      }, 3500);
    };

    if (window.electronAPI?.checkForUpdate) check();
    else window.addEventListener('tauri-ready', check, { once: true });
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      window.removeEventListener('tauri-ready', check);
    };
  }, []);

  if (!update) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: 'fixed', right: 14, bottom: 14, zIndex: 10050,
        width: 'min(360px, calc(100vw - 28px))',
        display: 'flex', alignItems: 'center', gap: 12,
        padding: '11px 12px',
        background: 'var(--surface)', border: '1px solid var(--border)',
        borderRadius: 'var(--radius-lg)', boxShadow: 'var(--shadow-xl)',
      }}
    >
      <span style={{
        width: 30, height: 30, display: 'grid', placeItems: 'center', flexShrink: 0,
        color: 'var(--accent)', background: 'color-mix(in srgb, var(--accent) 12%, transparent)',
        border: '1px solid color-mix(in srgb, var(--accent) 25%, var(--border))',
        borderRadius: 'var(--radius-md)',
      }}>
        <Icon.Refresh size={14} />
      </span>
      <div style={{ minWidth: 0, flex: 1 }}>
        <p style={{ fontSize: 12, fontWeight: 600, color: 'var(--text)', margin: 0 }}>
          {state === 'installing' ? 'Installing update' : state === 'error' ? 'Update needs attention' : 'Flamel Desktop update'}
        </p>
        <p style={{ fontSize: 11, color: 'var(--text-muted)', margin: '2px 0 0' }}>
          {state === 'installing'
            ? `Downloading ${update.version}. Flamel will restart when ready.`
            : state === 'error'
              ? 'The update could not be installed. Try again.'
              : `${update.version} is ready to install.`}
        </p>
      </div>
      <button
        type="button"
        disabled={state === 'installing'}
        onClick={async () => {
          setState('installing');
          try {
            await window.electronAPI?.installUpdate?.();
          } catch {
            setState('error');
          }
        }}
        style={{
          padding: '6px 10px', flexShrink: 0, borderRadius: 'var(--radius-md)',
          border: '1px solid var(--accent)', background: 'var(--accent)', color: '#fff',
          fontSize: 11, fontWeight: 600, cursor: state === 'installing' ? 'default' : 'pointer',
          opacity: state === 'installing' ? 0.55 : 1,
        }}
      >
        {state === 'installing' ? 'Updating…' : state === 'error' ? 'Retry' : 'Update'}
      </button>
      {state !== 'installing' && (
        <button
          type="button"
          aria-label="Dismiss update"
          title="Dismiss this version"
          onClick={() => {
            localStorage.setItem('flamel-dismissed-update', update.version);
            setUpdate(null);
          }}
          style={{
            width: 24, height: 24, display: 'grid', placeItems: 'center', flexShrink: 0,
            border: 'none', background: 'transparent', color: 'var(--text-dim)', cursor: 'pointer',
          }}
        >
          <Icon.Close size={13} />
        </button>
      )}
    </div>
  );
}

function AuthGuard({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<AuthState>('loading');
  const [setupInfo, setSetupInfo] = useState<SetupInfo>({ flamelRegistered: false });

  async function checkAuth() {
    useStartupStore.getState().advance('authenticating');
    const [setupRes, meRes] = await Promise.all([
      apiFetch('/api/setup'),
      apiFetch('/api/me', { credentials: 'include' }),
    ]);
    const data = await setupRes.json();
    setSetupInfo({ flamelRegistered: !!data.flamel_registered || !!data.local });
    if (!meRes.ok) {
      useStartupStore.getState().hide();
      setState('login');
      return;
    }

    // Clear cached state when user identity changes (prevents cross-user data leaks)
    try {
      const me = await meRes.json();
      const lastUserId = localStorage.getItem('flamel-last-user-id');
      if (lastUserId && lastUserId !== me.id) {
        // Different user — nuke all persisted zustand stores
        ['flamel-tabs', 'flamel-app', 'flamel-connection', 'flamel-chats'].forEach(k => localStorage.removeItem(k));
        useTabStore.persist.rehydrate?.();
      }
      localStorage.setItem('flamel-last-user-id', me.id);
      if (me.userHome) localStorage.setItem('flamel-user-home', me.userHome);
    } catch { /* non-fatal — worst case they see stale tabs that 403 on load */ }

    useStartupStore.getState().advance('workspace');
    setState('authed');
  }

  useEffect(() => {
    useStartupStore.getState().advance('authenticating');
    const hadPreviousSession = !!localStorage.getItem('flamel-last-user-id');

    const tryConnect = (attemptsLeft: number, delayMs: number) => {
      checkAuth().catch(() => {
        if (attemptsLeft <= 0) {
          // Exhausted retries — show login only if it looks like a real auth failure,
          // not just a server blip. If they had a previous session, show a manual retry.
          if (hadPreviousSession) useStartupStore.getState().advance('connecting');
          else useStartupStore.getState().hide();
          setState(hadPreviousSession ? 'reconnecting' : 'login');
          return;
        }
        // If we know who the user is, show reconnecting instead of a blank spinner
        if (hadPreviousSession && attemptsLeft < 4) setState('reconnecting');
        setTimeout(() => tryConnect(attemptsLeft - 1, Math.min(delayMs * 1.5, 5000)), delayMs);
      });
    };

    tryConnect(6, 800);
  }, []);

  if (state === 'loading' || state === 'reconnecting') {
    return (
      <AuthPageShell>
        <div className="flex flex-col items-center justify-center gap-3" style={{ flex: 1 }}>
          {state === 'reconnecting' && (
            <button
              onClick={() => { setState('loading'); window.location.reload(); }}
              style={{ fontSize: 11, color: 'var(--accent)', background: 'none', border: 'none', cursor: 'pointer', textDecoration: 'underline' }}
            >
              Reload
            </button>
          )}
        </div>
      </AuthPageShell>
    );
  }

  if (state === 'login') return (
    <AuthPageShell>
      <LoginPage
        onComplete={() => {
          useStartupStore.getState().begin('workspace');
          setState('authed');
        }}
        flamelRegistered={setupInfo.flamelRegistered}
      />
    </AuthPageShell>
  );
  return <>{children}</>;
}

/** Minimal shell with drag region + window controls for non-authed pages */
function AuthPageShell({ children }: { children: React.ReactNode }) {
  const isElectron = !!window.electronAPI;
  const isMac = window.electronAPI?.platform === 'darwin';
  const showControls = isElectron && !isMac;
  const appName = import.meta.env.VITE_MODE === 'local' ? 'Flamel Desktop' : 'flamel';
  const [isMax, setIsMax] = useState(false);

  useEffect(() => {
    if (!window.electronAPI) return;
    window.electronAPI.isMaximized().then(setIsMax);
    window.electronAPI.onMaximizedChange?.(setIsMax);
  }, []);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100vh', background: 'var(--bg)' }}>
      <div
        data-tauri-drag-region
        className="drag-region flex items-center justify-between shrink-0"
        style={{
          height: 'var(--header-h)',
          borderBottom: '1px solid var(--border-soft)',
          paddingLeft: isMac ? 80 : 12,
          paddingRight: showControls ? 0 : 12,
        }}
      >
        <div className="flex items-center gap-2" data-tauri-drag-region>
          <Icon.Logo size={15} style={{ color: 'var(--accent)' }} data-tauri-drag-region />
          <span style={{ color: 'var(--text)', fontWeight: 600, fontSize: 13, letterSpacing: '-0.01em' }} data-tauri-drag-region>{appName}</span>
        </div>
        {showControls && (
          <div className="flex items-stretch" style={{ height: 'var(--header-h)' }}>
            <AuthWinBtn title="Minimize" onClick={() => window.electronAPI!.minimize()}>
              <svg width="10" height="1" viewBox="0 0 10 1"><rect width="10" height="1" fill="currentColor"/></svg>
            </AuthWinBtn>
            <AuthWinBtn title={isMax ? 'Restore' : 'Maximize'} onClick={() => window.electronAPI!.maximize()}>
              {isMax ? (
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <rect x="2" y="0" width="8" height="8" fill="none" stroke="currentColor" strokeWidth="1"/>
                  <rect x="0" y="2" width="8" height="8" fill="var(--bg)" stroke="currentColor" strokeWidth="1"/>
                </svg>
              ) : (
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <rect x="0" y="0" width="10" height="10" fill="none" stroke="currentColor" strokeWidth="1"/>
                </svg>
              )}
            </AuthWinBtn>
            <AuthWinBtn title="Close" onClick={() => window.electronAPI!.closeWindow()} isClose>
              <svg width="10" height="10" viewBox="0 0 10 10">
                <line x1="0" y1="0" x2="10" y2="10" stroke="currentColor" strokeWidth="1.2"/>
                <line x1="10" y1="0" x2="0" y2="10" stroke="currentColor" strokeWidth="1.2"/>
              </svg>
            </AuthWinBtn>
          </div>
        )}
      </div>
      {children}
    </div>
  );
}

function AuthWinBtn({ children, title, onClick, isClose }: {
  children: React.ReactNode; title: string; onClick: () => void; isClose?: boolean;
}) {
  const [h, setH] = useState(false);
  return (
    <button title={title} onClick={onClick} onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)}
      className="no-drag flex items-center justify-center"
      style={{
        width: 46, height: 36, border: 'none', cursor: 'default',
        background: h ? (isClose ? '#c42b1c' : 'var(--surface-2)') : 'transparent',
        color: h ? (isClose ? '#fff' : 'var(--text)') : 'var(--text-muted)',
        transition: 'background 0.1s, color 0.1s', flexShrink: 0,
      }}
    >{children}</button>
  );
}

// Register Electron IPC listeners (protocol handler, native menu events)
function ElectronBridge() {
  const { addTab, setActiveTab } = useTabStore();

  useEffect(() => {
    if (!window.electronAPI) return;

    window.electronAPI.onOpenPath((projectPath) => {
      const id = addTab({ projectPath, title: projectPath.split('/').pop() || projectPath });
      setActiveTab(id);
    });
  }, []);

  return null;
}

/** True when running the hosted web client bundle, either in a browser or inside the desktop shell. */
const isWebMode = import.meta.env.VITE_MODE === 'web';
const isFlamelLocal = import.meta.env.VITE_MODE === 'local';

function ConnectionGuard({ children }: { children: React.ReactNode }) {
  const connected = useConnectionStore(s=>s.connected);
  if (!connected) return <SplashScreen onConnect={()=>useStartupStore.getState().advance('authenticating')} />;
  return <>{children}</>;
}

class AppErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: string | null }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { error: null };
  }
  static getDerivedStateFromError(err: unknown) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
  render() {
    if (this.state.error) {
      return (
        <div style={{
          display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
          height: '100vh', gap: 12, background: 'var(--bg)', color: 'var(--text)',
        }}>
          <div style={{ fontSize: 13, color: 'var(--error)', maxWidth: 480, textAlign: 'center', padding: '0 24px' }}>
            Something went wrong: {this.state.error}
          </div>
          <button
            onClick={() => window.location.reload()}
            style={{
              padding: '6px 16px', fontSize: 12, fontFamily: 'var(--font-sans)',
              background: 'var(--accent)', color: '#fff', border: 'none',
              borderRadius: 'var(--radius-md)', cursor: 'pointer',
            }}
          >
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

export default function App() {
  const connectionMode = useConnectionStore(s => s.mode);
  const isLocalConnection = isFlamelLocal && connectionMode !== 'remote';
  const location = useBrowserLocation();
  const sharedSession = matchRoute('/share/:token', location.pathname);
  const sharedPlan = matchRoute('/shared-plans/:token', location.pathname);
  const toolWidgetFixture =
    import.meta.env.DEV && location.pathname === '/__fixtures/tool-widgets';

  useEffect(() => {
    if (sharedSession || sharedPlan || toolWidgetFixture) useStartupStore.getState().hide();
  }, [sharedPlan, sharedSession, toolWidgetFixture]);

  let content: React.ReactNode;
  if (toolWidgetFixture && ToolWidgetGallery) {
    content = (
      <React.Suspense fallback={null}>
        <ToolWidgetGallery />
      </React.Suspense>
    );
  } else if (sharedSession) {
    content = <ShareView token={sharedSession.token} />;
  } else if (sharedPlan) {
    content = (
      <SharedPlanView
        token={sharedPlan.token}
        server={new URLSearchParams(location.search).get('server')}
      />
    );
  } else {
    content = (
      <ConnectionGuard>
        {location.pathname === '/login' ? (
          isLocalConnection ? <Shell /> : <AuthPageShell><LoginPage /></AuthPageShell>
        ) : (
          <AuthGuard>
            <Shell />
          </AuthGuard>
        )}
      </ConnectionGuard>
    );
  }

  return (
    <AppErrorBoundary>
      <ElectronBridge />
      <DesktopUpdateNotice />
      <ContextMenuPortal />
      {content}
      <StartupOverlay />
    </AppErrorBoundary>
  );
}

export { Shell as Workspace, AuthGuard, AuthPageShell };
