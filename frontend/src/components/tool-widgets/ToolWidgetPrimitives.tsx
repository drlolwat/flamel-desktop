import React, {
  type KeyboardEvent,
  type ReactNode,
  useId,
  useRef,
  useState,
} from "react";
import type { ToolArtifact } from "../../lib/toolPresentation";
import { Icon } from "../ui/Icons";
import type { ToolWidgetStatus } from "./ToolWidgetShell";
import "./toolWidgets.css";

export interface ToolWidgetTab {
  id: string;
  label: string;
  content: ReactNode;
}

export function ToolWidgetTabs({
  tabs,
  label = "Tool result views",
  defaultTab,
}: {
  tabs: ToolWidgetTab[];
  label?: string;
  defaultTab?: string;
}) {
  const initial = tabs.some((tab) => tab.id === defaultTab) ? defaultTab! : tabs[0]?.id;
  const [active, setActive] = useState(initial);
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const groupId = useId();
  const activeTab = tabs.find((tab) => tab.id === active) ?? tabs[0];

  const moveFocus = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? tabs.length - 1
          : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    const next = tabs[nextIndex];
    if (!next) return;
    setActive(next.id);
    refs.current[nextIndex]?.focus();
  };

  if (!activeTab) return null;

  return (
    <div className="tool-widget-tabs">
      <div className="tool-widget-tabs__list" role="tablist" aria-label={label}>
        {tabs.map((tab, index) => {
          const selected = tab.id === activeTab.id;
          return (
            <button
              key={tab.id}
              ref={(node) => {
                refs.current[index] = node;
              }}
              type="button"
              role="tab"
              id={`${groupId}-tab-${tab.id}`}
              aria-controls={`${groupId}-panel-${tab.id}`}
              aria-selected={selected}
              tabIndex={selected ? 0 : -1}
              className="tool-widget-tabs__tab"
              onClick={() => setActive(tab.id)}
              onKeyDown={(event) => moveFocus(event, index)}
            >
              {tab.label}
            </button>
          );
        })}
      </div>
      <div
        className="tool-widget-tabs__panel"
        role="tabpanel"
        id={`${groupId}-panel-${activeTab.id}`}
        aria-labelledby={`${groupId}-tab-${activeTab.id}`}
      >
        {activeTab.content}
      </div>
    </div>
  );
}

export function ToolWidgetSection({
  title,
  description,
  action,
  children,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="tool-widget-section">
      <header className="tool-widget-section__header">
        <div>
          <h4>{title}</h4>
          {description ? <p>{description}</p> : null}
        </div>
        {action ? <div className="tool-widget-section__action">{action}</div> : null}
      </header>
      {children}
    </section>
  );
}

export interface ToolWidgetField {
  label: string;
  value: ReactNode;
  mono?: boolean;
}

