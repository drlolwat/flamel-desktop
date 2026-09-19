import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import { useAuthTokenStore } from './authTokenStore';

export type ConnectionMode = 'local' | 'remote';

interface ConnectionState {
  mode: ConnectionMode | null; // null = not yet chosen (show splash)
  remoteUrl: string;
  localApiBase: string;
  connected: boolean;
  flamelSignedIn: boolean;
  setFlamelSignedIn: (signedIn: boolean) => void;
  setMode: (mode: ConnectionMode) => void;
  setRemoteUrl: (url: string) => void;
  setLocalApiBase: (url: string) => void;
  setConnected: (connected: boolean) => void;
  disconnect: () => void;
  /** Returns the base URL for API calls — empty string for local, the remote URL for remote */
  getBaseUrl: () => string;
}

export const useConnectionStore = create<ConnectionState>()(
  persist(
    (set, get) => ({
      mode: null,
      remoteUrl: '',
      localApiBase: '',
      connected: false,
      flamelSignedIn: false,
      setFlamelSignedIn: (flamelSignedIn) => set({ flamelSignedIn }),
      setMode: (mode) => set({ mode }),
      setRemoteUrl: (url) => set({ remoteUrl: url.replace(/\/+$/, '') }),
      setLocalApiBase: (url) => set({ localApiBase: url.replace(/\/+$/, '') }),
      setConnected: (connected) => set({ connected }),
      disconnect: () => {
        const { remoteUrl } = get();
        if (remoteUrl) useAuthTokenStore.getState().clearToken(remoteUrl);
        set({ mode: null, connected: false });
      },
      getBaseUrl: () => {
        const { mode, remoteUrl, localApiBase } = get();
        return mode === 'remote' ? remoteUrl : localApiBase;
      },
    }),
    {
      name: 'flamel-connection',
      partialize: (s) => ({ mode: s.mode, remoteUrl: s.remoteUrl }),
    }
  )
);

/** Get the current API base URL (non-reactive, for use outside React) */
export function getApiBase(): string {
  return useConnectionStore.getState().getBaseUrl();
}
