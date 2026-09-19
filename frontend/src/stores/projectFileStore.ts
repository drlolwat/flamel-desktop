import { create } from 'zustand';
import { createJSONStorage, persist } from 'zustand/middleware';
import type { FileNode, FileReference, GitFileStatus, GitStatus } from '@flamel/shared';
import { api } from '../lib/api';

const memoryStorage = (() => {
  const values = new Map<string, string>();
  return {
    getItem: (name: string) => values.get(name) ?? null,
    setItem: (name: string, value: string) => { values.set(name, value); },
    removeItem: (name: string) => { values.delete(name); },
  };
})();

// Keep only the latest request per project; a slow old response must not replace
// the branch/index snapshot the user is about to commit.
const gitRequests = new Map<string, AbortController>();

const projectFileStorage = createJSONStorage(() => {
  if (typeof window === 'undefined' || !window.localStorage) return memoryStorage;
  return window.localStorage;
});

export type FileSortMode = 'name' | 'type';

export interface ProjectFileViewState {
  expandedPaths: string[];
  selectedPath: string | null;
  previewPath: string | null;
  search: string;
  showHidden: boolean;
  sortMode: FileSortMode;
  scrollTop: number;
  lastOpenedPath: string | null;
}

export interface ProjectFileMutationMeta {
  path: string;
  source: 'tool_result' | 'workflow_event';
  toolName?: string;
  action?: string;
  eventKind?: string;
  updatedAt: number;
}

export interface ProjectFileRuntimeState {
  tree: FileNode[];
  gitStatus: GitStatus | null;
  loading: boolean;
  gitLoading: boolean;
  error: string | null;
  gitError: string | null;
  loadedAt: number | null;
  gitLoadedAt: number | null;
  loadedShowHidden: boolean | null;
  recentMutations: Record<string, ProjectFileMutationMeta>;
}

interface ProjectFileStore {
  projects: Record<string, ProjectFileRuntimeState>;
  views: Record<string, ProjectFileViewState>;
  ensureProject: (projectPath: string) => void;
  loadProject: (projectPath: string, options?: { force?: boolean }) => Promise<void>;
  refreshGitStatus: (projectPath: string) => Promise<void>;
  recordFileMutation: (projectPath: string, paths: string[], meta: Omit<ProjectFileMutationMeta, 'path' | 'updatedAt'>) => void;
  toggleExpanded: (projectPath: string, path: string) => void;
  setExpanded: (projectPath: string, path: string, expanded: boolean) => void;
  selectPath: (projectPath: string, path: string | null) => void;
  setPreviewPath: (projectPath: string, path: string | null) => void;
  setSearch: (projectPath: string, search: string) => void;
  setShowHidden: (projectPath: string, showHidden: boolean) => void;
  setSortMode: (projectPath: string, sortMode: FileSortMode) => void;
  setScrollTop: (projectPath: string, scrollTop: number) => void;
  recordOpen: (projectPath: string, path: string) => void;
}

export const DEFAULT_PROJECT_FILE_VIEW: ProjectFileViewState = {
  expandedPaths: [],
  selectedPath: null,
  previewPath: null,
  search: '',
  showHidden: false,
  sortMode: 'name',
  scrollTop: 0,
  lastOpenedPath: null,
};

export const DEFAULT_PROJECT_FILE_RUNTIME: ProjectFileRuntimeState = {
  tree: [],
  gitStatus: null,
  loading: false,
  gitLoading: false,
  error: null,
  gitError: null,
  loadedAt: null,
  gitLoadedAt: null,
  loadedShowHidden: null,
  recentMutations: {},
};

function ensureView(view?: Partial<ProjectFileViewState>): ProjectFileViewState {
  return { ...DEFAULT_PROJECT_FILE_VIEW, ...view };
}

function ensureRuntime(runtime?: Partial<ProjectFileRuntimeState>): ProjectFileRuntimeState {
  return { ...DEFAULT_PROJECT_FILE_RUNTIME, ...runtime };
}

