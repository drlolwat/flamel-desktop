import React, { useEffect, useRef, useState } from 'react';
import ReactDOM from 'react-dom';
import type {
  CapabilityCatalogEntry,
  CapabilityTool,
  GovernedSkill,
  MarketplaceCapability,
  GitAccount,
} from '@flamel/shared';
import { Icon } from './Icons';
import { api, apiFetch } from '../../lib/api';
import { useConnectionStore } from '../../stores/connectionStore';
import {
  CODEX_MODELS,
  normalizeReasoningEffort,
  reasoningEffortsForModel,
} from '../../lib/models';
import { hasHostPanel, renderHostPanel } from '../../lib/workspaceHost';
import './SettingsModal.css';

// ─── Types ──────────────────────────────────────────────────────────────────

interface Me {
  id: string;
  username: string;
  isAdmin: boolean;
}
interface UserSettings {
  git_name: string;
  git_email: string;
  git_signing_key: string;
  preferred_provider: 'codex' | 'claude';
  cross_provider_fallback: boolean;
  codex_planning_reasoning_effort: PlanningReasoningEffort;
  claude_planning_reasoning_effort: PlanningReasoningEffort;
}
type PlanningReasoningEffort = 'high' | 'xhigh' | 'max' | 'ultra';
interface CompactionSettings {
  codex_compaction_model: string;
  codex_compaction_reasoning_effort: string;
  codex_compaction_fallback_enabled: boolean;
}

interface Props {
  me: Me | null;
  onClose: () => void;
  initialTab?: string;
}

// ─── Shared form primitives ──────────────────────────────────────────────────

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <label
        style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-muted)' }}
      >
        {label}
      </label>
      {children}
      {hint && (
        <p style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 1 }}>
          {hint}
        </p>
      )}
    </div>
  );
}

const inputCss: React.CSSProperties = {
  padding: '7px 10px',
  borderRadius: 'var(--radius-md)',
  fontSize: 13,
  outline: 'none',
  background: 'var(--surface-2)',
  border: '1px solid var(--border)',
  color: 'var(--text)',
  width: '100%',
  boxSizing: 'border-box',
};

function Input(props: React.InputHTMLAttributes<HTMLInputElement>) {
  const [focused, setFocused] = useState(false);
  return (
    <input
      {...props}
      style={{
        ...inputCss,
        borderColor: focused ? 'var(--border-focus)' : 'var(--border)',
        ...(props.style ?? {}),
      }}
      onFocus={(e) => {
        setFocused(true);
        props.onFocus?.(e);
      }}
      onBlur={(e) => {
        setFocused(false);
        props.onBlur?.(e);
      }}
    />
  );
}

function Select(props: React.SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      {...props}
      style={{ ...inputCss, cursor: 'pointer', ...(props.style ?? {}) }}
    />
  );
}

function Btn({
  children,
  onClick,
  variant = 'default',
  disabled,
  type = 'button',
  fullWidth,
}: {
  children: React.ReactNode;
  onClick?: () => void;
  variant?: 'default' | 'primary' | 'danger' | 'ghost' | 'success';
  disabled?: boolean;
  type?: 'button' | 'submit';
  fullWidth?: boolean;
}) {
  const bg =
    variant === 'primary'
      ? 'var(--accent)'
      : variant === 'danger'
        ? 'var(--error)'
        : variant === 'success'
          ? 'var(--success, #22c55e)'
          : 'var(--surface-2)';
  const col =
    variant === 'primary' || variant === 'danger' || variant === 'success'
      ? '#fff'
      : 'var(--text-muted)';
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      style={{
        padding: '7px 14px',
        borderRadius: 'var(--radius-md)',
        fontSize: 12,
        fontWeight: 500,
        background: bg,
        color: col,
        border: `1px solid ${variant === 'ghost' ? 'transparent' : 'var(--border)'}`,
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.45 : 1,
        transition: 'background 0.15s, opacity 0.15s',
        width: fullWidth ? '100%' : undefined,
      }}
    >
      {children}
    </button>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h3
      style={{
        fontSize: 11,
        fontWeight: 600,
        letterSpacing: '0.07em',
        textTransform: 'uppercase',
        color: 'var(--text-dim)',
        marginBottom: 12,
        marginTop: 0,
      }}
    >
      {children}
    </h3>
  );
}

function Card({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        background: 'var(--surface-2)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--radius-lg)',
        padding: '14px 16px',
      }}
    >
      {children}
    </div>
  );
}

function Row({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 16,
      }}
    >
      {children}
    </div>
  );
}

function Toggle({ on, onToggle }: { on: boolean; onToggle: () => void }) {
  return (
    <button
      onClick={onToggle}
      style={{
        width: 36,
        height: 20,
        borderRadius: 10,
        background: on ? 'var(--accent)' : 'var(--surface-3)',
        border: `1px solid ${on ? 'var(--accent)' : 'var(--border)'}`,
        cursor: 'pointer',
        position: 'relative',
        transition: 'background 0.2s, border-color 0.2s',
        flexShrink: 0,
      }}
    >
      <span
        style={{
          position: 'absolute',
          top: 2,
          left: on ? 17 : 2,
          width: 14,
          height: 14,
          borderRadius: '50%',
          background: '#fff',
          transition: 'left 0.2s',
          boxShadow: '0 1px 3px rgba(0,0,0,0.4)',
        }}
      />
    </button>
  );
}

// ─── Tab sections ────────────────────────────────────────────────────────────

function GeneralTab({
  isElectron,
  autoLaunch,
  setAutoLaunch,
  appVersion,
  userSettings,
  setUserSettings,
}: {
  isElectron: boolean;
  autoLaunch: boolean;
  setAutoLaunch: (v: boolean) => void;
  appVersion: string;
  userSettings: UserSettings;
  setUserSettings: React.Dispatch<React.SetStateAction<UserSettings>>;
}) {
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [updateState, setUpdateState] = useState<
    'idle' | 'checking' | 'installing' | 'current' | 'error'
  >('idle');
  const [routingSaved, setRoutingSaved] = useState(false);
  const [routingError, setRoutingError] = useState('');

  async function saveRouting() {
    setRoutingError('');
    const response = await apiFetch('/api/me/settings', {
      method: 'PUT',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(userSettings),
    });
    if (!response.ok) {
      const payload = await response.json().catch(() => ({}));
      setRoutingError(payload.error || 'Failed to save routing preferences');
      return;
    }
    setRoutingSaved(true);
    setTimeout(() => setRoutingSaved(false), 2500);
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 24 }}>
      <div>
        <SectionTitle>AI routing</SectionTitle>
        <Card>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
            <Field
              label="Preferred provider"
              hint="Auto uses this provider first, then chooses the model and reasoning level for the task."
            >
              <Select
                value={userSettings.preferred_provider}
                onChange={(event) =>
                  setUserSettings((current) => ({
                    ...current,
                    preferred_provider: event.target
                      .value as UserSettings['preferred_provider'],
                  }))
                }
              >
                <option value="codex">OpenAI Codex</option>
                <option value="claude">Claude</option>
              </Select>
            </Field>
            <div style={{ height: 1, background: 'var(--border-soft)' }} />
            <Row>
              <div>
                <p
                  style={{
                    fontSize: 13,
                    color: 'var(--text)',
                    marginBottom: 2,
                  }}
                >
                  Provider fallback
                </p>
                <p style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                  Use the other connected provider only when the preferred
                  provider cannot run the task.
                </p>
              </div>
              <Toggle
                on={userSettings.cross_provider_fallback}
                onToggle={() =>
                  setUserSettings((current) => ({
                    ...current,
                    cross_provider_fallback: !current.cross_provider_fallback,
                  }))
                }
              />
            </Row>
            {routingError && (
              <p style={{ fontSize: 12, color: 'var(--error)' }}>
                {routingError}
              </p>
            )}
            {routingSaved && (
              <p style={{ fontSize: 12, color: 'var(--success)' }}>Saved</p>
            )}
            <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
              <Btn variant="primary" onClick={saveRouting}>
                Save routing
              </Btn>
            </div>
          </div>
        </Card>
      </div>
      {isElectron && (
        <div>
          <SectionTitle>App</SectionTitle>
          <Card>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
              <Row>
                <div>
                  <p
                    style={{
                      fontSize: 13,
                      color: 'var(--text)',
                      marginBottom: 2,
                    }}
                  >
                    Launch at login
                  </p>
                  <p style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                    Start Flamel Desktop when you log in
                  </p>
                </div>
                <Toggle
                  on={autoLaunch}
                  onToggle={async () => {
                    const next = !autoLaunch;
                    await window.electronAPI!.setAutoLaunch(next);
                    setAutoLaunch(next);
                  }}
                />
              </Row>
              <div style={{ height: 1, background: 'var(--border-soft)' }} />
              <Row>
                <div>
                  <p
                    style={{
                      fontSize: 13,
                      color: 'var(--text)',
                      marginBottom: 2,
                    }}
                  >
                    Flamel Desktop
                  </p>
                  <p style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                    {updateVersion
                      ? `${updateVersion} is available`
                      : updateState === 'current'
                        ? 'You are up to date'
                        : appVersion
                          ? `Version ${appVersion}`
                          : 'Desktop release'}
                  </p>
                </div>
                <Btn
                  variant={updateVersion ? 'primary' : 'default'}
                  disabled={
                    updateState === 'checking' || updateState === 'installing'
                  }
                  onClick={async () => {
                    if (updateVersion) {
                      setUpdateState('installing');
                      try {
                        await window.electronAPI?.installUpdate?.();
                      } catch {
                        setUpdateState('error');
                      }
                      return;
                    }
                    setUpdateState('checking');
                    try {
                      const available =
                        await window.electronAPI?.checkForUpdate?.();
                      setUpdateVersion(available?.version ?? null);
                      setUpdateState(available ? 'idle' : 'current');
                    } catch {
                      setUpdateState('error');
                    }
                  }}
                >
                  {updateState === 'checking'
                    ? 'Checking…'
                    : updateState === 'installing'
                      ? 'Updating…'
                      : updateState === 'error'
                        ? 'Try again'
                        : updateVersion
                          ? 'Update'
                          : 'Check for updates'}
                </Btn>
              </Row>
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}

type ProviderAuthStatus = {
  connected: boolean;
  expired?: boolean;
  source?: 'flamel' | 'local_cli' | 'env' | string;
};

const PLANNING_REASONING_OPTIONS: Array<{
  id: PlanningReasoningEffort;
  label: string;
  detail: string;
}> = [
  {
    id: 'high',
    label: 'High',
    detail: 'Faster planning for familiar, bounded work',
  },
  {
    id: 'xhigh',
    label: 'Extra high',
    detail: 'Recommended balance for architecture and implementation plans',
  },
  {
    id: 'max',
    label: 'Maximum',
    detail: 'Deeper analysis for broad or high-risk changes',
  },
  {
    id: 'ultra',
    label: 'Ultra',
    detail: 'Strongest available planning profile and delegation',
  },
];

function planningReasoningEffort(value: unknown): PlanningReasoningEffort {
  return typeof value === 'string' &&
    ['high', 'xhigh', 'max', 'ultra'].includes(value)
    ? (value as PlanningReasoningEffort)
    : 'xhigh';
}

function PlanningDepthSettings({
  provider,
  userSettings,
  setUserSettings,
}: {
  provider: 'codex' | 'claude';
  userSettings: UserSettings;
  setUserSettings: React.Dispatch<React.SetStateAction<UserSettings>>;
}) {
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState('');
  const field =
    provider === 'codex'
      ? 'codex_planning_reasoning_effort'
      : 'claude_planning_reasoning_effort';
  const value = userSettings[field];

  async function save() {
    setSaving(true);
    setError('');
    try {
      const response = await apiFetch('/api/me/settings', {
        method: 'PUT',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(userSettings),
      });
      const payload = await response.json().catch(() => ({}));
      if (!response.ok) {
        setError(payload.error || 'Failed to save planning depth');
        return;
      }
      setUserSettings((current) => ({ ...current, ...payload }));
      setSaved(true);
      setTimeout(() => setSaved(false), 2500);
    } catch {
      setError('Failed to save planning depth');
    } finally {
      setSaving(false);
    }
  }

  return (
    <div>
      <SectionTitle>Automatic planning</SectionTitle>
      <Card>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
          <div>
            <p style={{ fontSize: 13, color: 'var(--text)', marginBottom: 3 }}>
              Planning depth
            </p>
            <p
              style={{
                fontSize: 11,
                color: 'var(--text-muted)',
                lineHeight: 1.5,
              }}
            >
              Used when reasoning is Auto and Flamel detects a planning request.
              The selected model receives the closest level it supports; manual
              reasoning always takes precedence.
            </p>
          </div>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
              gap: 8,
            }}
          >
            {PLANNING_REASONING_OPTIONS.map((option) => {
              const active = value === option.id;
              return (
                <button
                  key={option.id}
                  type="button"
                  onClick={() =>
                    setUserSettings((current) => ({
                      ...current,
                      [field]: option.id,
                    }))
                  }
                  style={{
                    minHeight: 58,
                    padding: '9px 11px',
                    textAlign: 'left',
                    borderRadius: 'var(--radius-md)',
                    border: `1px solid ${active ? 'var(--accent)' : 'var(--border)'}`,
                    background: active
                      ? 'var(--accent-subtle)'
                      : 'var(--surface-2)',
                    color: 'var(--text)',
                    cursor: 'pointer',
                  }}
                >
                  <span
                    style={{ display: 'block', fontSize: 12, fontWeight: 600 }}
                  >
                    {option.label}
                  </span>
                  <span
                    style={{
                      display: 'block',
                      marginTop: 3,
                      fontSize: 10,
                      lineHeight: 1.35,
                      color: 'var(--text-muted)',
                    }}
                  >
                    {option.detail}
                  </span>
                </button>
              );
            })}
          </div>
          {provider === 'claude' && (
            <p
              style={{
                fontSize: 10,
                color: 'var(--text-dim)',
                lineHeight: 1.45,
              }}
            >
              Anthropic models use the equivalent provider-managed planning
              profile rather than a Codex effort flag.
            </p>
          )}
          {error && (
            <p style={{ fontSize: 12, color: 'var(--error)' }}>{error}</p>
          )}
          {saved && (
            <p style={{ fontSize: 12, color: 'var(--success)' }}>Saved</p>
          )}
          <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
            <Btn variant="primary" disabled={saving} onClick={save}>
              {saving ? 'Saving…' : 'Save planning depth'}
            </Btn>
          </div>
        </div>
      </Card>
    </div>
  );
}

