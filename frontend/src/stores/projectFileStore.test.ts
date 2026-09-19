import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../lib/api';
import type { FileNode, GitStatus } from '@flamel/shared';
import {
  filterFileTree,
  getProjectFileRuntime,
  getProjectFileView,
  gitStatusByPath,
  normalizeGitStatus,
  relativeFilePath,
  useProjectFileStore,
} from './projectFileStore';

const PROJECT = '/workspace/flamel';

const tree: FileNode[] = [
  {
    name: 'src',
    path: `${PROJECT}/src`,
    type: 'directory',
    children: [
      { name: 'App.tsx', path: `${PROJECT}/src/App.tsx`, type: 'file' },
      { name: 'stores', path: `${PROJECT}/src/stores`, type: 'directory', children: [
        { name: 'projectFileStore.ts', path: `${PROJECT}/src/stores/projectFileStore.ts`, type: 'file' },
      ] },
    ],
  },
  { name: 'README.md', path: `${PROJECT}/README.md`, type: 'file' },
];

describe('project file store', () => {
  afterEach(() => vi.restoreAllMocks());
  beforeEach(() => {
    useProjectFileStore.setState({ projects: {}, views: {} });
  });

  it('returns stable default snapshots before a project is ensured', () => {
    const state = useProjectFileStore.getState();

    expect(getProjectFileView(state.views, PROJECT)).toBe(getProjectFileView(state.views, PROJECT));
    expect(getProjectFileRuntime(state.projects, PROJECT)).toBe(getProjectFileRuntime(state.projects, PROJECT));
  });

  it('keeps expansion and selected path outside the file tree component lifecycle', () => {
    const file = `${PROJECT}/src/stores/projectFileStore.ts`;

    useProjectFileStore.getState().selectPath(PROJECT, file);

    const view = getProjectFileView(useProjectFileStore.getState().views, PROJECT);
    expect(view.selectedPath).toBe(file);
    expect(view.expandedPaths).toContain(PROJECT);
    expect(view.expandedPaths).toContain(`${PROJECT}/src`);
    expect(view.expandedPaths).toContain(`${PROJECT}/src/stores`);
  });

  it('records opened files as selected preview state for panel reopen', () => {
    const file = `${PROJECT}/src/stores/projectFileStore.ts`;

    useProjectFileStore.getState().recordOpen(PROJECT, file);

    const view = getProjectFileView(useProjectFileStore.getState().views, PROJECT);
    expect(view.selectedPath).toBe(file);
    expect(view.previewPath).toBe(file);
    expect(view.lastOpenedPath).toBe(file);
    expect(view.expandedPaths).toContain(`${PROJECT}/src/stores`);
  });

  it('toggles expanded folders idempotently', () => {
    const folder = `${PROJECT}/src`;

    useProjectFileStore.getState().toggleExpanded(PROJECT, folder);
    expect(getProjectFileView(useProjectFileStore.getState().views, PROJECT).expandedPaths).toContain(folder);

    useProjectFileStore.getState().toggleExpanded(PROJECT, folder);
    expect(getProjectFileView(useProjectFileStore.getState().views, PROJECT).expandedPaths).not.toContain(folder);
  });

  it('filters through parent directories for quick project navigation', () => {
    const filtered = filterFileTree(tree, 'projectfile', 'type');

    expect(filtered).toHaveLength(1);
    expect(filtered[0].name).toBe('src');
    expect(filtered[0].children?.[0].name).toBe('stores');
    expect(filtered[0].children?.[0].children?.[0].name).toBe('projectFileStore.ts');
  });

  it('maps git status by relative and absolute file paths', () => {
    const status: GitStatus = {
      branch: 'main',
      files: [{ status: 'M', path: 'src/App.tsx', added: 2, removed: 1 }],
      added: 2,
      removed: 1,
      uncommittedCount: 1,
      isRepo: true,
    };

    const byPath = gitStatusByPath(status, PROJECT);
    expect(byPath['src/App.tsx'].status).toBe('M');
    expect(byPath[`${PROJECT}/src/App.tsx`].status).toBe('M');
  });

  it('normalizes older git status payloads for live diff rendering', () => {
    const status = normalizeGitStatus({
      branch: 'main',
      files: [
        { status: 'M', path: 'src/App.tsx', added: 2, removed: 1 },
        { status: 'A', path: 'src/New.tsx', stagedAdded: 4, stagedRemoved: 0 },
      ],
    });

    expect(status.isRepo).toBe(true);
    expect(status.uncommittedCount).toBe(2);
    expect(status.added).toBe(6);
    expect(status.removed).toBe(1);
  });

  it('creates compact relative paths for project files', () => {
    expect(relativeFilePath(PROJECT, `${PROJECT}/src/App.tsx`)).toBe('src/App.tsx');
  });

  it('records recent file mutation metadata by relative and absolute path', () => {
    useProjectFileStore.getState().recordFileMutation(PROJECT, ['src/App.tsx'], {
      source: 'tool_result',
      toolName: 'file_edit',
      action: 'replace',
    });

    const runtime = getProjectFileRuntime(useProjectFileStore.getState().projects, PROJECT);
    expect(runtime.recentMutations['src/App.tsx']).toMatchObject({ toolName: 'file_edit', action: 'replace' });
    expect(runtime.recentMutations[`${PROJECT}/src/App.tsx`]).toMatchObject({ toolName: 'file_edit', action: 'replace' });
  });

  it('does not let an older Git response replace the current review revision', async () => {
    let finishOld!: (status: GitStatus) => void;
    const old = new Promise<GitStatus>(resolve => { finishOld = resolve; });
    const snapshot: GitStatus = { isRepo: true, branch: 'main', files: [], added: 0, removed: 0, uncommittedCount: 0, revision: 'new' };
    vi.spyOn(api.git, 'status').mockReturnValueOnce(old).mockResolvedValueOnce(snapshot);
    const first = useProjectFileStore.getState().refreshGitStatus(PROJECT);
    await useProjectFileStore.getState().refreshGitStatus(PROJECT);
    finishOld({ ...snapshot, revision: 'stale', branch: 'old' });
    await first;
    expect(getProjectFileRuntime(useProjectFileStore.getState().projects, PROJECT).gitStatus?.revision).toBe('new');
  });

  it('retains the last snapshot but marks failed refreshes explicitly', async () => {
    const snapshot: GitStatus = { isRepo: true, branch: 'main', files: [], added: 0, removed: 0, uncommittedCount: 0 };
    vi.spyOn(api.git, 'status').mockResolvedValueOnce(snapshot).mockRejectedValueOnce(new Error('Git timed out'));
    await useProjectFileStore.getState().refreshGitStatus(PROJECT);
    await useProjectFileStore.getState().refreshGitStatus(PROJECT);
    const runtime = getProjectFileRuntime(useProjectFileStore.getState().projects, PROJECT);
    expect(runtime.gitStatus?.branch).toBe('main');
    expect(runtime.gitError).toBe('Git timed out');
  });
});
