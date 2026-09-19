import React, { useMemo } from "react";
import type { ToolCall } from "../../lib/messageProcessor";
import { Icon } from "../ui/Icons";
import { EmbeddedCodeView } from "./EmbeddedCodeView";
import {
  ToolArtifactActions,
  ToolWidgetFields,
  ToolWidgetList,
  ToolWidgetSection,
  ToolWidgetState,
  ToolWidgetTable,
  ToolWidgetTabs,
  ToolWidgetTimeline,
} from "./ToolWidgetPrimitives";
import { ToolWidgetShell, type ToolWidgetSprite } from "./ToolWidgetShell";
import {
  adaptAdaptiveDataTool,
  isAdaptiveDataToolCall,
  type AdaptiveToolView,
} from "./adaptiveToolAdapters";

export { adaptAdaptiveDataTool, isAdaptiveDataToolCall };

function spriteFor(view: AdaptiveToolView): ToolWidgetSprite {
  if (view.family === "integrations" || view.family === "external") {
    return "integrations";
  }
  return "data";
}

function iconFor(view: AdaptiveToolView) {
  if (view.family === "knowledge") return <Icon.History size={14} />;
  if (view.family === "integrations") return <Icon.Prompts size={14} />;
  if (view.family === "external") return <Icon.ExternalLink size={14} />;
  return <Icon.Drive size={14} />;
}

