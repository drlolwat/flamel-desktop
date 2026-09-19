import { beforeEach, describe, expect, it } from 'vitest';
import { useIdeStore } from './ideStore';

const APP_FILE = '/workspace/flamel/frontend/src/App.tsx';
const STORE_FILE = '/workspace/flamel/frontend/src/stores/ideStore.ts';

describe('ide store', () => {
  beforeEach(() => {
    useIdeStore.getState().closeAll();
  });

  it('keeps the active open file in store state instead of component-local state', () => {
    useIdeStore.getState().openFile(APP_FILE, 'export default function App() {}');

    expect(useIdeStore.getState().activeFilePath).toBe(APP_FILE);
    expect(useIdeStore.getState().files[0]).toMatchObject({
      path: APP_FILE,
      language: 'tsx',
    });
  });

  it('refreshes clean files on reopen without clobbering dirty edits', () => {
    useIdeStore.getState().openFile(APP_FILE, 'old content');
    useIdeStore.getState().openFile(APP_FILE, 'fresh content');

    expect(useIdeStore.getState().files[0]).toMatchObject({
      content: 'fresh content',
      savedContent: 'fresh content',
    });

    useIdeStore.getState().updateContent(APP_FILE, 'dirty local edit');
    useIdeStore.getState().openFile(APP_FILE, 'new disk content');

    expect(useIdeStore.getState().files[0]).toMatchObject({
      content: 'dirty local edit',
      savedContent: 'new disk content',
    });
  });

  it('falls back to a real open file when activating a missing path', () => {
    useIdeStore.getState().openFile(APP_FILE, 'app');
    useIdeStore.getState().openFile(STORE_FILE, 'store');
    useIdeStore.getState().setActiveFile('/workspace/flamel/deleted.ts');

    expect(useIdeStore.getState().activeFilePath).toBe(STORE_FILE);
  });

  it('moves active focus to the previous file when the current file closes', () => {
    useIdeStore.getState().openFile(APP_FILE, 'app');
    useIdeStore.getState().openFile(STORE_FILE, 'store');
    useIdeStore.getState().closeFile(STORE_FILE);

    expect(useIdeStore.getState().activeFilePath).toBe(APP_FILE);
  });

  it('tracks diagnostics per file and clears them with the file lifecycle', () => {
    useIdeStore.getState().openFile(APP_FILE, 'const answer = nope;');
    useIdeStore.getState().setDiagnostics(APP_FILE, [{
      id: `${APP_FILE}:1:16:typescript`,
      path: APP_FILE,
      severity: 'error',
      message: 'Cannot find name nope',
      startLine: 1,
      startColumn: 16,
      source: 'typescript',
    }]);

    expect(useIdeStore.getState().diagnostics[APP_FILE]).toHaveLength(1);

    useIdeStore.getState().closeFile(APP_FILE);
    expect(useIdeStore.getState().diagnostics[APP_FILE]).toBeUndefined();
  });

  it('requests an exact editor reveal location for a problem', () => {
    useIdeStore.getState().openFile(APP_FILE, 'const answer = nope;');
    useIdeStore.getState().requestReveal(APP_FILE, 7.8, 13.2);

    expect(useIdeStore.getState()).toMatchObject({
      activeFilePath: APP_FILE,
      revealLocation: { path: APP_FILE, line: 7, column: 13 },
    });

    useIdeStore.getState().clearReveal();
    expect(useIdeStore.getState().revealLocation).toBeNull();
  });
});
