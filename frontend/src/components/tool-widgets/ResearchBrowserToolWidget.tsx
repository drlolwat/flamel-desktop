import React, { useMemo } from "react";
import type { ToolCall } from "../../lib/messageProcessor";
import { shortToolName } from "../../lib/toolCapabilities";
export { isBrowserToolCall, isResearchToolCall } from "./toolWidgetRouting";
import { Icon } from "../ui/Icons";
import {
  ToolWidgetFields,
  ToolWidgetInspector,
  ToolWidgetList,
  ToolWidgetSection,
  ToolWidgetState,
  ToolWidgetTimeline,
  type ToolTimelineItem,
} from "./ToolWidgetPrimitives";
import { buildMediaToolView, MediaPreviewGallery } from "./MediaToolWidget";
import { ToolWidgetShell } from "./ToolWidgetShell";

type JsonRecord = Record<string, unknown>;
type CardKind = "research" | "browser";
type WidgetStatus = "pending" | "running" | "success" | "error" | "empty";

export interface ResearchSource {
  id: string;
  title: string;
  url?: string;
  domain?: string;
  snippet?: string;
}

export interface ResearchView {
  query: string;
  sources: ResearchSource[];
  status: WidgetStatus;
  summary: string;
}

export interface BrowserView {
  action: string;
  url?: string;
  title?: string;
  timeline: ToolTimelineItem[];
  status: WidgetStatus;
  summary: string;
  consoleCount: number;
  networkCount: number;
  auditCount: number;
  errors: string[];
}

function asRecord(value: unknown): JsonRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : undefined;
}

function stringValue(value: unknown, ...keys: string[]): string | undefined {
  const record = asRecord(value);
  for (const key of keys) {
    const candidate = record?.[key];
    if (typeof candidate === "string" && candidate.trim())
      return candidate.trim();
  }
  return undefined;
}

function safeHttpUrl(value: unknown): string | undefined {
  if (typeof value !== "string" || value.length > 4096) return undefined;
  return /^https?:\/\//i.test(value) ? value : undefined;
}

function outputRecord(tool: ToolCall): JsonRecord | undefined {
  const structured = asRecord(tool.presentation?.structuredContent);
  if (structured) return structured;
  if (!tool.output) return undefined;
  try {
    const parsed = asRecord(JSON.parse(tool.output));
    return (
      asRecord(parsed?.structuredContent) ??
      asRecord(parsed?.structured_content) ??
      asRecord(parsed?.data) ??
      parsed
    );
  } catch {
    return undefined;
  }
}

function statusFor(tool: ToolCall, hasResult: boolean): WidgetStatus {
  if (tool.status === "running") return "running";
  if (tool.status === "pending") return "pending";
  if (tool.status === "error" || tool.presentation?.isError) return "error";
  return hasResult ? "success" : "empty";
}

function firstArray(record: JsonRecord | undefined, keys: string[]): unknown[] {
  if (!record) return [];
  for (const key of keys) {
    const candidate = record[key];
    if (Array.isArray(candidate)) return candidate;
  }
  for (const value of Object.values(record)) {
    const nested = asRecord(value);
    if (!nested) continue;
    const found = firstArray(nested, keys);
    if (found.length) return found;
  }
  return [];
}

function domainFor(url?: string): string | undefined {
  if (!url) return undefined;
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return undefined;
  }
}

function queryFromInput(input: JsonRecord): string {
  const direct = stringValue(input, "query", "q", "url");
  if (direct) return direct;
  for (const key of ["search_query", "image_query"]) {
    const collection = input[key];
    if (Array.isArray(collection)) {
      const query = collection
        .map((candidate) => stringValue(candidate, "q", "query"))
        .filter(Boolean)
        .join(" · ");
      if (query) return query;
    }
  }
  return stringValue(input.action, "query", "q", "url") ?? "Web research";
}

function markdownSources(text?: string): ResearchSource[] {
  if (!text) return [];
  const sources: ResearchSource[] = [];
  const markdownLink = /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g;
  for (const match of text.matchAll(markdownLink)) {
    const url = safeHttpUrl(match[2]);
    if (!url || sources.some((source) => source.url === url)) continue;
    sources.push({
      id: url,
      title: match[1].trim() || domainFor(url) || url,
      url,
      domain: domainFor(url),
    });
  }
  return sources.slice(0, 24);
}

