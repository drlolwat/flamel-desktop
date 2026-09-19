import React, { useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { DisplayBlock, ToolCall } from "../../lib/messageProcessor";
import { parseCompactionSummary } from "../../lib/compactionSummary";
import { TypedRevealMarkdown } from "../ui/TypedReveal";
import { THINKING_MD_COMPONENTS } from "./MessageMarkdown";

function extractThought(tool: ToolCall): string {
  if (typeof tool.input?.thought === "string") {
    return tool.input.thought;
  }
  const partial = tool.partialInput;
  if (!partial) return "";
  // Try to parse complete JSON first
  try {
    const p = JSON.parse(partial);
    return typeof p.thought === "string" ? p.thought : "";
  } catch {}
  // Extract raw string value after `"thought":` (may be incomplete / unescaped)
  const m = partial.match(/"thought"\s*:\s*"([\s\S]*)/);
  if (!m) return "";
  return m[1]
      .replace(/\\$/, "") // drop trailing partial escape
      .replace(/\\n/g, "\n")
      .replace(/\\t/g, "\t")
      .replace(/\\r/g, "\r")
      .replace(/\\"/g, '"')
      .replace(/\\\\/g, "\\");
}

function formatCompactNumber(value?: number): string | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1000) return `${Math.round(value / 1000)}K`;
  return String(value);
}

export function CompactionSummaryView({
  block,
  isLive,
  mdComponents,
  workingLabel,
}: {
  block: DisplayBlock;
  isLive?: boolean;
  mdComponents: any;
  workingLabel?: string;
}) {
  const details = block.compaction;
  const summary = details?.summary?.trim() || "";
  const observedLiveRef = useRef(Boolean(isLive));
  if (isLive) observedLiveRef.current = true;
  const animateSummary = observedLiveRef.current;
  // Older builds persisted Codex's lifetime usage as context fill. Do not
  // repeat obviously impossible values from those historical checkpoints.
  const rawPreTokens = details?.preTokens ?? block.preTokens;
  const plausiblePreTokens =
    rawPreTokens !== undefined && rawPreTokens >= 0 && rawPreTokens <= 4_000_000
      ? rawPreTokens
      : undefined;
  const rawRecoveredTokens = details?.estimatedRecoveredTokens;
  const plausibleRecoveredTokens =
    rawRecoveredTokens !== undefined &&
    rawRecoveredTokens >= 0 &&
    rawRecoveredTokens <= 4_000_000 &&
    (plausiblePreTokens === undefined ||
      rawRecoveredTokens <= plausiblePreTokens)
      ? rawRecoveredTokens
      : undefined;
  const preTokens = formatCompactNumber(plausiblePreTokens);
  const recoveredTokens = formatCompactNumber(plausibleRecoveredTokens);
  const summaryTokens = formatCompactNumber(details?.estimatedSummaryTokens);
  const stats = [
    recoveredTokens && { value: `~${recoveredTokens}`, label: "context freed" },
    summaryTokens && { value: `~${summaryTokens}`, label: "summary tokens" },
    details?.summarizedTurns && {
      value: String(details.summarizedTurns),
      label: "older turns folded",
    },
  ].filter(Boolean) as Array<{ value: string; label: string }>;
  const technicalDetails = [
    preTokens && `${preTokens} context tokens before compaction`,
    details?.preservedRecentMessages &&
      `${details.preservedRecentMessages} recent messages preserved verbatim`,
    details?.model && `Summarized with ${details.model}`,
    details?.reasoningEffort && `${details.reasoningEffort} reasoning effort`,
    details?.fallbackUsed && "Fallback summarizer used",
  ].filter(Boolean) as string[];
  const sections = summary && !isLive ? parseCompactionSummary(summary) : [];
  const overview = sections.find((section) => section.key === "current");
  const detailSections = sections.filter((section) => section !== overview);

  return (
    <div
      role="status"
      aria-live="polite"
      className={`compaction-checkpoint${isLive ? " is-live" : ""}`}
    >
      <div className="compaction-checkpoint__header">
        <div className="compaction-checkpoint__title">
          {isLive && (
            <span className="spinner compaction-checkpoint__spinner" />
          )}
          <span>
            {isLive
              ? workingLabel || "Compacting context..."
              : "Context checkpoint"}
          </span>
        </div>
        {!isLive && (
          <span className="compaction-checkpoint__status">Complete</span>
        )}
      </div>
      {!isLive && stats.length > 0 && (
        <div className="compaction-checkpoint__stats">
          {stats.map((stat) => (
            <div className="compaction-checkpoint__stat" key={stat.label}>
              <strong>{stat.value}</strong>
              <span>{stat.label}</span>
            </div>
          ))}
        </div>
      )}
      {summary && isLive && (
        <div className="compaction-checkpoint__live-summary">
          <TypedRevealMarkdown
            text={summary}
            active={animateSummary}
            components={mdComponents}
            charsPerSecond={900}
            maxStep={96}
            snapBacklog={2400}
          />
        </div>
      )}
      {!isLive && overview && (
        <div className="compaction-checkpoint__overview">
          <span>{overview.title}</span>
          <ReactMarkdown remarkPlugins={[remarkGfm]} components={mdComponents}>
            {overview.body}
          </ReactMarkdown>
        </div>
      )}
      {!isLive && !summary && details?.providerNative && (
        <div className="compaction-checkpoint__overview">
          <span>Provider checkpoint</span>
          <p>
            The provider compacted its active context without exposing the
            generated summary.
          </p>
        </div>
      )}
      {!isLive && detailSections.length > 0 && (
        <div className="compaction-checkpoint__sections">
          {detailSections.map((section) => (
            <section
              className={`compaction-checkpoint__section is-${section.key}`}
              key={section.key}
            >
              <h4>{section.title}</h4>
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                components={mdComponents}
              >
                {section.body}
              </ReactMarkdown>
            </section>
          ))}
        </div>
      )}
      {!isLive &&
        (technicalDetails.length > 0 ||
          details?.queuedPrompt !== undefined) && (
          <details className="compaction-checkpoint__technical">
            <summary>Checkpoint details</summary>
            <div>
              {technicalDetails.map((item) => (
                <span key={item}>{item}</span>
              ))}
              {details?.queuedPrompt !== undefined && (
                <span>
                  {details.queuedPrompt
                    ? "Queued message will run next"
                    : "No queued message"}
                </span>
              )}
            </div>
          </details>
        )}
    </div>
  );
}

export function ThinkingSegment({
  text,
  isStreaming,
  redacted,
}: {
  text: string;
  isStreaming?: boolean;
  redacted?: boolean;
}) {
  // Codex emits an empty redacted marker while private reasoning is active.
  // It is activity state, not displayable chat content; the block spinner covers it.
  if (redacted || !text.trim()) return null;

  return (
    <div
      style={{
        color: "var(--text-dim)",
        fontSize: 12,
        lineHeight: 1.55,
        margin: "3px 0 8px",
        paddingLeft: 10,
        borderLeft: "1px solid var(--border-soft)",
        overflowX: "hidden",
        minWidth: 0,
      }}
    >
      <div
        style={{
          color: "var(--text-dim)",
          fontFamily: "var(--font-mono)",
          fontSize: 10,
          marginBottom: 3,
        }}
      >
        Reasoning
      </div>
      <TypedRevealMarkdown
        text={text}
        active={isStreaming}
        components={THINKING_MD_COMPONENTS}
        charsPerSecond={900}
        maxStep={96}
        snapBacklog={2400}
      />
    </div>
  );
}

export function RecoverySummarySegment({
  label,
  text,
  mdComponents,
}: {
  label: string;
  text: string;
  mdComponents: any;
}) {
  const [expanded, setExpanded] = useState(false);

  const lines = text.split("\n").length;
  const chars = text.length;
  const summary =
    lines > 1
      ? `${lines} lines`
      : chars > 80
        ? `${Math.round(chars / 100) * 100} chars`
        : "";

  return (
    <div style={{ marginBottom: 6 }}>
      <button
        onClick={() => setExpanded((v) => !v)}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 6,
          background: expanded
            ? "none"
            : "color-mix(in srgb, var(--accent) 10%, transparent)",
          border: expanded
            ? "none"
            : "1px solid color-mix(in srgb, var(--accent) 24%, transparent)",
          borderRadius: "var(--radius-md)",
          cursor: "pointer",
          padding: expanded ? "2px 0" : "3px 10px 3px 8px",
          color: "var(--accent)",
          fontSize: 11,
          transition: "all var(--transition-fast)",
        }}
        onMouseEnter={(e) => {
          if (!expanded)
            (e.currentTarget as HTMLElement).style.background =
              "color-mix(in srgb, var(--accent) 16%, transparent)";
        }}
        onMouseLeave={(e) => {
          if (!expanded)
            (e.currentTarget as HTMLElement).style.background =
              "color-mix(in srgb, var(--accent) 10%, transparent)";
        }}
      >
        <span
          style={{
            fontSize: 9,
            transition: "transform 0.15s",
            transform: expanded ? "rotate(90deg)" : "rotate(0deg)",
          }}
        >
          ▸
        </span>
        <span>{label}</span>
        {!expanded && summary && (
          <span style={{ color: "var(--accent)", opacity: 0.72 }}>
            ({summary})
          </span>
        )}
      </button>
      {expanded && (
        <div
          style={{
            color: "var(--text)",
            fontSize: 13,
            lineHeight: 1.65,
            paddingLeft: 13,
            overflowX: "hidden",
            minWidth: 0,
          }}
        >
          <TypedRevealMarkdown
            text={text}
            active={false}
            components={mdComponents}
          />
        </div>
      )}
    </div>
  );
}

export const ThinkingToolRow = React.memo(function ThinkingToolRow({
  tool,
  blockStreaming,
}: {
  tool: ToolCall;
  blockStreaming?: boolean;
}) {
  const thought = extractThought(tool);
  if (!thought) return null;
  return (
    <ThinkingSegment
      text={thought}
      isStreaming={
        blockStreaming || tool.status === "running" || tool.status === "pending"
      }
    />
  );
});
