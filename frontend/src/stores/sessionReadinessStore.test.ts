import { beforeEach, describe, expect, it } from 'vitest';
import { canPresentSession, readinessForBinding, useSessionReadinessStore } from './sessionReadinessStore';

beforeEach(() => useSessionReadinessStore.setState({ entries: {}, retries: {}, nextAttempt: 0 }));

describe('chat preload readiness', () => {
  it('starts with history pending and requires an explicit live-state completion', () => {
    const store = useSessionReadinessStore.getState();
    const attempt = store.begin('tab', 'session', 2);
    expect(useSessionReadinessStore.getState().entries.tab.phase).toBe('history');
    store.transition('tab', attempt, 'state');
    expect(useSessionReadinessStore.getState().entries.tab.phase).toBe('state');
    store.transition('tab', attempt, 'ready');
    expect(useSessionReadinessStore.getState().entries.tab.phase).toBe('ready');
  });

  it('does not accept a previous session or binding generation as ready', () => {
    const store = useSessionReadinessStore.getState();
    const attempt = store.begin('tab', 'old-session', 1);
    store.transition('tab', attempt, 'ready');
    const entry = useSessionReadinessStore.getState().entries.tab;
    expect(readinessForBinding(entry, 'new-session', 1)).toBeUndefined();
    expect(readinessForBinding(entry, 'old-session', 2)).toBeUndefined();
    expect(readinessForBinding(entry, undefined, 1)).toBeUndefined();
    expect(readinessForBinding(entry, 'old-session', 1)?.phase).toBe('ready');
  });

  it('ignores a late callback and cleanup from the same binding before retry', () => {
    const store = useSessionReadinessStore.getState();
    const old = store.begin('tab', 'session', 1);
    store.transition('tab', old, 'error', 'Connection lost');
    store.retry('tab');
    const current = store.begin('tab', 'session', 1);
    expect(current).toBeGreaterThan(old);
    expect(store.transition('tab', old, 'ready')).toBe(false);
    store.remove('tab', old);
    expect(useSessionReadinessStore.getState().entries.tab).toMatchObject({ attempt: current, phase: 'history' });
    expect(useSessionReadinessStore.getState().retries.tab).toBe(1);
  });

  it('keeps readiness and retries independent for each open tab', () => {
    const store = useSessionReadinessStore.getState();
    const a = store.begin('a', 'session-a', 0);
    const b = store.begin('b', 'session-b', 0);
    store.transition('a', a, 'ready');
    store.transition('b', b, 'error', 'Could not connect');
    store.retry('b');
    expect(useSessionReadinessStore.getState().entries.a.phase).toBe('ready');
    expect(useSessionReadinessStore.getState().retries.a).toBeUndefined();
    store.remove('b', b);
    expect(useSessionReadinessStore.getState().entries.b).toBeUndefined();
  });
});

it('keeps loaded history readable through slow or failed live sync without claiming live readiness', () => {
  const store = useSessionReadinessStore.getState();
  const attempt = store.begin('tab', 'session', 1);
  expect(canPresentSession(useSessionReadinessStore.getState().entries.tab)).toBe(false);
  store.transition('tab', attempt, 'state');
  expect(canPresentSession(useSessionReadinessStore.getState().entries.tab)).toBe(true);
  expect(useSessionReadinessStore.getState().entries.tab.phase).toBe('state');
  store.transition('tab', attempt, 'error', 'Live connection timed out');
  expect(canPresentSession(useSessionReadinessStore.getState().entries.tab)).toBe(true);
  expect(canPresentSession(readinessForBinding(useSessionReadinessStore.getState().entries.tab, 'other', 1))).toBe(false);
  store.begin('tab', 'other', 2);
  expect(canPresentSession(useSessionReadinessStore.getState().entries.tab)).toBe(false);
});
