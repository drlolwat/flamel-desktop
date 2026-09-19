import { create } from 'zustand';

export type StartupPhase = 'starting' | 'connecting' | 'authenticating' | 'workspace' | 'chats' | 'welcome';

const PHASE_PROGRESS: Record<StartupPhase, number> = {
  starting: 10,
  connecting: 38,
  authenticating: 64,
  workspace: 88,
  chats: 96,
  welcome: 100,
};

interface StartupState {
  active: boolean;
  phase: StartupPhase;
  targetProgress: number;
  run: number;
  error: string | null;
  retry: (() => void) | null;
  openWorkspace: (() => void) | null;
  begin: (phase?: StartupPhase) => void;
  advance: (phase: StartupPhase) => void;
  complete: () => void;
  fail: (message: string, retry: () => void, openWorkspace?: () => void) => void;
  hide: () => void;
}

export const useStartupStore = create<StartupState>((set) => ({
  active: true,
  phase: 'starting',
  targetProgress: PHASE_PROGRESS.starting,
  run: 0,
  error: null,
  retry: null,
  openWorkspace: null,
  begin: (phase = 'connecting') => set((state) => ({
    active: true,
    error: null,
    retry: null,
    openWorkspace: null,
    phase,
    targetProgress: PHASE_PROGRESS[phase],
    run: state.active ? state.run : state.run + 1,
  })),
  advance: (phase) => set((state) => ({
    active: true,
    error: null,
    retry: null,
    openWorkspace: null,
    phase,
    targetProgress: Math.max(state.targetProgress, PHASE_PROGRESS[phase]),
  })),
  complete: () => set({ active: true, phase: 'welcome', targetProgress: 100, error: null, retry: null, openWorkspace: null }),
  fail: (error, retry, openWorkspace) => set({ active: true, error, retry, openWorkspace: openWorkspace || null }),
  hide: () => set({ active: false }),
}));
