import { beforeEach, describe, expect, it } from 'vitest';
import type { SessionMeta } from '@flamel/shared';
import { isAutonomousOrchestrationTab, normalizeSessionTabs, type Tab, useTabStore } from './tabStore';

function tab(id: string, sessionId?: string, owner?: string): Tab {
  return {
    id,
    kind: 'chat',
    sessionId,
    projectPath: '/workspace/project',
    title: id,
    isStreaming: false,
    permissionMode: 'bypassPermissions',
    model: 'gpt-5.6-sol',
    reasoningEffort: 'high',
    ponytailEnabled: false,
    sessionOwnerKind: owner,
    bindingGeneration: 0,
  };
}

function session(id: string, title: string): SessionMeta {
  return {
    id,
    projectPath: '/workspace/canonical',
    title,
    createdAt: '2026-07-19T00:00:00Z',
    updatedAt: '2026-07-19T00:00:00Z',
    messageCount: 1,
    pinned: false,
    model: 'gpt-5.6-sol',
    requestedModel: 'auto',
    requestedReasoningEffort: 'auto',
    inputTokens: 0,
    outputTokens: 0,
  };
}

describe('tab session identity', () => {
  beforeEach(() => {
    useTabStore.setState({ tabs: [tab('blank')], activeTabId: 'blank' });
  });

  it('restores the authoritative separate workspace on reconnect', () => {
    useTabStore.setState({ tabs: [tab('active', 'session-1')], activeTabId: 'active' });
    const saved = { ...session('session-1', 'Separate task'), workspace: {
      sourcePath: '/repo', projectPath: '/home/user/workspaces/owned', branch: 'flamel/session-owned', isolated: true,
    }, projectPath: '/home/user/workspaces/owned' };
    useTabStore.getState().reconcileSessions([saved]);
    expect(useTabStore.getState().tabs[0]).toMatchObject({ projectPath: saved.projectPath, workspace: saved.workspace });
  });

  it('keeps one tab per persisted session and preserves the active copy', () => {
    const normalized = normalizeSessionTabs(
      [tab('older', 'session-1'), tab('active', 'session-1'), tab('other', 'session-2')],
      'active',
    );

    expect(normalized.tabs.map(value => value.id)).toEqual(['active', 'other']);
    expect(normalized.activeTabId).toBe('active');
  });

  it('creates sessions with an explicit provider, automatic reasoning, and Ponytail off', () => {
    const id = useTabStore.getState().addTab();

    expect(useTabStore.getState().tabs.find(value => value.id === id)).toMatchObject({
      model: 'gpt-5.6-sol',
      reasoningEffort: 'auto',
      ponytailEnabled: false,
    });
  });

  it('defaults legacy tabs to Ponytail off during normalization', () => {
    const legacy = { ...tab('legacy'), ponytailEnabled: undefined } as unknown as Tab;
    const normalized = normalizeSessionTabs([legacy], 'legacy');

    expect(normalized.tabs[0].ponytailEnabled).toBe(false);
  });

  it('classifies worker-owned sessions as internal autonomous sessions', () => {
    expect(isAutonomousOrchestrationTab(tab('goal', 'goal-session', 'goal_worker'))).toBe(true);
    expect(isAutonomousOrchestrationTab(tab('job', 'job-session', 'job_worker'))).toBe(true);
    expect(isAutonomousOrchestrationTab(tab('chat', 'chat-session'))).toBe(false);
  });

  it('removes internal autonomous worker tabs during persisted-state recovery', () => {
    const orchestration = { ...tab('orchestration', 'orchestration-session'), kind: 'orchestration' as const };
    const normalized = normalizeSessionTabs(
      [
        tab('goal', 'goal-session', 'goal_worker'),
        tab('job', 'job-session', 'job_worker'),
        orchestration,
        tab('chat', 'chat-session'),
      ],
      'job',
    );

    expect(normalized.tabs.map(value => value.id)).toEqual(['chat']);
    expect(normalized.activeTabId).toBe('chat');
  });

  it('collapses legacy empty placeholders while preserving the active one', () => {
    const older = { ...tab('older'), title: 'Untitled session' };
    const active = { ...tab('active'), title: 'New session' };
    const named = { ...tab('draft'), title: 'Release notes draft' };
    const normalized = normalizeSessionTabs([older, active, named], 'active');

    expect(normalized.tabs.map(value => value.id)).toEqual(['active', 'draft']);
    expect(normalized.activeTabId).toBe('active');
  });

  it('reuses an existing tab when the same session is opened again', () => {
    const first = useTabStore.getState().addTab({ sessionId: 'session-1', title: 'First' });
    const second = useTabStore.getState().addTab({ sessionId: 'session-1', title: 'Canonical' });

    expect(second).toBe(first);
    expect(useTabStore.getState().tabs.filter(value => value.sessionId === 'session-1')).toHaveLength(1);
    expect(useTabStore.getState().tabs.find(value => value.id === first)?.title).toBe('Canonical');
  });

  it('reconciles the selected tab with canonical server metadata', () => {
    const id = useTabStore.getState().addTab({ sessionId: 'session-1', title: 'Untitled session' });
    useTabStore.getState().reconcileSessions([session('session-1', 'Replex')]);

    expect(useTabStore.getState().tabs.find(value => value.id === id)).toMatchObject({
      title: 'Replex',
      projectPath: '/workspace/canonical',
      model: 'gpt-5.6-sol',
      reasoningEffort: 'auto',
    });
  });

  it('removes stale placeholder tabs that are absent from the server session list', () => {
    useTabStore.setState({
      tabs: [
        { ...tab('stale', 'missing-session'), title: 'Untitled session' },
        tab('real', 'session-1'),
      ],
      activeTabId: 'stale',
    });

    useTabStore.getState().reconcileSessions([session('session-1', 'Replex')]);

    expect(useTabStore.getState().tabs.map(value => value.id)).toEqual(['real']);
    expect(useTabStore.getState().activeTabId).toBe('real');
  });

  it('collapses a stale duplicate when the server assigns a session id', () => {
    useTabStore.setState({
      tabs: [tab('stale', 'session-1'), tab('connected')],
      activeTabId: 'stale',
    });

    useTabStore.getState().updateTab('connected', { sessionId: 'session-1', title: 'Canonical' });

    expect(useTabStore.getState().tabs.map(value => value.id)).toEqual(['connected']);
    expect(useTabStore.getState().activeTabId).toBe('connected');
  });

  it('invalidates stale responses across an A to B to A rebind', () => {
    useTabStore.setState({ tabs: [tab('active', 'session-a')], activeTabId: 'active' });
    const firstGeneration = useTabStore.getState().tabs[0].bindingGeneration;

    useTabStore.getState().updateTab('active', { sessionId: 'session-b' });
    useTabStore.getState().updateTab('active', { sessionId: 'session-a' });

    const rebound = useTabStore.getState().tabs[0];
    expect(rebound.sessionId).toBe('session-a');
    expect(rebound.bindingGeneration).toBe(firstGeneration + 2);
  });
});
