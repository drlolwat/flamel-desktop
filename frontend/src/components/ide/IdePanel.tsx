import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Editor, { loader, type BeforeMount, type OnMount, type OnValidate } from '@monaco-editor/react';
import type { editor } from 'monaco-editor';
import type { DiagnosticSeverity } from '@flamel/shared';
import { api, FileRevisionConflictError } from '../../lib/api';
import { monaco } from '../../lib/monaco';
import { useIdeStore } from '../../stores/ideStore';
import { Icon } from '../ui/Icons';

// Keep the editor self-contained in Flamel instead of loading Monaco from a CDN.
loader.config({ monaco });

const defineTheme: BeforeMount = (monaco) => {
  monaco.editor.defineTheme('flamel-dark', {
    base: 'vs-dark',
    inherit: true,
    rules: [
      { token: 'comment', foreground: '6f6b68' },
      { token: 'keyword', foreground: 'ef6b5a' },
      { token: 'string', foreground: 'd9a779' },
      { token: 'number', foreground: 'b9a0dc' },
      { token: 'type', foreground: '72b7a9' },
    ],
    colors: {
      'editor.background': '#090909',
      'editor.foreground': '#d9d7d5',
      'editor.lineHighlightBackground': '#12100f',
      'editor.selectionBackground': '#5f2c2870',
      'editor.inactiveSelectionBackground': '#33242170',
      'editorCursor.foreground': '#ef4938',
      'editorLineNumber.foreground': '#504b48',
      'editorLineNumber.activeForeground': '#aaa4a0',
      'editorIndentGuide.background1': '#22201e',
      'editorIndentGuide.activeBackground1': '#47413d',
      'editorWhitespace.foreground': '#262321',
      'editorGutter.background': '#090909',
      'editorOverviewRuler.border': '#00000000',
      'scrollbarSlider.background': '#3a353180',
      'scrollbarSlider.hoverBackground': '#514a45aa',
      'scrollbarSlider.activeBackground': '#665d57bb',
    },
  });
};

function shortPath(path: string): string {
  const segments = path.split('/').filter(Boolean);
  return segments.slice(-4).join('/');
}

function diagnosticSeverity(value: number): DiagnosticSeverity {
  if (value === monaco.MarkerSeverity.Error) return 'error';
  if (value === monaco.MarkerSeverity.Warning) return 'warning';
  if (value === monaco.MarkerSeverity.Info) return 'information';
  return 'hint';
}