export function ToolWidgetFields({ fields }: { fields: ToolWidgetField[] }) {
  if (fields.length === 0) return null;
  return (
    <dl className="tool-widget-fields">
      {fields.map((field) => (
        <div className="tool-widget-fields__row" key={field.label}>
          <dt>{field.label}</dt>
          <dd className={field.mono ? "tool-widget-mono" : undefined}>{field.value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function ToolWidgetList({
  items,
  emptyLabel = "No items",
}: {
  items: Array<{ id: string; primary: ReactNode; secondary?: ReactNode; trailing?: ReactNode }>;
  emptyLabel?: string;
}) {
  if (items.length === 0) {
    return <ToolWidgetState status="empty" title={emptyLabel} />;
  }
  return (
    <ul className="tool-widget-list">
      {items.map((item) => (
        <li key={item.id}>
          <span className="tool-widget-list__text">
            <span className="tool-widget-list__primary">{item.primary}</span>
            {item.secondary ? (
              <span className="tool-widget-list__secondary">{item.secondary}</span>
            ) : null}
          </span>
          {item.trailing ? <span className="tool-widget-list__trailing">{item.trailing}</span> : null}
        </li>
      ))}
    </ul>
  );
}

export function ToolWidgetTable({
  columns,
  rows,
  caption,
  compact = false,
}: {
  columns: Array<{ key: string; label: string; align?: "left" | "right" }>;
  rows: Array<Record<string, ReactNode>>;
  caption?: string;
  compact?: boolean;
}) {
  return (
    <div className="tool-widget-table-wrap">
      <table className="tool-widget-table" style={compact ? { minWidth: 0, fontSize: 11 } : undefined}>
        {caption ? <caption>{caption}</caption> : null}
        <thead>
          <tr>
            {columns.map((column) => (
              <th key={column.key} style={{ textAlign: column.align ?? "left", ...(compact ? { padding: '5px 7px' } : {}) }}>
                {column.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <tr key={String(row.id ?? index)}>
              {columns.map((column, columnIndex) => (
                <td key={column.key} style={{ textAlign: column.align ?? "left", ...(compact ? { padding: '5px 7px', ...(columnIndex === 0 ? { whiteSpace: 'normal', overflowWrap: 'anywhere' } : {}) } : {}) }}>
                  {row[column.key] ?? "—"}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export interface ToolTimelineItem {
  id: string;
  title: ReactNode;
  detail?: ReactNode;
  meta?: ReactNode;
  status?: ToolWidgetStatus;
}

export function ToolWidgetTimeline({ items }: { items: ToolTimelineItem[] }) {
  return (
    <ol className="tool-widget-timeline">
      {items.map((item) => (
        <li key={item.id} data-status={item.status ?? "success"}>
          <span className="tool-widget-timeline__rail" aria-hidden="true">
            <span className="tool-widget-timeline__mark" />
          </span>
          <span className="tool-widget-timeline__copy">
            <span className="tool-widget-timeline__title">{item.title}</span>
            {item.detail ? <span className="tool-widget-timeline__detail">{item.detail}</span> : null}
          </span>
          {item.meta ? <span className="tool-widget-timeline__meta">{item.meta}</span> : null}
        </li>
      ))}
    </ol>
  );
}

export function ToolArtifactActions({ artifacts }: { artifacts: ToolArtifact[] }) {
  if (artifacts.length === 0) return null;
  return (
    <ul className="tool-widget-artifacts" aria-label="Artifacts">
      {artifacts.map((artifact) => (
        <li key={artifact.id || artifact.contentUrl}>
          <span className="tool-widget-artifacts__icon" aria-hidden="true">
            {artifact.kind === "image" ? <Icon.Image size={14} /> : <Icon.File size={14} />}
          </span>
          <span className="tool-widget-artifacts__copy">
            <span className="tool-widget-artifacts__name">{artifact.name}</span>
            <span className="tool-widget-artifacts__meta">
              {artifact.mediaType} · {formatBytes(artifact.byteSize)}
            </span>
          </span>
          <a href={artifact.contentUrl} target="_blank" rel="noreferrer">
            Open
          </a>
          <a href={artifact.downloadUrl ?? artifact.contentUrl} download={artifact.name}>
            <Icon.Download size={13} />
            <span>Download</span>
          </a>
        </li>
      ))}
    </ul>
  );
}

export function ToolWidgetState({
  status,
  title,
  detail,
}: {
  status: Extract<ToolWidgetStatus, "pending" | "running" | "error" | "empty">;
  title: string;
  detail?: string;
}) {
  return (
    <div
      className="tool-widget-state"
      data-status={status}
      role={status === "error" ? "alert" : "status"}
      aria-live={status === "running" ? "polite" : undefined}
    >
      <span className="tool-widget-state__mark" aria-hidden="true" />
      <span>
        <strong>{title}</strong>
        {detail ? <span>{detail}</span> : null}
      </span>
    </div>
  );
}

export function ToolWidgetInspector({
  input,
  output,
  raw,
}: {
  input?: unknown;
  output?: unknown;
  raw?: unknown;
}) {
  const [open, setOpen] = useState(false);
  const tabs: ToolWidgetTab[] = [];
  if (input !== undefined) {
    tabs.push({ id: "input", label: "Input", content: <InspectorValue value={input} /> });
  }
  if (output !== undefined) {
    tabs.push({ id: "output", label: "Output", content: <InspectorValue value={output} /> });
  }
  if (raw !== undefined) {
    tabs.push({ id: "raw", label: "Raw", content: <InspectorValue value={raw} /> });
  }

  if (tabs.length === 0) return null;

  return (
    <details
      className="tool-widget-raw"
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>Inspect raw data</summary>
      {open ? <ToolWidgetTabs tabs={tabs} label="Raw tool data" /> : null}
    </details>
  );
}

function InspectorValue({ value }: { value: unknown }) {
  const [copied, setCopied] = useState(false);
  const serialized = serializeInspectorValue(value);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(serialized);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1_200);
    } catch {
      setCopied(false);
    }
  };

  const download = () => {
    const blob = new Blob([serialized], { type: "application/json;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "tool-result.json";
    anchor.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="tool-widget-inspector-value">
      <div className="tool-widget-inspector-value__actions">
        <button type="button" onClick={copy}>
          <Icon.Copy size={12} />
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
        <button type="button" onClick={download}>
          <Icon.Download size={12} />
          <span>Download</span>
        </button>
        <span className="sr-only" aria-live="polite">
          {copied ? "Raw data copied" : ""}
        </span>
      </div>
      <pre tabIndex={0}>{serialized}</pre>
    </div>
  );
}

const MAX_INSPECTOR_CHARS = 64 * 1024;
const INSPECTOR_CLIPPED_NOTE = "\n… raw data clipped for this inspector";

export function serializeInspectorValue(value: unknown): string {
  let serialized: string;
  if (typeof value === "string") {
    serialized = value;
  } else {
    try {
      serialized = JSON.stringify(value, null, 2);
    } catch {
      serialized = String(value);
    }
  }
  if (serialized.length <= MAX_INSPECTOR_CHARS) return serialized;
  return serialized.slice(0, MAX_INSPECTOR_CHARS) + INSPECTOR_CLIPPED_NOTE;
}

function formatBytes(bytes: number): string {
  if (bytes < 1_024) return `${bytes} B`;
  if (bytes < 1_048_576) return `${(bytes / 1_024).toFixed(1)} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}