function ClaudeTab({
  claudeAuth,
  claudeAuthLoading,
  setClaudeAuth,
  setClaudeAuthLoading,
  userSettings,
  setUserSettings,
}: {
  claudeAuth: ProviderAuthStatus | null;
  claudeAuthLoading: boolean;
  setClaudeAuth: (v: any) => void;
  setClaudeAuthLoading: (v: boolean) => void;
  userSettings: UserSettings;
  setUserSettings: React.Dispatch<React.SetStateAction<UserSettings>>;
}) {
  const [oauthUrl, setOauthUrl] = useState<string | null>(null);
  const [callbackUrl, setCallbackUrl] = useState('');
  const usingLocalCli = claudeAuth?.source === 'local_cli';
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 24 }}>
      <div>
        <SectionTitle>Account</SectionTitle>
        <Card>
          {claudeAuth?.connected ? (
            <Row>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span
                  style={{
                    width: 8,
                    height: 8,
                    borderRadius: '50%',
                    flexShrink: 0,
                    background: claudeAuth.expired
                      ? 'var(--warning)'
                      : 'var(--success)',
                  }}
                />
                <div>
                  <p style={{ fontSize: 13, color: 'var(--text)' }}>
                    {usingLocalCli
                      ? 'Using local Claude CLI login'
                      : claudeAuth.expired
                        ? 'Connected · token expired'
                        : 'Connected to Claude.ai'}
                  </p>
                  {usingLocalCli && (
                    <p style={{ fontSize: 11, color: 'var(--text-muted)' }}>
                      Flamel Desktop detected your existing Claude CLI
                      credentials and will use them directly.
                    </p>
                  )}
                </div>
              </div>
              {!usingLocalCli && (
                <div style={{ display: 'flex', gap: 8 }}>
                  {claudeAuth.expired && (
                    <Btn
                      variant="primary"
                      disabled={claudeAuthLoading}
                      onClick={async () => {
                        setClaudeAuthLoading(true);
                        try {
                          const res = await apiFetch(
                            '/api/me/claude-auth/refresh',
                            { method: 'POST', credentials: 'include' },
                          );
                          const data = await res.json();
                          if (data.ok) {
                            const r = await apiFetch('/api/me/claude-auth', {
                              credentials: 'include',
                            });
                            setClaudeAuth(await r.json());
                          } else {
                            alert(data.error ?? 'Refresh failed');
                          }
                        } catch {
                          alert('Refresh failed');
                        }
                        setClaudeAuthLoading(false);
                      }}
                    >
                      {claudeAuthLoading ? 'Refreshing…' : 'Refresh Token'}
                    </Btn>
                  )}
                  <Btn
                    disabled={claudeAuthLoading}
                    onClick={async () => {
                      setClaudeAuthLoading(true);
                      await apiFetch('/api/me/claude-auth', {
                        method: 'DELETE',
                        credentials: 'include',
                      });
                      const r = await apiFetch('/api/me/claude-auth', {
                        credentials: 'include',
                      });
                      setClaudeAuth(await r.json());
                      setClaudeAuthLoading(false);
                    }}
                  >
                    Disconnect
                  </Btn>
                </div>
              )}
            </Row>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              <p style={{ fontSize: 13, color: 'var(--text)' }}>
                Not connected
              </p>
              <p style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                Connect your Claude.ai account so flamel can use it when running
                Claude Code sessions.
              </p>
              {oauthUrl && (
                <div
                  style={{ display: 'flex', flexDirection: 'column', gap: 6 }}
                >
                  <p style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                    Complete authorization in your browser. If it didn't open
                    automatically:
                  </p>
                  <Btn
                    fullWidth
                    onClick={() => {
                      if (window.electronAPI?.openExternal) {
                        window.electronAPI.openExternal(oauthUrl);
                      } else {
                        window.open(oauthUrl, '_blank', 'noopener,noreferrer');
                      }
                    }}
                  >
                    Open Authorization Page
                  </Btn>
                  <p
                    style={{
                      fontSize: 12,
                      color: 'var(--text-muted)',
                      marginTop: 8,
                    }}
                  >
                    After authorizing, your browser will show an error page.
                    Copy the full URL from your address bar and paste it here:
                  </p>
                  <div style={{ display: 'flex', gap: 6 }}>
                    <input
                      type="text"
                      placeholder="http://127.0.0.1:..."
                      value={callbackUrl}
                      onChange={(e) => setCallbackUrl(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter' && callbackUrl.trim())
                          (e.target as HTMLElement)
                            .closest('div')
                            ?.querySelector('button')
                            ?.click();
                      }}
                      style={{
                        flex: 1,
                        padding: '6px 10px',
                        fontSize: 12,
                        background: 'var(--surface-2)',
                        border: '1px solid var(--border)',
                        borderRadius: 6,
                        color: 'var(--text)',
                        outline: 'none',
                      }}
                    />
                    <Btn
                      disabled={!callbackUrl.trim()}
                      onClick={async () => {
                        try {
                          const res = await apiFetch(
                            '/api/me/claude-auth/oauth/complete',
                            {
                              method: 'POST',
                              credentials: 'include',
                              headers: { 'Content-Type': 'application/json' },
                              body: JSON.stringify({ url: callbackUrl.trim() }),
                            },
                          );
                          const data = await res.json();
                          if (data.ok) {
                            const r = await apiFetch('/api/me/claude-auth', {
                              credentials: 'include',
                            });
                            const d = await r.json();
                            if (d.connected) {
                              setClaudeAuth(d);
                              setOauthUrl(null);
                              setCallbackUrl('');
                              setClaudeAuthLoading(false);
                              window.dispatchEvent(
                                new CustomEvent('flamel:claude-connected'),
                              );
                            }
                          } else {
                            alert(data.error || 'Failed to complete auth');
                          }
                        } catch (e: any) {
                          alert('Error: ' + e.message);
                        }
                      }}
                    >
                      Submit
                    </Btn>
                  </div>
                </div>
              )}
              <Btn
                variant="primary"
                fullWidth
                disabled={claudeAuthLoading}
                onClick={async () => {
                  setClaudeAuthLoading(true);
                  setOauthUrl(null);
                  try {
                    const res = await apiFetch(
                      '/api/me/claude-auth/oauth/start',
                      { credentials: 'include' },
                    );
                    const data = await res.json();
                    if (data.error) {
                      alert(data.error);
                      setClaudeAuthLoading(false);
                      return;
                    }
                    if (data.url) {
                      setOauthUrl(data.url);
                      if (window.electronAPI?.openExternal) {
                        window.electronAPI.openExternal(data.url);
                      } else {
                        window.open(data.url, '_blank', 'noopener,noreferrer');
                      }
                      let attempt = 0;
                      const maxAttempts = 10;
                      const startedAt = Date.now();
                      const maxMs = 2 * 60 * 1000;
                      let timeoutId: ReturnType<typeof setTimeout>;
                      function scheduleNext() {
                        if (
                          attempt >= maxAttempts ||
                          Date.now() - startedAt >= maxMs
                        ) {
                          setClaudeAuthLoading(false);
                          return;
                        }
                        const delay = Math.min(
                          2000 * Math.pow(2, attempt),
                          10000,
                        );
                        attempt++;
                        timeoutId = setTimeout(async () => {
                          try {
                            const r = await apiFetch('/api/me/claude-auth', {
                              credentials: 'include',
                            });
                            const d = await r.json();
                            if (d.connected) {
                              setClaudeAuth(d);
                              setOauthUrl(null);
                              window.dispatchEvent(
                                new CustomEvent('flamel:claude-connected'),
                              );
                              setClaudeAuthLoading(false);
                              return;
                            }
                          } catch {
                            /* keep polling */
                          }
                          scheduleNext();
                        }, delay);
                      }
                      scheduleNext();
                    }
                  } catch {
                    setClaudeAuthLoading(false);
                  }
                }}
              >
                {claudeAuthLoading
                  ? 'Waiting for browser…'
                  : 'Connect with Claude.ai'}
              </Btn>
            </div>
          )}
        </Card>
      </div>
      <PlanningDepthSettings
        provider="claude"
        userSettings={userSettings}
        setUserSettings={setUserSettings}
      />
    </div>
  );
}

