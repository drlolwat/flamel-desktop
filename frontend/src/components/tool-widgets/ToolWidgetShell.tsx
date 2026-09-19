import React, {
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useId,
  useRef,
  useState,
} from "react";
import { Icon } from "../ui/Icons";
import "./toolWidgetShell.css";

export type ToolWidgetStatus = "pending" | "running" | "success" | "error" | "empty" | "unknown";

export type ToolWidgetSprite =
  | "workspace"
  | "terminal"
  | "source-control"
  | "browser-research"
  | "media"
  | "data"
  | "assistant-workflow"
  | "integrations";

const SPRITE_PATHS: Record<ToolWidgetSprite, string> = {
  workspace: "/tool-sprites/workspace.webp",
  terminal: "/tool-sprites/terminal.webp",
  "source-control": "/tool-sprites/source-control.webp",
  "browser-research": "/tool-sprites/browser-research.webp",
  media: "/tool-sprites/media.webp",
  data: "/tool-sprites/data.webp",
  "assistant-workflow": "/tool-sprites/assistant-workflow.webp",
  integrations: "/tool-sprites/integrations.webp",
};

const STATUS_LABELS: Record<ToolWidgetStatus, string> = {
  pending: "Queued",
  running: "Running",
  success: "Done",
  error: "Error",
  empty: "Empty",
  unknown: "Unconfirmed",
};

export function spritePathFor(sprite: ToolWidgetSprite): string {
  return SPRITE_PATHS[sprite];
}

export interface ToolWidgetShellProps {
  title: string;
  summary: string;
  status: ToolWidgetStatus;
  children?: ReactNode;
  icon?: ReactNode;
  eyebrow?: string;
  detail?: string;
  statusLabel?: string;
  trailing?: ReactNode;
  sprite?: ToolWidgetSprite;
  spritesEnabled?: boolean;
  defaultExpanded?: boolean;
  expanded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
  className?: string;
  inspector?: ReactNode;
}

export function ToolWidgetShell({
  title,
  summary,
  status,
  children,
  icon,
  eyebrow,
  detail,
  statusLabel,
  trailing,
  defaultExpanded = false,
  expanded: controlledExpanded,
  onExpandedChange,
  className,
  inspector,
}: ToolWidgetShellProps) {
  const [internalExpanded, setInternalExpanded] = useState(defaultExpanded);
  const headerRef = useRef<HTMLButtonElement>(null);
  const generatedId = useId();
  const bodyId = `tool-widget-${generatedId.replace(/:/g, "")}`;
  const expanded = controlledExpanded ?? internalExpanded;
  const canExpand = Boolean(children || inspector);

  const setExpanded = useCallback(
    (next: boolean) => {
      if (!canExpand) return;
      if (controlledExpanded === undefined) setInternalExpanded(next);
      onExpandedChange?.(next);
    },
    [canExpand, controlledExpanded, onExpandedChange],
  );

  const handleRootKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== "Escape" || !expanded) return;
    event.stopPropagation();
    setExpanded(false);
    headerRef.current?.focus();
  };

  return (
    <section
      className={["tool-widget", `tool-widget--${status}`, className]
        .filter(Boolean)
        .join(" ")}
      aria-busy={status === "running" || undefined}
      onKeyDown={handleRootKeyDown}
    >
      <button
        ref={headerRef}
        type="button"
        className="tool-widget__header"
        aria-expanded={canExpand ? expanded : undefined}
        aria-controls={canExpand ? bodyId : undefined}
        onClick={() => setExpanded(!expanded)}
        disabled={!canExpand}
      >
        <span className="tool-widget__identity">
          <span className="tool-widget__icon" aria-hidden="true">
            {icon ?? <Icon.Logo size={15} />}
          </span>
          <span className="tool-widget__heading">
            <span className="tool-widget__title" title={eyebrow ? `${eyebrow} · ${title}` : title}>{title}</span>
          </span>
        </span>

        <span className="tool-widget__summary">
          <span className="tool-widget__summary-text" title={summary}>{summary}</span>
          {detail ? <span className="tool-widget__detail" title={detail}>{detail}</span> : null}
        </span>

        {trailing ? <span className="tool-widget__trailing">{trailing}</span> : null}

        <span className="tool-widget__status" data-status={status}>
          <span className="tool-widget__status-mark" aria-hidden="true" />
          <span>{statusLabel ?? STATUS_LABELS[status]}</span>
        </span>

        {canExpand ? (
          <span className="tool-widget__chevron" aria-hidden="true">
            {expanded ? <Icon.ChevronDown size={12} /> : <Icon.ChevronRight size={12} />}
          </span>
        ) : null}
      </button>

      {expanded && canExpand ? (
        <div
          id={bodyId}
          className="tool-widget__body"
          role="region"
          aria-label={`${title} details`}
          tabIndex={0}
        >
          <div className="tool-widget__content">{children}</div>
          {inspector ? <div className="tool-widget__inspector">{inspector}</div> : null}
        </div>
      ) : null}
    </section>
  );
}
