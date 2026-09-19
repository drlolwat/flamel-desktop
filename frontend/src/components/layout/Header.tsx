import React, { useEffect, useState, useRef } from 'react';
import SettingsModal from '../ui/SettingsModal';
import ReactDOM from 'react-dom';
import { useTabStore } from '../../stores/tabStore';
import { useUsageStore } from '../../stores/usageStore';
import { navigate } from '../../lib/router';
import { Icon } from '../ui/Icons';
import { apiFetch } from '../../lib/api';
import { useConnectionStore, getApiBase } from '../../stores/connectionStore';
import { useAuthTokenStore } from '../../stores/authTokenStore';
import { logoutHost } from '../../lib/workspaceHost';

interface Me {
  id: string;
  username: string;
  isAdmin: boolean;
}

const SETTINGS_DEEP_LINK_KEYS = [
  'settings',
  'view',
  'object',
  'assistant',
] as const;

export function settingsTargetFromSearch(search: string): string | undefined {
  const params = new URLSearchParams(search);
  const settings = params.get('settings');
  if (settings === 'assistant') {
    return `assistant:${params.get('view') || 'overview'}:${params.get('object') || ''}:${params.get('assistant') || ''}`;
  }
  if (settings === 'integrations') {
    return `integrations:${params.get('view') || 'connected'}`;
  }
  return undefined;
}

/* ─── Reusable Modal ────────────────────────────────────────────────────── */
function Modal({
  title,
  onClose,
  width = 480,
  children,
}: {
  title: string;
  onClose: () => void;
  width?: number;
  children: React.ReactNode;
}) {
  /* Escape key */
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose();
    }
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  const backdrop: React.CSSProperties = {
    position: 'fixed',
    inset: 0,
    background: 'rgba(0,0,0,0.65)',
    zIndex: 9999,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
  };

  const card: React.CSSProperties = {
    background: 'var(--surface)',
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius-xl)',
    width,
    maxHeight: '80vh',
    overflowY: 'auto',
    padding: 24,
    boxShadow: 'var(--shadow-xl)',
    animation: 'modal-in 0.18s ease-out',
    position: 'relative',
  };

  const headerRow: React.CSSProperties = {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    marginBottom: 20,
  };

  const titleStyle: React.CSSProperties = {
    fontSize: 15,
    fontWeight: 600,
    color: 'var(--text)',
  };

  return ReactDOM.createPortal(
    <div
      style={backdrop}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div style={card} onMouseDown={(e) => e.stopPropagation()}>
        <div style={headerRow}>
          <span style={titleStyle}>{title}</span>
          <button
            onClick={onClose}
            aria-label="Close"
            style={{
              color: 'var(--text-muted)',
              background: 'none',
              border: 'none',
              cursor: 'pointer',
              padding: 4,
              borderRadius: 'var(--radius-sm)',
              display: 'flex',
              transition: 'color var(--transition-fast)',
            }}
            onMouseEnter={(e) =>
              ((e.currentTarget as HTMLButtonElement).style.color =
                'var(--text)')
            }
            onMouseLeave={(e) =>
              ((e.currentTarget as HTMLButtonElement).style.color =
                'var(--text-muted)')
            }
          >
            <Icon.Close size={14} />
          </button>
        </div>
        {children}
      </div>
    </div>,
    document.body,
  );
}

