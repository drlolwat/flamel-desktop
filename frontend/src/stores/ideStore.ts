import { create } from 'zustand';
import { createJSONStorage, persist } from 'zustand/middleware';
import type { WorkbenchDiagnostic } from '@flamel/shared';

export interface IdeFile {
  path: string;
  content: string;
  savedContent: string;
  language: string;
  revision?: string;
}

interface IdeStore {
  files: IdeFile[];
  activeFilePath: string | null;
  diagnostics: Record<string, WorkbenchDiagnostic[]>;
  revealLocation: { path: string; line: number; column: number } | null;
  openFile: (path: string, content: string, revision?: string) => void;
  closeFile: (path: string) => void;
  closeAll: () => void;
  updateContent: (path: string, content: string) => void;
  markSaved: (path: string, content: string, revision?: string) => void;
  acceptServerVersion: (path: string, content: string, revision: string) => void;
  setActiveFile: (path: string) => void;
  setDiagnostics: (path: string, diagnostics: WorkbenchDiagnostic[]) => void;
  requestReveal: (path: string, line: number, column?: number) => void;
  clearReveal: () => void;
}

const MAX_PERSISTED_IDE_FILES = 24;
const MAX_PERSISTED_FILE_BYTES = 512_000;

const memoryStorage = (() => {
  const values = new Map<string, string>();
  return {
    getItem: (name: string) => values.get(name) ?? null,
    setItem: (name: string, value: string) => { values.set(name, value); },
    removeItem: (name: string) => { values.delete(name); },
  };
})();

const ideStorage = createJSONStorage(() => {
  if (typeof window === 'undefined' || !window.localStorage) return memoryStorage;
  return {
    getItem: (name: string) => {
      try { return window.localStorage.getItem(name); }
      catch { return null; }
    },
    setItem: (name: string, value: string) => {
      try { window.localStorage.setItem(name, value); }
      catch { /* quota exceeded - open files are still kept in memory */ }
    },
    removeItem: (name: string) => {
      try { window.localStorage.removeItem(name); }
      catch { /* ignore */ }
    },
  };
});

function getLang(filename: string): string {
  const base = filename.split('/').pop()?.toLowerCase() || '';
  if (base === 'dockerfile' || base.startsWith('dockerfile.')) return 'dockerfile';
  if (base === 'makefile' || base === 'gnumakefile') return 'shell';
  if (base === '.env' || base.startsWith('.env.')) return 'ini';
  const ext = filename.split('.').pop()?.toLowerCase() || '';
  const map: Record<string, string> = {
    ts: 'typescript', tsx: 'tsx', js: 'javascript', jsx: 'jsx',
    py: 'python', rs: 'rust', go: 'go', java: 'java', kt: 'kotlin',
    css: 'css', scss: 'scss', html: 'html', json: 'json', yaml: 'yaml',
    yml: 'yaml', md: 'markdown', sh: 'shell', bash: 'shell', zsh: 'shell',
    toml: 'ini', sql: 'sql', graphql: 'graphql', vue: 'html',
    svelte: 'html', c: 'cpp', cpp: 'cpp', cs: 'csharp', rb: 'ruby',
    php: 'php', swift: 'swift', dart: 'dart', lua: 'lua',
    xml: 'xml', svg: 'xml', plist: 'xml', ini: 'ini', cfg: 'ini', conf: 'ini',
    properties: 'ini', gradle: 'java', groovy: 'java', scala: 'scala',
    r: 'r', pl: 'perl', pm: 'perl', ps1: 'powershell', psm1: 'powershell',
    m: 'objective-c', mm: 'objective-c', h: 'cpp', hpp: 'cpp', cc: 'cpp', cxx: 'cpp',
    less: 'less', hbs: 'handlebars', handlebars: 'handlebars', pug: 'pug', jade: 'pug',
    tf: 'hcl', hcl: 'hcl', proto: 'protobuf', sol: 'solidity', ex: 'elixir', exs: 'elixir',
    fs: 'fsharp', fsx: 'fsharp', vb: 'vb', coffee: 'javascript', wasm: 'plaintext',
  };
  return map[ext] || 'plaintext';
}

function activeFrom(files: IdeFile[], activeFilePath: string | null | undefined): string | null {
  if (activeFilePath && files.some(file => file.path === activeFilePath)) return activeFilePath;
  return files[files.length - 1]?.path ?? null;
}

