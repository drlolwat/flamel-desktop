import { create } from 'zustand';

export type SessionLoadPhase = 'history' | 'state' | 'ready' | 'error';
export interface SessionReadiness {
  sessionId: string;
  generation: number;
  attempt: number;
  phase: SessionLoadPhase;
  error?: string;
  historyLoaded?: boolean;
  /** Captured before server replay, never inferred from replayed running state. */
  allowLocalPresentation: boolean;
}

interface SessionReadinessState {
  entries: Record<string, SessionReadiness>;
  retries: Record<string, number>;
  nextAttempt: number;
  begin: (tabId: string, sessionId: string, generation: number, allowLocalPresentation?: boolean) => number;
  transition: (tabId: string, attempt: number, phase: SessionLoadPhase, error?: string) => boolean;
  retry: (tabId: string) => void;
  remove: (tabId: string, attempt: number) => void;
}

/** Ephemeral: a persisted "ready" flag cannot prove a fresh connection is ready. */
export const useSessionReadinessStore = create<SessionReadinessState>((set) => ({
  entries: {},
  retries: {},
  nextAttempt: 0,
  begin: (tabId, sessionId, generation, allowLocalPresentation = false) => {
    let attempt = 0;
    set(state => {
      attempt = state.nextAttempt + 1;
      return {
        nextAttempt: attempt,
        entries: { ...state.entries, [tabId]: { sessionId, generation, attempt, phase: 'history', allowLocalPresentation } },
      };
    });
    return attempt;
  },
  transition: (tabId, attempt, phase, error) => {
    let accepted = false;
    set(state => {
      const current = state.entries[tabId];
      if (!current || current.attempt !== attempt) return state;
      accepted = true;
      if (current.phase === phase && current.error === error) return state;
      return { entries: { ...state.entries, [tabId]: { ...current, phase, error, historyLoaded: current.historyLoaded || phase === 'state' || phase === 'ready' } } };
    });
    return accepted;
  },
  retry: tabId => set(state => ({ retries: { ...state.retries, [tabId]: (state.retries[tabId] || 0) + 1 } })),
  remove: (tabId, attempt) => set(state => {
    if (state.entries[tabId]?.attempt !== attempt) return state;
    const entries = { ...state.entries };
    delete entries[tabId];
    return { entries };
  }),
}));

export function readinessForBinding(
  entry: SessionReadiness | undefined,
  sessionId: string | undefined,
  generation = 0,
): SessionReadiness | undefined {
  return entry?.sessionId === sessionId && entry?.generation === generation ? entry : undefined;
}

/** Reading saved history does not require the live connection to catch up. */
export function canPresentSession(entry: SessionReadiness | undefined): boolean {
  return entry?.historyLoaded === true || entry?.allowLocalPresentation === true;
}