/* ─── Header ────────────────────────────────────────────────────────────── */
export default function Header() {
  const { tabs } = useTabStore();
  const { stats, fetch: fetchUsage } = useUsageStore();
  const [showMenu, setShowMenu] = useState(false);
  const [showUsage, setShowUsage] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [settingsInitialTab, setSettingsInitialTab] = useState<
    string | undefined
  >(undefined);
  const [me, setMe] = useState<Me | null>(null);

  const menuRef = useRef<HTMLDivElement>(null);
  const [isElectron, setIsElectron] = useState(
    !!window.electronAPI?.isElectron,
  );
  const isMac = window.electronAPI?.platform === 'darwin';
  const showCustomControls = isElectron && !isMac;
  const isFlamelLocal = import.meta.env.VITE_MODE === 'local';
  const connectionMode = useConnectionStore((s) => s.mode);
  const flamelSignedIn = useConnectionStore((s) => s.flamelSignedIn);
  const isLocalConnection = isFlamelLocal && connectionMode !== 'remote';
  const [isMaximized, setIsMaximized] = useState(false);

  const activeStreaming = tabs.filter((t) => t.isStreaming).length;

  useEffect(() => {
    // Check immediately in case tauri-ready fired before this effect ran
    if (window.electronAPI?.isElectron) {
      setIsElectron(true);
      return;
    }
    const handler = () => setIsElectron(!!window.electronAPI?.isElectron);
    window.addEventListener('tauri-ready', handler);
    return () => window.removeEventListener('tauri-ready', handler);
  }, []);

  useEffect(() => {
    if (!window.electronAPI) return;
    window.electronAPI
      .isMaximized()
      .then(setIsMaximized)
      .catch(() => {});
    window.electronAPI.onMaximizedChange(setIsMaximized);
  }, [isElectron]);

  useEffect(() => {
    apiFetch('/api/me', { credentials: 'include' })
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => {
        if (data) setMe(data);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (showUsage) fetchUsage();
  }, [showUsage]);

  useEffect(() => {
    const target = settingsTargetFromSearch(window.location.search);
    if (!target) return;
    setSettingsInitialTab(target);
    setShowSettings(true);

    const clean = new URL(window.location.href);
    SETTINGS_DEEP_LINK_KEYS.forEach((key) => clean.searchParams.delete(key));
    window.history.replaceState(
      window.history.state,
      '',
      `${clean.pathname}${clean.search}${clean.hash}`,
    );
  }, []);

  // Open settings when other components (e.g. InputBar connect banner) request it
  useEffect(() => {
    const handler = (e: Event) => {
      const tab = (e as CustomEvent).detail?.tab;
      setSettingsInitialTab(tab);
      setShowSettings(true);
    };
    window.addEventListener('flamel:open-settings', handler);
    return () => window.removeEventListener('flamel:open-settings', handler);
  }, []);

  /* Click-outside only closes the small dropdown, not the modals */
  useEffect(() => {
    function onClickOutside(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(false);
      }
    }
    if (showMenu) document.addEventListener('mousedown', onClickOutside);
    return () => document.removeEventListener('mousedown', onClickOutside);
  }, [showMenu]);

  function formatTokens(n: number) {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1000) return `${Math.round(n / 1000)}K`;
    return String(n);
  }

  async function handleLogout() {
    if (isFlamelLocal) {
      await logoutHost();
      return;
    }
    const base = getApiBase();
    // Clear stored JWT so the next login can't accidentally re-use a stale token
    if (base) {
      useAuthTokenStore.getState().clearToken(base);
    } else {
      useAuthTokenStore.setState({ tokens: {} });
    }
    // Best-effort server-side logout (clears flamel_jwt cookie on the flamel server)
    await apiFetch('/api/auth/flamel/logout', {
      method: 'POST',
      credentials: 'include',
    }).catch(() => {});
    await logoutHost();
    navigate('/login');
  }

  return (
    <>
      <header
        data-tauri-drag-region
        className="flex items-center justify-between shrink-0 relative z-10"
        style={{
          height: 'var(--header-h)',
          borderBottom: '1px solid var(--border-soft)',
          background: 'var(--bg)',
          paddingLeft: isMac ? 80 : 12,
          paddingRight: showCustomControls ? 0 : 12,
          userSelect: 'none',
        }}
      >
        {/* ── Left: logo + name (draggable area) ──────────────────────── */}
        <div className="flex items-center gap-2" data-tauri-drag-region>
          <Icon.Logo
            size={15}
            style={{ color: 'var(--accent)', flexShrink: 0 }}
            data-tauri-drag-region
          />
          <span
            style={{
              color: 'var(--text)',
              fontWeight: 600,
              fontSize: 13,
              letterSpacing: '-0.01em',
            }}
            data-tauri-drag-region
          >
            {isFlamelLocal ? 'Flamel Desktop' : 'flamel'}
          </span>
        </div>

        {/* ── Right: streaming indicator + user menu + win controls ───── */}
        <div className="flex items-center" style={{ gap: 4 }}>
          {activeStreaming > 0 && (
            <span
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 5,
                color: 'var(--accent)',
                fontSize: 11,
                padding: '2px 8px',
                borderRadius: 'var(--radius-md)',
                background: 'var(--accent-glow)',
                marginRight: 4,
              }}
            >
              <span
                className="spinner"
                style={{
                  width: 7,
                  height: 7,
                  border: '1.5px solid var(--accent)',
                  borderTopColor: 'transparent',
                  borderRadius: '50%',
                  display: 'inline-block',
                }}
              />
              {activeStreaming === 1
                ? 'streaming'
                : `${activeStreaming} streaming`}
            </span>
          )}

          {/* User menu */}
          <div className="relative" ref={menuRef}>
            <button
              onClick={() => setShowMenu((prev) => !prev)}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 5,
                padding: '3px 8px',
                borderRadius: 'var(--radius-md)',
                color: showMenu ? 'var(--text)' : 'var(--text-muted)',
                background: showMenu ? 'var(--surface-2)' : 'transparent',
                border: 'none',
                cursor: 'pointer',
                fontSize: 12,
                transition:
                  'background var(--transition-fast), color var(--transition-fast)',
              }}
              onMouseEnter={(e) => {
                if (!showMenu) {
                  e.currentTarget.style.background = 'var(--surface-2)';
                  e.currentTarget.style.color = 'var(--text)';
                }
              }}
              onMouseLeave={(e) => {
                if (!showMenu) {
                  e.currentTarget.style.background = 'transparent';
                  e.currentTarget.style.color = 'var(--text-muted)';
                }
              }}
            >
              <Icon.User size={12} />
              <span>{isLocalConnection && flamelSignedIn ? 'Flamel account · Local' : me?.username || '…'}</span>
              <Icon.ChevronDown size={10} style={{ opacity: 0.6 }} />
            </button>

            {showMenu && (
              <div
                className="absolute right-0 z-50"
                style={{
                  top: 'calc(100% + 4px)',
                  minWidth: 168,
                  background: 'var(--surface)',
                  border: '1px solid var(--border)',
                  borderRadius: 'var(--radius-lg)',
                  boxShadow: 'var(--shadow-lg)',
                  padding: '4px 0',
                  overflow: 'hidden',
                }}
              >
                {[
                  {
                    label: 'Settings',
                    action: () => {
                      setShowMenu(false);
                      setShowSettings(true);
                    },
                  },
                  // Usage hidden for now — data still tracked, re-enable by uncommenting:
                  // { label: 'Usage', action: () => { setShowMenu(false); setShowUsage(true); } },
                ].map((item) => (
                  <button
                    key={item.label}
                    className="w-full text-left"
                    onClick={item.action}
                    style={{
                      padding: '6px 12px',
                      fontSize: 12,
                      color: 'var(--text)',
                      background: 'transparent',
                      border: 'none',
                      cursor: 'pointer',
                      display: 'block',
                      width: '100%',
                      transition: 'background var(--transition-fast)',
                    }}
                    onMouseEnter={(e) =>
                      (e.currentTarget.style.background = 'var(--surface-2)')
                    }
                    onMouseLeave={(e) =>
                      (e.currentTarget.style.background = 'transparent')
                    }
                  >
                    {item.label}
                  </button>
                ))}
                {(isFlamelLocal || !isLocalConnection) && (
                  <>
                    <div
                      style={{
                        borderTop: '1px solid var(--border-soft)',
                        margin: '4px 0',
                      }}
                    />
                    <button
                      className="w-full text-left"
                      onClick={() => {
                        useConnectionStore.getState().disconnect();
                        window.location.reload();
                      }}
                      style={{
                        padding: '6px 12px',
                        fontSize: 12,
                        color: 'var(--text-muted)',
                        background: 'transparent',
                        border: 'none',
                        cursor: 'pointer',
                        display: 'block',
                        width: '100%',
                        transition: 'background var(--transition-fast)',
                      }}
                      onMouseEnter={(e) =>
                        (e.currentTarget.style.background = 'var(--surface-2)')
                      }
                      onMouseLeave={(e) =>
                        (e.currentTarget.style.background = 'transparent')
                      }
                    >
                      {isFlamelLocal ? 'Switch connection' : 'Switch server'}
                    </button>
                    {(!isLocalConnection || flamelSignedIn) && (
                      <button
                        className="w-full text-left"
                        onClick={handleLogout}
                        style={{
                          padding: '6px 12px',
                          fontSize: 12,
                          color: 'var(--error)',
                          background: 'transparent',
                          border: 'none',
                          cursor: 'pointer',
                          display: 'block',
                          width: '100%',
                          transition: 'background var(--transition-fast)',
                        }}
                        onMouseEnter={(e) =>
                          (e.currentTarget.style.background =
                            'var(--error-glow)')
                        }
                        onMouseLeave={(e) =>
                          (e.currentTarget.style.background = 'transparent')
                        }
                      >
                        Sign out
                      </button>
                    )}
                  </>
                )}
              </div>
            )}
          </div>

          {/* Custom window controls — Linux/Windows only */}
          {showCustomControls && (
            <div
              className="flex items-stretch"
              style={{ marginLeft: 4, height: 'var(--header-h)' }}
            >
              <WinBtn
                title="Minimize"
                onClick={() => window.electronAPI!.minimize()}
              >
                <svg width="10" height="1" viewBox="0 0 10 1">
                  <rect width="10" height="1" fill="currentColor" />
                </svg>
              </WinBtn>
              <WinBtn
                title={isMaximized ? 'Restore' : 'Maximize'}
                onClick={() => window.electronAPI!.maximize()}
              >
                {isMaximized ? (
                  <svg width="10" height="10" viewBox="0 0 10 10">
                    <rect
                      x="2"
                      y="0"
                      width="8"
                      height="8"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1"
                    />
                    <rect
                      x="0"
                      y="2"
                      width="8"
                      height="8"
                      fill="var(--bg)"
                      stroke="currentColor"
                      strokeWidth="1"
                    />
                  </svg>
                ) : (
                  <svg width="10" height="10" viewBox="0 0 10 10">
                    <rect
                      x="0"
                      y="0"
                      width="10"
                      height="10"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1"
                    />
                  </svg>
                )}
              </WinBtn>
              <WinBtn
                title="Close"
                onClick={() => window.electronAPI!.closeWindow()}
                isClose
              >
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <line
                    x1="0"
                    y1="0"
                    x2="10"
                    y2="10"
                    stroke="currentColor"
                    strokeWidth="1.2"
                  />
                  <line
                    x1="10"
                    y1="0"
                    x2="0"
                    y2="10"
                    stroke="currentColor"
                    strokeWidth="1.2"
                  />
                </svg>
              </WinBtn>
            </div>
          )}
        </div>
      </header>

      {/* ── Settings modal ──────────────────────────────────────────────── */}
      {showSettings && (
        <SettingsModal
          me={me}
          initialTab={settingsInitialTab}
          onClose={() => {
            setShowSettings(false);
            setSettingsInitialTab(undefined);
          }}
        />
      )}

      {/* ── Usage modal ─────────────────────────────────────────────────── */}
      {showUsage && stats && (
        <Modal title="Usage" onClose={() => setShowUsage(false)}>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(3, 1fr)',
              gap: 16,
              marginBottom: 20,
            }}
          >
            {[
              { label: 'Today', val: stats.today },
              { label: 'This week', val: stats.week },
              { label: 'This month', val: stats.month },
            ].map(({ label, val }) => (
              <div key={label}>
                <p
                  style={{
                    color: 'var(--text-muted)',
                    fontSize: 11,
                    marginBottom: 4,
                  }}
                >
                  {label}
                </p>
                <p
                  style={{
                    color: 'var(--text)',
                    fontSize: 20,
                    fontWeight: 600,
                  }}
                >
                  {formatTokens(val.inputTokens + val.outputTokens)}
                  <span
                    style={{
                      fontSize: 11,
                      fontWeight: 400,
                      color: 'var(--text-muted)',
                      marginLeft: 3,
                    }}
                  >
                    tok
                  </span>
                </p>
              </div>
            ))}
          </div>

          {stats.byProject.length > 0 && (
            <>
              <p
                style={{
                  color: 'var(--text-muted)',
                  fontSize: 12,
                  marginBottom: 10,
                }}
              >
                By project
              </p>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                {stats.byProject.slice(0, 5).map((p) => (
                  <div
                    key={p.projectPath}
                    style={{
                      display: 'flex',
                      justifyContent: 'space-between',
                      alignItems: 'center',
                    }}
                  >
                    <span
                      style={{
                        color: 'var(--text-muted)',
                        fontSize: 12,
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                        maxWidth: 300,
                      }}
                    >
                      {p.projectPath.split('/').pop()}
                    </span>
                    <span
                      style={{
                        color: 'var(--text)',
                        fontSize: 12,
                        flexShrink: 0,
                        marginLeft: 12,
                      }}
                    >
                      {formatTokens(p.inputTokens + p.outputTokens)} tok
                    </span>
                  </div>
                ))}
              </div>
            </>
          )}
        </Modal>
      )}
    </>
  );
}

/* ─── WinBtn ────────────────────────────────────────────────────────────── */
function WinBtn({
  children,
  title,
  onClick,
  isClose,
}: {
  children: React.ReactNode;
  title: string;
  onClick: () => void;
  isClose?: boolean;
}) {
  const [hovered, setHovered] = useState(false);
  return (
    <button
      title={title}
      onClick={onClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className="no-drag flex items-center justify-center"
      style={{
        width: 46,
        height: 36,
        border: 'none',
        cursor: 'default',
        background: hovered
          ? isClose
            ? '#c42b1c'
            : 'var(--surface-2)'
          : 'transparent',
        color: hovered
          ? isClose
            ? '#fff'
            : 'var(--text)'
          : 'var(--text-muted)',
        transition: 'background 0.1s, color 0.1s',
        flexShrink: 0,
      }}
    >
      {children}
    </button>
  );
}
