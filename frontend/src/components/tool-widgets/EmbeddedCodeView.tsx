import React, {
  Suspense,
  lazy,
  useEffect,
  useId,
  useMemo,
  useState,
} from "react";
import { languageForPath } from "../../lib/monacoPresentation";
import {
  canUseMonaco,
  editorHeightFor,
  lightweightPreview,
} from "./codePresentation";
import "./toolWidgets.css";

const MonacoCodeSurface = lazy(() =>
  import("./MonacoSurface").then((module) => ({
    default: module.MonacoCodeSurface,
  })),
);

export interface EmbeddedCodeViewProps {
  path: string;
  value: string;
  language?: string;
  label?: string;
  pathLabel?: string;
  detail?: string;
  maximumHeight?: number;
  minimumHeight?: number;
  showToolbar?: boolean;
  onOpenInIde?: () => void;
}

interface LightweightCodeFallbackProps {
  value: string;
  height: number;
  label: string;
  loading?: boolean;
  notice?: string;
}

export function LightweightCodeFallback({
  value,
  height,
  label,
  loading = false,
  notice,
}: LightweightCodeFallbackProps) {
  const preview = lightweightPreview(value);

  return (
    <div
      className="embedded-code__fallback"
      style={{ maxHeight: height }}
      data-truncated={preview.truncated || undefined}
    >
      {loading ? (
        <div className="embedded-code__fallback-note" role="status">
          Loading the editor
        </div>
      ) : notice ? (
        <div className="embedded-code__fallback-note">{notice}</div>
      ) : preview.truncated ? (
        <div className="embedded-code__fallback-note">
          Large {label.toLowerCase()} preview · open in the IDE for the complete file
        </div>
      ) : null}
      <pre tabIndex={0} aria-label={label}>
        <code>{preview.text || "No content"}</code>
      </pre>
    </div>
  );
}

export function EmbeddedCodeView({
  path,
  value,
  language,
  label,
  pathLabel,
  detail,
  maximumHeight = 520,
  minimumHeight = 170,
  showToolbar = true,
  onOpenInIde,
}: EmbeddedCodeViewProps) {
  const generatedId = useId();
  const scopeId = useMemo(() => generatedId.replace(/:/g, ""), [generatedId]);
  const resolvedLanguage = language || languageForPath(path);
  const height = editorHeightFor([value], minimumHeight, maximumHeight);
  const eligibleForMonaco = canUseMonaco([value]);
  const [copied, setCopied] = useState(false);
  const accessibleLabel = label || `Code view for ${path}`;

  useEffect(() => {
    if (!copied) return undefined;
    const timeout = window.setTimeout(() => setCopied(false), 1400);
    return () => window.clearTimeout(timeout);
  }, [copied]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  return (
    <section className="embedded-code" aria-label={accessibleLabel}>
      {showToolbar ? (
        <header className="embedded-code__toolbar">
          <span className="embedded-code__path" title={path}>
            <span className="embedded-code__path-name">
              {pathLabel ?? path}
            </span>
            {detail ? (
              <span className="embedded-code__detail">{detail}</span>
            ) : null}
          </span>
          <span className="embedded-code__language">{resolvedLanguage}</span>
          <span className="embedded-code__actions">
            <button type="button" onClick={() => void copy()}>
              {copied ? "Copied" : "Copy"}
            </button>
            {onOpenInIde ? (
              <button type="button" onClick={onOpenInIde}>
                Open in IDE
              </button>
            ) : null}
          </span>
        </header>
      ) : null}

      <div className="embedded-code__viewport" style={{ height }}>
        {eligibleForMonaco ? (
          <Suspense
            fallback={
              <LightweightCodeFallback
                value={value}
                height={height}
                label={accessibleLabel}
                loading
              />
            }
          >
            <MonacoCodeSurface
              key={`${scopeId}:${path}`}
              scopeId={scopeId}
              path={path}
              value={value}
              language={resolvedLanguage}
              height={height}
              ariaLabel={accessibleLabel}
            />
          </Suspense>
        ) : (
          <LightweightCodeFallback
            value={value}
            height={height}
            label={accessibleLabel}
          />
        )}
      </div>
    </section>
  );
}
