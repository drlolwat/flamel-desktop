import { beforeEach, describe, expect, it } from 'vitest';
import { useAppStore } from './appStore';

const PROJECT = '/workspace/flamel';

function resetAppStore() {
  useAppStore.setState({
    leftPanel: 'drive',
    rightPanel: 'plan',
    leftPanelOpen: true,
    rightPanelOpen: true,
    leftPanelWidth: 220,
    rightPanelWidth: 420,
    centerView: 'chat',
    selectedChange: null,
    mobilePanel: 'chat',
    sessionsVersion: 0,
  });
}

describe('app store workspace persistence state', () => {
  beforeEach(() => {
    resetAppStore();
  });

  it('keeps file manager layout and active workspace surface in app state', () => {
    const store = useAppStore.getState();

    store.setLeftPanel('files');
    store.setLeftPanelOpen(true);
    store.setLeftPanelWidth(900);
    store.setCenterView('changes');
    store.setSelectedChange({
      projectPath: PROJECT,
      file: { path: 'frontend/src/App.tsx', status: 'M', unstaged: true, added: 3, removed: 1 },
    });
    store.setMobilePanel('drive');

    const state = useAppStore.getState();
    expect(state.leftPanel).toBe('files');
    expect(state.leftPanelOpen).toBe(true);
    expect(state.leftPanelWidth).toBe(500);
    expect(state.centerView).toBe('changes');
    expect(state.mobilePanel).toBe('drive');
    expect(state.selectedChange).toMatchObject({
      projectPath: PROJECT,
      file: { path: 'frontend/src/App.tsx', status: 'M', unstaged: true, added: 3, removed: 1 },
    });
  });

  it('opens the active plan on both desktop and mobile surfaces', () => {
    const store = useAppStore.getState();
    store.setRightPanel('plan-history');
    store.setRightPanelOpen(false);
    store.setMobilePanel('chat');

    store.openPlanPanel();

    const state = useAppStore.getState();
    expect(state.rightPanel).toBe('plan');
    expect(state.rightPanelOpen).toBe(true);
    expect(state.mobilePanel).toBe('plan');
  });

  it('normalizes legacy git status labels for restored diff selections', () => {
    useAppStore.getState().setSelectedChange({
      projectPath: PROJECT,
      file: { path: 'frontend/src/App.tsx', status: 'modified', unstaged: true },
    } as any);

    expect(useAppStore.getState().selectedChange).toMatchObject({
      projectPath: PROJECT,
      file: { path: 'frontend/src/App.tsx', status: 'M', unstaged: true },
    });
  });

  it('drops malformed persisted diff selections', () => {
    useAppStore.getState().setSelectedChange({
      projectPath: '',
      file: { path: 'frontend/src/App.tsx', status: 'M' },
    } as any);

    expect(useAppStore.getState().selectedChange).toBeNull();
  });
});