function normalizeIdeFile(value: unknown): IdeFile | null {
  if (!value || typeof value !== 'object') return null;
  const candidate = value as Partial<IdeFile>;
  if (typeof candidate.path !== 'string' || candidate.path.length === 0) return null;
  if (typeof candidate.content !== 'string' || typeof candidate.savedContent !== 'string') return null;
  return {
    path: candidate.path,
    content: candidate.content,
    savedContent: candidate.savedContent,
    language: getLang(candidate.path),
    revision: typeof candidate.revision === 'string' ? candidate.revision : undefined,
  };
}

function normalizeIdeFiles(files: unknown): IdeFile[] {
  if (!Array.isArray(files)) return [];
  const byPath = new Map<string, IdeFile>();
  for (const raw of files) {
    const file = normalizeIdeFile(raw);
    if (file) byPath.set(file.path, file);
  }
  return Array.from(byPath.values()).slice(-MAX_PERSISTED_IDE_FILES);
}

function persistableIdeFiles(files: IdeFile[]): IdeFile[] {
  return files
    .filter(file => file.content.length <= MAX_PERSISTED_FILE_BYTES && file.savedContent.length <= MAX_PERSISTED_FILE_BYTES)
    .slice(-MAX_PERSISTED_IDE_FILES);
}

export const useIdeStore = create<IdeStore>()(
  persist(
    (set, get) => ({
      files: [],
      activeFilePath: null,
      diagnostics: {},
      revealLocation: null,

      openFile: (path, content, revision) => {
        const existing = get().files.find(f => f.path === path);
        if (existing) {
          set(s => ({
            files: s.files.map(f => {
              if (f.path !== path) return f;
              const hasUnsavedChanges = f.content !== f.savedContent;
              return {
                ...f,
                content: hasUnsavedChanges ? f.content : content,
                savedContent: content,
                language: getLang(path),
                revision: revision ?? f.revision,
              };
            }),
            activeFilePath: path,
          }));
          return;
        }
        const file: IdeFile = { path, content, savedContent: content, language: getLang(path), revision };
        set(s => ({ files: [...s.files, file], activeFilePath: path }));
      },

      closeFile: (path) => {
        set(s => {
          const files = s.files.filter(f => f.path !== path);
          const diagnostics = { ...s.diagnostics };
          delete diagnostics[path];
          const activeFilePath = s.activeFilePath === path
            ? (files[files.length - 1]?.path ?? null)
            : activeFrom(files, s.activeFilePath);
          return { files, activeFilePath, diagnostics };
        });
      },

      updateContent: (path, content) => {
        set(s => ({
          files: s.files.map(f => f.path === path ? { ...f, content } : f),
        }));
      },

      markSaved: (path, content, revision) => {
        set(s => ({
          files: s.files.map(f => f.path === path ? { ...f, content, savedContent: content, revision: revision ?? f.revision } : f),
        }));
      },

      acceptServerVersion: (path, content, revision) => {
        set(s => ({
          files: s.files.map(f => f.path === path ? { ...f, content, savedContent: content, revision } : f),
        }));
      },

      setActiveFile: (path) => {
        set(s => ({ activeFilePath: activeFrom(s.files, path) }));
      },

      setDiagnostics: (path, diagnostics) => set(s => ({
        diagnostics: { ...s.diagnostics, [path]: diagnostics },
      })),

      requestReveal: (path, line, column = 1) => set(s => ({
        activeFilePath: activeFrom(s.files, path),
        revealLocation: {
          path,
          line: Math.max(1, Math.floor(line)),
          column: Math.max(1, Math.floor(column)),
        },
      })),

      clearReveal: () => set({ revealLocation: null }),

      closeAll: () => set({ files: [], activeFilePath: null, diagnostics: {}, revealLocation: null }),
    }),
    {
      name: 'flamel-ide',
      storage: ideStorage,
      partialize: (state) => {
        const files = persistableIdeFiles(state.files);
        return {
          files,
          activeFilePath: activeFrom(files, state.activeFilePath),
        };
      },
      merge: (persisted, current) => {
        const restored = persisted as Partial<IdeStore> | undefined;
        const files = normalizeIdeFiles(restored?.files);
        return {
          ...current,
          files,
          activeFilePath: activeFrom(files, restored?.activeFilePath),
        };
      },
    },
  ),
);
