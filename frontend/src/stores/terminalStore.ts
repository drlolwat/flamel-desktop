import { create } from 'zustand';
import { apiFetch } from '../lib/api';

export interface WorkbenchTerminal {
  id: string;
  title: string;
  projectPath: string;
  sessionId?: string;
}

interface TerminalStore {
  terminals: WorkbenchTerminal[];
  activeTerminalId: string | null;
  openTerminal: (projectPath: string) => string;
  attachTerminal: (sessionId: string, projectPath: string, title?: string) => string;
  restoreTerminals: () => Promise<void>;
  setActiveTerminal: (id: string | null) => void;
  setSessionId: (id: string, sessionId: string) => void;
  closeTerminal: (id: string) => void;
}

export const useTerminalStore = create<TerminalStore>((set, get) => ({
  terminals: [],
  activeTerminalId: null,
  openTerminal: (projectPath) => {
    const number = get().terminals.length + 1;
    const id = crypto.randomUUID();
    set((state) => ({
      terminals: [...state.terminals, { id, title: number === 1 ? 'Terminal' : `Terminal ${number}`, projectPath }],
      activeTerminalId: id,
    }));
    return id;
  },
  attachTerminal: (sessionId, projectPath, title = 'Terminal') => {
    const existing = get().terminals.find(terminal => terminal.sessionId === sessionId);
    if (existing) { set({ activeTerminalId: existing.id }); return existing.id; }
    const id = crypto.randomUUID();
    set(state => ({ terminals: [...state.terminals, { id, title, projectPath, sessionId }], activeTerminalId: id }));
    return id;
  },
  restoreTerminals: async () => {
    const response = await apiFetch('/api/terminal');
    if (!response.ok) return;
    const payload = await response.json() as { terminals: Array<{ session_id: string; cwd: string; command: string; exited: boolean }> };
    set(state => {
      const known = new Set(state.terminals.map(terminal => terminal.sessionId));
      const restored = payload.terminals.filter(terminal => !terminal.exited && !known.has(terminal.session_id)).map(terminal => ({
        id: crypto.randomUUID(), sessionId: terminal.session_id, projectPath: terminal.cwd, title: 'Terminal',
      }));
      return { terminals: [...state.terminals, ...restored] };
    });
  },
  setActiveTerminal: (activeTerminalId) => set({ activeTerminalId }),
  setSessionId: (id, sessionId) => set((state) => ({
    terminals: state.terminals.map((terminal) => terminal.id === id ? { ...terminal, sessionId } : terminal),
  })),
  closeTerminal: (id) => set((state) => {
    const terminals = state.terminals.filter((terminal) => terminal.id !== id);
    return {
      terminals,
      activeTerminalId: state.activeTerminalId === id ? (terminals.at(-1)?.id ?? null) : state.activeTerminalId,
    };
  }),
}));
