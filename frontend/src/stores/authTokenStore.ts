import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';

interface AuthTokenState {
  // JWT per server base URL (e.g. "https://flamel.lolwat.net" -> "eyJ...")
  tokens: Record<string, string>;
  setToken: (serverUrl: string, token: string) => void;
  getToken: (serverUrl: string) => string | undefined;
  clearToken: (serverUrl: string) => void;
}

export const useAuthTokenStore = create<AuthTokenState>()(
  persist(
    (set, get) => ({
      tokens: {},
      setToken: (serverUrl, token) =>
        set(s => ({ tokens: { ...s.tokens, [serverUrl]: token } })),
      getToken: (serverUrl) => get().tokens[serverUrl],
      clearToken: (serverUrl) =>
        set(s => { const t = { ...s.tokens }; delete t[serverUrl]; return { tokens: t }; }),
    }),
    { name: 'flamel-auth-tokens', storage: createJSONStorage(() => {
      if(typeof window!=='undefined' && '__TAURI_INTERNALS__' in window) {
        localStorage.removeItem('flamel-auth-tokens');
        return {getItem:()=>null,setItem:()=>{},removeItem:()=>{}};
      }
      return localStorage;
    }) }
  )
);

/** Non-reactive getter for use outside React (e.g. in apiFetch) */
export function getAuthToken(serverUrl: string): string | undefined {
  return useAuthTokenStore.getState().getToken(serverUrl);
}

export function setAuthToken(serverUrl: string, token: string) {
  useAuthTokenStore.getState().setToken(serverUrl, token);
}
