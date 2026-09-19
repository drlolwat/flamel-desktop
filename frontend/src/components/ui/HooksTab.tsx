import React, { useEffect, useState, useCallback } from 'react';
import { apiFetch } from '../../lib/api';
import { Icon } from './Icons';

// ─── Types ───────────────────────────────────────────────────────────────────

const HOOK_EVENTS = [
  'PreToolUse',
  'PostToolUse',
  'UserPromptSubmit',
  'Stop',
  'PreCompact',
  'Notification',
] as const;

type HookEvent = typeof HOOK_EVENTS[number];

interface HookEntry {
  id: string;
  event: HookEvent;
  matcher: string;
  command: string;
  enabled: boolean;
}

// Backend shape: { PreToolUse: [{ matcher?, hooks: [{ type, command }] }] }
type HooksJson = Record<string, Array<{ matcher?: string; hooks: Array<{ type: string; command?: string }> }>>;

function flatten(hooks: HooksJson): HookEntry[] {
  const result: HookEntry[] = [];
  for (const [event, groups] of Object.entries(hooks)) {
    if (!Array.isArray(groups)) continue;
    for (const group of groups) {
      if (!Array.isArray(group.hooks)) continue;
      for (const hook of group.hooks) {
        if (hook.type === 'command' && hook.command) {
          result.push({
            id: `${event}-${Math.random().toString(36).slice(2)}`,
            event: event as HookEvent,
            matcher: group.matcher || '',
            command: hook.command,
            enabled: true,
          });
        }
      }
    }
  }
  return result;
}

function unflatten(entries: HookEntry[]): HooksJson {
  const result: HooksJson = {};
  for (const entry of entries) {
    if (!entry.enabled) continue;
    if (!result[entry.event]) result[entry.event] = [];
    result[entry.event].push({
      ...(entry.matcher ? { matcher: entry.matcher } : {}),
      hooks: [{ type: 'command', command: entry.command }],
    });
  }
  return result;
}

// ─── Colour per event ────────────────────────────────────────────────────────

const EVENT_COLORS: Record<string, string> = {
  PreToolUse:        '#7c6af7',
  PostToolUse:       '#5ba4e6',
  UserPromptSubmit:  '#3ab97a',
  Stop:              '#e0623a',
  PreCompact:        '#d4a017',
  Notification:      '#8e8e9e',
};

// ─── Sub-components ──────────────────────────────────────────────────────────

function EventBadge({ event }: { event: string }) {
  const color = EVENT_COLORS[event] || 'var(--text-dim)';
  return (
    <span style={{
      fontSize: 11, fontWeight: 600, padding: '2px 7px',
      borderRadius: 4, background: `${color}22`,
      color, letterSpacing: '0.03em', whiteSpace: 'nowrap',
    }}>
      {event}
    </span>
  );
}