export default function IdePanel() {
  const {
    files,
    activeFilePath,
    revealLocation,
    updateContent,
    markSaved,
    acceptServerVersion,
    setDiagnostics,
    clearReveal,
  } = useIdeStore();
  const file = files.find(candidate => candidate.path === activeFilePath);
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const [saving, setSaving] = useState(false);
  const [notice, setNotice] = useState<{ tone: 'success' | 'error' | 'warning'; text: string } | null>(null);
  const [conflict, setConflict] = useState<FileRevisionConflictError | null>(null);
  const dirty = Boolean(file && file.content !== file.savedContent);

  const revealRequestedLocation = useCallback((instance: editor.IStandaloneCodeEditor | null) => {
    if (!instance || !file || revealLocation?.path !== file.path) return;
    const position = { lineNumber: revealLocation.line, column: revealLocation.column };
    instance.setPosition(position);
    instance.revealPositionInCenter(position);
    instance.focus();
    clearReveal();
  }, [clearReveal, file, revealLocation]);

  const save = useCallback(async () => {
    if (!file || saving) return;
    setSaving(true);
    setNotice(null);
    try {
      const result = await api.fileContent.save(file.path, file.content, file.revision);
      markSaved(file.path, file.content, result.file.revision);
      setNotice({ tone: 'success', text: 'Saved' });
      window.setTimeout(() => setNotice(current => current?.text === 'Saved' ? null : current), 1400);
    } catch (error) {
      if (error instanceof FileRevisionConflictError) {
        setConflict(error);
        setNotice({ tone: 'warning', text: 'Newer version on disk' });
      } else {
        setNotice({ tone: 'error', text: error instanceof Error ? error.message : 'Save failed' });
      }
    } finally {
      setSaving(false);
    }
  }, [file, markSaved, saving]);

  const onMount: OnMount = useCallback((instance, monaco) => {
    editorRef.current = instance;
    instance.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => void save());
    revealRequestedLocation(instance);
    if (!useIdeStore.getState().revealLocation) instance.focus();
  }, [revealRequestedLocation, save]);

  useEffect(() => {
    revealRequestedLocation(editorRef.current);
  }, [revealRequestedLocation]);

  const handleValidate: OnValidate = useCallback((markers) => {
    if (!file) return;
    setDiagnostics(file.path, markers.map((marker, index) => ({
      id: `${file.path}:${marker.startLineNumber}:${marker.startColumn}:${marker.code ?? index}`,
      path: file.path,
      source: marker.source || 'Monaco',
      message: marker.message,
      severity: diagnosticSeverity(marker.severity),
      startLine: marker.startLineNumber,
      startColumn: marker.startColumn,
      endLine: marker.endLineNumber,
      endColumn: marker.endColumn,
      code: typeof marker.code === 'string' ? marker.code : marker.code?.value,
    })));
  }, [file, setDiagnostics]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
        event.preventDefault();
        void save();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [save]);

  const options = useMemo<editor.IStandaloneEditorConstructionOptions>(() => ({
    automaticLayout: true,
    bracketPairColorization: { enabled: true },
    codeLens: true,
    colorDecorators: true,
    cursorBlinking: 'smooth',
    cursorSmoothCaretAnimation: 'on',
    detectIndentation: true,
    folding: true,
    fontFamily: 'var(--font-mono)',
    fontLigatures: false,
    fontSize: 13,
    glyphMargin: true,
    guides: { bracketPairs: true, indentation: true },
    lineHeight: 21,
    minimap: { enabled: true, maxColumn: 100, renderCharacters: false, scale: 1 },
    padding: { top: 12, bottom: 24 },
    renderLineHighlight: 'gutter',
    renderWhitespace: 'selection',
    scrollBeyondLastLine: false,
    smoothScrolling: true,
    stickyScroll: { enabled: true },
    tabSize: 2,
    wordWrap: 'off',
  }), []);

  if (!file) {
    return (
      <div className="h-full flex items-center justify-center" style={{ color: 'var(--text-dim)', background: 'var(--bg)' }}>
        Open a file from the project explorer.
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-hidden" style={{ background: 'var(--bg)' }}>
      <div className="flex items-center gap-2 shrink-0" style={{ minHeight: 31, padding: '0 9px', borderBottom: '1px solid var(--border-soft)', background: 'var(--surface)' }}>
        <Icon.File size={12} style={{ color: 'var(--text-dim)' }} />
        <span title={file.path} style={{ color: 'var(--text-muted)', fontSize: 11, fontFamily: 'var(--font-mono)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {shortPath(file.path)}
        </span>
        {dirty && <span title="Unsaved changes" style={{ width: 6, height: 6, borderRadius: '50%', background: 'var(--warning)', flexShrink: 0 }} />}
        <span style={{ marginLeft: 'auto', color: notice?.tone === 'error' ? 'var(--error)' : notice?.tone === 'warning' ? 'var(--warning)' : 'var(--success)', fontSize: 10 }}>
          {notice?.text}
        </span>
        <button
          type="button"
          onClick={() => void save()}
          disabled={!dirty || saving}
          title="Save file (Ctrl+S)"
          className="inline-flex items-center justify-center"
          style={{ width: 25, height: 23, color: dirty ? 'var(--text)' : 'var(--text-dim)', border: '1px solid var(--border-soft)', borderRadius: 'var(--radius-sm)', background: dirty ? 'var(--surface-2)' : 'transparent', opacity: saving ? 0.6 : 1 }}
        >
          <Icon.Check size={12} />
        </button>
      </div>

      <div className="flex-1 min-h-0">
        <Editor
          key={file.path}
          path={file.path}
          language={file.language === 'tsx' ? 'typescript' : file.language === 'jsx' ? 'javascript' : file.language}
          value={file.content}
          beforeMount={defineTheme}
          onMount={onMount}
          onValidate={handleValidate}
          onChange={(value: string | undefined) => updateContent(file.path, value ?? '')}
          theme="flamel-dark"
          options={options}
          loading={<div className="h-full flex items-center justify-center" style={{ color: 'var(--text-dim)' }}>Loading editor...</div>}
        />
      </div>

      {conflict && (
        <div role="dialog" aria-modal="true" aria-label="File save conflict" className="absolute inset-0 flex items-center justify-center" style={{ zIndex: 30, background: 'rgba(0,0,0,0.62)', padding: 20 }}>
          <div style={{ width: 'min(520px, 100%)', border: '1px solid var(--warning-dim, #5a4420)', background: 'var(--surface)', borderRadius: 'var(--radius-md)', boxShadow: '0 18px 50px rgba(0,0,0,.48)' }}>
            <div style={{ padding: '14px 16px', borderBottom: '1px solid var(--border-soft)' }}>
              <div style={{ color: 'var(--text)', fontSize: 13, fontWeight: 650 }}>This file changed on disk</div>
              <div style={{ color: 'var(--text-muted)', fontSize: 11, marginTop: 4 }}>Keep your editor version, or reload the newer server version. Nothing has been overwritten.</div>
            </div>
            <div className="flex justify-end gap-2" style={{ padding: 12 }}>
              <button type="button" onClick={() => setConflict(null)} style={{ padding: '6px 10px', color: 'var(--text-muted)', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', background: 'transparent', fontSize: 11 }}>Keep editing</button>
              <button type="button" onClick={() => { acceptServerVersion(file.path, conflict.current.content, conflict.current.revision); setConflict(null); setNotice({ tone: 'success', text: 'Reloaded' }); }} style={{ padding: '6px 10px', color: '#fff', border: '1px solid var(--accent)', borderRadius: 'var(--radius-sm)', background: 'var(--accent)', fontSize: 11 }}>Reload server version</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