function GitTab({
  gitSettings,
  setGitSettings,
}: {
  gitSettings: UserSettings;
  setGitSettings: React.Dispatch<React.SetStateAction<UserSettings>>;
}) {
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState('');

  async function handleSave(e: React.FormEvent) {
    e.preventDefault();
    setSaveError('');
    const res = await apiFetch('/api/me/settings', {
      method: 'PUT',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(gitSettings),
    });
    if (res.ok) {
      setSaved(true);
      setTimeout(() => setSaved(false), 2500);
    } else {
      const d = await res.json();
      setSaveError(d.error || 'Failed to save');
    }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 24 }}>
      <div>
        <SectionTitle>Identity</SectionTitle>
        <form onSubmit={handleSave}>
          <Card>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
              <Field
                label="Name"
                hint="Used by Claude when making git commits on your behalf."
              >
                <Input
                  placeholder="Your Name"
                  value={gitSettings.git_name}
                  onChange={(e) =>
                    setGitSettings((s) => ({ ...s, git_name: e.target.value }))
                  }
                />
              </Field>
              <Field label="Email">
                <Input
                  type="email"
                  placeholder="you@example.com"
                  value={gitSettings.git_email}
                  onChange={(e) =>
                    setGitSettings((s) => ({ ...s, git_email: e.target.value }))
                  }
                />
              </Field>
              <Field
                label="GPG signing key"
                hint="Optional. Key ID or fingerprint for signed commits."
              >
                <Input
                  placeholder="Key ID or fingerprint"
                  value={gitSettings.git_signing_key}
                  onChange={(e) =>
                    setGitSettings((s) => ({
                      ...s,
                      git_signing_key: e.target.value,
                    }))
                  }
                />
              </Field>
              {saveError && (
                <p style={{ fontSize: 12, color: 'var(--error)' }}>
                  {saveError}
                </p>
              )}
              {saved && (
                <p style={{ fontSize: 12, color: 'var(--success, #22c55e)' }}>
                  ✓ Saved
                </p>
              )}
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'flex-end',
                  marginTop: 4,
                }}
              >
                <Btn type="submit" variant="primary">
                  Save changes
                </Btn>
              </div>
            </div>
          </Card>
        </form>
      </div>
    </div>
  );
}

function GitAccountsTab({
  gitAccounts,
  setGitAccounts,
}: {
  gitAccounts: GitAccount[];
  setGitAccounts: React.Dispatch<React.SetStateAction<GitAccount[]>>;
}) {
  const [form, setForm] = useState({ type: 'github', baseUrl: '', token: '' });
  const [error, setError] = useState('');
  const [adding, setAdding] = useState(false);
  const [adding2, setAdding2] = useState(false);

  function getTokenUrl() {
    if (form.type === 'github') {
      return 'https://github.com/settings/tokens/new?scopes=repo,read:org&description=flamel';
    }
    const base =
      form.baseUrl.replace(/\/$/, '') || 'https://your-gitea-instance.com';
    return `${base}/user/settings/applications`;
  }

  function openTokenPage() {
    const url = getTokenUrl();
    if (window.electronAPI?.openExternal) {
      window.electronAPI.openExternal(url);
    } else {
      window.open(url, '_blank', 'noopener,noreferrer');
    }
  }

  async function handleAdd(e: React.FormEvent) {
    e.preventDefault();
    setError('');
    if (!form.token.trim()) {
      setError('Token is required');
      return;
    }
    if (form.type === 'gitea' && !form.baseUrl) {
      setError('Base URL required for Gitea');
      return;
    }
    setAdding2(true);
    try {
      const res = await apiFetch('/api/git-accounts', {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          type: form.type,
          baseUrl: form.baseUrl || undefined,
          token: form.token.trim(),
        }),
      });
      const data = await res.json();
      if (res.ok) {
        setGitAccounts((prev) => [data, ...prev]);
        setForm({ type: 'github', baseUrl: '', token: '' });
        setAdding(false);
      } else {
        setError(data.error || 'Failed to add account');
      }
    } finally {
      setAdding2(false);
    }
  }

  async function handleRemove(acct: GitAccount) {
    if (!confirm(`Remove "${acct.label}"?`)) return;
    await apiFetch(`/api/git-accounts/${acct.id}`, {
      method: 'DELETE',
      credentials: 'include',
    });
    setGitAccounts((prev) => prev.filter((a) => a.id !== acct.id));
  }

  const typeBadge = (type: string) =>
    ({
      padding: '2px 7px',
      borderRadius: 99,
      fontSize: 11,
      fontWeight: 600,
      background:
        type === 'github' ? 'var(--accent-glow)' : 'rgba(234,179,8,0.12)',
      color: type === 'github' ? 'var(--accent)' : 'var(--warning)',
      border: `1px solid ${type === 'github' ? 'var(--accent)' : 'var(--warning)'}`,
      flexShrink: 0,
    }) as React.CSSProperties;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 24 }}>
      <div>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 12,
          }}
        >
          <SectionTitle>Connected accounts</SectionTitle>
          {!adding && (
            <Btn onClick={() => setAdding(true)} variant="primary">
              + Add account
            </Btn>
          )}
        </div>

        {gitAccounts.length === 0 && !adding && (
          <Card>
            <p
              style={{
                fontSize: 13,
                color: 'var(--text-muted)',
                textAlign: 'center',
                padding: '12px 0',
              }}
            >
              No accounts connected yet. Add a GitHub or Gitea personal access
              token to enable issue and PR features.
            </p>
          </Card>
        )}

        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {gitAccounts.map((acct) => (
            <Card key={acct.id}>
              <Row>
                <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                  {acct.avatarUrl ? (
                    <img
                      src={acct.avatarUrl}
                      alt={acct.username || acct.label}
                      style={{
                        width: 32,
                        height: 32,
                        borderRadius: '50%',
                        flexShrink: 0,
                        background: 'var(--surface-2)',
                      }}
                    />
                  ) : (
                    <div
                      style={{
                        width: 32,
                        height: 32,
                        borderRadius: '50%',
                        background: 'var(--surface-2)',
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        flexShrink: 0,
                      }}
                    >
                      {acct.type === 'github' ? (
                        <span
                          style={{ fontSize: 14, color: 'var(--text-muted)' }}
                        >
                          ⊞
                        </span>
                      ) : (
                        <Icon.Logo
                          size={14}
                          style={{ color: 'var(--accent)' }}
                        />
                      )}
                    </div>
                  )}
                  <div>
                    <div
                      style={{ display: 'flex', alignItems: 'center', gap: 8 }}
                    >
                      <p
                        style={{
                          fontSize: 13,
                          color: 'var(--text)',
                          fontWeight: 500,
                        }}
                      >
                        {acct.label}
                      </p>
                      <span style={typeBadge(acct.type)}>
                        {acct.type === 'github' ? 'GitHub' : 'Gitea'}
                      </span>
                    </div>
                    {acct.username && (
                      <p
                        style={{
                          fontSize: 12,
                          color: 'var(--text-muted)',
                          marginTop: 2,
                        }}
                      >
                        @{acct.username}
                      </p>
                    )}
                    {acct.baseUrl &&
                      acct.baseUrl !== 'https://api.github.com' && (
                        <p
                          style={{
                            fontSize: 11,
                            color: 'var(--text-muted)',
                            marginTop: 2,
                            opacity: 0.7,
                          }}
                        >
                          {acct.baseUrl}
                        </p>
                      )}
                  </div>
                </div>
                <Btn variant="danger" onClick={() => handleRemove(acct)}>
                  Remove
                </Btn>
              </Row>
            </Card>
          ))}
        </div>

        {adding && (
          <div style={{ marginTop: gitAccounts.length > 0 ? 16 : 0 }}>
            <SectionTitle>New account</SectionTitle>
            <Card>
              <form
                onSubmit={handleAdd}
                style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
              >
                {/* Provider pill toggle */}
                <Field label="Provider">
                  <div
                    style={{
                      display: 'flex',
                      gap: 0,
                      background: 'var(--bg)',
                      border: '1px solid var(--border)',
                      borderRadius: 8,
                      padding: 3,
                      width: 'fit-content',
                    }}
                  >
                    {(['github', 'gitea'] as const).map((p) => (
                      <button
                        key={p}
                        type="button"
                        onClick={() =>
                          setForm((s) => ({ ...s, type: p, baseUrl: '' }))
                        }
                        style={{
                          padding: '5px 16px',
                          borderRadius: 6,
                          fontSize: 12,
                          fontWeight: 500,
                          background:
                            form.type === p
                              ? 'var(--surface-2)'
                              : 'transparent',
                          color:
                            form.type === p
                              ? 'var(--text)'
                              : 'var(--text-muted)',
                          border:
                            form.type === p
                              ? '1px solid var(--border)'
                              : '1px solid transparent',
                          cursor: 'pointer',
                          transition: 'all 0.12s',
                        }}
                      >
                        {p === 'github' ? 'GitHub' : 'Gitea'}
                      </button>
                    ))}
                  </div>
                </Field>

                {form.type === 'gitea' && (
                  <Field label="Gitea instance URL">
                    <Input
                      placeholder="https://gitea.example.com"
                      value={form.baseUrl}
                      onChange={(e) =>
                        setForm((s) => ({ ...s, baseUrl: e.target.value }))
                      }
                    />
                  </Field>
                )}

                <Field label="Personal access token">
                  <div style={{ display: 'flex', gap: 8 }}>
                    <Input
                      type="password"
                      placeholder={
                        form.type === 'github' ? 'ghp_…' : 'gitea token'
                      }
                      value={form.token}
                      onChange={(e) =>
                        setForm((s) => ({ ...s, token: e.target.value }))
                      }
                      style={{ flex: 1 }}
                    />
                    <button
                      type="button"
                      onClick={openTokenPage}
                      style={{
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: 4,
                        padding: '0 12px',
                        borderRadius: 6,
                        fontSize: 12,
                        fontWeight: 500,
                        background: 'var(--surface-2)',
                        color: 'var(--accent)',
                        border: '1px solid var(--border)',
                        cursor: 'pointer',
                        whiteSpace: 'nowrap',
                        flexShrink: 0,
                        height: 32,
                        transition: 'border-color 0.12s',
                      }}
                    >
                      Generate token ↗
                    </button>
                  </div>
                  <p
                    style={{
                      fontSize: 11,
                      color: 'var(--text-muted)',
                      marginTop: 4,
                    }}
                  >
                    {form.type === 'github'
                      ? 'Needs repo and read:org scopes'
                      : 'Needs repository and issue scopes'}
                  </p>
                </Field>

                {error && (
                  <p style={{ fontSize: 12, color: 'var(--error)' }}>{error}</p>
                )}
                <div
                  style={{
                    display: 'flex',
                    gap: 8,
                    justifyContent: 'flex-end',
                  }}
                >
                  <Btn
                    onClick={() => {
                      setAdding(false);
                      setError('');
                    }}
                  >
                    Cancel
                  </Btn>
                  <Btn
                    type="submit"
                    variant="primary"
                    disabled={
                      !form.token.trim() ||
                      (form.type === 'gitea' && !form.baseUrl) ||
                      adding2
                    }
                  >
                    {adding2 ? 'Verifying…' : 'Add account'}
                  </Btn>
                </div>
              </form>
            </Card>
          </div>
        )}
      </div>
    </div>
  );
}