function HookRow({ entry, onChange, onDelete }: {
  entry: HookEntry;
  onChange: (updated: HookEntry) => void;
  onDelete: () => void;
}) {
  return (
    <div style={{
      display: 'flex', alignItems: 'flex-start', gap: 10,
      padding: '10px 12px',
      background: entry.enabled ? 'var(--surface)' : 'var(--bg)',
      border: '1px solid var(--border-soft)',
      borderRadius: 'var(--radius-md)',
      opacity: entry.enabled ? 1 : 0.55,
    }}>
      {/* Toggle */}
      <button
        title={entry.enabled ? 'Disable' : 'Enable'}
        onClick={() => onChange({ ...entry, enabled: !entry.enabled })}
        style={{
          marginTop: 2, flexShrink: 0,
          width: 16, height: 16,
          borderRadius: 3,
          border: `1.5px solid ${entry.enabled ? 'var(--accent)' : 'var(--border)'}`,
          background: entry.enabled ? 'var(--accent)' : 'transparent',
          cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}
      >
        {entry.enabled && (
          <svg width="9" height="7" viewBox="0 0 9 7" fill="none">
            <path d="M1 3.5L3.5 6L8 1" stroke="white" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        )}
      </button>

      {/* Main content */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 5 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          <EventBadge event={entry.event} />
          {entry.matcher && (
            <span style={{
              fontSize: 11, padding: '2px 6px',
              background: 'var(--surface-2)', border: '1px solid var(--border-soft)',
              borderRadius: 4, color: 'var(--text-muted)', fontFamily: 'monospace',
            }}>
              {entry.matcher}
            </span>
          )}
        </div>
        <code style={{
          fontSize: 12, color: 'var(--text-muted)',
          background: 'var(--bg)', padding: '3px 6px',
          borderRadius: 4, display: 'block',
          whiteSpace: 'pre-wrap', wordBreak: 'break-all',
        }}>
          {entry.command}
        </code>
      </div>

      {/* Delete */}
      <button
        onClick={onDelete}
        title="Delete hook"
        style={{
          flexShrink: 0, marginTop: 1,
          padding: '3px 5px', borderRadius: 4,
          background: 'transparent', border: 'none',
          color: 'var(--text-dim)', cursor: 'pointer',
          transition: 'color 0.12s, background 0.12s',
        }}
        onMouseEnter={e => { e.currentTarget.style.color = '#e05555'; e.currentTarget.style.background = '#e0555518'; }}
        onMouseLeave={e => { e.currentTarget.style.color = 'var(--text-dim)'; e.currentTarget.style.background = 'transparent'; }}
      >
        <Icon.Close size={13} />
      </button>
    </div>
  );
}

// ─── Add hook form ───────────────────────────────────────────────────────────

function AddHookForm({ onAdd, onCancel }: {
  onAdd: (entry: Omit<HookEntry, 'id' | 'enabled'>) => void;
  onCancel: () => void;
}) {
  const [event, setEvent] = useState<HookEvent>('PreToolUse');
  const [matcher, setMatcher] = useState('');
  const [command, setCommand] = useState('');

  const inputStyle: React.CSSProperties = {
    width: '100%', boxSizing: 'border-box',
    padding: '6px 9px', borderRadius: 'var(--radius-sm)',
    border: '1px solid var(--border)', background: 'var(--bg)',
    color: 'var(--text)', fontSize: 13, outline: 'none',
  };

  const labelStyle: React.CSSProperties = {
    fontSize: 11, fontWeight: 600, color: 'var(--text-dim)',
    textTransform: 'uppercase', letterSpacing: '0.06em',
    display: 'block', marginBottom: 4,
  };

  return (
    <div style={{
      border: '1px solid var(--accent)', borderRadius: 'var(--radius-md)',
      padding: '14px 14px', background: 'var(--surface)',
      display: 'flex', flexDirection: 'column', gap: 10,
    }}>
      <div style={{ display: 'flex', gap: 10 }}>
        {/* Event */}
        <div style={{ flex: 1 }}>
          <label style={labelStyle}>Event</label>
          <select
            value={event}
            onChange={e => setEvent(e.target.value as HookEvent)}
            style={{ ...inputStyle, cursor: 'pointer' }}
          >
            {HOOK_EVENTS.map(ev => (
              <option key={ev} value={ev}>{ev}</option>
            ))}
          </select>
        </div>
        {/* Matcher */}
        <div style={{ flex: 1 }}>
          <label style={labelStyle}>Tool matcher <span style={{ fontWeight: 400, textTransform: 'none', opacity: 0.6 }}>(optional)</span></label>
          <input
            value={matcher}
            onChange={e => setMatcher(e.target.value)}
            placeholder='e.g. Bash or Bash(git *)'
            style={{ ...inputStyle, fontFamily: 'monospace' }}
          />
        </div>
      </div>
      {/* Command */}
      <div>
        <label style={labelStyle}>Command</label>
        <textarea
          value={command}
          onChange={e => setCommand(e.target.value)}
          placeholder='e.g. echo "$CLAUDE_TOOL_NAME" >> ~/hooks.log'
          rows={3}
          style={{ ...inputStyle, resize: 'vertical', fontFamily: 'monospace' }}
        />
      </div>
      {/* Buttons */}
      <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <button
          onClick={onCancel}
          style={{
            padding: '6px 14px', borderRadius: 'var(--radius-sm)',
            border: '1px solid var(--border)', background: 'transparent',
            color: 'var(--text-muted)', fontSize: 13, cursor: 'pointer',
          }}
        >
          Cancel
        </button>
        <button
          onClick={() => {
            if (!command.trim()) return;
            onAdd({ event, matcher: matcher.trim(), command: command.trim() });
          }}
          disabled={!command.trim()}
          style={{
            padding: '6px 14px', borderRadius: 'var(--radius-sm)',
            border: 'none', background: 'var(--accent)',
            color: '#fff', fontSize: 13, cursor: command.trim() ? 'pointer' : 'not-allowed',
            opacity: command.trim() ? 1 : 0.5,
          }}
        >
          Add hook
        </button>
      </div>
    </div>
  );
}

// ─── Main tab ─────────────────────────────────────────────────────────────────

export default function HooksTab() {
  const [entries, setEntries] = useState<HookEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [showAdd, setShowAdd] = useState(false);
  const [error, setError] = useState('');

  // Load on mount
  useEffect(() => {
    apiFetch('/api/me/hooks', { credentials: 'include' })
      .then(r => r.ok ? r.json() : null)
      .then(d => { if (d?.hooks) setEntries(flatten(d.hooks)); })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const save = useCallback(async (updated: HookEntry[]) => {
    setSaving(true);
    setError('');
    try {
      const res = await apiFetch('/api/me/hooks', {
        method: 'PUT',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ hooks: unflatten(updated) }),
      });
      if (!res.ok) setError('Failed to save');
    } catch {
      setError('Failed to save');
    } finally {
      setSaving(false);
    }
  }, []);

  const update = (updated: HookEntry[]) => {
    setEntries(updated);
    save(updated);
  };

  if (loading) {
    return <p style={{ color: 'var(--text-dim)', fontSize: 13 }}>Loading…</p>;
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <p style={{ fontSize: 13, color: 'var(--text-muted)', maxWidth: 460, margin: 0 }}>
          Lifecycle hooks run shell commands on Claude Code events.
          Written to <code style={{ fontSize: 12, background: 'var(--surface-2)', padding: '1px 4px', borderRadius: 3 }}>~/.claude/settings.json</code>.
        </p>
        {!showAdd && (
          <button
            onClick={() => setShowAdd(true)}
            style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '6px 12px', borderRadius: 'var(--radius-sm)',
              border: '1px solid var(--border)', background: 'var(--surface-2)',
              color: 'var(--text)', fontSize: 13, cursor: 'pointer', whiteSpace: 'nowrap',
              transition: 'background 0.12s',
            }}
            onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-3)'; }}
            onMouseLeave={e => { e.currentTarget.style.background = 'var(--surface-2)'; }}
          >
            <Icon.Plus size={13} />
            Add hook
          </button>
        )}
      </div>

      {/* Add form */}
      {showAdd && (
        <AddHookForm
          onCancel={() => setShowAdd(false)}
          onAdd={({ event, matcher, command }) => {
            const entry: HookEntry = {
              id: `${event}-${Math.random().toString(36).slice(2)}`,
              event, matcher, command, enabled: true,
            };
            const next = [...entries, entry];
            setShowAdd(false);
            update(next);
          }}
        />
      )}

      {/* Hook list */}
      {entries.length === 0 && !showAdd ? (
        <div style={{
          padding: '32px 24px', textAlign: 'center',
          border: '1px dashed var(--border)', borderRadius: 'var(--radius-md)',
          color: 'var(--text-dim)', fontSize: 13,
        }}>
          No hooks configured. Click <strong>Add hook</strong> to automate your workflow.
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {entries.map(entry => (
            <HookRow
              key={entry.id}
              entry={entry}
              onChange={updated => update(entries.map(e => e.id === entry.id ? updated : e))}
              onDelete={() => update(entries.filter(e => e.id !== entry.id))}
            />
          ))}
        </div>
      )}

      {/* Status */}
      {saving && <p style={{ fontSize: 12, color: 'var(--text-dim)', margin: 0 }}>Saving…</p>}
      {error && <p style={{ fontSize: 12, color: '#e05555', margin: 0 }}>{error}</p>}
    </div>
  );
}
