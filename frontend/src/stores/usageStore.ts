import { create } from 'zustand';
import type { UsageStats } from '@flamel/shared';
import { apiFetch } from '../lib/api';

interface UsageStore {
  stats: UsageStats | null;
  loading: boolean;
  fetch: () => Promise<void>;
}

export const useUsageStore = create<UsageStore>((set) => ({
  stats: null,
  loading: false,
  fetch: async () => {
    set({ loading: true });
    try {
      const res = await apiFetch('/api/usage');
      if (res.ok) {
        set({ stats: await res.json() });
      }
    } finally {
      set({ loading: false });
    }
  },
}));