type DeviceState = {
  deviceCode: string;
  userCode: string;
  verificationUrl: string;
  interval: number;
  expiresAt: number;
};

function CodexTab({
  codexAuth,
  codexAuthLoading,
  setCodexAuth,
  setCodexAuthLoading,
  compactionSettings,
  setCompactionSettings,
  userSettings,
  setUserSettings,
}: {
  codexAuth: ProviderAuthStatus | null;
  codexAuthLoading: boolean;
  setCodexAuth: (v: any) => void;
  setCodexAuthLoading: (v: boolean) => void;
  compactionSettings: CompactionSettings;
  setCompactionSettings: React.Dispatch<
    React.SetStateAction<CompactionSettings>
  >;
  userSettings: UserSettings;
  setUserSettings: React.Dispatch<React.SetStateAction<UserSettings>>;
}) {
  const [device, setDevice] = useState<DeviceState | null>(null);
  const [compactionSaved, setCompactionSaved] = useState(false);
  const [compactionError, setCompactionError] = useState('');
  const [compactionSaving, setCompactionSaving] = useState(false);
  const pollRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const usingLocalCli = codexAuth?.source === 'local_cli';

  async function saveCompactionSettings() {
    setCompactionSaving(true);
    setCompactionError('');
    try {
      const res = await apiFetch('/api/me/settings/compaction', {
        method: 'PUT',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(compactionSettings),
      });
      const data = await res.json();
      if (!res.ok) {
        setCompactionError(data.error || 'Failed to save');
        return;
      }
      setCompactionSettings({
        codex_compaction_model: data.codex_compaction_model || 'gpt-5.6-luna',
        codex_compaction_reasoning_effort:
          data.codex_compaction_reasoning_effort || 'medium',
        codex_compaction_fallback_enabled:
          data.codex_compaction_fallback_enabled !== false,
      });
      setCompactionSaved(true);
      setTimeout(() => setCompactionSaved(false), 2500);
    } catch {
      setCompactionError('Failed to save');
    } finally {
      setCompactionSaving(false);
    }
  }

  const stopPolling = () => {
    if (pollRef.current) {
      clearTimeout(pollRef.current);
      pollRef.current = null;
    }
  };

  const startDeviceFlow = async () => {
    stopPolling();
    setCodexAuthLoading(true);
    setDevice(null);
    try {
      const res = await apiFetch('/api/me/codex-auth/device/start', {
        method: 'POST',
        credentials: 'include',
      });
      const data = await res.json();
      if (data.error) {
        alert(data.error);
        setCodexAuthLoading(false);
        return;
      }

      const ds: DeviceState = {
        deviceCode: data.device_code,
        userCode: data.user_code,
        verificationUrl:
          data.verification_url ||
          data.verification_uri_complete ||
          data.verification_uri,
        interval: (data.interval ?? 5) * 1000,
        expiresAt: Date.now() + (data.expires_in ?? 900) * 1000,
      };
      setDevice(ds);

      const poll = async (d: DeviceState) => {
        if (Date.now() > d.expiresAt) {
          setDevice(null);
          setCodexAuthLoading(false);
          alert('Device code expired — please try again.');
          return;
        }
        try {
          const r = await apiFetch('/api/me/codex-auth/device/poll', {
            method: 'POST',
            credentials: 'include',
          });
          const p = await r.json();
          if (p.ok) {
            const s = await (
              await apiFetch('/api/me/codex-auth', { credentials: 'include' })
            ).json();
            setCodexAuth(s);
            setDevice(null);
            setCodexAuthLoading(false);
            window.dispatchEvent(new CustomEvent('flamel:codex-connected'));
            return;
          }
          if (p.error) {
            alert(p.error);
            setDevice(null);
            setCodexAuthLoading(false);
            return;
          }
          const interval = p.slow_down ? d.interval + 5000 : d.interval;
          pollRef.current = setTimeout(
            () => poll({ ...d, interval }),
            interval,
          );
        } catch {
          pollRef.current = setTimeout(() => poll(d), d.interval);
        }
      };

      pollRef.current = setTimeout(() => poll(ds), ds.interval);
    } catch {
      setCodexAuthLoading(false);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 24 }}>
      <div>
        <SectionTitle>Account</SectionTitle>
        <Card>
          {codexAuth?.connected ? (
            <Row>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span
                  style={{
                    width: 8,
                    height: 8,
                    borderRadius: '50%',
                    flexShrink: 0,
                    background: codexAuth.expired
                      ? 'var(--warning)'
                      : 'var(--success)',
                  }}
                />
                <div>
                  <p style={{ fontSize: 13, color: 'var(--text)' }}>
                    {usingLocalCli
                      ? 'Using local Codex CLI login'
                      : codexAuth.expired
                        ? 'Connected · token expired'
                        : 'Connected to ChatGPT'}
                  </p>
                  {usingLocalCli && (
                    <p style={{ fontSize: 11, color: 'var(--text-muted)' }}>
                      Flamel Desktop detected your existing Codex CLI
                      credentials and will use them directly.
                    </p>
                  )}
                </div>
              </div>
              {!usingLocalCli && (
                <div style={{ display: 'flex', gap: 8 }}>
                  {codexAuth.expired && (
                    <Btn
                      variant="primary"
                      disabled={codexAuthLoading}
                      onClick={async () => {
                        setCodexAuthLoading(true);
                        try {
                          const res = await apiFetch(
                            '/api/me/codex-auth/refresh',
                            { method: 'POST', credentials: 'include' },
                          );
                          const data = await res.json();
                          if (data.ok) {
                            const r = await apiFetch('/api/me/codex-auth', {
                              credentials: 'include',
                            });
                            setCodexAuth(await r.json());
                          } else {
                            alert(data.error ?? 'Refresh failed');
                          }
                        } catch {
                          alert('Refresh failed');
                        }
                        setCodexAuthLoading(false);
                      }}
                    >
                      {codexAuthLoading ? 'Refreshing…' : 'Refresh Token'}
                    </Btn>
                  )}
                  <Btn
                    disabled={codexAuthLoading}
                    onClick={async () => {
                      setCodexAuthLoading(true);
                      await apiFetch('/api/me/codex-auth', {
                        method: 'DELETE',
                        credentials: 'include',
                      });
                      const r = await apiFetch('/api/me/codex-auth', {
                        credentials: 'include',
                      });
                      setCodexAuth(await r.json());
                      setCodexAuthLoading(false);
                    }}
                  >
                    Disconnect
                  </Btn>
                </div>
              )}
            </Row>
          ) : device ? (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              <p style={{ fontSize: 13, color: 'var(--text)' }}>
                Open the link below and enter the code to sign in:
              </p>
              <div
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'center',
                  gap: 10,
                  padding: '16px',
                  background: 'var(--surface-2)',
                  borderRadius: 8,
                  border: '1px solid var(--border)',
                }}
              >
                <p
                  style={{
                    fontSize: 11,
                    color: 'var(--text-muted)',
                    textTransform: 'uppercase',
                    letterSpacing: '0.08em',
                  }}
                >
                  Your code
                </p>
                <p
                  style={{
                    fontSize: 28,
                    fontWeight: 700,
                    letterSpacing: '0.15em',
                    color: 'var(--text)',
                    fontFamily: 'monospace',
                  }}
                >
                  {device.userCode}
                </p>
                <a
                  href={device.verificationUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  style={{
                    fontSize: 13,
                    color: 'var(--accent)',
                    wordBreak: 'break-all',
                    textAlign: 'center',
                  }}
                >
                  {device.verificationUrl}
                </a>
              </div>
              <p
                style={{
                  fontSize: 12,
                  color: 'var(--text-muted)',
                  textAlign: 'center',
                }}
              >
                Waiting for you to complete sign-in…
              </p>
              <Btn
                fullWidth
                onClick={() => {
                  stopPolling();
                  setDevice(null);
                  setCodexAuthLoading(false);
                }}
              >
                Cancel
              </Btn>
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              <p style={{ fontSize: 13, color: 'var(--text)' }}>
                Not connected
              </p>
              <p style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                Connect your ChatGPT account so flamel can use Codex models.
                Requires device code login to be enabled in your{' '}
                <a
                  href="https://chatgpt.com/settings/security"
                  target="_blank"
                  rel="noopener noreferrer"
                  style={{ color: 'var(--accent)' }}
                >
                  ChatGPT security settings
                </a>
                .
              </p>
              <Btn
                variant="primary"
                fullWidth
                disabled={codexAuthLoading}
                onClick={startDeviceFlow}
              >
                {codexAuthLoading ? 'Starting…' : 'Connect with ChatGPT'}
              </Btn>
            </div>
          )}
        </Card>
      </div>

      <PlanningDepthSettings
        provider="codex"
        userSettings={userSettings}
        setUserSettings={setUserSettings}
      />

      <div>
        <SectionTitle>Compaction</SectionTitle>
        <Card>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
            <Field label="Model" hint="Used only for compacting older context.">
              <Select
                value={compactionSettings.codex_compaction_model}
                onChange={(e) =>
                  setCompactionSettings((s) => ({
                    ...s,
                    codex_compaction_model: e.target.value,
                    codex_compaction_reasoning_effort: normalizeReasoningEffort(
                      e.target.value,
                      s.codex_compaction_reasoning_effort,
                    ),
                  }))
                }
              >
                {CODEX_MODELS.map((model) => (
                  <option key={model.id} value={model.id}>
                    {model.label}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="Reasoning effort">
              <Select
                value={normalizeReasoningEffort(
                  compactionSettings.codex_compaction_model,
                  compactionSettings.codex_compaction_reasoning_effort,
                )}
                onChange={(e) =>
                  setCompactionSettings((s) => ({
                    ...s,
                    codex_compaction_reasoning_effort: e.target.value,
                  }))
                }
              >
                {reasoningEffortsForModel(
                  compactionSettings.codex_compaction_model,
                ).map((effort) => (
                  <option key={effort.id} value={effort.id}>
                    {effort.label}
                  </option>
                ))}
              </Select>
            </Field>
            <Row>
              <div>
                <p
                  style={{
                    fontSize: 13,
                    color: 'var(--text)',
                    marginBottom: 2,
                  }}
                >
                  Fallback to chat model
                </p>
                <p style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                  Retry with the active Codex model if the compactor fails.
                </p>
              </div>
              <Toggle
                on={compactionSettings.codex_compaction_fallback_enabled}
                onToggle={() =>
                  setCompactionSettings((s) => ({
                    ...s,
                    codex_compaction_fallback_enabled:
                      !s.codex_compaction_fallback_enabled,
                  }))
                }
              />
            </Row>
            {compactionError && (
              <p style={{ fontSize: 12, color: 'var(--error)' }}>
                {compactionError}
              </p>
            )}
            {compactionSaved && (
              <p style={{ fontSize: 12, color: 'var(--success, #22c55e)' }}>
                Saved
              </p>
            )}
            <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
              <Btn
                variant="primary"
                disabled={compactionSaving}
                onClick={saveCompactionSettings}
              >
                {compactionSaving ? 'Saving…' : 'Save compaction'}
              </Btn>
            </div>
          </div>
        </Card>
      </div>
    </div>
  );
}

type CapabilityHubView = 'connected' | 'marketplace' | 'skills' | 'activity';

function IntegrationsTab({
  isAdmin,
  initialView,
}: {
  isAdmin: boolean;
  initialView?: CapabilityHubView;
}) {
  const [entries, setEntries] = useState<CapabilityCatalogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [showAdd, setShowAdd] = useState(false);
  const [error, setError] = useState('');
  const [view, setView] = useState<CapabilityHubView>(
    initialView || 'connected',
  );
  const [marketplace, setMarketplace] = useState<MarketplaceCapability[]>([]);
  const [skills, setSkills] = useState<GovernedSkill[]>([]);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [tools, setTools] = useState<Record<string, CapabilityTool[]>>({});
  const [metrics, setMetrics] = useState({
    calls: 0,
    failures: 0,
    successRate: 1,
    averageLatencyMs: 0,
  });
  const [activity, setActivity] = useState<
    Array<{
      id: string;
      capabilityId?: string;
      eventType: string;
      decision: string;
      toolName?: string;
      summary: string;
      createdAt: string;
    }>
  >([]);
  const [credentialFor, setCredentialFor] = useState<string | null>(null);
  const [credential, setCredential] = useState('');
  const [draft, setDraft] = useState({
    slug: '',
    title: '',
    description: '',
    transport: 'streamable_http',
    command: '',
    remoteUrl: '',
    args: '',
    authKind: 'none',
  });

  const load = async () => {
    setLoading(true);
    try {
      const [response, market, governedSkills, usage, audit] =
        await Promise.all([
          api.capabilities.catalog(),
          api.capabilities.marketplace(),
          api.capabilities.skills(),
          api.capabilities.metrics(),
          api.capabilities.audit(),
        ]);
      setEntries(response.capabilities);
      setMarketplace(market.entries);
      setSkills(governedSkills.skills);
      setMetrics(usage);
      setActivity(audit.events);
      setError('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not load integrations');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  async function act(
    entry: CapabilityCatalogEntry,
    action: 'connect' | 'disconnect' | 'test',
  ) {
    setBusy(`${entry.id}:${action}`);
    setError('');
    try {
      if (action === 'connect') await api.capabilities.connect(entry.id);
      else if (action === 'disconnect')
        await api.capabilities.disconnect(entry.id);
      else await api.capabilities.test(entry.id);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Integration action failed');
      await load();
    } finally {
      setBusy(null);
    }
  }

  async function addIntegration() {
    setBusy('add');
    setError('');
    try {
      await api.capabilities.create({
        slug: draft.slug.trim(),
        title: draft.title.trim() || draft.slug.trim(),
        description: draft.description.trim(),
        transport: draft.transport as CapabilityCatalogEntry['transport'],
        command: draft.transport === 'stdio' ? draft.command.trim() : null,
        remoteUrl: draft.transport === 'stdio' ? null : draft.remoteUrl.trim(),
        args: draft.args.split(/\s+/).filter(Boolean),
        authKind: draft.authKind,
        category: 'developer',
      });
      setDraft({
        slug: '',
        title: '',
        description: '',
        transport: 'streamable_http',
        command: '',
        remoteUrl: '',
        args: '',
        authKind: 'none',
      });
      setShowAdd(false);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not add integration');
    } finally {
      setBusy(null);
    }
  }

  async function expandTools(entry: CapabilityCatalogEntry) {
    if (expanded === entry.id) {
      setExpanded(null);
      return;
    }
    setExpanded(entry.id);
    if (!tools[entry.id]) {
      try {
        const response = await api.capabilities.tools(entry.id);
        setTools((current) => ({ ...current, [entry.id]: response.tools }));
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Could not load tools');
      }
    }
  }

  async function toggleTool(
    entry: CapabilityCatalogEntry,
    tool: CapabilityTool,
  ) {
    setBusy(`${entry.id}:${tool.name}`);
    try {
      await api.capabilities.setTool(entry.id, tool.name, !tool.enabled);
      setTools((current) => ({
        ...current,
        [entry.id]: (current[entry.id] || []).map((item) =>
          item.name === tool.name ? { ...item, enabled: !item.enabled } : item,
        ),
      }));
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not update tool');
    } finally {
      setBusy(null);
    }
  }

  async function install(entry: MarketplaceCapability) {
    setBusy(`market:${entry.id}`);
    setError('');
    try {
      await api.capabilities.install(entry.id);
      setView('connected');
      await load();
    } catch (e) {
      setError(
        e instanceof Error ? e.message : 'Could not approve integration',
      );
    } finally {
      setBusy(null);
    }
  }

  async function managePack(
    entry: CapabilityCatalogEntry,
    action: 'rollback' | 'uninstall',
  ) {
    if (!entry.packId) return;
    if (
      action === 'uninstall' &&
      !window.confirm(
        `Uninstall ${entry.title}? This revokes its credentials and disables its schedules.`,
      )
    )
      return;
    setBusy(`${entry.id}:pack-${action}`);
    setError('');
    try {
      if (action === 'rollback')
        await api.capabilities.rollbackPack(entry.packId);
      else await api.capabilities.uninstallPack(entry.packId);
      setExpanded(null);
      await load();
    } catch (e) {
      setError(
        e instanceof Error ? e.message : `Could not ${action} capability pack`,
      );
    } finally {
      setBusy(null);
    }
  }

  async function manageSkill(
    entry: GovernedSkill,
    action: 'install' | 'toggle' | 'rollback' | 'uninstall',
  ) {
    if (
      action === 'uninstall' &&
      !window.confirm(
        `Uninstall ${entry.title}? Provider projections will be removed.`,
      )
    )
      return;
    setBusy(`skill:${entry.id}:${action}`);
    setError('');
    try {
      if (action === 'install') {
        await api.capabilities.installSkill(entry.id, {
          providerProjectionEnabled: true,
        });
      } else if (action === 'toggle') {
        await api.capabilities.setSkillState(entry.id, {
          enabled: !entry.active,
          providerProjectionEnabled: entry.providerProjectionEnabled,
        });
      } else if (action === 'rollback') {
        await api.capabilities.rollbackSkill(entry.id);
      } else {
        await api.capabilities.uninstallSkill(entry.id);
      }
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : `Could not ${action} skill`);
      await load();
    } finally {
      setBusy(null);
    }
  }

  function configure(entry: MarketplaceCapability) {
    setDraft({
      slug: entry.slug,
      title: entry.title,
      description: entry.description,
      transport: entry.remoteUrl ? entry.transport : 'stdio',
      command: '',
      remoteUrl: entry.remoteUrl || '',
      args: '',
      authKind: entry.authKind,
    });
    setQuery('');
    setView('connected');
    setShowAdd(true);
    setError('');
  }

  async function saveCredential(entry: CapabilityCatalogEntry) {
    if (!credential.trim()) return;
    setBusy(`${entry.id}:credential`);
    setError('');
    try {
      await api.capabilities.setCredential(
        entry.id,
        credential.trim(),
        'token',
      );
      setCredential('');
      setCredentialFor(null);
      if (!entry.connected) await api.capabilities.connect(entry.id);
      await api.capabilities.test(entry.id);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not save credential');
    } finally {
      setBusy(null);
    }
  }

  const filtered = entries.filter((entry) =>
    `${entry.title} ${entry.description} ${entry.category}`
      .toLowerCase()
      .includes(query.toLowerCase()),
  );
  const filteredSkills = skills.filter((skill) =>
    `${skill.title} ${skill.description} ${skill.category} ${skill.triggers.join(' ')}`
      .toLowerCase()
      .includes(query.toLowerCase()),
  );

  const statusColor = (status: CapabilityCatalogEntry['status']) => {
    if (status === 'ready') return 'var(--success)';
    if (status === 'failed') return 'var(--error)';
    if (status === 'needs_auth') return 'var(--warning)';
    return 'var(--text-dim)';
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Find integrations"
        />
        {isAdmin && (
          <Btn onClick={() => setShowAdd((value) => !value)}>
            <Icon.Plus size={12} /> Add
          </Btn>
        )}
      </div>

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 12,
        }}
      >
        <div
          style={{
            display: 'inline-flex',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius-md)',
            overflow: 'hidden',
          }}
        >
          {(['connected', 'marketplace', 'skills', 'activity'] as const).map(
            (option) => (
              <button
                key={option}
                onClick={() => setView(option)}
                style={{
                  padding: '6px 10px',
                  fontSize: 11,
                  background:
                    view === option ? 'var(--surface-3)' : 'transparent',
                  color: view === option ? 'var(--text)' : 'var(--text-muted)',
                  borderRight:
                    option !== 'activity' ? '1px solid var(--border)' : 'none',
                }}
              >
                {option === 'connected'
                  ? 'Connections'
                  : option === 'marketplace'
                    ? 'Packs'
                    : option === 'skills'
                      ? 'Skills'
                      : 'Activity'}
              </button>
            ),
          )}
        </div>
        <span style={{ fontSize: 10, color: 'var(--text-dim)' }}>
          {metrics.calls} calls · {Math.round(metrics.successRate * 100)}%
          reliable · {Math.round(metrics.averageLatencyMs)} ms avg
        </span>
      </div>

      {showAdd && (
        <Card>
          <SectionTitle>Add approved integration</SectionTitle>
          <div
            style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}
          >
            <Field label="Name">
              <Input
                value={draft.title}
                onChange={(e) =>
                  setDraft((d) => ({
                    ...d,
                    title: e.target.value,
                    slug:
                      d.slug ||
                      e.target.value.toLowerCase().replace(/[^a-z0-9]+/g, '-'),
                  }))
                }
                placeholder="Sentry"
              />
            </Field>
            <Field label="ID">
              <Input
                value={draft.slug}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, slug: e.target.value }))
                }
                placeholder="sentry"
              />
            </Field>
            <Field label="Connection">
              <Select
                value={draft.transport}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, transport: e.target.value }))
                }
              >
                <option value="streamable_http">Remote MCP</option>
                <option value="stdio">Local command</option>
                <option value="sse">Legacy remote SSE</option>
              </Select>
            </Field>
            <Field label="Authentication">
              <Select
                value={draft.authKind}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, authKind: e.target.value }))
                }
              >
                <option value="none">None</option>
                <option value="oauth2">Browser sign-in</option>
                <option value="token">Access token</option>
              </Select>
            </Field>
          </div>
          <div style={{ marginTop: 12 }}>
            {draft.transport === 'stdio' ? (
              <div
                style={{
                  display: 'grid',
                  gridTemplateColumns: '1fr 1fr',
                  gap: 12,
                }}
              >
                <Field label="Command">
                  <Input
                    value={draft.command}
                    onChange={(e) =>
                      setDraft((d) => ({ ...d, command: e.target.value }))
                    }
                    placeholder="npx"
                  />
                </Field>
                <Field label="Arguments">
                  <Input
                    value={draft.args}
                    onChange={(e) =>
                      setDraft((d) => ({ ...d, args: e.target.value }))
                    }
                    placeholder="-y @vendor/mcp"
                  />
                </Field>
              </div>
            ) : (
              <Field label="MCP URL">
                <Input
                  value={draft.remoteUrl}
                  onChange={(e) =>
                    setDraft((d) => ({ ...d, remoteUrl: e.target.value }))
                  }
                  placeholder="https://mcp.example.com/mcp"
                />
              </Field>
            )}
          </div>
          <div style={{ marginTop: 12 }}>
            <Field label="Description">
              <Input
                value={draft.description}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, description: e.target.value }))
                }
                placeholder="What this integration lets Flamel do"
              />
            </Field>
          </div>
          <div
            style={{
              display: 'flex',
              justifyContent: 'flex-end',
              gap: 8,
              marginTop: 14,
            }}
          >
            <Btn variant="ghost" onClick={() => setShowAdd(false)}>
              Cancel
            </Btn>
            <Btn
              variant="primary"
              disabled={busy === 'add' || !draft.slug.trim()}
              onClick={addIntegration}
            >
              {busy === 'add' ? 'Adding…' : 'Add integration'}
            </Btn>
          </div>
        </Card>
      )}

      {error && <p style={{ color: 'var(--error)', fontSize: 12 }}>{error}</p>}

      {view === 'connected' ? (
        <div
          style={{
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius-lg)',
            overflow: 'hidden',
          }}
        >
          {loading && entries.length === 0 ? (
            <p style={{ padding: 18, color: 'var(--text-dim)', fontSize: 12 }}>
              Loading integrations…
            </p>
          ) : (
            filtered.map((entry, index) => (
              <React.Fragment key={entry.id}>
                <div
                  style={{
                    minHeight: 66,
                    padding: '11px 14px',
                    display: 'flex',
                    alignItems: 'center',
                    gap: 12,
                    borderBottom:
                      index < filtered.length - 1
                        ? '1px solid var(--border-soft)'
                        : 'none',
                  }}
                >
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      borderRadius: '50%',
                      background: statusColor(
                        entry.builtin ? 'ready' : entry.status,
                      ),
                      flexShrink: 0,
                    }}
                  />
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div
                      style={{ display: 'flex', gap: 7, alignItems: 'center' }}
                    >
                      <strong style={{ fontSize: 13, color: 'var(--text)' }}>
                        {entry.title}
                      </strong>
                      <span
                        style={{
                          fontSize: 9,
                          color: 'var(--text-dim)',
                          border: '1px solid var(--border)',
                          borderRadius: 3,
                          padding: '1px 4px',
                        }}
                      >
                        {entry.builtin
                          ? 'built-in'
                          : entry.packSigned
                            ? 'signed pack'
                            : entry.transport === 'stdio'
                              ? 'local'
                              : 'remote'}
                      </span>
                      {entry.packVersion && (
                        <span style={{ fontSize: 9, color: 'var(--success)' }}>
                          v{entry.packVersion}
                        </span>
                      )}
                    </div>
                    <p
                      style={{
                        marginTop: 3,
                        fontSize: 11,
                        color: 'var(--text-muted)',
                        whiteSpace: 'nowrap',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                      }}
                    >
                      {entry.description || entry.slug}
                    </p>
                    {entry.packSigned && (
                      <p
                        style={{
                          marginTop: 3,
                          fontSize: 10,
                          color: 'var(--text-dim)',
                        }}
                      >
                        {entry.permissions.length
                          ? `Exact scopes: ${entry.permissions.join(' · ')}`
                          : 'No account scopes'}
                        {entry.scheduleEligible
                          ? ' · schedule eligible'
                          : ' · interactive only'}
                      </p>
                    )}
                    {(entry.connected || entry.builtin) && (
                      <p
                        style={{
                          marginTop: 3,
                          fontSize: 10,
                          color: 'var(--text-dim)',
                        }}
                      >
                        {entry.toolCount || 0} tools
                        {entry.latencyMs != null
                          ? ` · ${entry.latencyMs} ms`
                          : ''}
                        {entry.lastSuccessAt
                          ? ` · last ready ${new Date(entry.lastSuccessAt).toLocaleString()}`
                          : ''}
                        {entry.restartCount
                          ? ` · recovered ${entry.restartCount}×`
                          : ''}
                      </p>
                    )}
                    {entry.lastError && (
                      <p
                        style={{
                          marginTop: 3,
                          fontSize: 10,
                          color: 'var(--error)',
                        }}
                      >
                        {entry.lastError}
                      </p>
                    )}
                  </div>
                  <div style={{ display: 'flex', gap: 6 }}>
                    {entry.toolCount > 0 && (
                      <Btn
                        disabled={busy !== null}
                        onClick={() => void expandTools(entry)}
                      >
                        {expanded === entry.id ? 'Hide tools' : 'Tools'}
                      </Btn>
                    )}
                    {isAdmin && entry.packId && entry.packPreviousVersion && (
                      <Btn
                        disabled={busy !== null}
                        onClick={() => void managePack(entry, 'rollback')}
                      >
                        {busy === `${entry.id}:pack-rollback`
                          ? 'Rolling back…'
                          : `Rollback to ${entry.packPreviousVersion}`}
                      </Btn>
                    )}
                    {isAdmin && entry.packId && (
                      <Btn
                        variant="ghost"
                        disabled={busy !== null}
                        onClick={() => void managePack(entry, 'uninstall')}
                      >
                        {busy === `${entry.id}:pack-uninstall`
                          ? 'Uninstalling…'
                          : 'Uninstall pack'}
                      </Btn>
                    )}
                    {!entry.builtin && (
                      <>
                        {entry.authKind !== 'none' && (
                          <Btn
                            disabled={busy !== null}
                            onClick={() => {
                              setCredentialFor(entry.id);
                              setCredential('');
                            }}
                          >
                            Credentials
                          </Btn>
                        )}
                        {entry.connected && (
                          <Btn
                            disabled={busy !== null}
                            onClick={() => act(entry, 'test')}
                          >
                            {busy === `${entry.id}:test` ? 'Testing…' : 'Test'}
                          </Btn>
                        )}
                        <Btn
                          variant={entry.connected ? 'ghost' : 'primary'}
                          disabled={busy !== null}
                          onClick={() =>
                            act(
                              entry,
                              entry.connected ? 'disconnect' : 'connect',
                            )
                          }
                        >
                          {busy?.startsWith(entry.id)
                            ? 'Working…'
                            : entry.connected
                              ? 'Disconnect'
                              : 'Connect'}
                        </Btn>
                      </>
                    )}
                  </div>
                </div>
                {credentialFor === entry.id && (
                  <div
                    style={{
                      padding: '10px 14px 12px 34px',
                      display: 'flex',
                      gap: 8,
                      borderBottom: '1px solid var(--border-soft)',
                      background: 'var(--surface-2)',
                    }}
                  >
                    <Input
                      type="password"
                      autoComplete="off"
                      value={credential}
                      onChange={(event) => setCredential(event.target.value)}
                      placeholder="Access token"
                    />
                    <Btn
                      variant="primary"
                      disabled={!credential.trim() || busy !== null}
                      onClick={() => void saveCredential(entry)}
                    >
                      Save and test
                    </Btn>
                    <Btn
                      variant="ghost"
                      onClick={() => {
                        setCredentialFor(null);
                        setCredential('');
                      }}
                    >
                      Cancel
                    </Btn>
                  </div>
                )}
                {expanded === entry.id && (
                  <div
                    style={{
                      padding: '8px 14px 12px 34px',
                      borderBottom:
                        index < filtered.length - 1
                          ? '1px solid var(--border-soft)'
                          : 'none',
                      background: 'var(--surface-2)',
                    }}
                  >
                    {(tools[entry.id] || []).map((tool) => (
                      <div
                        key={tool.name}
                        style={{
                          minHeight: 34,
                          display: 'flex',
                          alignItems: 'center',
                          gap: 10,
                          borderBottom: '1px solid var(--border-soft)',
                        }}
                      >
                        <div style={{ minWidth: 0, flex: 1 }}>
                          <strong style={{ fontSize: 11 }}>{tool.name}</strong>
                          <p
                            style={{
                              fontSize: 10,
                              color: 'var(--text-muted)',
                              overflow: 'hidden',
                              textOverflow: 'ellipsis',
                              whiteSpace: 'nowrap',
                            }}
                          >
                            {tool.description || 'No description provided'}
                          </p>
                        </div>
                        <Toggle
                          on={tool.enabled}
                          onToggle={() => void toggleTool(entry, tool)}
                        />
                      </div>
                    ))}
                  </div>
                )}
              </React.Fragment>
            ))
          )}
        </div>
      ) : view === 'marketplace' ? (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
            gap: 10,
          }}
        >
          {marketplace
            .filter((entry) =>
              `${entry.title} ${entry.description} ${entry.category}`
                .toLowerCase()
                .includes(query.toLowerCase()),
            )
            .map((entry) => {
              const installed = entries.some(
                (item) => item.slug === entry.slug,
              );
              return (
                <Card key={entry.id}>
                  <div
                    style={{
                      display: 'flex',
                      gap: 10,
                      alignItems: 'flex-start',
                    }}
                  >
                    <span
                      style={{
                        width: 8,
                        height: 8,
                        marginTop: 5,
                        borderRadius: '50%',
                        background: entry.official
                          ? 'var(--success)'
                          : 'var(--warning)',
                        flexShrink: 0,
                      }}
                    />
                    <div style={{ minWidth: 0, flex: 1 }}>
                      <div
                        style={{
                          display: 'flex',
                          alignItems: 'center',
                          gap: 6,
                        }}
                      >
                        <strong style={{ fontSize: 13 }}>{entry.title}</strong>
                        {entry.official && (
                          <span
                            style={{ fontSize: 9, color: 'var(--success)' }}
                          >
                            {entry.metadata?.signed
                              ? 'signed by Flamel'
                              : 'official'}
                          </span>
                        )}
                        {entry.metadata?.packVersion && (
                          <span
                            style={{ fontSize: 9, color: 'var(--text-dim)' }}
                          >
                            v{entry.metadata.packVersion}
                          </span>
                        )}
                      </div>
                      <p
                        style={{
                          marginTop: 5,
                          minHeight: 34,
                          fontSize: 11,
                          lineHeight: 1.45,
                          color: 'var(--text-muted)',
                        }}
                      >
                        {entry.description}
                      </p>
                      <p
                        style={{
                          marginTop: 7,
                          fontSize: 10,
                          color: 'var(--text-dim)',
                        }}
                      >
                        {entry.permissions.length
                          ? entry.permissions.join(' · ')
                          : 'Permissions configured during connection'}
                      </p>
                      <div
                        style={{
                          display: 'flex',
                          justifyContent: 'flex-end',
                          marginTop: 10,
                        }}
                      >
                        <Btn
                          variant={installed ? 'ghost' : 'primary'}
                          disabled={installed || !isAdmin || busy !== null}
                          onClick={() =>
                            entry.remoteUrl
                              ? void install(entry)
                              : configure(entry)
                          }
                        >
                          {installed
                            ? 'Available'
                            : !isAdmin
                              ? 'Admin approval needed'
                              : entry.remoteUrl
                                ? busy === `market:${entry.id}`
                                  ? 'Adding…'
                                  : 'Add to Flamel'
                                : 'Set up'}
                        </Btn>
                      </div>
                    </div>
                  </div>
                </Card>
              );
            })}
        </div>
      ) : view === 'skills' ? (
        <div style={{ display: 'grid', gap: 10 }}>
          {loading && skills.length === 0 ? (
            <p style={{ padding: 18, color: 'var(--text-dim)', fontSize: 12 }}>
              Loading governed skills…
            </p>
          ) : filteredSkills.length === 0 ? (
            <p style={{ padding: 18, color: 'var(--text-dim)', fontSize: 12 }}>
              No skills match this filter.
            </p>
          ) : (
            filteredSkills.map((skill) => (
              <Card key={skill.id}>
                <div
                  style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}
                >
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      marginTop: 5,
                      borderRadius: '50%',
                      background: skill.quarantined
                        ? 'var(--error)'
                        : skill.active
                          ? 'var(--success)'
                          : skill.installed
                            ? 'var(--warning)'
                            : 'var(--text-dim)',
                      flexShrink: 0,
                    }}
                  />
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div
                      style={{
                        display: 'flex',
                        flexWrap: 'wrap',
                        alignItems: 'center',
                        gap: 6,
                      }}
                    >
                      <strong style={{ fontSize: 13 }}>{skill.title}</strong>
                      <span
                        style={{
                          fontSize: 9,
                          color:
                            skill.reviewState === 'approved'
                              ? 'var(--success)'
                              : 'var(--warning)',
                        }}
                      >
                        {skill.reviewState}
                      </span>
                      {skill.signed && (
                        <span style={{ fontSize: 9, color: 'var(--accent)' }}>
                          signed
                        </span>
                      )}
                      <span
                        style={{
                          fontSize: 9,
                          color: skill.quarantined
                            ? 'var(--error)'
                            : 'var(--text-dim)',
                        }}
                      >
                        {skill.quarantined ? 'quarantined' : skill.installState}
                      </span>
                      <span style={{ fontSize: 9, color: 'var(--text-dim)' }}>
                        v{skill.activeVersion || skill.version}
                      </span>
                    </div>
                    <p
                      style={{
                        marginTop: 4,
                        fontSize: 11,
                        lineHeight: 1.45,
                        color: 'var(--text-muted)',
                      }}
                    >
                      {skill.description}
                    </p>
                    <p
                      style={{
                        marginTop: 6,
                        fontSize: 10,
                        color: 'var(--text-dim)',
                      }}
                    >
                      Providers:{' '}
                      {skill.providerCompatibility.join(' · ') || 'none'} ·
                      Scope: {skill.installationScope || skill.scope}
                    </p>
                    <p
                      style={{
                        marginTop: 3,
                        fontSize: 10,
                        color: 'var(--text-dim)',
                      }}
                    >
                      {skill.requiredCapabilities.length
                        ? `Required capabilities: ${skill.requiredCapabilities.join(' · ')}`
                        : 'No external capabilities required'}
                    </p>
                    {skill.quarantineReason && (
                      <p
                        style={{
                          marginTop: 4,
                          fontSize: 10,
                          color: 'var(--error)',
                        }}
                      >
                        {skill.quarantineReason}
                      </p>
                    )}
                  </div>
                  <div
                    style={{
                      display: 'flex',
                      flexWrap: 'wrap',
                      justifyContent: 'flex-end',
                      gap: 6,
                    }}
                  >
                    {!skill.installed ? (
                      <Btn
                        variant="primary"
                        disabled={
                          busy !== null ||
                          skill.reviewState !== 'approved' ||
                          skill.quarantined
                        }
                        onClick={() => void manageSkill(skill, 'install')}
                      >
                        {busy === `skill:${skill.id}:install`
                          ? 'Installing…'
                          : 'Install'}
                      </Btn>
                    ) : (
                      <>
                        <Btn
                          disabled={busy !== null || skill.quarantined}
                          onClick={() => void manageSkill(skill, 'toggle')}
                        >
                          {busy === `skill:${skill.id}:toggle`
                            ? 'Updating…'
                            : skill.active
                              ? 'Disable'
                              : 'Enable'}
                        </Btn>
                        {skill.previousVersion && (
                          <Btn
                            disabled={busy !== null}
                            onClick={() => void manageSkill(skill, 'rollback')}
                          >
                            Rollback to {skill.previousVersion}
                          </Btn>
                        )}
                        <Btn
                          variant="ghost"
                          disabled={busy !== null}
                          onClick={() => void manageSkill(skill, 'uninstall')}
                        >
                          {busy === `skill:${skill.id}:uninstall`
                            ? 'Uninstalling…'
                            : 'Uninstall'}
                        </Btn>
                      </>
                    )}
                  </div>
                </div>
              </Card>
            ))
          )}
        </div>
      ) : (
        <div
          style={{
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius-lg)',
            overflow: 'hidden',
          }}
        >
          {activity.length === 0 ? (
            <p style={{ padding: 18, color: 'var(--text-dim)', fontSize: 12 }}>
              No integration activity yet.
            </p>
          ) : (
            activity.map((event, index) => (
              <div
                key={event.id}
                style={{
                  padding: '10px 14px',
                  display: 'grid',
                  gridTemplateColumns: '8px minmax(0,1fr) auto',
                  alignItems: 'start',
                  gap: 10,
                  borderBottom:
                    index < activity.length - 1
                      ? '1px solid var(--border-soft)'
                      : 'none',
                }}
              >
                <span
                  style={{
                    width: 8,
                    height: 8,
                    marginTop: 4,
                    borderRadius: '50%',
                    background:
                      event.decision === 'error'
                        ? 'var(--error)'
                        : event.decision === 'deny'
                          ? 'var(--warning)'
                          : 'var(--success)',
                  }}
                />
                <div>
                  <strong style={{ fontSize: 11, color: 'var(--text)' }}>
                    {event.summary}
                  </strong>
                  <p
                    style={{
                      marginTop: 2,
                      fontSize: 10,
                      color: 'var(--text-muted)',
                    }}
                  >
                    {event.toolName || event.eventType}
                  </p>
                </div>
                <time style={{ fontSize: 9, color: 'var(--text-dim)' }}>
                  {new Date(event.createdAt).toLocaleString()}
                </time>
              </div>
            ))
          )}
        </div>
      )}
      <p style={{ fontSize: 11, color: 'var(--text-dim)' }}>
        Flamel-owned packs, connections, and governed skills expose exact scopes
        and server-confirmed state. Uninstalling a pack revokes its credentials
        and disables its schedules.
      </p>
    </div>
  );
}