function updateView(
  state: Pick<ProjectFileStore, 'views'>,
  projectPath: string,
  updater: (view: ProjectFileViewState) => ProjectFileViewState,
): Record<string, ProjectFileViewState> {
  const current = ensureView(state.views[projectPath]);
  const next = updater(current);
  if (next === current) return state.views;
  return { ...state.views, [projectPath]: next };
}

function uniq(items: string[]): string[] {
  return Array.from(new Set(items.filter(Boolean)));
}

export function getProjectFileView(views: Record<string, ProjectFileViewState>, projectPath?: string | null): ProjectFileViewState {
  if (!projectPath) return DEFAULT_PROJECT_FILE_VIEW;
  return views[projectPath] ?? DEFAULT_PROJECT_FILE_VIEW;
}

export function getProjectFileRuntime(projects: Record<string, ProjectFileRuntimeState>, projectPath?: string | null): ProjectFileRuntimeState {
  if (!projectPath) return DEFAULT_PROJECT_FILE_RUNTIME;
  return projects[projectPath] ?? DEFAULT_PROJECT_FILE_RUNTIME;
}

export function relativeFilePath(projectPath: string, path: string): string {
  const normalizedProject = projectPath.replace(/\\/g, '/').replace(/\/$/, '');
  const normalizedPath = path.replace(/\\/g, '/');
  if (normalizedPath === normalizedProject) return '';
  if (normalizedPath.startsWith(`${normalizedProject}/`)) {
    return normalizedPath.slice(normalizedProject.length + 1);
  }
  return path;
}

export function createFileReference(projectPath: string, node: FileNode, source: FileReference['source'] = 'tree'): FileReference {
  return {
    projectPath,
    path: node.path,
    relativePath: relativeFilePath(projectPath, node.path),
    name: node.name,
    kind: node.type,
    source,
  };
}

export function parentPaths(path: string): string[] {
  const normalized = path.replace(/\\/g, '/');
  const parts = normalized.split('/').filter(Boolean);
  if (parts.length <= 1) return [];
  const parents: string[] = [];
  const absolutePrefix = normalized.startsWith('/') ? '/' : '';
  for (let i = 1; i < parts.length; i += 1) {
    parents.push(`${absolutePrefix}${parts.slice(0, i).join('/')}`);
  }
  return parents;
}

export function collectFiles(nodes: FileNode[]): FileNode[] {
  const files: FileNode[] = [];
  const visit = (node: FileNode) => {
    if (node.type === 'file') files.push(node);
    node.children?.forEach(visit);
  };
  nodes.forEach(visit);
  return files;
}

