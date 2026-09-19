import { beforeEach, describe, expect, it } from 'vitest';
import { useStartupStore } from './startupStore';

function resetStartupStore() {
  useStartupStore.setState({
    active: true,
    phase: 'starting',
    targetProgress: 10,
    run: 0,
    error: null,
    retry: null,
    openWorkspace: null,
  });
}

describe('startup store', () => {
  beforeEach(resetStartupStore);

  it('advances through one continuous startup without moving progress backward', () => {
    const store = useStartupStore.getState();

    store.advance('connecting');
    store.advance('authenticating');
    store.advance('workspace');

    expect(useStartupStore.getState()).toMatchObject({
      active: true,
      phase: 'workspace',
      targetProgress: 88,
      run: 0,
    });

    useStartupStore.getState().advance('connecting');
    expect(useStartupStore.getState()).toMatchObject({
      phase: 'connecting',
      targetProgress: 88,
    });
  });

  it('starts a distinct run after the overlay was hidden', () => {
    useStartupStore.getState().complete();
    useStartupStore.getState().hide();
    useStartupStore.getState().begin('connecting');

    expect(useStartupStore.getState()).toMatchObject({
      active: true,
      phase: 'connecting',
      targetProgress: 38,
      run: 1,
    });
  });

  it('keeps the loading bar incomplete while history and live state arrive', () => {
    useStartupStore.getState().advance('chats');
    expect(useStartupStore.getState()).toMatchObject({ active: true, phase: 'chats', targetProgress: 96 });
    useStartupStore.getState().complete();
    expect(useStartupStore.getState()).toMatchObject({ phase: 'welcome', targetProgress: 100 });
  });

  it('shows a retryable failure without pretending that loading finished', () => {
    useStartupStore.getState().advance('chats');
    let retried = false;
    useStartupStore.getState().fail('Could not load chat', () => { retried = true; });
    expect(useStartupStore.getState()).toMatchObject({ active: true, phase: 'chats', targetProgress: 96, error: 'Could not load chat' });
    useStartupStore.getState().retry?.();
    expect(retried).toBe(true);
    useStartupStore.getState().advance('chats');
    expect(useStartupStore.getState()).toMatchObject({ error: null, retry: null });
  });
});
