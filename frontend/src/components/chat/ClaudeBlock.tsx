import React, { useState, useEffect, useCallback, useRef } from "react";
import type { DisplayBlock, ToolCall } from "../../lib/messageProcessor";
import { useContextMenu } from "../ui/ContextMenu";
import { Icon } from "../ui/Icons";
import { BlockErrorBoundary } from "../ui/BlockErrorBoundary";
import { TypedRevealMarkdown } from "../ui/TypedReveal";
import {
  isToolHidden,
  toolRenderKind,
  routineActivityGroup,
} from "../tool-widgets/toolWidgetRouting";
import { parseToolName } from "./toolPresentation";
import "./chatActivity.css";
import { MD_COMPONENTS } from "./MessageMarkdown";
import { LegacyToolRow } from "./LegacyToolRenderer";
import { ToolWidgetShell } from "../tool-widgets/ToolWidgetShell";
import {
  CompactionSummaryView,
  RecoverySummarySegment,
  ThinkingSegment,
  ThinkingToolRow,
} from "./AssistantMessageSegments";

const ToolWidgetRenderer = React.lazy(() =>
  import("../tool-widgets/ToolWidgetRenderer").then((module) => ({
    default: module.ToolWidgetRenderer,
  })),
);

function useElapsedSeconds(active: boolean): number {
  const [elapsed, setElapsed] = useState(0);
  const startRef = useRef<number>(Date.now());
  useEffect(() => {
    if (!active) {
      setElapsed(0);
      return;
    }
    startRef.current = Date.now();
    setElapsed(0);
    const id = setInterval(
      () => setElapsed(Math.floor((Date.now() - startRef.current) / 1000)),
      500,
    );
    return () => clearInterval(id);
  }, [active]);
  return elapsed;
}

interface Props {
  block: DisplayBlock;
  onOpenPlan?: () => void;
  isContinuation?: boolean;
}

export function hasAssistantContent(block: DisplayBlock): boolean {
  return !!(
    block.content?.trim() ||
    block.thinking?.trim() ||
    block.tools?.some((tool) => !isToolHidden(tool)) ||
    block.segments?.some((segment) =>
      (segment.type === "text" || (segment.type === "thinking" && !segment.redacted)) &&
      !!segment.text.trim(),
    )
  );
}

export function isAssistantBlockVisible(block: DisplayBlock): boolean {
  return block.type === "compact" || hasAssistantContent(block) || !!block.isStreaming || !!block.usage;
}


function AuthoredToolWidget({
  tool,
  blockStreaming,
}: {
  tool: ToolCall;
  blockStreaming?: boolean;
}) {
  return (
    <React.Suspense
      fallback={
        <div
          role="status"
          aria-label="Preparing tool result"
          style={{
            minHeight: 32,
            margin: "2px 0",
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "3px 8px",
            color: "var(--text-dim)",
            border: "1px solid var(--border-soft)",
            borderRadius: "var(--radius-md)",
          }}
        >
          <span className="spinner" style={{ width: 9, height: 9 }} />
          <span style={{ fontSize: 12 }}>Preparing tool result</span>
        </div>
      }
    >
      <ToolWidgetRenderer
        tool={tool}
        blockStreaming={blockStreaming}
        LegacyRenderer={LegacyToolRow}
        ThinkingRenderer={ThinkingToolRow}
      />
    </React.Suspense>
  );
}

