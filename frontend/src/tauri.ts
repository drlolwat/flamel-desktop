/**
 * Tauri v2 adapter — maps the ElectronAPI interface to Tauri invoke() calls.
 * Imported when no electronAPI is present (see main.tsx).
 */

export {}; // Make this a module

type DesktopNdjsonEventPayload = {
  streamId: string;
  kind: 'chunk' | 'done' | 'error';
  chunk?: number[];
  error?: string;
};

initTauri().catch(() => {
  // Not in Tauri — silently ignore
});

async function readTauriClipboardImage(): Promise<{ data: string; mediaType: string; name: string } | null> {
  try {
    const { readImage } = await import('@tauri-apps/plugin-clipboard-manager');
    const image = await readImage();
    const size = await image.size();
    const rgba = await image.rgba();
    const canvas = document.createElement('canvas');
    canvas.width = size.width;
    canvas.height = size.height;
    const ctx = canvas.getContext('2d');
    if (!ctx) return null;
    ctx.putImageData(new ImageData(new Uint8ClampedArray(rgba), size.width, size.height), 0, 0);
    const dataUrl = canvas.toDataURL('image/png');
    const data = dataUrl.split(',')[1];
    if (!data) return null;
    return { data, mediaType: 'image/png', name: 'clipboard.png' };
  } catch {
    return null;
  }
}

async function initTauri() {
  // Try importing Tauri core — if it fails, we're not in a Tauri webview
  const { invoke } = await import('@tauri-apps/api/core');
  const { listen } = await import('@tauri-apps/api/event');

  // Quick check: try a trivial invoke to confirm Tauri IPC is alive
  try {
    await invoke('cmd_is_maximized');
    await invoke('runtime_reset');
  } catch {
    // IPC not available — not actually in Tauri
    return;
  }

  let pendingUpdate: Awaited<ReturnType<(typeof import('@tauri-apps/plugin-updater'))['check']>> | null = null;

  const tauriAPI: Window['electronAPI'] = {
    isElectron: true,
    platform: navigator.platform.includes('Mac') ? 'darwin' : navigator.platform.includes('Win') ? 'win32' : 'linux',

    openFolderDialog: () => invoke<string | null>('cmd_open_folder_dialog'),
    openPath: (filePath: string) => invoke('cmd_open_path', { path: filePath }),
    openExternal: (url: string) => invoke('cmd_open_external', { url }),
    navigate: (url: string) => invoke('cmd_navigate', { url }),
    getLocalApiBase: () => invoke<string>('cmd_get_local_api_base'),
    startLocalServer: () => invoke<string>('cmd_start_local_server'),
    getAutoLaunch: async () => false,
    setAutoLaunch: async (_enabled: boolean) => {},
    getAppVersion: async () => {
      const { getVersion } = await import('@tauri-apps/api/app');
      return getVersion();
    },
    checkForUpdate: async () => null,
    installUpdate: async () => { throw new Error('Automatic updates are not configured for this candidate'); },

    notify: (title: string, body: string) => invoke('cmd_notify', { title, body }),
    readClipboardImage: readTauriClipboardImage,

    onOpenPath: (cb: (path: string) => void) => {
      listen('protocol:open-path', (e) => cb(e.payload as string));
    },

    minimize: () => invoke('cmd_minimize'),
    maximize: () => invoke('cmd_maximize'),
    closeWindow: () => invoke('cmd_close_window'),
    isMaximized: () => invoke<boolean>('cmd_is_maximized'),
    onMaximizedChange: (cb: (maximized: boolean) => void) => {
      listen('window:maximized', (e) => cb(e.payload as boolean));
    },
  };

  (window as any).electronAPI = tauriAPI;
  window.dispatchEvent(new Event('tauri-ready'));

  // Remote pages open in the system browser; API artifacts use native transport.
  document.addEventListener('click', (event) => {
    const anchor=(event.target as HTMLElement).closest('a');
    const href=anchor?.getAttribute('href');
    if(!anchor||!href||href.startsWith('#')||href.startsWith('blob:'))return;
    const url=new URL(href,window.location.href);
    if(href.startsWith('/api/')) {
      event.preventDefault();
      void import('./lib/api').then(async({apiFetch})=>{
        const response=await apiFetch(href);if(!response.ok)throw new Error(`Download failed (${response.status})`);
        const objectUrl=URL.createObjectURL(await response.blob());
        const link=document.createElement('a');link.href=objectUrl;link.download=anchor.download||'flamel-artifact';link.click();
        setTimeout(()=>URL.revokeObjectURL(objectUrl),60_000);
      });
    } else if(['http:','https:'].includes(url.protocol)) {
      event.preventDefault();void invoke('cmd_open_external',{url:url.toString()});
    }
  });

  // Native titlebar dragging is provided by data-tauri-drag-region. Starting a
  // second drag/maximize cycle from DOM events makes Windows process a
  // double-click twice and can strand the frameless window minimized.
}
