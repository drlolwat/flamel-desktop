import type { GitChangeStatus, GitFileStatus } from '@flamel/shared';
import { create } from 'zustand';
import { createJSONStorage, persist } from 'zustand/middleware';

type LeftPanel = 'drive' | 'files' | 'changes' | 'runners' | 'admin';
type RightPanel = 'plan' | 'plan-history';
export type CenterView = 'chat' | 'ide' | 'changes' | 'terminal' | 'problems';
export type MobilePanel = 'chat' | 'drive' | 'plan';

export interface WorkspaceSelectedChange {
  projectPath: string;
  file: GitFileStatus;
}

type PersistedAppState = {
  leftPanel?: unknown;
  rightPanel?: unknown;
  leftPanelOpen?: unknown;
  rightPanelOpen?: unknown;
  leftPanelWidth?: unknown;
  rightPanelWidth?: unknown;
  centerView?: unknown;
  selectedChange?: unknown;
  mobilePanel?: unknown;
};

interface AppStore {
  leftPanel: LeftPanel;
  rightPanel: RightPanel;
  leftPanelOpen: boolean;
  rightPanelOpen: boolean;
  leftPanelWidth: number;
  rightPanelWidth: number;
  centerView: CenterView;
  selectedChange: WorkspaceSelectedChange | null;
  mobilePanel: MobilePanel;
  sessionsVersion: number;
  setLeftPanel: (panel: LeftPanel) => void;
  setRightPanel: (panel: RightPanel) => void;
  setLeftPanelOpen: (open: boolean) => void;
  setRightPanelOpen: (open: boolean) => void;
  setLeftPanelWidth: (width: number) => void;
  setRightPanelWidth: (width: number) => void;
  setCenterView: (view: CenterView) => void;
  setSelectedChange: (change: WorkspaceSelectedChange | null) => void;
  setMobilePanel: (panel: MobilePanel) => void;
  openPlanPanel: () => void;
  refreshSessions: () => void;
}

const GIT_CHANGE_STATUSES = new Set(['A', 'M', 'D', 'R', 'C', 'T', 'U', '?', '!']);
const LEGACY_GIT_CHANGE_STATUS: Record<string, GitChangeStatus> = {
  added: 'A',
  copied: 'C',
  deleted: 'D',
  ignored: '!',
  modified: 'M',
  removed: 'D',
  renamed: 'R',
  untracked: '?',
};

const memoryStorage = (() => {
  const values = new Map<string, string>();
  return {
    getItem: (name: string) => values.get(name) ?? null,
    setItem: (name: string, value: string) => { values.set(name, value); },
    removeItem: (name: string) => { values.delete(name); },
  };
})();

const appStorage = createJSONStorage(() => {
  if (typeof window === 'undefined' || !window.localStorage) return memoryStorage;
  return {
    getItem: (name: string) => {
      try { return window.localStorage.getItem(name); }
      catch { return null; }
    },
    setItem: (name: string, value: string) => {
      try { window.localStorage.setItem(name, value); }
      catch { /* layout preferences stay in memory when storage is unavailable */ }
    },
    removeItem: (name: string) => {
      try { window.localStorage.removeItem(name); }
      catch { /* ignore */ }
    },
  };
});

function normalizeLeftPanel(panel: unknown): LeftPanel {
  return panel === 'files' || panel === 'changes' || panel === 'runners' || panel === 'admin'
    ? panel
    : 'drive';
}

function normalizeRightPanel(panel: unknown): RightPanel {
  return panel === 'plan-history' ? 'plan-history' : 'plan';
}

function normalizeCenterView(view: unknown): CenterView {
  return view === 'ide' || view === 'changes' || view === 'terminal' || view === 'problems' ? view : 'chat';
}

function normalizeMobilePanel(panel: unknown): MobilePanel {
  return panel === 'drive' || panel === 'plan' ? panel : 'chat';
}

function clampWidth(width: unknown, fallback: number, min: number, max: number): number {
  return typeof width === 'number' && Number.isFinite(width)
    ? Math.max(min, Math.min(max, width))
    : fallback;
}

function normalizeGitChangeStatus(value: unknown): GitChangeStatus | null {
  if (typeof value !== 'string') return null;
  if (GIT_CHANGE_STATUSES.has(value)) return value as GitChangeStatus;
  return LEGACY_GIT_CHANGE_STATUS[value.toLowerCase()] ?? null;
}

function normalizeOptionalGitChangeStatus(value: unknown): GitChangeStatus | ' ' | undefined {
  if (value === ' ') return ' ';
  return normalizeGitChangeStatus(value) ?? undefined;
}

function optionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

function optionalBoolean(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined;
}

function normalizeSelectedChange(change: unknown): WorkspaceSelectedChange | null {
  if (!change || typeof change !== 'object') return null;
  const candidate = change as { projectPath?: unknown; file?: unknown };
  if (typeof candidate.projectPath !== 'string' || candidate.projectPath.length === 0) return null;
  if (!candidate.file || typeof candidate.file !== 'object') return null;

  const rawFile = candidate.file as Partial<GitFileStatus>;
  const status = normalizeGitChangeStatus(rawFile.status);
  if (!status || typeof rawFile.path !== 'string' || rawFile.path.length === 0) return null;

  const file: GitFileStatus = {
    path: rawFile.path,
    status,
  };
  file.added = optionalNumber(rawFile.added);
  file.removed = optionalNumber(rawFile.removed);
  file.staged = optionalBoolean(rawFile.staged);
  file.unstaged = optionalBoolean(rawFile.unstaged);
  file.untracked = optionalBoolean(rawFile.untracked);
  file.indexStatus = normalizeOptionalGitChangeStatus(rawFile.indexStatus);
  file.worktreeStatus = normalizeOptionalGitChangeStatus(rawFile.worktreeStatus);
  file.stagedAdded = optionalNumber(rawFile.stagedAdded);
  file.stagedRemoved = optionalNumber(rawFile.stagedRemoved);
  file.unstagedAdded = optionalNumber(rawFile.unstagedAdded);
  file.unstagedRemoved = optionalNumber(rawFile.unstagedRemoved);

  return { projectPath: candidate.projectPath, file };
}

export const useAppStore = create<AppStore>()(
  persist(
    (set) => ({
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
      setLeftPanel: (panel) => set({ leftPanel: normalizeLeftPanel(panel) }),
      setRightPanel: (panel) => set({ rightPanel: normalizeRightPanel(panel) }),
      setLeftPanelOpen: (open) => set({ leftPanelOpen: open }),
      setRightPanelOpen: (open) => set({ rightPanelOpen: open }),
      setLeftPanelWidth: (width) => set({ leftPanelWidth: clampWidth(width, 220, 160, 500) }),
      setRightPanelWidth: (width) => set({ rightPanelWidth: clampWidth(width, 420, 280, 700) }),
      setCenterView: (view) => set({ centerView: normalizeCenterView(view) }),
      setSelectedChange: (change) => set({ selectedChange: normalizeSelectedChange(change) }),
      setMobilePanel: (panel) => set({ mobilePanel: normalizeMobilePanel(panel) }),
      openPlanPanel: () => set({ rightPanel: 'plan', rightPanelOpen: true, mobilePanel: 'plan' }),
      refreshSessions: () => set(s => ({ sessionsVersion: s.sessionsVersion + 1 })),
    }),
    {
      name: 'flamel-app',
      storage: appStorage,
      partialize: (s) => ({
        leftPanel: normalizeLeftPanel(s.leftPanel),
        rightPanel: normalizeRightPanel(s.rightPanel),
        leftPanelOpen: s.leftPanelOpen,
        rightPanelOpen: s.rightPanelOpen,
        leftPanelWidth: clampWidth(s.leftPanelWidth, 220, 160, 500),
        rightPanelWidth: clampWidth(s.rightPanelWidth, 420, 280, 700),
        centerView: normalizeCenterView(s.centerView),
        selectedChange: normalizeSelectedChange(s.selectedChange),
        mobilePanel: normalizeMobilePanel(s.mobilePanel),
      }),
      version: 4,
      migrate: (persistedState) => {
        const state = persistedState as PersistedAppState;
        const legacyPlansPanel = state.leftPanel === 'plans';
        return {
          leftPanel: normalizeLeftPanel(state.leftPanel),
          rightPanel: legacyPlansPanel ? 'plan-history' : normalizeRightPanel(state.rightPanel),
          leftPanelOpen: typeof state.leftPanelOpen === 'boolean' ? state.leftPanelOpen : true,
          rightPanelOpen: legacyPlansPanel ? true : (typeof state.rightPanelOpen === 'boolean' ? state.rightPanelOpen : true),
          leftPanelWidth: clampWidth(state.leftPanelWidth, 220, 160, 500),
          rightPanelWidth: clampWidth(state.rightPanelWidth, 420, 280, 700),
          centerView: normalizeCenterView(state.centerView),
          selectedChange: normalizeSelectedChange(state.selectedChange),
          mobilePanel: normalizeMobilePanel(state.mobilePanel),
        };
      },
    }
  )
);
