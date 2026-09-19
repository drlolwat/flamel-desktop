import { create } from 'zustand';
import type { PlanApprovalStatus, PlanArtifact, PlanExecutionStatus, PlanItem, PlanVersion } from '@flamel/shared';

interface TabPlanState {
  planId?: string;
  title?: string;
  description?: string;
  items: PlanItem[];
  versions: PlanVersion[];
  artifact?: PlanArtifact;
  finished?: boolean;
  executionStatus?: PlanExecutionStatus;
  approvalStatus?: PlanApprovalStatus;
  activeItemId?: string;
  approvalRequestedAt?: string;
  approvedAt?: string;
  executionStartedAt?: string;
  executionUpdatedAt?: string;
  streamingItemIds?: Set<string>;
}

interface SetPlanMeta {
  artifact?: PlanArtifact | null;
  versions?: PlanVersion[] | null;
  finished?: boolean | null;
  executionStatus?: PlanExecutionStatus | null;
  approvalStatus?: PlanApprovalStatus | null;
  activeItemId?: string | null;
  approvalRequestedAt?: string | null;
  approvedAt?: string | null;
  executionStartedAt?: string | null;
  executionUpdatedAt?: string | null;
}

function hasMetaKey(meta: SetPlanMeta | undefined, key: keyof SetPlanMeta): boolean {
  return !!meta && Object.prototype.hasOwnProperty.call(meta, key);
}

interface PlanStore {
  plans: Record<string, TabPlanState>;
  getPlan: (tabId: string) => TabPlanState;
  setPlan: (tabId: string, planId: string, title: string, description: string | undefined, items: PlanItem[], snapshot?: boolean, meta?: SetPlanMeta) => void;
  updateItem: (tabId: string, itemId: string, updates: Partial<PlanItem>) => void;
  addItems: (tabId: string, items: PlanItem[]) => void;
  removeItem: (tabId: string, itemId: string) => void;
  clearPlan: (tabId: string) => void;
  setFinished: (tabId: string, finished: boolean) => void;
  setExecutionMeta: (tabId: string, meta: SetPlanMeta) => void;
  setStreamingIds: (tabId: string, itemIds: string[]) => void;
  clearStreamingIds: (tabId: string) => void;
  // Legacy compat
  updatePlan: (tabId: string, items: PlanItem[], title?: string) => void;
}

const defaultPlan = (): TabPlanState => ({ items: [], versions: [] });

function shallowRecordEqual(a: Record<string, unknown>, b: Record<string, unknown>): boolean {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const key of keys) {
    if (!Object.is(a[key], b[key])) return false;
  }
  return true;
}

function sameStringSet(existing: Set<string> | undefined, incoming: Set<string> | undefined): boolean {
  if (existing === incoming) return true;
  if (!existing || !incoming) return false;
  if (existing.size !== incoming.size) return false;
  for (const value of existing) {
    if (!incoming.has(value)) return false;
  }
  return true;
}

function mergePlanItems(existing: PlanItem[], incoming: PlanItem[]): PlanItem[] {
  const byId = new Map(existing.map(item => [item.id, item]));
  let changed = existing.length !== incoming.length;
  const merged = incoming.map((item, index) => {
    const previous = byId.get(item.id);
    if (previous && shallowRecordEqual(previous as unknown as Record<string, unknown>, item as unknown as Record<string, unknown>)) {
      if (previous !== existing[index]) changed = true;
      return previous;
    }
    changed = true;
    return item;
  });
  return changed ? merged : existing;
}

function samePlanState(a: TabPlanState, b: TabPlanState): boolean {
  const { streamingItemIds: aStreamingIds, ...aRest } = a;
  const { streamingItemIds: bStreamingIds, ...bRest } = b;
  return shallowRecordEqual(aRest as unknown as Record<string, unknown>, bRest as unknown as Record<string, unknown>) &&
    sameStringSet(aStreamingIds, bStreamingIds);
}

function snapshotVersion(items: PlanItem[]): PlanVersion {
  return {
    id: crypto.randomUUID(),
    items: items.map(i => ({ ...i })),
    timestamp: new Date().toISOString(),
  };
}

