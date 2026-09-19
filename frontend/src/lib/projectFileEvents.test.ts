import { describe, expect, it } from 'vitest';
import { extractProjectFileInvalidation } from './projectFileEvents';

describe('project file events', () => {
  it('extracts file_edit replacement invalidations without forcing a tree refresh', () => {
    const invalidation = extractProjectFileInvalidation('mcp__flamel__file_edit', {
      action: 'replace',
      relative_path: 'src/App.tsx',
    });

    expect(invalidation).toMatchObject({
      paths: ['src/App.tsx'],
      refreshTree: false,
      refreshGit: true,
      reason: 'tool_result',
      toolName: 'file_edit',
      action: 'replace',
    });
  });

  it('forces a tree refresh for created files', () => {
    const invalidation = extractProjectFileInvalidation('file_edit', {
      action: 'create',
      relative_path: 'src/new-file.ts',
    });

    expect(invalidation?.paths).toEqual(['src/new-file.ts']);
    expect(invalidation?.refreshTree).toBe(true);
  });

  it('collects edited files from batch replacements', () => {
    const invalidation = extractProjectFileInvalidation('mcp__flamel__multi_replace', {
      edits: [
        { file: 'src/a.ts' },
        { file: 'src/b.ts' },
      ],
    });

    expect(invalidation?.paths).toEqual(['src/a.ts', 'src/b.ts']);
    expect(invalidation?.refreshTree).toBe(true);
  });

  it('collects touched files from unified diffs', () => {
    const invalidation = extractProjectFileInvalidation('mcp__flamel__file_edit', {
      action: 'diff_apply',
      diff: [
        'diff --git a/src/a.ts b/src/a.ts',
        '--- a/src/a.ts',
        '+++ b/src/a.ts',
        '@@ -1 +1 @@',
        '-old',
        '+new',
        'diff --git a/src/new.ts b/src/new.ts',
        '--- /dev/null',
        '+++ b/src/new.ts',
      ].join('\n'),
    });

    expect(invalidation?.paths).toEqual(['src/a.ts', 'src/new.ts']);
    expect(invalidation?.refreshTree).toBe(true);
  });

  it('ignores read-only and non-file tools', () => {
    expect(extractProjectFileInvalidation('file_read', { file: 'src/App.tsx' })).toBeNull();
    expect(extractProjectFileInvalidation('plan', { action: 'create' })).toBeNull();
  });
});
