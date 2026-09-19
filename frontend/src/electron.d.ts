// Type declarations for the Electron preload contextBridge API
interface DesktopNdjsonStreamRequest {
  streamId: string;
  url: string;
  method: string;
  headers: Record<string, string>;
  body?: string;
}

interface DesktopNdjsonStreamStart {
  status: number;
  ok: boolean;
  statusText: string;
}

interface DesktopNdjsonStreamEvent {
  streamId: string;
  kind: 'chunk' | 'done' | 'error';
  chunk?: number[];
  error?: string;
}

interface DesktopUpdateInfo {
  currentVersion: string;
  version: string;
  body?: string;
  date?: string;
}

interface ElectronAPI {
  isElectron: boolean;
  platform: string;
  openFolderDialog: () => Promise<string | null>;
  openPath: (filePath: string) => Promise<void>;
  openExternal: (url: string) => Promise<void>;
  navigate?: (url: string) => Promise<void>;
  getLocalApiBase: () => Promise<string>;
  startLocalServer?: () => Promise<string>;
  streamNdjson?: (request: DesktopNdjsonStreamRequest) => Promise<DesktopNdjsonStreamStart>;
  cancelNdjsonStream?: (streamId: string) => Promise<void>;
  onNdjsonStream?: (cb: (event: DesktopNdjsonStreamEvent) => void) => Promise<() => void>;
  getAutoLaunch: () => Promise<boolean>;
  setAutoLaunch: (enabled: boolean) => Promise<void>;
  getAppVersion?: () => Promise<string>;
  checkForUpdate?: () => Promise<DesktopUpdateInfo | null>;
  installUpdate?: () => Promise<void>;
  notify: (title: string, body: string) => Promise<void>;
  readClipboardImage?: () => Promise<{ data: string; mediaType: string; name: string } | null>;
  onOpenPath: (cb: (path: string) => void) => void;
  // Custom window controls (Linux/Windows frameless)
  minimize: () => Promise<void>;
  maximize: () => Promise<void>;
  closeWindow: () => Promise<void>;
  isMaximized: () => Promise<boolean>;
  onMaximizedChange: (cb: (maximized: boolean) => void) => void;
}

declare global {
  interface Window {
    electronAPI?: ElectronAPI;
  }
}

export {};

interface ImportMetaEnv {
  /** Set to "web" for web.flamel.dev or "local" for Flamel Desktop. */
  readonly VITE_MODE?: string;
}