export function buildResearchView(tool: ToolCall): ResearchView {
  const record = outputRecord(tool);
  const candidates = firstArray(record, [
    "results",
    "sources",
    "items",
    "search_results",
    "searchResults",
  ]);
  const sources = candidates
    .map((candidate, index): ResearchSource | undefined => {
      const item = asRecord(candidate);
      if (!item) return undefined;
      const url = safeHttpUrl(
        item.url ?? item.link ?? item.href ?? item.uri ?? item.source_url,
      );
      const title =
        stringValue(item, "title", "name", "label") ??
        domainFor(url) ??
        (url ? url : undefined);
      if (!title) return undefined;
      return {
        id: stringValue(item, "id") ?? url ?? `source-${index}`,
        title,
        url,
        domain: stringValue(item, "domain", "site") ?? domainFor(url),
        snippet: stringValue(item, "snippet", "description", "summary", "text"),
      };
    })
    .filter((source): source is ResearchSource => Boolean(source));
  const combined = [...sources, ...markdownSources(tool.output)].filter(
    (source, index, all) =>
      all.findIndex(
        (candidate) =>
          (candidate.url ?? candidate.title) === (source.url ?? source.title),
      ) === index,
  );
  const status = statusFor(tool, combined.length > 0 || Boolean(tool.output));
  const query = queryFromInput(tool.input ?? {});
  const summary =
    status === "running"
      ? "Searching and ranking sources"
      : status === "pending"
        ? "Waiting to search"
        : status === "error"
          ? "Research failed"
          : combined.length
            ? `${combined.length} ${combined.length === 1 ? "source" : "sources"}`
            : status === "empty"
              ? "No sources returned"
              : "Research complete";
  return { query, sources: combined.slice(0, 24), status, summary };
}

function actionLabel(value: unknown, index: number): string {
  if (typeof value === "string") return value.replace(/_/g, " ");
  const record = asRecord(value);
  return (
    stringValue(record, "action", "type", "name", "fn")?.replace(/_/g, " ") ??
    `Step ${index + 1}`
  );
}

function actionDetail(value: unknown): string | undefined {
  const record = asRecord(value);
  return stringValue(
    record,
    "url",
    "text",
    "selector",
    "condition",
    "uid",
    "result",
  );
}

function timelineStatus(value: unknown): ToolTimelineItem["status"] {
  const record = asRecord(value);
  const status = stringValue(record, "status", "state")?.toLowerCase();
  if (status?.includes("error") || status?.includes("fail")) return "error";
  if (status?.includes("running") || status?.includes("active"))
    return "running";
  if (status?.includes("pending") || status?.includes("queued"))
    return "pending";
  return "success";
}

function countCollection(
  record: JsonRecord | undefined,
  keys: string[],
): number {
  if (!record) return 0;
  for (const key of keys) {
    const value = record[key];
    if (Array.isArray(value)) return value.length;
    if (typeof value === "number" && Number.isFinite(value)) return value;
    const nested = asRecord(value);
    if (nested) {
      const count = countCollection(nested, keys);
      if (count) return count;
    }
  }
  return 0;
}

function errorsFrom(record: JsonRecord | undefined, output?: string): string[] {
  const errors = firstArray(record, ["errors", "failures"])
    .map((value) =>
      typeof value === "string"
        ? value
        : stringValue(value, "message", "error", "detail"),
    )
    .filter((value): value is string => Boolean(value));
  if (!errors.length && output && /(?:error|failed|timeout)/i.test(output)) {
    errors.push(
      ...output
        .split(/\r?\n/)
        .filter((line) => /(?:error|failed|timeout)/i.test(line))
        .slice(0, 12),
    );
  }
  return errors.slice(0, 24);
}

export function buildBrowserView(tool: ToolCall): BrowserView {
  const record = outputRecord(tool);
  const inputActions = Array.isArray(tool.input.actions)
    ? tool.input.actions
    : [];
  const outputActions = firstArray(record, ["actions", "steps", "timeline"]);
  const actions = outputActions.length ? outputActions : inputActions;
  const errors = errorsFrom(record, tool.output);
  const url =
    safeHttpUrl(tool.input.url) ??
    safeHttpUrl(record?.url) ??
    safeHttpUrl(asRecord(record?.page)?.url);
  const title =
    stringValue(record, "title") ?? stringValue(record?.page, "title");
  const action = stringValue(tool.input, "action") ?? shortToolName(tool.name);
  const timeline = actions
    .slice(0, 24)
    .map((candidate, index): ToolTimelineItem => ({
      id: stringValue(candidate, "id") ?? `browser-step-${index}`,
      title: actionLabel(candidate, index),
      detail: actionDetail(candidate),
      meta: stringValue(candidate, "duration", "elapsed"),
      status: timelineStatus(candidate),
    }));
  const status = statusFor(
    tool,
    timeline.length > 0 ||
      Boolean(url) ||
      Boolean(title) ||
      Boolean(tool.output),
  );
  const summary =
    status === "running"
      ? `${Math.max(timeline.length, inputActions.length)} actions in progress`
      : status === "pending"
        ? "Browser run queued"
        : status === "error"
          ? "Browser action failed"
          : timeline.length
            ? `${timeline.length} ${timeline.length === 1 ? "step" : "steps"} complete`
            : url
              ? "Page state captured"
              : "Browser session updated";

  return {
    action: action.replace(/_/g, " "),
    url,
    title,
    timeline,
    status,
    summary,
    consoleCount: countCollection(record, [
      "console",
      "consoleMessages",
      "console_count",
    ]),
    networkCount: countCollection(record, [
      "network",
      "requests",
      "network_count",
    ]),
    auditCount: countCollection(record, [
      "audits",
      "lighthouse",
      "audit_count",
    ]),
    errors,
  };
}