export default React.memo(function ClaudeBlock({
  block,
  onOpenPlan,
  isContinuation,
}: Props) {
  const isWaiting = block.isWaiting;
  const hasContent = hasAssistantContent(block);
  const isStartingUp = !!block.isStreaming && !hasContent;
  const isCompactingStatus =
    block.statusText?.startsWith("__compacting__") ?? false;
  const showInlineActivity =
    !!block.isStreaming &&
    hasContent &&
    (block.statusText === "__thinking__" || isCompactingStatus);
  const elapsed = useElapsedSeconds(isStartingUp || showInlineActivity);

  function getWorkingLabel(): string {
    const mcpNames = (block.mcpServerNames || []).filter((n) => n !== "flamel");
    const suffix = elapsed >= 8 ? ` ${elapsed}s` : "";

    if (block.statusText === "__thinking__") {
      return `Thinking…${suffix}`;
    }
    if (isCompactingStatus) {
      const label = block.statusText
        ?.slice("__compacting__".length)
        .replace(/^:/, "")
        .trim();
      return `${label || "Compacting context"}…${suffix}`;
    }
    if (block.statusText) {
      const { server, label, isReclaude } = parseToolName(block.statusText);
      const toolLabel = isReclaude
        ? label
        : server
          ? `${server} · ${label}`
          : label;
      return `Using ${toolLabel}…${suffix}`;
    }
    if (mcpNames.length > 0) {
      return `Connecting tools…${suffix}`;
    }
    return `Working…${suffix}`;
  }

  const onContextMenu = useContextMenu(
    useCallback(() => {
      const items: any[] = [];
      if (block.content) {
        items.push({
          label: "Copy response",
          icon: "⎘",
          onClick: () => navigator.clipboard.writeText(block.content || ""),
        });
      }
      if (block.thinking) {
        items.push({
          label: "Copy thinking",
          icon: "💭",
          onClick: () => navigator.clipboard.writeText(block.thinking || ""),
        });
      }
      if (block.content && block.tools && block.tools.length > 0) {
        items.push({ separator: true } as any);
      }
      if (block.tools && block.tools.length > 0) {
        const toolNames = block.tools.map((t) => t.name).join(", ");
        items.push({
          label: "Copy tool names",
          icon: "🔧",
          onClick: () => navigator.clipboard.writeText(toolNames),
          disabled: false,
        });
      }
      return items.length > 0
        ? items
        : [
            {
              label: "Nothing to copy",
              icon: "–",
              onClick: () => {},
              disabled: true,
            },
          ];
    }, [block.content, block.thinking, block.tools]),
  );

  // Compaction checkpoint — persistent marker in the chat showing what context was compacted.
  if (block.type === "compact") {
    return (
      <CompactionSummaryView
        block={block}
        isLive={block.isStreaming}
        mdComponents={MD_COMPONENTS}
        workingLabel={`${block.compaction?.label || "Compacting context"}…`}
      />
    );
  }

  if (isStartingUp && isCompactingStatus && block.compaction) {
    return (
      <CompactionSummaryView
        block={block}
        isLive
        mdComponents={MD_COMPONENTS}
        workingLabel={getWorkingLabel()}
      />
    );
  }

  // Don't render empty blocks (e.g. after initialization spinner disappears)
  if (!isAssistantBlockVisible(block)) {
    return null;
  }

  return (
    <div
      onContextMenu={onContextMenu}
      style={{ padding: `${isContinuation ? 2 : 8}px 16px 8px 16px` }}
    >
      {/* Label row — hidden on continuation blocks */}
      {!isContinuation &&
        (() => {
          const isCodex =
            block.model?.startsWith("gpt-") || block.model?.startsWith("codex");
          const accentColor = !block.model ? "var(--text-muted)" : isCodex ? "#10a37f" : "#e2b86e";
          const providerLabel = !block.model ? "Assistant" : isCodex ? "Codex" : "Claude";
          const modelShort = block.model
            ? isCodex
              ? block.model
              : block.model.replace(/^claude-/, "").replace(/-\d{8}$/, "")
            : undefined;
          return (
            <div
              className="chat-assistant-header flex items-center gap-2"
              style={{ marginBottom: 6 }}
            >
              {isCodex ? (
                <span
                  style={{
                    color: accentColor,
                    fontSize: 12,
                    fontWeight: 700,
                    flexShrink: 0,
                  }}
                >
                  ⬡
                </span>
              ) : (
                <Icon.Logo
                  size={12}
                  style={{ color: accentColor, flexShrink: 0 }}
                />
              )}
              <span
                style={{
                  color: accentColor,
                  fontSize: 12,
                  fontWeight: 600,
                  letterSpacing: "-0.01em",
                }}
              >
                {providerLabel}
              </span>
              {modelShort && (
                <span
                  style={{
                    color: "var(--text-dim)",
                    fontSize: 10,
                    fontFamily: "var(--font-mono)",
                  }}
                >
                  {modelShort}
                </span>
              )}
              {isWaiting && (
                <span
                  style={{
                    color: "var(--warning)",
                    fontSize: 10,
                    marginLeft: 2,
                    opacity: 0.8,
                  }}
                >
                  waiting for input
                </span>
              )}
              {block.recoveryNote && !block.recoverySummary && (
                <span
                  style={{
                    color: "var(--accent)",
                    background:
                      "color-mix(in srgb, var(--accent) 10%, transparent)",
                    border:
                      "1px solid color-mix(in srgb, var(--accent) 24%, transparent)",
                    borderRadius: 999,
                    padding: "2px 7px",
                    fontSize: 10,
                    lineHeight: 1.2,
                  }}
                >
                  {block.recoveryNote}
                </span>
              )}
              {block.isStreaming && !isWaiting && !isStartingUp && (
                <span
                  className="spinner"
                  style={{
                    width: 8,
                    height: 8,
                    border: "1.5px solid var(--accent)",
                    borderTopColor: "transparent",
                    borderRadius: "50%",
                    display: "inline-block",
                  }}
                />
              )}
            </div>
          );
        })()}

      {/* Interleaved tools + content via segments */}
      {(() => {
        const toolsMap = new Map((block.tools || []).map((t) => [t.id, t]));
        const mdComponents = MD_COMPONENTS;

        // Block-level thinking (no segments path)
        const blockThinkingNode =
          block.thinking &&
          (!block.segments ||
            !block.segments.some((s) => s.type === "thinking")) ? (
            <ThinkingSegment
              key="block-thinking"
              text={block.thinking}
              isStreaming={block.isStreaming}
            />
          ) : null;
        const recoverySummaryNode =
          block.recoverySummary && block.recoverySummary.trim() ? (
            <RecoverySummarySegment
              key="recovery-summary"
              label={block.recoveryNote || "Recovery summary"}
              text={block.recoverySummary}
              mdComponents={mdComponents}
            />
          ) : null;
        const compactionSummaryNode =
          isCompactingStatus && block.compaction ? (
            <CompactionSummaryView
              key="compaction-summary"
              block={block}
              isLive
              mdComponents={mdComponents}
              workingLabel={getWorkingLabel()}
            />
          ) : null;

        if (block.segments && block.segments.length > 0) {
          // Segments-based interleaved rendering
          const nodes: React.ReactNode[] = [];
          if (recoverySummaryNode) nodes.push(recoverySummaryNode);
          if (blockThinkingNode) nodes.push(blockThinkingNode);
          for (let i = 0; i < block.segments.length; i++) {
            const seg = block.segments[i];
            if (seg.type === "thinking") {
              nodes.push(
                <ThinkingSegment
                  key={`thinking-${i}`}
                  text={seg.text}
                  redacted={seg.redacted}
                  isStreaming={block.isStreaming}
                />,
              );
            } else if (seg.type === "text") {
              if (seg.text) {
                nodes.push(
                  <div
                    key={i}
                    style={{
                      color: "var(--text)",
                      fontSize: 13,
                      lineHeight: 1.65,
                    }}
                  >
                    <TypedRevealMarkdown
                      text={seg.text}
                      active={block.isStreaming}
                      components={mdComponents}
                      charsPerSecond={900}
                      maxStep={96}
                      snapBacklog={2400}
                    />
                  </div>,
                );
              }
            } else {
              const tool = toolsMap.get(seg.toolId);
              if (!tool) continue;
              const group = routineActivityGroup(block.segments, i, toolsMap);
              if (group.length) {
                nodes.push(
                  <ToolWidgetShell
                    key={`activity-${tool.id}`}
                    className="chat-activity-group"
                    title="Reads and searches"
                    summary={`${group.length} completed`}
                    status="success"
                    icon={<Icon.FolderOpen size={14} />}
                  >
                    {group.map(item => <AuthoredToolWidget key={item.id} tool={item} />)}
                  </ToolWidgetShell>,
                );
                i += group.length - 1;
                continue;
              }
              nodes.push(
                <AuthoredToolWidget
                  key={seg.toolId}
                  tool={tool}
                  blockStreaming={block.isStreaming}
                />,
              );
            }
          }

          if (compactionSummaryNode) nodes.push(compactionSummaryNode);

          // Working indicator when streaming has no content yet, plus status-only phases
          // like Codex private reasoning after earlier tool rows already exist.
          if (
            block.isStreaming &&
            (!hasContent || showInlineActivity) &&
            !isCompactingStatus
          ) {
            nodes.push(
              <div
                key="working"
                style={{ display: "flex", alignItems: "center", gap: 8 }}
              >
                <span
                  className="spinner"
                  style={{
                    width: 9,
                    height: 9,
                    border: "1.5px solid var(--accent)",
                    borderTopColor: "transparent",
                    borderRadius: "50%",
                    display: "inline-block",
                    flexShrink: 0,
                  }}
                />
                <span style={{ color: "var(--text-dim)", fontSize: 12 }}>
                  {getWorkingLabel()}
                </span>
              </div>,
            );
          }

          return <>{nodes}</>;
        }

        // Fallback: flat rendering (for old blocks without segments)
        // Text comes before tools — matches the typical API order (thinking → text → tool_use)
        const thinkingTools = (block.tools || []).filter(
          (tool) => toolRenderKind(tool) === "thinking",
        );
        const renderedTools = (block.tools || []).filter((tool) => {
          const kind = toolRenderKind(tool);
          return (
            kind !== "hidden" && kind !== "question" && kind !== "thinking"
          );
        });
        return (
          <>
            {recoverySummaryNode}
            {blockThinkingNode}
            {thinkingTools.map((tool) => (
              <AuthoredToolWidget
                key={tool.id}
                tool={tool}
                blockStreaming={block.isStreaming}
              />
            ))}
            {block.content && (
              <div
                style={{
                  color: "var(--text)",
                  fontSize: 13,
                  lineHeight: 1.65,
                  marginBottom: renderedTools.length > 0 ? 6 : 0,
                }}
              >
                <TypedRevealMarkdown
                  text={block.content}
                  active={block.isStreaming}
                  components={mdComponents}
                  charsPerSecond={900}
                  maxStep={96}
                  snapBacklog={2400}
                />
              </div>
            )}
            {renderedTools.length > 0 && (
              <div>
                {renderedTools.map((tool) => (
                  <AuthoredToolWidget
                    key={tool.id}
                    tool={tool}
                    blockStreaming={block.isStreaming}
                  />
                ))}
              </div>
            )}
            {compactionSummaryNode}
            {block.isStreaming &&
              (!hasContent || showInlineActivity) &&
              !isCompactingStatus && (
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <span
                    className="spinner"
                    style={{
                      width: 9,
                      height: 9,
                      border: "1.5px solid var(--accent)",
                      borderTopColor: "transparent",
                      borderRadius: "50%",
                      display: "inline-block",
                      flexShrink: 0,
                    }}
                  />
                  <span style={{ color: "var(--text-dim)", fontSize: 12 }}>
                    {getWorkingLabel()}
                  </span>
                </div>
              )}
          </>
        );
      })()}
    </div>
  );
});
