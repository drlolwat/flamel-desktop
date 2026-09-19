import { useEffect, useRef } from "react";
import { monaco } from "../../lib/monaco";
import {
  FLAMEL_MONACO_THEME,
  READ_ONLY_MONACO_OPTIONS,
  registerFlamelMonacoTheme,
} from "../../lib/monacoPresentation";
import { monacoModelUri } from "./codePresentation";

registerFlamelMonacoTheme(monaco);

interface MonacoCodeSurfaceProps {
  scopeId: string;
  path: string;
  value: string;
  language: string;
  height: number;
  ariaLabel?: string;
}

interface MonacoDiffSurfaceProps {
  scopeId: string;
  path: string;
  original: string;
  modified: string;
  language: string;
  height: number;
  renderSideBySide: boolean;
  ariaLabel?: string;
}

export function MonacoCodeSurface({
  scopeId,
  path,
  value,
  language,
  height,
  ariaLabel,
}: MonacoCodeSurfaceProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return undefined;

    const model = monaco.editor.createModel(
      value,
      language,
      monaco.Uri.parse(monacoModelUri(scopeId, "code", path)),
    );
    const editor = monaco.editor.create(container, {
      ...READ_ONLY_MONACO_OPTIONS,
      model,
      theme: FLAMEL_MONACO_THEME,
      ariaLabel: ariaLabel || `Read-only code for ${path}`,
      automaticLayout: true,
      minimap: { enabled: false },
      lineNumbers: "on",
      lineNumbersMinChars: 3,
      glyphMargin: false,
      folding: false,
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      renderLineHighlight: "none",
      renderWhitespace: "selection",
      contextmenu: true,
      links: true,
      scrollbar: {
        verticalScrollbarSize: 8,
        horizontalScrollbarSize: 8,
        alwaysConsumeMouseWheel: false,
      },
    });

    return () => {
      editor.setModel(null);
      editor.dispose();
      if (!model.isDisposed()) model.dispose();
    };
  }, [ariaLabel, height, language, path, scopeId, value]);

  return (
    <div
      ref={containerRef}
      className="embedded-code__monaco"
      data-monaco-surface="code"
      style={{ height }}
    />
  );
}

export function MonacoDiffSurface({
  scopeId,
  path,
  original,
  modified,
  language,
  height,
  renderSideBySide,
  ariaLabel,
}: MonacoDiffSurfaceProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return undefined;

    const originalModel = monaco.editor.createModel(
      original,
      language,
      monaco.Uri.parse(monacoModelUri(scopeId, "original", path)),
    );
    const modifiedModel = monaco.editor.createModel(
      modified,
      language,
      monaco.Uri.parse(monacoModelUri(scopeId, "modified", path)),
    );
    const editor = monaco.editor.createDiffEditor(container, {
      ...READ_ONLY_MONACO_OPTIONS,
      theme: FLAMEL_MONACO_THEME,
      ariaLabel: ariaLabel || `Read-only diff for ${path}`,
      originalAriaLabel: `Original ${path}`,
      modifiedAriaLabel: `Modified ${path}`,
      automaticLayout: true,
      originalEditable: false,
      renderSideBySide,
      enableSplitViewResizing: true,
      renderOverviewRuler: false,
      minimap: { enabled: false },
      lineNumbers: "on",
      lineNumbersMinChars: 3,
      glyphMargin: false,
      folding: false,
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      renderLineHighlight: "none",
      renderWhitespace: "selection",
      diffWordWrap: "on",
      ignoreTrimWhitespace: false,
      renderIndicators: true,
      scrollbar: {
        verticalScrollbarSize: 8,
        horizontalScrollbarSize: 8,
        alwaysConsumeMouseWheel: false,
      },
    });
    editor.setModel({ original: originalModel, modified: modifiedModel });

    return () => {
      editor.setModel(null);
      editor.dispose();
      if (!originalModel.isDisposed()) originalModel.dispose();
      if (!modifiedModel.isDisposed()) modifiedModel.dispose();
    };
  }, [
    ariaLabel,
    height,
    language,
    modified,
    original,
    path,
    renderSideBySide,
    scopeId,
  ]);

  return (
    <div
      ref={containerRef}
      className="embedded-code__monaco"
      data-monaco-surface="diff"
      data-render-side-by-side={renderSideBySide}
      style={{ height }}
    />
  );
}