function AdaptiveMetrics({ view }: { view: AdaptiveToolView }) {
  if (!view.metrics.length) return null;
  return (
    <dl className="adaptive-data__metrics" aria-label="Metrics">
      {view.metrics.map((metric) => (
        <div key={metric.id} data-trend={metric.trend}>
          <dt>{metric.label}</dt>
          <dd>
            <strong>{metric.value}</strong>
            {metric.detail ? <span>{metric.detail}</span> : null}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function AdaptiveResources({ view }: { view: AdaptiveToolView }) {
  if (!view.resources.length) return null;
  return (
    <ToolWidgetSection
      title="Resources"
      description="Only HTTP(S) and application-relative links are activated."
    >
      <ul className="adaptive-data__resources">
        {view.resources.map((resource) => (
          <li key={resource.id}>
            <span>
              <strong>{resource.title}</strong>
              {resource.description ? (
                <span>{resource.description}</span>
              ) : null}
              <code>{resource.uri}</code>
            </span>
            <span>
              {resource.mediaType ? <small>{resource.mediaType}</small> : null}
              {resource.href ? (
                <a href={resource.href} target="_blank" rel="noreferrer">
                  Open
                </a>
              ) : (
                <small>Link disabled</small>
              )}
            </span>
          </li>
        ))}
      </ul>
    </ToolWidgetSection>
  );
}

function AdaptiveOverview({
  tool,
  view,
}: {
  tool: ToolCall;
  view: AdaptiveToolView;
}) {
  if (view.status === "error") {
    return (
      <ToolWidgetState
        status="error"
        title="Tool call failed"
        detail={view.summary}
      />
    );
  }
  if (view.status === "running" || view.status === "pending") {
    return (
      <ToolWidgetState
        status={view.status}
        title={
          view.status === "running"
            ? "Receiving structured evidence"
            : "Waiting to start"
        }
        detail="The authored layout will settle as validated result shapes arrive."
      />
    );
  }
  if (view.empty) {
    return (
      <ToolWidgetState
        status="empty"
        title="No structured result"
        detail="The call completed without displayable fields, rows, documents, or resources."
      />
    );
  }

  return (
    <>
      <AdaptiveMetrics view={view} />
      {view.fields.length ? (
        <ToolWidgetFields
          fields={view.fields.map((field) => ({
            label: field.label,
            value: field.value === null ? "—" : String(field.value),
            mono: field.mono,
          }))}
        />
      ) : null}
      {view.table ? (
        <ToolWidgetSection
          title={view.table.title}
          description={
            view.table.truncated
              ? `Showing ${view.table.shown} of ${view.table.total} rows within the card limit.`
              : `${view.table.total} row${view.table.total === 1 ? "" : "s"}`
          }
        >
          <ToolWidgetTable
            columns={view.table.columns}
            rows={view.table.rows}
            caption={view.table.title}
          />
        </ToolWidgetSection>
      ) : null}
      {view.collection.length ? (
        <ToolWidgetSection
          title={view.family === "knowledge" ? "Matches" : "Results"}
          description="A bounded, validated collection from the provider response."
        >
          <ToolWidgetList
            items={view.collection.map((item) => ({
              id: item.id,
              primary: item.href ? (
                <a
                  className="adaptive-data__link"
                  href={item.href}
                  target="_blank"
                  rel="noreferrer"
                >
                  {item.title}
                </a>
              ) : (
                item.title
              ),
              secondary: item.description,
              trailing: item.meta,
            }))}
          />
        </ToolWidgetSection>
      ) : null}
      {view.timeline.length ? (
        <ToolWidgetSection title="Timeline">
          <ToolWidgetTimeline items={view.timeline} />
        </ToolWidgetSection>
      ) : null}
      {view.document ? (
        <ToolWidgetSection
          title={view.family === "knowledge" ? "Document" : "Response"}
        >
          <article className="adaptive-data__document">{view.document}</article>
        </ToolWidgetSection>
      ) : null}
      <AdaptiveResources view={view} />
      {tool.artifacts?.length ? (
        <ToolWidgetSection title="Artifacts">
          <ToolArtifactActions artifacts={tool.artifacts} />
        </ToolWidgetSection>
      ) : null}
      {view.limited ? (
        <div className="adaptive-data__limit-note">
          The visible layout is bounded for readability. The redacted source
          remains available under Raw.
        </div>
      ) : null}
    </>
  );
}

function downloadRaw(view: AdaptiveToolView, tool: ToolCall) {
  const blob = new Blob([view.rawDownload], {
    type: "application/json;charset=utf-8",
  });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `${tool.name.replace(/[^a-z0-9_-]+/gi, "-") || "tool"}-result.json`;
  anchor.click();
  URL.revokeObjectURL(url);
}

function AdaptiveRaw({
  tool,
  view,
}: {
  tool: ToolCall;
  view: AdaptiveToolView;
}) {
  return (
    <div className="adaptive-data__raw">
      {(view.rawTruncated || view.limited) && (
        <div className="adaptive-data__raw-toolbar">
          <span>
            {view.rawTruncated
              ? "Editor preview truncated"
              : "Visible layout bounded"}
          </span>
          <button type="button" onClick={() => downloadRaw(view, tool)}>
            Download redacted result
          </button>
        </div>
      )}
      <EmbeddedCodeView
        path={`${tool.name.replace(/[^a-z0-9_-]+/gi, "-") || "tool"}-result.json`}
        value={view.rawPreview || "No raw result"}
        language="json"
        label="Read-only JSON result"
        maximumHeight={480}
        minimumHeight={180}
      />
    </div>
  );
}

export function AdaptiveDataToolWidget({
  tool,
  defaultExpanded,
}: {
  tool: ToolCall;
  defaultExpanded?: boolean;
}) {
  const view = useMemo(() => adaptAdaptiveDataTool(tool), [tool]);
  if (!view) return null;

  return (
    <ToolWidgetShell
      title={view.title}
      eyebrow={view.eyebrow}
      summary={view.summary}
      detail={view.detail}
      status={view.status}
      icon={iconFor(view)}
      sprite={spriteFor(view)}
      trailing={
        view.table?.truncated
          ? `${view.table.shown}/${view.table.total}`
          : view.rawTruncated
            ? "bounded"
            : undefined
      }
      defaultExpanded={defaultExpanded ?? false}
    >
      <ToolWidgetTabs
        label="Structured and raw result views"
        tabs={[
          {
            id: "overview",
            label: "Overview",
            content: <AdaptiveOverview tool={tool} view={view} />,
          },
          {
            id: "raw",
            label: "Raw",
            content: <AdaptiveRaw tool={tool} view={view} />,
          },
        ]}
      />
    </ToolWidgetShell>
  );
}
