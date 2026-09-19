import React, {
  Suspense,
  lazy,
  type KeyboardEvent,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  type EmbeddedDiffFile,
  canUseMonaco,
  editorHeightFor,
  normalizeDiffFile,
  parseUnifiedDiff,
} from "./codePresentation";
import { EmbeddedCodeView, LightweightCodeFallback } from "./EmbeddedCodeView";
import "./toolWidgets.css";

const MonacoDiffSurface = lazy(() =>
  import("./MonacoSurface").then((module) => ({
    default: module.MonacoDiffSurface,
  })),
);

export interface EmbeddedDiffViewProps {
  files?: EmbeddedDiffFile[];
  patch?: string;
  preamble?: string;
  maximumHeight?: number;
  minimumHeight?: number;
  onOpenInIde?: (file: EmbeddedDiffFile) => void;
}

function baseName(path: string): string {
  return path.split("/").pop() || path;
}

function statusLabel(file: EmbeddedDiffFile): string {
  switch (file.status) {
    case "added":
      return "New file";
    case "deleted":
      return "Deleted";
    case "renamed":
      return "Renamed";
    default:
      return "Modified";
  }
}

function useNarrowDiff(): boolean {
  const [narrow, setNarrow] = useState(false);

  useEffect(() => {
    const query = window.matchMedia("(max-width: 640px)");
    const update = () => setNarrow(query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  return narrow;
}

export function EmbeddedDiffView({
  files,
  patch,
  preamble,
  maximumHeight = 520,
  minimumHeight = 190,
  onOpenInIde,
}: EmbeddedDiffViewProps) {
  const parsed = useMemo(
    () => (patch ? parseUnifiedDiff(patch) : { files: [] }),
    [patch],
  );
  const resolvedFiles = useMemo(
    () => (files?.length ? files : parsed.files).map(normalizeDiffFile),
    [files, parsed.files],
  );
  const resolvedPreamble = preamble ?? parsed.preamble;
  const [activeIndex, setActiveIndex] = useState(0);
  const [mode, setMode] = useState<"split" | "inline">("split");
  const [copied, setCopied] = useState(false);
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const generatedId = useId();
  const scopeId = useMemo(() => generatedId.replace(/:/g, ""), [generatedId]);
  const narrow = useNarrowDiff();

  useEffect(() => {
    if (activeIndex >= resolvedFiles.length) setActiveIndex(0);
  }, [activeIndex, resolvedFiles.length]);

  useEffect(() => {
    if (!copied) return undefined;
    const timeout = window.setTimeout(() => setCopied(false), 1400);
    return () => window.clearTimeout(timeout);
  }, [copied]);

  const selected = resolvedFiles[activeIndex] || resolvedFiles[0];
  if (!selected) {
    return (
      <div className="embedded-diff embedded-diff--empty" role="status">
        No diff content is available.
      </div>
    );
  }

  const original = selected.original;
  const modified = selected.modified;
  const hasCompletePair =
    typeof original === "string" && typeof modified === "string";
  const contentForCopy =
    selected.patch ??
    (typeof modified === "string"
      ? modified
      : typeof original === "string"
        ? original
        : "");
  const height = editorHeightFor(
    hasCompletePair ? [original, modified] : [selected.patch],
    minimumHeight,
    maximumHeight,
  );
  const eligibleForMonaco = canUseMonaco(
    hasCompletePair ? [original, modified] : [selected.patch],
  );
  const effectiveMode = narrow ? "inline" : mode;

  const selectTab = (index: number) => {
    setActiveIndex(index);
    requestAnimationFrame(() => tabRefs.current[index]?.focus());
  };

  const handleTabKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => {
    let next: number | undefined;
    if (event.key === "ArrowRight") next = (index + 1) % resolvedFiles.length;
    if (event.key === "ArrowLeft")
      next = (index - 1 + resolvedFiles.length) % resolvedFiles.length;
    if (event.key === "Home") next = 0;
    if (event.key === "End") next = resolvedFiles.length - 1;
    if (next === undefined) return;
    event.preventDefault();
    selectTab(next);
  };

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(contentForCopy);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  return (
    <section
      className="embedded-diff"
      aria-label={`Diff viewer for ${selected.path}`}
      data-mode={effectiveMode}
    >
      {resolvedPreamble ? (
        <div className="embedded-diff__preamble">{resolvedPreamble}</div>
      ) : null}

      {resolvedFiles.length > 1 ? (
        <div
          className="embedded-diff__tabs"
          role="tablist"
          aria-label="Changed files"
        >
          {resolvedFiles.map((file, index) => (
            <button
              key={file.id || file.path}
              ref={(node) => {
                tabRefs.current[index] = node;
              }}
              type="button"
              role="tab"
              aria-selected={index === activeIndex}
              tabIndex={index === activeIndex ? 0 : -1}
              title={file.path}
              onClick={() => setActiveIndex(index)}
              onKeyDown={(event) => handleTabKeyDown(event, index)}
            >
              <span className="embedded-diff__tab-name">
                {baseName(file.path)}
              </span>
              <span className="embedded-diff__tab-stats">
                <span className="embedded-diff__addition">
                  +{file.additions ?? 0}
                </span>
                <span className="embedded-diff__deletion">
                  −{file.deletions ?? 0}
                </span>
              </span>
            </button>
          ))}
        </div>
      ) : null}

      <header className="embedded-diff__toolbar">
        <span className="embedded-diff__path" title={selected.path}>
          {selected.path}
        </span>
        <span className="embedded-diff__status">{statusLabel(selected)}</span>
        <span className="embedded-diff__stats" aria-label="Diff statistics">
          <span className="embedded-diff__addition">
            +{selected.additions ?? 0}
          </span>
          <span className="embedded-diff__deletion">
            −{selected.deletions ?? 0}
          </span>
        </span>

        {hasCompletePair ? (
          <span className="embedded-diff__modes" aria-label="Diff layout">
            <button
              type="button"
              aria-pressed={effectiveMode === "split"}
              onClick={() => setMode("split")}
              disabled={narrow}
              title={
                narrow
                  ? "Inline view is used at this width"
                  : "Side-by-side diff"
              }
            >
              Split
            </button>
            <button
              type="button"
              aria-pressed={effectiveMode === "inline"}
              onClick={() => setMode("inline")}
            >
              Inline
            </button>
          </span>
        ) : null}

        <span className="embedded-diff__actions">
          <button type="button" onClick={() => void copy()}>
            {copied
              ? "Copied"
              : hasCompletePair && !selected.patch
                ? "Copy file"
                : "Copy patch"}
          </button>
          {onOpenInIde ? (
            <button type="button" onClick={() => onOpenInIde(selected)}>
              Open in IDE
            </button>
          ) : null}
        </span>
      </header>

      <div className="embedded-diff__viewport" style={{ height }}>
        {hasCompletePair ? (
          eligibleForMonaco ? (
            <Suspense
              fallback={
                <LightweightCodeFallback
                  value={selected.patch || modified}
                  height={height}
                  label={`Diff for ${selected.path}`}
                  loading
                />
              }
            >
              <MonacoDiffSurface
                key={`${scopeId}:${selected.id}:${effectiveMode}`}
                scopeId={scopeId}
                path={selected.path}
                original={original}
                modified={modified}
                language={selected.language || "plaintext"}
                height={height}
                renderSideBySide={effectiveMode === "split"}
                ariaLabel={`Read-only diff for ${selected.path}`}
              />
            </Suspense>
          ) : (
            <LightweightCodeFallback
              value={selected.patch || modified}
              height={height}
              label={`Diff for ${selected.path}`}
              notice={`Large diff for ${selected.path} preview · open in the IDE for the complete change`}
            />
          )
        ) : (
          <EmbeddedCodeView
            key={selected.id || selected.path}
            path={`${selected.path}.diff`}
            value={selected.patch || ""}
            language="diff"
            label={`Unified diff for ${selected.path}`}
            maximumHeight={maximumHeight}
            minimumHeight={minimumHeight}
            showToolbar={false}
          />
        )}
      </div>
    </section>
  );
}
