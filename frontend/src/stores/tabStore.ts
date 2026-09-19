import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import { manualModelSelection } from '../lib/models';
import type { ModelId, ModelSelection, ReasoningEffort, ReasoningSelection } from '../lib/models';
import type { SessionMeta, SessionWorkspace } from '@flamel/shared';

export type { ModelId, ModelSelection, ReasoningEffort, ReasoningSelection } from '../lib/models';

// Wrap localStorage to swallow QuotaExceededError — a full quota must never crash the app.
const safeStorage = createJSONStorage(() => ({
  getItem: (name: string) => { try { return localStorage.getItem(name); } catch { return null; } },
  setItem: (name: string, value: string) => { try { localStorage.setItem(name, value); } catch { /* quota exceeded — tab prefs lost on reload but app keeps running */ } },
  removeItem: (name: string) => { try { localStorage.removeItem(name); } catch { /* ignore */ } },
}));

export type PermissionMode = 'default' | 'acceptEdits' | 'bypassPermissions' | 'plan';
export type TabKind = 'chat' | 'orchestration';
export type SessionOwnerKind = 'orchestration_coordinator' | 'orchestration_subagent' | string;

export interface Tab {
  id: string;
  kind?: TabKind;
  sessionId?: string;
  projectPath?: string;
  workspace?: SessionWorkspace;
  title: string;
  isStreaming: boolean;
  permissionMode: PermissionMode;
  model: ModelSelection;
  reasoningEffort: ReasoningSelection;
  ponytailEnabled: boolean;
  resolvedModel?: ModelId;
  resolvedReasoningEffort?: ReasoningEffort;
  routingReason?: string;
  activePlanId?: string;
  orchestrationId?: string;
  orchestrationRunId?: string;
  orchestrationRole?: string;
  sessionOwnerKind?: SessionOwnerKind | null;
  activeModule?: string;
  workflowView?: string;
  // Background agent tracking - not persisted
  streamingStartedAt?: number;
  currentTool?: string;
  requestId?: string;
  isSessionWarm?: boolean;  // true when the provider process is idle and alive (no cold-start on next send)
  requiresResume?: boolean;  // true when DB has a resumable provider session but the process is cold
  sessionWarmSince?: number;
  runnerId?: string;
  runnerName?: string;
  /** Changes whenever this tab is rebound, including A -> B -> A transitions. */
  bindingGeneration: number;
}

type AutonomousOrchestrationCandidate = {
  sessionOwnerKind?: string | null;
  orchestrationId?: string | null;
  orchestrationRunId?: string | null;
  orchestrationRole?: string | null;
};

export function isAutonomousOrchestrationTab(tab?: AutonomousOrchestrationCandidate | null): boolean {
  if (!tab) return false;
  const ownerKind = tab.sessionOwnerKind;
  return ownerKind === 'goal_worker'
    || ownerKind === 'job_worker'
    || !!ownerKind?.startsWith('orchestration_')
    || !!tab.orchestrationId
    || !!tab.orchestrationRunId
    || !!tab.orchestrationRole;
}

interface TabStore {
  tabs: Tab[];
  activeTabId: string | null;
  addTab: (partial?: Partial<Tab>) => string;
  reconcileSessions: (sessions: SessionMeta[]) => void;
  removeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  updateTab: (id: string, updates: Partial<Tab>) => void;
  reorderTabs: (oldIndex: number, newIndex: number) => void;
}

function createTab(partial?: Partial<Tab>): Tab {
  return {
    id: crypto.randomUUID(),
    title: 'New session',
    kind: 'chat',
    isStreaming: false,
    permissionMode: 'bypassPermissions',
    model: 'gpt-5.6-sol',
    reasoningEffort: 'auto',
    ponytailEnabled: false,
    bindingGeneration: 0,
    ...partial,
  };
}

function hasPlaceholderTitle(tab: Tab): boolean {
  const title = tab.title.trim().toLowerCase();
  return (
    title === '' ||
    title === 'new session' ||
    title === 'untitled' ||
    title === 'untitled session'
  );
}

export function normalizeSessionTabs(tabs: Tab[], activeTabId: string | null): { tabs: Tab[]; activeTabId: string | null } {
  const candidates = tabs.filter(tab => (
    tab.kind !== 'orchestration' &&
    !isAutonomousOrchestrationTab(tab) &&
    tab.sessionOwnerKind !== 'goal_worker' &&
    tab.sessionOwnerKind !== 'job_worker'
  ));
  const preferredOrder = [
    ...candidates.filter(tab => tab.id === activeTabId),
    ...candidates.filter(tab => tab.id !== activeTabId),
  ];
  const kept = new Set<string>();
  const keepTabIds = new Set<string>();
  let keptPlaceholder = false;
  for (const tab of preferredOrder) {
    const isEmptyPlaceholder = !tab.sessionId && hasPlaceholderTitle(tab);
    if (isEmptyPlaceholder && keptPlaceholder) continue;
    if (!tab.sessionId || !kept.has(tab.sessionId)) {
      keepTabIds.add(tab.id);
      if (tab.sessionId) kept.add(tab.sessionId);
      if (isEmptyPlaceholder) keptPlaceholder = true;
    }
  }
  const normalized = candidates
    .filter(tab => keepTabIds.has(tab.id))
    .map(tab => ({
      ...tab,
      ponytailEnabled: tab.ponytailEnabled ?? false,
      bindingGeneration: tab.bindingGeneration || 0,
    }));
  const nextTabs = normalized.length ? normalized : [createTab()];
  const nextActive = nextTabs.some(tab => tab.id === activeTabId)
    ? activeTabId
    : nextTabs[0].id;
  return { tabs: nextTabs, activeTabId: nextActive };
}