export const usePlanStore = create<PlanStore>((set, get) => ({
  plans: {},

  getPlan: (tabId) => get().plans[tabId] || defaultPlan(),

  setPlan: (tabId, planId, title, description, items, snapshot = false, meta) => {
    set(s => {
      const plan = s.plans[tabId] || defaultPlan();
      const isSamePlan = plan.planId === planId;
      const base = isSamePlan ? plan.versions : [];
      const mergedItems = mergePlanItems(isSamePlan ? plan.items : [], items);
      const versions = meta?.versions
        ? meta.versions
        : snapshot
          ? [...base, snapshotVersion(mergedItems)].slice(-20)
          : base;
      const nextPlan: TabPlanState = {
        ...plan,
        planId,
        title,
        description,
        items: mergedItems,
        versions,
        artifact: hasMetaKey(meta, 'artifact') ? meta?.artifact ?? undefined : (isSamePlan ? plan.artifact : undefined),
        finished: hasMetaKey(meta, 'finished') ? meta?.finished ?? undefined : (isSamePlan ? plan.finished : undefined),
        executionStatus: hasMetaKey(meta, 'executionStatus') ? meta?.executionStatus ?? undefined : (isSamePlan ? plan.executionStatus : undefined),
        approvalStatus: hasMetaKey(meta, 'approvalStatus') ? meta?.approvalStatus ?? undefined : (isSamePlan ? plan.approvalStatus : undefined),
        activeItemId: hasMetaKey(meta, 'activeItemId') ? meta?.activeItemId ?? undefined : (isSamePlan ? plan.activeItemId : undefined),
        approvalRequestedAt: hasMetaKey(meta, 'approvalRequestedAt') ? meta?.approvalRequestedAt ?? undefined : (isSamePlan ? plan.approvalRequestedAt : undefined),
        approvedAt: hasMetaKey(meta, 'approvedAt') ? meta?.approvedAt ?? undefined : (isSamePlan ? plan.approvedAt : undefined),
        executionStartedAt: hasMetaKey(meta, 'executionStartedAt') ? meta?.executionStartedAt ?? undefined : (isSamePlan ? plan.executionStartedAt : undefined),
        executionUpdatedAt: hasMetaKey(meta, 'executionUpdatedAt') ? meta?.executionUpdatedAt ?? undefined : (isSamePlan ? plan.executionUpdatedAt : undefined),
      };
      if (!snapshot && samePlanState(plan, nextPlan)) return s;
      return { plans: { ...s.plans, [tabId]: nextPlan } };
    });
  },

  updateItem: (tabId, itemId, updates) => {
    set(s => {
      const plan = s.plans[tabId] || defaultPlan();
      let changed = false;
      const items = plan.items.map(item => {
        if (item.id !== itemId) return item;
        const nextItem = { ...item, ...updates };
        if (shallowRecordEqual(item as unknown as Record<string, unknown>, nextItem as unknown as Record<string, unknown>)) {
          return item;
        }
        changed = true;
        return nextItem;
      });
      if (!changed) return s;
      const versions = updates.text !== undefined || updates.notes !== undefined || updates.section !== undefined
        ? [...plan.versions, snapshotVersion(items)].slice(-20)
        : plan.versions;
      return { plans: { ...s.plans, [tabId]: { ...plan, items, versions } } };
    });
  },

  addItems: (tabId, newItems) => {
    set(s => {
      if (newItems.length === 0) return s;
      const plan = s.plans[tabId] || defaultPlan();
      const items = [...plan.items, ...newItems];
      // No snapshot here — addItems is used for optimistic previews with pending_ IDs.
      // The confirmed ITEMS_ADDED: result calls setPlan(..., true) which snapshots.
      return { plans: { ...s.plans, [tabId]: { ...plan, items } } };
    });
  },

  removeItem: (tabId, itemId) => {
    set(s => {
      const plan = s.plans[tabId] || defaultPlan();
      const items = plan.items.filter(i => i.id !== itemId);
      if (items.length === plan.items.length) return s;
      return { plans: { ...s.plans, [tabId]: { ...plan, items } } };
    });
  },

  clearPlan: (tabId) => {
    set(s => {
      const plan = s.plans[tabId];
      if (!plan || samePlanState(plan, defaultPlan())) return s;
      return { plans: { ...s.plans, [tabId]: defaultPlan() } };
    });
  },

  setFinished: (tabId, finished) => {
    set(s => {
      const plan = s.plans[tabId] || defaultPlan();
      if (plan.finished === finished) return s;
      return { plans: { ...s.plans, [tabId]: { ...plan, finished } } };
    });
  },

  setExecutionMeta: (tabId, meta) => {
    set(s => {
      const plan = s.plans[tabId] || defaultPlan();
      const nextPlan: TabPlanState = {
        ...plan,
        finished: hasMetaKey(meta, 'finished') ? meta.finished ?? undefined : plan.finished,
        executionStatus: hasMetaKey(meta, 'executionStatus') ? meta.executionStatus ?? undefined : plan.executionStatus,
        approvalStatus: hasMetaKey(meta, 'approvalStatus') ? meta.approvalStatus ?? undefined : plan.approvalStatus,
        activeItemId: hasMetaKey(meta, 'activeItemId') ? meta.activeItemId ?? undefined : plan.activeItemId,
        approvalRequestedAt: hasMetaKey(meta, 'approvalRequestedAt') ? meta.approvalRequestedAt ?? undefined : plan.approvalRequestedAt,
        approvedAt: hasMetaKey(meta, 'approvedAt') ? meta.approvedAt ?? undefined : plan.approvedAt,
        executionStartedAt: hasMetaKey(meta, 'executionStartedAt') ? meta.executionStartedAt ?? undefined : plan.executionStartedAt,
        executionUpdatedAt: hasMetaKey(meta, 'executionUpdatedAt') ? meta.executionUpdatedAt ?? undefined : plan.executionUpdatedAt,
      };
      if (samePlanState(plan, nextPlan)) return s;
      return { plans: { ...s.plans, [tabId]: nextPlan } };
    });
  },

  setStreamingIds: (tabId, itemIds) => {
    set(s => {
      const plan = s.plans[tabId] || defaultPlan();
      const streamingItemIds = new Set(itemIds);
      if (sameStringSet(plan.streamingItemIds, streamingItemIds)) return s;
      return { plans: { ...s.plans, [tabId]: { ...plan, streamingItemIds } } };
    });
  },

  clearStreamingIds: (tabId) => {
    set(s => {
      const plan = s.plans[tabId] || defaultPlan();
      if (!plan.streamingItemIds) return s;
      return { plans: { ...s.plans, [tabId]: { ...plan, streamingItemIds: undefined } } };
    });
  },

  // Legacy compat: used by old TodoWrite / flamel_write_plan handlers
  updatePlan: (tabId, items, title) => {
    set(s => {
      const plan = s.plans[tabId] || defaultPlan();
      const version = snapshotVersion(items);
      const versions = [...plan.versions, version].slice(-20);
      return {
        plans: {
          ...s.plans,
          [tabId]: {
            ...plan,
            items,
            title: title ?? plan.title,
            versions,
          },
        },
      };
    });
  },
}));