function ResearchBody({ view }: { view: ResearchView }) {
  if (!view.sources.length) {
    return (
      <ToolWidgetState
        status={view.status === "success" ? "empty" : view.status}
        title={
          view.status === "running"
            ? "Collecting sources"
            : "No sources to display"
        }
        detail="Raw provider data remains available in the inspector."
      />
    );
  }
  return (
    <ToolWidgetSection
      title="Sources"
      description="Deduplicated links from the provider result."
    >
      <ToolWidgetList
        items={view.sources.map((source) => ({
          id: source.id,
          primary: source.url ? (
            <a
              className="research-widget__source"
              href={source.url}
              target="_blank"
              rel="noreferrer"
            >
              {source.title}
            </a>
          ) : (
            source.title
          ),
          secondary: source.snippet,
          trailing: source.domain,
        }))}
      />
    </ToolWidgetSection>
  );
}

function BrowserBody({ tool, view }: { tool: ToolCall; view: BrowserView }) {
  const screenshots = buildMediaToolView(tool).items.filter(
    (item) => item.kind === "image",
  );
  return (
    <>
      {view.url || view.title ? (
        <ToolWidgetFields
          fields={[
            ...(view.title ? [{ label: "Page", value: view.title }] : []),
            ...(view.url
              ? [{ label: "URL", value: view.url, mono: true }]
              : []),
          ]}
        />
      ) : null}
      {view.consoleCount || view.networkCount || view.auditCount ? (
        <ToolWidgetFields
          fields={[
            { label: "Console", value: String(view.consoleCount) },
            { label: "Network", value: String(view.networkCount) },
            { label: "Audits", value: String(view.auditCount) },
          ]}
        />
      ) : null}
      {view.timeline.length ? (
        <ToolWidgetTimeline items={view.timeline} />
      ) : null}
      {screenshots.length ? (
        <ToolWidgetSection
          title="Captured frames"
          description="Managed screenshots and authenticated workspace captures."
        >
          <MediaPreviewGallery items={screenshots} />
        </ToolWidgetSection>
      ) : null}
      {view.errors.length ? (
        <ToolWidgetSection title="Bounded errors">
          <ol className="browser-widget__errors">
            {view.errors.map((error, index) => (
              <li key={`${index}-${error.slice(0, 32)}`}>{error}</li>
            ))}
          </ol>
        </ToolWidgetSection>
      ) : null}
      {!view.timeline.length && !screenshots.length && !view.errors.length ? (
        <ToolWidgetState
          status={view.status === "success" ? "empty" : view.status}
          title={
            view.status === "running"
              ? "Browser is active"
              : "No browser evidence"
          }
          detail="The card will update when the browser returns a page, event, or capture."
        />
      ) : null}
    </>
  );
}

export const ResearchBrowserToolWidget = React.memo(
  function ResearchBrowserToolWidget({
    tool,
    kind,
    defaultExpanded,
  }: {
    tool: ToolCall;
    kind: CardKind;
    defaultExpanded?: boolean;
  }) {
    const research = useMemo(
      () => (kind === "research" ? buildResearchView(tool) : undefined),
      [kind, tool],
    );
    const browser = useMemo(
      () => (kind === "browser" ? buildBrowserView(tool) : undefined),
      [kind, tool],
    );
    const status = research?.status ?? browser?.status ?? "empty";
    const title =
      kind === "research"
        ? (research?.query ?? "Web research")
        : (browser?.title ?? "Browser");
    const summary = research?.summary ?? browser?.summary ?? "No result";
    const detail =
      kind === "browser" ? (browser?.url ?? browser?.action) : "ranked sources";

    return (
      <ToolWidgetShell
        title={title}
        eyebrow={kind === "research" ? "Research" : "Browser"}
        summary={summary}
        detail={detail}
        status={status}
        statusLabel={status === "running" ? "Live" : undefined}
        icon={
          kind === "research" ? (
            <Icon.History size={14} />
          ) : (
            <Icon.ExternalLink size={14} />
          )
        }
        sprite="browser-research"
        defaultExpanded={defaultExpanded ?? false}
        inspector={
          <ToolWidgetInspector
            input={tool.input}
            output={tool.output}
            raw={{ presentation: tool.presentation }}
          />
        }
      >
        {research ? <ResearchBody view={research} /> : null}
        {browser ? <BrowserBody tool={tool} view={browser} /> : null}
      </ToolWidgetShell>
    );
  },
);