function numericOr(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

export function normalizeGitStatus(status: Partial<GitStatus> | null | undefined): GitStatus {
  const files = Array.isArray(status?.files) ? status.files : [];
  const added = numericOr(
    status?.added,
    files.reduce((sum, file) => sum + numericOr(file.added, numericOr(file.stagedAdded, 0) + numericOr(file.unstagedAdded, 0)), 0),
  );
  const removed = numericOr(
    status?.removed,
    files.reduce((sum, file) => sum + numericOr(file.removed, numericOr(file.stagedRemoved, 0) + numericOr(file.unstagedRemoved, 0)), 0),
  );
  const uncommittedCount = numericOr(status?.uncommittedCount, files.length);
  const branch = typeof status?.branch === 'string' ? status.branch : '';
  const isRepo = typeof status?.isRepo === 'boolean' ? status.isRepo : Boolean(branch || files.length);

  return { ...(status ?? {}), branch, files, added, removed, uncommittedCount, isRepo } as GitStatus;
}

export function gitStatusByPath(status: GitStatus | null, projectPath: string): Record<string, GitFileStatus> {
  if (!status?.files?.length) return {};
  const byPath: Record<string, GitFileStatus> = {};
  for (const file of status.files) {
    byPath[file.path] = file;
    byPath[`${projectPath.replace(/\/$/, '')}/${file.path}`] = file;
  }
  return byPath;
}

function mutationKeys(projectPath: string, path: string): string[] {
  const normalizedProject = projectPath.replace(/\/$/, '');
  const normalizedPath = path.replace(/\\/g, '/');
  const relativePath = relativeFilePath(projectPath, normalizedPath);
  return uniq([
    normalizedPath,
    relativePath,
    `${normalizedProject}/${relativePath}`,
  ]);
}

export function sortFileNodes(nodes: FileNode[], mode: FileSortMode): FileNode[] {
  return [...nodes].sort((a, b) => {
    if (mode === 'type') {
      if (a.type === 'directory' && b.type !== 'directory') return -1;
      if (a.type !== 'directory' && b.type === 'directory') return 1;
    }
    return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
  });
}

export function filterFileTree(nodes: FileNode[], query: string, sortMode: FileSortMode): FileNode[] {
  const q = query.trim().toLowerCase();
  const filterNode = (node: FileNode): FileNode | null => {
    const matches = !q || node.name.toLowerCase().includes(q) || node.path.toLowerCase().includes(q);
    const children = node.children
      ? sortFileNodes(node.children, sortMode).map(filterNode).filter((child): child is FileNode => Boolean(child))
      : undefined;
    if (!matches && (!children || children.length === 0)) return null;
    return children ? { ...node, children } : node;
  };
  return sortFileNodes(nodes, sortMode).map(filterNode).filter((node): node is FileNode => Boolean(node));
}

export const useProjectFileStore = create<ProjectFileStore>()(
  persist(
    (set, get) => ({
      projects: {},
      views: {},

      ensureProject: (projectPath) => {
        set((state) => {
          if (state.projects[projectPath] && state.views[projectPath]) return state;
          return {
            projects: state.projects[projectPath]
              ? state.projects
              : { ...state.projects, [projectPath]: ensureRuntime() },
            views: state.views[projectPath]
              ? state.views
              : { ...state.views, [projectPath]: ensureView() },
          };
        });
      },

      loadProject: async (projectPath, options) => {
        get().ensureProject(projectPath);
        const view = ensureView(get().views[projectPath]);
        const runtime = ensureRuntime(get().projects[projectPath]);
        if (!options?.force && runtime.loadedAt && runtime.loadedShowHidden === view.showHidden) return;

        set((state) => ({
          projects: {
            ...state.projects,
            [projectPath]: { ...ensureRuntime(state.projects[projectPath]), loading: true, error: null },
          },
        }));

        try {
          const result = await api.files(projectPath, view.showHidden);
          set((state) => ({
            projects: {
              ...state.projects,
              [projectPath]: {
                ...ensureRuntime(state.projects[projectPath]),
                tree: result.tree,
                loading: false,
                error: null,
                loadedAt: Date.now(),
                loadedShowHidden: view.showHidden,
              },
            },
          }));
        } catch (error) {
          set((state) => ({
            projects: {
              ...state.projects,
              [projectPath]: {
                ...ensureRuntime(state.projects[projectPath]),
                loading: false,
                error: error instanceof Error ? error.message : 'Failed to load project files',
              },
            },
          }));
        }
      },

      refreshGitStatus: async (projectPath) => {
        gitRequests.get(projectPath)?.abort();
        const request = new AbortController();
        gitRequests.set(projectPath, request);
        get().ensureProject(projectPath);
        set((state) => ({
          projects: {
            ...state.projects,
            [projectPath]: { ...ensureRuntime(state.projects[projectPath]), gitLoading: true, gitError: null },
          },
        }));

        try {
          const gitStatus = normalizeGitStatus(await api.git.status(projectPath, request.signal));
          if (gitRequests.get(projectPath) !== request) return;
          set((state) => ({
            projects: {
              ...state.projects,
              [projectPath]: {
                ...ensureRuntime(state.projects[projectPath]),
                gitStatus,
                gitLoading: false,
                gitError: null,
                gitLoadedAt: Date.now(),
              },
            },
          }));
        } catch (error) {
          if (gitRequests.get(projectPath) !== request || request.signal.aborted) return;
          set((state) => ({
            projects: {
              ...state.projects,
              [projectPath]: {
                ...ensureRuntime(state.projects[projectPath]),
                gitLoading: false,
                gitError: error instanceof Error ? error.message : 'Failed to load git status',
              },
            },
          }));
        } finally {
          if (gitRequests.get(projectPath) === request) gitRequests.delete(projectPath);
        }
      },

      recordFileMutation: (projectPath, paths, meta) => {
        if (!paths.length) return;
        get().ensureProject(projectPath);
        const updatedAt = Date.now();
        set((state) => {
          const runtime = ensureRuntime(state.projects[projectPath]);
          const recentMutations = { ...runtime.recentMutations };
          for (const path of paths) {
            for (const key of mutationKeys(projectPath, path)) {
              recentMutations[key] = { ...meta, path: key, updatedAt };
            }
          }
          return {
            projects: {
              ...state.projects,
              [projectPath]: { ...runtime, recentMutations },
            },
          };
        });
      },

      toggleExpanded: (projectPath, path) => {
        set((state) => ({
          views: updateView(state, projectPath, (view) => {
            const isExpanded = view.expandedPaths.includes(path);
            return {
              ...view,
              expandedPaths: isExpanded
                ? view.expandedPaths.filter((item) => item !== path)
                : uniq([...view.expandedPaths, path]),
            };
          }),
        }));
      },

      setExpanded: (projectPath, path, expanded) => {
        set((state) => ({
          views: updateView(state, projectPath, (view) => {
            const hasPath = view.expandedPaths.includes(path);
            if (expanded === hasPath) return view;
            return {
              ...view,
              expandedPaths: expanded
                ? uniq([...view.expandedPaths, path])
                : view.expandedPaths.filter((item) => item !== path),
            };
          }),
        }));
      },

      selectPath: (projectPath, path) => {
        set((state) => ({
          views: updateView(state, projectPath, (view) => ({
            ...view,
            selectedPath: path,
            expandedPaths: path ? uniq([...view.expandedPaths, ...parentPaths(path)]) : view.expandedPaths,
          })),
        }));
      },

      setPreviewPath: (projectPath, path) => {
        set((state) => ({ views: updateView(state, projectPath, (view) => ({ ...view, previewPath: path })) }));
      },

      setSearch: (projectPath, search) => {
        set((state) => ({ views: updateView(state, projectPath, (view) => ({ ...view, search })) }));
      },

      setShowHidden: (projectPath, showHidden) => {
        set((state) => ({ views: updateView(state, projectPath, (view) => ({ ...view, showHidden })) }));
      },

      setSortMode: (projectPath, sortMode) => {
        set((state) => ({ views: updateView(state, projectPath, (view) => ({ ...view, sortMode })) }));
      },

      setScrollTop: (projectPath, scrollTop) => {
        set((state) => ({ views: updateView(state, projectPath, (view) => ({ ...view, scrollTop })) }));
      },

      recordOpen: (projectPath, path) => {
        set((state) => ({
          views: updateView(state, projectPath, (view) => ({
            ...view,
            selectedPath: path,
            previewPath: path,
            lastOpenedPath: path,
            expandedPaths: uniq([...view.expandedPaths, ...parentPaths(path)]),
          })),
        }));
      },
    }),
    {
      name: 'flamel-project-files',
      storage: projectFileStorage,
      partialize: (state) => ({ views: state.views, projects: {} }),
      merge: (persisted, current) => {
        const restored = persisted as Partial<ProjectFileStore> | undefined;
        return {
          ...current,
          views: Object.fromEntries(
            Object.entries(restored?.views ?? {}).map(([projectPath, view]) => [projectPath, ensureView(view)]),
          ),
          projects: {},
        };
      },
    },
  ),
);