export const useTabStore = create<TabStore>()(
  persist(
    (set, get) => ({
      tabs: [createTab()],
      activeTabId: null,

      addTab: (partial) => {
        if (partial?.sessionId) {
          const existing = get().tabs.find(tab => tab.sessionId === partial.sessionId);
          if (existing) {
            set(state => ({
              tabs: state.tabs.map(tab => tab.id === existing.id ? { ...tab, ...partial, id: tab.id } : tab),
              activeTabId: existing.id,
            }));
            return existing.id;
          }
        }
        const tab = createTab(partial);
        set(s => ({ tabs: [...s.tabs, tab], activeTabId: tab.id }));
        return tab.id;
      },

      reconcileSessions: (sessions) => {
        const canonical = new Map(sessions.map(session => [session.id, session]));
        set(state => {
          const reconciled = state.tabs.filter(tab => (
            !tab.sessionId ||
            canonical.has(tab.sessionId) ||
            tab.isStreaming ||
            !hasPlaceholderTitle(tab)
          )).map(tab => {
            const session = tab.sessionId ? canonical.get(tab.sessionId) : undefined;
            if (!session) return tab;
            return {
              ...tab,
              title: session.title?.trim() || tab.title,
              projectPath: session.projectPath || tab.projectPath,
              workspace: session.workspace,
              model: manualModelSelection(session.requestedModel, session.model),
              reasoningEffort: (session.requestedReasoningEffort as ReasoningSelection | undefined) || tab.reasoningEffort,
              resolvedModel: session.model as ModelId | undefined,
              resolvedReasoningEffort: session.resolvedReasoningEffort as ReasoningEffort | undefined,
              routingReason: session.routingReason ?? undefined,
              activePlanId: session.activePlanId ?? undefined,
              sessionOwnerKind: session.sessionOwnerKind ?? null,
            };
          });
          return normalizeSessionTabs(reconciled, state.activeTabId);
        });
      },

      removeTab: (id) => {
        const { tabs, activeTabId } = get();
        if (tabs.length === 1) {
          // Replace with a new blank tab instead of removing
          const newTab = createTab();
          set({ tabs: [newTab], activeTabId: newTab.id });
          return;
        }
        const idx = tabs.findIndex(t => t.id === id);
        const newTabs = tabs.filter(t => t.id !== id);
        let newActive = activeTabId;
        if (activeTabId === id) {
          newActive = newTabs[Math.min(idx, newTabs.length - 1)].id;
        }
        set({ tabs: newTabs, activeTabId: newActive });
      },

      setActiveTab: (id) => {
        set(s => {
          if (!s.tabs.find(t => t.id === id)) return s;
          return { activeTabId: id };
        });
      },

      updateTab: (id, updates) => {
        set(state => {
          const updated = state.tabs.map(tab => {
            if (tab.id !== id) return tab;
            const sessionChanged = updates.sessionId !== undefined && updates.sessionId !== tab.sessionId;
            return {
              ...tab,
              ...updates,
              bindingGeneration: sessionChanged
                ? (tab.bindingGeneration || 0) + 1
                : (tab.bindingGeneration || 0),
            };
          });
          if (!updates.sessionId) return { tabs: updated };
          // A session has one canonical tab. The tab receiving the live server
          // identity wins over stale persisted copies.
          const withoutDuplicate = updated.filter(tab => tab.id === id || tab.sessionId !== updates.sessionId);
          return {
            tabs: withoutDuplicate,
            activeTabId: withoutDuplicate.some(tab => tab.id === state.activeTabId)
              ? state.activeTabId
              : id,
          };
        });
      },

      reorderTabs: (oldIndex, newIndex) => {
        set(s => {
          const tabs = [...s.tabs];
          const [tab] = tabs.splice(oldIndex, 1);
          tabs.splice(newIndex, 0, tab);
          return { tabs };
        });
      },
    }),
    {
      name: 'flamel-tabs',
      storage: safeStorage,
      partialize: (s) => ({
        tabs: s.tabs.map(t => ({ ...t, isStreaming: false, streamingStartedAt: undefined, currentTool: undefined, requestId: undefined, isSessionWarm: undefined, requiresResume: undefined, sessionWarmSince: undefined })),
        activeTabId: s.activeTabId,
      }),
      onRehydrateStorage: () => (state) => {
        if (state) {
          const migrated = state.tabs.map(tab => ({
            ...tab,
            model: manualModelSelection(tab.model, tab.resolvedModel),
            ponytailEnabled: tab.ponytailEnabled ?? false,
          }));
          const normalized = normalizeSessionTabs(migrated, state.activeTabId);
          state.tabs = normalized.tabs;
          state.activeTabId = normalized.activeTabId;
        }
      },
    }
  )
);
