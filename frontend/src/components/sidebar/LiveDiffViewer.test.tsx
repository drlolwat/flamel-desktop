import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import LiveDiffViewer from './LiveDiffViewer';
import { DEFAULT_PROJECT_FILE_RUNTIME, useProjectFileStore } from '../../stores/projectFileStore';

vi.mock('../../stores/projectFileStore', async importOriginal => {
  const actual = await importOriginal<typeof import('../../stores/projectFileStore')>();
  return { ...actual, useProjectFileStore: Object.assign(
    (selector: (state: ReturnType<typeof actual.useProjectFileStore.getState>) => unknown) => selector(actual.useProjectFileStore.getState()),
    actual.useProjectFileStore,
  ) };
});

const projectPath = '/fixture/repo';

describe('repository review', () => {
  beforeEach(() => useProjectFileStore.setState({ projects: {}, views: {} }));

  it('does not call a failed initial load a missing or clean repository', () => {
    useProjectFileStore.setState({ projects: { [projectPath]: { ...DEFAULT_PROJECT_FILE_RUNTIME, gitError: 'Git timed out' } } });
    const html = renderToStaticMarkup(<LiveDiffViewer projectPath={projectPath} />);
    expect(html).toContain('Git timed out');
    expect(html).toContain('Retry');
    expect(html).not.toContain('not a Git repository');
    expect(html).not.toContain('Working tree clean');
  });

  it('makes review, local save and remote publication distinct', () => {
    useProjectFileStore.setState({ projects: { [projectPath]: { ...DEFAULT_PROJECT_FILE_RUNTIME, gitStatus: {
      isRepo: true, branch: 'feature', files: [{ path: 'a.txt', status: 'M', staged: true }], added: 1, removed: 0, uncommittedCount: 1,
      repositoryUrl: 'https://git.flamel.dev/a/repo',
    } } } });
    const html = renderToStaticMarkup(<LiveDiffViewer projectPath={projectPath} />);
    expect(html).toContain('Review');
    expect(html).toContain('Save version');
    expect(html).toContain('Publish');
    expect(html).toContain('https://git.flamel.dev/a/repo/pulls');
    expect(html).toContain('does not push or deploy');
  });
});
