import type { ReactNode } from "react";
/** Private hosted account management is injected by the SaaS shell. */
export interface WorkspaceHost {
  capabilities?: { sharing?: boolean };
  panels?: Partial<Record<'admin'|'runners'|'assistant'|'runnerPicker', (initialTab?:string)=>ReactNode>>;
  logout(): void | Promise<void>;
}
let host: WorkspaceHost = {
  async logout() {
    if ("__TAURI_INTERNALS__" in window) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("remote_logout");
    }
    window.location.reload();
  },
};
export function configureWorkspaceHost(value: WorkspaceHost): void {
  host = value;
}
export function logoutHost(): void | Promise<void> {
  return host.logout();
}

export function hasHostPanel(name:'admin'|'runners'|'assistant'|'runnerPicker'):boolean { return Boolean(host.panels?.[name]); }
export function renderHostPanel(name:'admin'|'runners'|'assistant'|'runnerPicker',initialTab?:string):ReactNode { return host.panels?.[name]?.(initialTab) ?? null; }

export function supportsSharing():boolean { return host.capabilities?.sharing === true; }