// ─── Nav items ───────────────────────────────────────────────────────────────

type TabId =
  | 'general'
  | 'assistant'
  | 'claude'
  | 'openai'
  | 'git'
  | 'git-accounts'
  | 'integrations'
  | 'runners';

interface NavItem {
  id: TabId;
  label: string;
  icon: React.ReactNode;
}

const NAV: NavItem[] = [
  { id: 'general', label: 'General', icon: <Icon.Settings size={14} /> },
  {
    id: 'assistant',
    label: 'Flamel Assistant',
    icon: <Icon.History size={14} />,
  },
  { id: 'claude', label: 'Anthropic', icon: <Icon.Logo size={14} /> },
  { id: 'openai', label: 'OpenAI', icon: <Icon.Chat size={14} /> },
  { id: 'git', label: 'Git', icon: <Icon.Branch size={14} /> },
  { id: 'git-accounts', label: 'Git Accounts', icon: <Icon.User size={14} /> },
  {
    id: 'integrations',
    label: 'Capability Hub',
    icon: <Icon.Server size={14} />,
  },
  { id: 'runners', label: 'Workers', icon: <Icon.Server size={14} /> },
];

// ─── Main modal ──────────────────────────────────────────────────────────────

export default function SettingsModal({ me, onClose, initialTab }: Props) {
  const connectionMode = useConnectionStore((s) => s.mode);
  const isLocalConnection =
    import.meta.env.VITE_MODE === 'local' && connectionMode !== 'remote';

  const [tab, setTab] = useState<TabId>(() => {
    const requested = (initialTab?.split(':')[0] ||
      new URLSearchParams(window.location.search).get('settings') ||
      'general') as TabId;
    const valid = NAV.some((item) => item.id === requested)
      ? requested
      : 'general';
    return (valid === 'runners' || valid === 'assistant') && !hasHostPanel(valid) ? 'general' : valid;
  });
  const [gitSettings, setGitSettings] = useState<UserSettings>({
    git_name: '',
    git_email: '',
    git_signing_key: '',
    preferred_provider: 'codex',
    cross_provider_fallback: true,
    codex_planning_reasoning_effort: 'xhigh',
    claude_planning_reasoning_effort: 'xhigh',
  });
  const [compactionSettings, setCompactionSettings] =
    useState<CompactionSettings>({
      codex_compaction_model: 'gpt-5.6-luna',
      codex_compaction_reasoning_effort: 'medium',
      codex_compaction_fallback_enabled: true,
    });
  const [autoLaunch, setAutoLaunch] = useState(false);
  const [appVersion, setAppVersion] = useState('');
  const [claudeAuth, setClaudeAuth] = useState<ProviderAuthStatus | null>(null);
  const [claudeAuthLoading, setClaudeAuthLoading] = useState(false);
  const [codexAuth, setCodexAuth] = useState<ProviderAuthStatus | null>(null);
  const [codexAuthLoading, setCodexAuthLoading] = useState(false);
  const [gitAccounts, setGitAccounts] = useState<GitAccount[]>([]);
  const isElectron = !!window.electronAPI?.isElectron;

  // Escape to close
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  // Load data on open
  useEffect(() => {
    apiFetch('/api/me/settings', { credentials: 'include' })
      .then((r) => (r.ok ? r.json() : null))
      .then((d) => {
        if (d)
          setGitSettings({
            git_name: d.git_name || '',
            git_email: d.git_email || '',
            git_signing_key: d.git_signing_key || '',
            preferred_provider:
              d.preferred_provider === 'claude' ? 'claude' : 'codex',
            cross_provider_fallback: d.cross_provider_fallback !== false,
            codex_planning_reasoning_effort: planningReasoningEffort(
              d.codex_planning_reasoning_effort,
            ),
            claude_planning_reasoning_effort: planningReasoningEffort(
              d.claude_planning_reasoning_effort,
            ),
          });
      })
      .catch(() => {});

    apiFetch('/api/me/claude-auth', { credentials: 'include' })
      .then((r) => (r.ok ? r.json() : null))
      .then((d) => {
        if (d) setClaudeAuth(d);
      })
      .catch(() => {});

    apiFetch('/api/me/codex-auth', { credentials: 'include' })
      .then((r) => (r.ok ? r.json() : null))
      .then((d) => {
        if (d) setCodexAuth(d);
      })
      .catch(() => {});

    apiFetch('/api/me/settings/compaction', { credentials: 'include' })
      .then((r) => (r.ok ? r.json() : null))
      .then((d) => {
        if (d)
          setCompactionSettings({
            codex_compaction_model: d.codex_compaction_model || 'gpt-5.6-luna',
            codex_compaction_reasoning_effort:
              d.codex_compaction_reasoning_effort || 'medium',
            codex_compaction_fallback_enabled:
              d.codex_compaction_fallback_enabled !== false,
          });
      })
      .catch(() => {});

    apiFetch('/api/git-accounts', { credentials: 'include' })
      .then((r) => (r.ok ? r.json() : null))
      .then((d) => {
        if (d) setGitAccounts(d);
      })
      .catch(() => {});

    if (window.electronAPI) {
      window.electronAPI
        .getAutoLaunch()
        .then(setAutoLaunch)
        .catch(() => {});
      window.electronAPI
        .getAppVersion?.()
        .then(setAppVersion)
        .catch(() => {});
    }
  }, []);

  const visibleNav = NAV.filter(item => (item.id !== 'runners' && item.id !== 'assistant') || (!isLocalConnection && hasHostPanel(item.id)));

  useEffect(() => {
    if (isLocalConnection && tab === 'runners') setTab('general');
  }, [isLocalConnection, tab]);

  return ReactDOM.createPortal(
    <div
      className="settings-modal__backdrop"
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,0.75)',
        zIndex: 9999,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        backdropFilter: 'blur(4px)',
      }}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="settings-modal__dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Flamel settings"
        style={{
          width: 'min(980px, 94vw)',
          height: 'min(580px, 88vh)',
          background: 'var(--surface)',
          border: '1px solid var(--border)',
          borderRadius: 'var(--radius-xl)',
          boxShadow: 'var(--shadow-xl)',
          display: 'flex',
          overflow: 'hidden',
          animation: 'modal-in 0.18s ease-out',
        }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* ── Left nav ────────────────────────────────────────────────── */}
        <div
          className="settings-modal__nav"
          style={{
            width: 188,
            flexShrink: 0,
            background: 'var(--bg)',
            borderRight: '1px solid var(--border-soft)',
            display: 'flex',
            flexDirection: 'column',
            padding: '20px 0',
          }}
        >
          <p
            style={{
              fontSize: 11,
              fontWeight: 600,
              letterSpacing: '0.07em',
              textTransform: 'uppercase',
              color: 'var(--text-dim)',
              padding: '0 16px',
              marginBottom: 8,
            }}
          >
            Settings
          </p>

          <nav
            className="settings-modal__nav-list"
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 1,
              padding: '0 8px',
            }}
          >
            {visibleNav.map((item) => {
              const active = tab === item.id;
              return (
                <button
                  key={item.id}
                  onClick={() => setTab(item.id)}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 9,
                    padding: '7px 10px',
                    borderRadius: 'var(--radius-md)',
                    fontSize: 13,
                    fontWeight: active ? 500 : 400,
                    color: active ? 'var(--text)' : 'var(--text-muted)',
                    background: active ? 'var(--surface-2)' : 'transparent',
                    border: 'none',
                    cursor: 'pointer',
                    textAlign: 'left',
                    transition: 'background 0.12s, color 0.12s',
                  }}
                  onMouseEnter={(e) => {
                    if (!active) {
                      e.currentTarget.style.background = 'var(--surface-2)';
                      e.currentTarget.style.color = 'var(--text)';
                    }
                  }}
                  onMouseLeave={(e) => {
                    if (!active) {
                      e.currentTarget.style.background = 'transparent';
                      e.currentTarget.style.color = 'var(--text-muted)';
                    }
                  }}
                >
                  <span
                    style={{
                      color: active ? 'var(--accent)' : 'var(--text-dim)',
                      flexShrink: 0,
                    }}
                  >
                    {item.icon}
                  </span>
                  {item.label}
                </button>
              );
            })}
          </nav>

          {/* Bottom: close + version */}
          <div style={{ marginTop: 'auto', padding: '0 8px' }}>
            <button
              onClick={onClose}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 9,
                padding: '7px 10px',
                width: '100%',
                borderRadius: 'var(--radius-md)',
                fontSize: 13,
                color: 'var(--text-muted)',
                background: 'transparent',
                border: 'none',
                cursor: 'pointer',
                transition: 'background 0.12s, color 0.12s',
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.background = 'var(--surface-2)';
                e.currentTarget.style.color = 'var(--text)';
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.background = 'transparent';
                e.currentTarget.style.color = 'var(--text-muted)';
              }}
            >
              <Icon.Close size={13} />
              Close
            </button>
          </div>
        </div>

        {/* ── Content area ────────────────────────────────────────────── */}
        <div
          className="settings-modal__content"
          style={{ flex: 1, overflowY: 'auto', padding: '28px 32px' }}
        >
          {/* Tab heading */}
          <h2
            style={{
              fontSize: 18,
              fontWeight: 600,
              color: 'var(--text)',
              marginBottom: 24,
              letterSpacing: '-0.01em',
            }}
          >
            {visibleNav.find((n) => n.id === tab)?.label}
          </h2>

          {tab === 'general' && (
            <GeneralTab
              isElectron={isElectron}
              autoLaunch={autoLaunch}
              setAutoLaunch={setAutoLaunch}
              appVersion={appVersion}
              userSettings={gitSettings}
              setUserSettings={setGitSettings}
            />
          )}
          {tab === 'claude' && (
            <ClaudeTab
              claudeAuth={claudeAuth}
              claudeAuthLoading={claudeAuthLoading}
              setClaudeAuth={setClaudeAuth}
              setClaudeAuthLoading={setClaudeAuthLoading}
              userSettings={gitSettings}
              setUserSettings={setGitSettings}
            />
          )}
          {tab === 'openai' && (
            <CodexTab
              codexAuth={codexAuth}
              codexAuthLoading={codexAuthLoading}
              setCodexAuth={setCodexAuth}
              setCodexAuthLoading={setCodexAuthLoading}
              compactionSettings={compactionSettings}
              setCompactionSettings={setCompactionSettings}
              userSettings={gitSettings}
              setUserSettings={setGitSettings}
            />
          )}
          {tab === 'git' && (
            <GitTab gitSettings={gitSettings} setGitSettings={setGitSettings} />
          )}
          {tab === 'git-accounts' && (
            <GitAccountsTab
              gitAccounts={gitAccounts}
              setGitAccounts={setGitAccounts}
            />
          )}
          {tab === 'assistant' && (
            renderHostPanel('assistant', initialTab)
          )}
          {tab === 'integrations' && (
            <IntegrationsTab
              isAdmin={isLocalConnection || me?.isAdmin === true}
              initialView={
                initialTab?.startsWith('integrations:')
                  ? (initialTab.split(':')[1] as CapabilityHubView)
                  : undefined
              }
            />
          )}
          {!isLocalConnection && tab === 'runners' && renderHostPanel('runners')}
        </div>
      </div>
    </div>,
    document.body,
  );
}
