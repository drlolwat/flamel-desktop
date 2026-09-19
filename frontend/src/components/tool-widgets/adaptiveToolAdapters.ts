import type { ToolCall } from "../../lib/messageProcessor";
import {
  capabilityForTool,
  type ToolCapability,
  type ToolFamily,
} from "../../lib/toolCapabilities";
import type { ToolWidgetStatus } from "./ToolWidgetShell";

export type AdaptiveRecord = Record<string, unknown>;
export type AdaptiveScalar = string | number | boolean | null;

export interface AdaptiveField {
  label: string;
  value: AdaptiveScalar;
  mono?: boolean;
}

export interface AdaptiveMetric {
  id: string;
  label: string;
  value: string;
  detail?: string;
  trend?: "up" | "down" | "neutral";
}

export interface AdaptiveTable {
  title: string;
  columns: Array<{ key: string; label: string; align?: "left" | "right" }>;
  rows: Array<Record<string, AdaptiveScalar>>;
  shown: number;
  total: number;
  truncated: boolean;
}

export interface AdaptiveCollectionItem {
  id: string;
  title: string;
  description?: string;
  meta?: string;
  href?: string;
}

export interface AdaptiveTimelineItem {
  id: string;
  title: string;
  detail?: string;
  meta?: string;
  status: ToolWidgetStatus;
}

export interface AdaptiveResource {
  id: string;
  title: string;
  description?: string;
  uri: string;
  href?: string;
  mediaType?: string;
}

export interface AdaptiveToolView {
  capability: ToolCapability;
  family: Extract<
    ToolFamily,
    "data" | "knowledge" | "integrations" | "external"
  >;
  title: string;
  eyebrow: string;
  summary: string;
  detail?: string;
  status: ToolWidgetStatus;
  fields: AdaptiveField[];
  metrics: AdaptiveMetric[];
  table?: AdaptiveTable;
  collection: AdaptiveCollectionItem[];
  timeline: AdaptiveTimelineItem[];
  document?: string;
  resources: AdaptiveResource[];
  rawPreview: string;
  rawDownload: string;
  rawTruncated: boolean;
  limited: boolean;
  empty: boolean;
}

const MAX_PARSE_CHARACTERS = 2_000_000;
const MAX_RAW_PREVIEW_CHARACTERS = 80_000;
const MAX_DOCUMENT_CHARACTERS = 12_000;
const MAX_CELL_CHARACTERS = 240;
const MAX_ROWS = 50;
const MAX_COLUMNS = 12;
const MAX_COLLECTION_ITEMS = 50;
const MAX_TIMELINE_ITEMS = 50;
const MAX_RESOURCES = 24;
const SECRET_KEY =
  /(?:secret|token|password|passphrase|api[_-]?key|authorization|cookie|private[_-]?key|credential|client[_-]?secret)/i;
const TOKEN_VALUE =
  /(?:Bearer\s+)[A-Za-z0-9._~+/=-]{12,}|\b(?:sk|ghp|github_pat|xox[baprs])[-_][A-Za-z0-9_-]{12,}\b/gi;
const TITLE_KEYS = [
  "title",
  "name",
  "label",
  "display_name",
  "displayName",
  "subject",
];
const DESCRIPTION_KEYS = [
  "description",
  "summary",
  "snippet",
  "excerpt",
  "detail",
  "message",
  "content",
];
const STATUS_KEYS = ["status", "state", "result", "outcome"];
const URL_KEYS = ["url", "href", "link", "uri", "web_url", "html_url"];
const TIME_KEYS = [
  "timestamp",
  "created_at",
  "updated_at",
  "occurred_at",
  "started_at",
  "completed_at",
  "date",
  "time",
];
const ARRAY_PRIORITY = [
  "rows",
  "records",
  "results",
  "items",
  "data",
  "entries",
  "documents",
  "matches",
  "tools",
  "capabilities",
  "tables",
  "columns",
];

function record(value: unknown): AdaptiveRecord {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as AdaptiveRecord)
    : {};
}

function scalar(value: unknown): AdaptiveScalar | undefined {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return value;
  }
  return undefined;
}

function stringValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  return "";
}

function firstString(value: AdaptiveRecord, keys: string[]): string {
  for (const key of keys) {
    const candidate = stringValue(value[key]).trim();
    if (candidate) return candidate;
  }
  return "";
}

function humanize(value: string): string {
  return value
    .replace(/[-_.]+/g, " ")
    .replace(/([a-z\d])([A-Z])/g, "$1 $2")
    .trim()
    .replace(/^./, (letter) => letter.toUpperCase());
}

function clip(value: string, maximum = MAX_CELL_CHARACTERS): string {
  if (value.length <= maximum) return value;
  return `${value.slice(0, Math.max(0, maximum - 1)).trimEnd()}…`;
}

export function redactAdaptiveString(value: string): string {
  return value
    .replace(TOKEN_VALUE, (match) => {
      const prefix = match.toLowerCase().startsWith("bearer") ? "Bearer " : "";
      return `${prefix}[redacted]`;
    })
    .replace(
      /("(?:secret|token|password|passphrase|api[_-]?key|authorization|cookie|private[_-]?key|credential|client[_-]?secret)"\s*:\s*")([^"]*)(")/gi,
      "$1[redacted]$3",
    );
}

export function redactAdaptiveValue(
  value: unknown,
  key = "",
  depth = 0,
  seen = new WeakSet<object>(),
): unknown {
  if (SECRET_KEY.test(key)) return "[redacted]";
  if (typeof value === "string") return redactAdaptiveString(value);
  if (value === null || typeof value !== "object") return value;
  if (depth >= 8) return "[depth limited]";
  if (seen.has(value)) return "[circular]";
  seen.add(value);
  if (Array.isArray(value)) {
    return value
      .slice(0, 500)
      .map((entry) => redactAdaptiveValue(entry, key, depth + 1, seen));
  }
  return Object.fromEntries(
    Object.entries(record(value))
      .slice(0, 500)
      .map(([entryKey, entryValue]) => [
        entryKey,
        redactAdaptiveValue(entryValue, entryKey, depth + 1, seen),
      ]),
  );
}

function parseJson(text: string): unknown {
  if (!text.trim() || text.length > MAX_PARSE_CHARACTERS) return undefined;
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

function serializeRaw(value: unknown): string {
  if (typeof value === "string") return redactAdaptiveString(value);
  try {
    return JSON.stringify(redactAdaptiveValue(value), null, 2);
  } catch {
    return redactAdaptiveString(String(value));
  }
}

function safeHref(value: string): string | undefined {
  const candidate = value.trim();
  if (!candidate) return undefined;
  if (candidate.startsWith("/") && !candidate.startsWith("//"))
    return candidate;
  try {
    const url = new URL(candidate);
    return url.protocol === "https:" || url.protocol === "http:"
      ? url.toString()
      : undefined;
  } catch {
    return undefined;
  }
}

interface UnwrappedOutput {
  payload: AdaptiveRecord;
  text: string;
  raw: unknown;
  resources: AdaptiveResource[];
}

function resourceFromRecord(
  value: unknown,
  index: number,
): AdaptiveResource | undefined {
  const item = record(value);
  const uri = firstString(item, URL_KEYS);
  if (!uri) return undefined;
  return {
    id: firstString(item, ["id"]) || `resource-${index}`,
    title:
      firstString(item, TITLE_KEYS) ||
      humanize(uri.split("/").pop() || "Resource"),
    description: clip(firstString(item, DESCRIPTION_KEYS), 360) || undefined,
    uri: clip(redactAdaptiveString(uri), 1_000),
    href: safeHref(uri),
    mediaType:
      firstString(item, ["mimeType", "mime_type", "mediaType"]) || undefined,
  };
}

function unwrapOutput(tool: ToolCall): UnwrappedOutput {
  const parsed = parseJson(tool.output || "");
  const outer = record(parsed);
  const presentation = record(tool.presentation?.structuredContent);
  const directStructured = record(
    outer.structuredContent ?? outer.structured_content,
  );
  const content = Array.isArray(outer.content) ? outer.content : [];
  const textBlocks = content
    .map((block) => {
      const item = record(block);
      return item.type === "text" ? stringValue(item.text) : "";
    })
    .filter(Boolean);
  const blockJson = textBlocks
    .map((text) => parseJson(text))
    .find((value) => value !== undefined);
  const blockPayload = record(blockJson);
  const resources = content
    .map((block, index) => {
      const item = record(block);
      return item.type === "resource_link"
        ? resourceFromRecord(item, index)
        : undefined;
    })
    .filter((value): value is AdaptiveResource => Boolean(value));
  const payload =
    Object.keys(presentation).length > 0
      ? presentation
      : Object.keys(directStructured).length > 0
        ? directStructured
        : Object.keys(blockPayload).length > 0
          ? blockPayload
          : Object.keys(outer).length > 0
            ? outer
            : Array.isArray(parsed)
              ? { items: parsed }
              : scalar(parsed) !== undefined
                ? { value: parsed }
                : {};
  const text =
    firstString(payload, ["text", "message", "summary", "answer"]) ||
    textBlocks.join("\n") ||
    (parsed === undefined
      ? tool.output || ""
      : typeof parsed === "string"
        ? parsed
        : "");
  return {
    payload,
    text,
    raw: parsed ?? tool.output ?? "",
    resources,
  };
}

interface WalkEntry {
  key: string;
  path: string[];
  value: unknown;
}

function walk(
  value: unknown,
  depth = 0,
  path: string[] = [],
  seen = new WeakSet<object>(),
): WalkEntry[] {
  if (value === null || typeof value !== "object" || depth > 4) return [];
  if (seen.has(value)) return [];
  seen.add(value);
  const entries: WalkEntry[] = [];
  for (const [key, child] of Object.entries(record(value)).slice(0, 120)) {
    const nextPath = [...path, key];
    entries.push({ key, path: nextPath, value: child });
    if (child && typeof child === "object" && !Array.isArray(child)) {
      entries.push(...walk(child, depth + 1, nextPath, seen));
    }
  }
  return entries;
}

function semanticRoot(payload: AdaptiveRecord): AdaptiveRecord {
  let current = payload;
  for (let depth = 0; depth < 3; depth += 1) {
    const keys = Object.keys(current);
    const wrapperKeys = new Set([
      "result",
      "data",
      "response",
      "payload",
      "ok",
      "status",
      "state",
      "meta",
      "error",
      "message",
    ]);
    const wrapper = ["result", "data", "response", "payload"].find(
      (key) =>
        key in current &&
        Object.keys(record(current[key])).length > 0 &&
        keys.every((candidate) => wrapperKeys.has(candidate)),
    );
    if (!wrapper) break;
    current = record(current[wrapper]);
  }
  return current;
}

function arrayCandidates(payload: AdaptiveRecord): WalkEntry[] {
  const candidates = walk(payload).filter(
    (entry) => Array.isArray(entry.value) && entry.value.length > 0,
  );
  return candidates.sort((left, right) => {
    const leftPriority = ARRAY_PRIORITY.indexOf(left.key);
    const rightPriority = ARRAY_PRIORITY.indexOf(right.key);
    const leftScore = leftPriority < 0 ? ARRAY_PRIORITY.length : leftPriority;
    const rightScore =
      rightPriority < 0 ? ARRAY_PRIORITY.length : rightPriority;
    return leftScore - rightScore || left.path.length - right.path.length;
  });
}

function scalarKeys(rows: AdaptiveRecord[]): string[] {
  const counts = new Map<string, number>();
  for (const row of rows.slice(0, 25)) {
    for (const [key, value] of Object.entries(row)) {
      if (scalar(value) !== undefined) {
        counts.set(key, (counts.get(key) ?? 0) + 1);
      }
    }
  }
  return [...counts.entries()]
    .sort((left, right) => right[1] - left[1])
    .map(([key]) => key)
    .slice(0, MAX_COLUMNS);
}

function cellValue(key: string, value: unknown): AdaptiveScalar {
  if (SECRET_KEY.test(key)) return "[redacted]";
  const direct = scalar(value);
  if (direct === null) return null;
  if (direct !== undefined) {
    return typeof direct === "string"
      ? clip(redactAdaptiveString(direct))
      : direct;
  }
  if (Array.isArray(value)) return `${value.length} items`;
  const object = record(value);
  return Object.keys(object).length
    ? `${Object.keys(object).length} fields`
    : "—";
}

function tableFromCandidate(
  candidate: WalkEntry,
  capability: ToolCapability,
): AdaptiveTable | undefined {
  const values = candidate.value as unknown[];
  const rows = values.map(record).filter((row) => Object.keys(row).length > 0);
  if (!rows.length) return undefined;
  const columns = scalarKeys(rows);
  const databaseShape =
    capability.family === "data" ||
    /rows|records|tables|columns|schema/i.test(candidate.key);
  if (columns.length < 2 && !databaseShape) return undefined;
  return {
    title: /tables|schema/i.test(candidate.key)
      ? "Schema"
      : capability.family === "data"
        ? "Query result"
        : humanize(candidate.key),
    columns: columns.map((key) => ({
      key,
      label: humanize(key),
      align:
        rows.some((row) => typeof row[key] === "number") &&
        rows.every((row) => row[key] == null || typeof row[key] === "number")
          ? "right"
          : "left",
    })),
    rows: rows
      .slice(0, MAX_ROWS)
      .map((row) =>
        Object.fromEntries(
          columns.map((key) => [key, cellValue(key, row[key])]),
        ),
      ),
    shown: Math.min(rows.length, MAX_ROWS),
    total: rows.length,
    truncated: rows.length > MAX_ROWS || columns.length >= MAX_COLUMNS,
  };
}

function collectionFromCandidate(
  candidate: WalkEntry,
): AdaptiveCollectionItem[] {
  const values = candidate.value as unknown[];
  return values
    .slice(0, MAX_COLLECTION_ITEMS)
    .map((value, index) => {
      const item = record(value);
      const direct = scalar(value);
      if (!Object.keys(item).length) {
        const title = direct === undefined ? "" : stringValue(direct);
        return title
          ? {
              id: `${candidate.path.join("-")}-${index}`,
              title: clip(redactAdaptiveString(title), 360),
            }
          : undefined;
      }
      const title = firstString(item, TITLE_KEYS);
      const uri = firstString(item, URL_KEYS);
      const description = firstString(item, DESCRIPTION_KEYS);
      const meta =
        firstString(item, STATUS_KEYS) ||
        firstString(item, ["type", "kind", "category", "version"]);
      if (!title && !description && !uri) return undefined;
      return {
        id:
          firstString(item, ["id", "key", "slug"]) ||
          `${candidate.path.join("-")}-${index}`,
        title: clip(
          redactAdaptiveString(
            title || description || uri || `Result ${index + 1}`,
          ),
          360,
        ),
        description:
          title && description
            ? clip(redactAdaptiveString(description), 600)
            : undefined,
        meta: clip(redactAdaptiveString(meta), 120) || undefined,
        href: safeHref(uri),
      };
    })
    .filter((value): value is AdaptiveCollectionItem => Boolean(value));
}

function isTimelineCandidate(candidate: WalkEntry): boolean {
  return /events|history|timeline|activity|updates|runs|audit|logs/i.test(
    candidate.key,
  );
}

function timelineFromCandidate(candidate: WalkEntry): AdaptiveTimelineItem[] {
  const values = candidate.value as unknown[];
  return values
    .slice(0, MAX_TIMELINE_ITEMS)
    .map<AdaptiveTimelineItem | undefined>((value, index) => {
      const item = record(value);
      if (!Object.keys(item).length) return undefined;
      const title =
        firstString(item, TITLE_KEYS) ||
        firstString(item, ["type", "kind", "event", "action", "status"]);
      if (!title) return undefined;
      const state = firstString(item, STATUS_KEYS).toLowerCase();
      const status: ToolWidgetStatus = /error|fail|denied|cancel|block/.test(
        state,
      )
        ? "error"
        : /running|active|progress/.test(state)
          ? "running"
          : /pending|queue|wait/.test(state)
            ? "pending"
            : "success";
      return {
        id:
          firstString(item, ["id", "event_id", "run_id"]) ||
          `timeline-${index}`,
        title: clip(redactAdaptiveString(title), 360),
        detail:
          clip(
            redactAdaptiveString(firstString(item, DESCRIPTION_KEYS)),
            600,
          ) || undefined,
        meta:
          clip(redactAdaptiveString(firstString(item, TIME_KEYS)), 160) ||
          undefined,
        status,
      };
    })
    .filter((value): value is AdaptiveTimelineItem => Boolean(value));
}

function metricsFromPayload(payload: AdaptiveRecord): AdaptiveMetric[] {
  const candidate = walk(payload).find(
    (entry) =>
      entry.key.toLowerCase() === "metrics" && Array.isArray(entry.value),
  );
  if (!candidate) return [];
  return (candidate.value as unknown[])
    .slice(0, 16)
    .map<AdaptiveMetric | undefined>((value, index) => {
      const item = record(value);
      const label = firstString(item, ["label", "name", "title", "metric"]);
      const metricValue = firstString(item, [
        "value",
        "count",
        "total",
        "score",
      ]);
      if (!label || !metricValue) return undefined;
      const trendValue = firstString(item, [
        "trend",
        "direction",
      ]).toLowerCase();
      return {
        id: firstString(item, ["id", "key"]) || `metric-${index}`,
        label: clip(redactAdaptiveString(label), 120),
        value: clip(redactAdaptiveString(metricValue), 120),
        detail:
          clip(
            redactAdaptiveString(
              firstString(item, ["delta", "change", "description"]),
            ),
            160,
          ) || undefined,
        trend: /up|increase|positive/.test(trendValue)
          ? "up"
          : /down|decrease|negative/.test(trendValue)
            ? "down"
            : "neutral",
      } satisfies AdaptiveMetric;
    })
    .filter((value): value is AdaptiveMetric => Boolean(value));
}

function documentFromPayload(
  payload: AdaptiveRecord,
  fallbackText: string,
): string | undefined {
  const candidate = walk(payload).find(
    (entry) =>
      /^(?:document|markdown|body|answer|content|text|description)$/i.test(
        entry.key,
      ) &&
      typeof entry.value === "string" &&
      entry.value.trim().length >= 40,
  );
  const text = stringValue(candidate?.value) || fallbackText;
  if (!text.trim()) return undefined;
  return clip(redactAdaptiveString(text.trim()), MAX_DOCUMENT_CHARACTERS);
}

function fieldsFromRoot(root: AdaptiveRecord): AdaptiveField[] {
  const ignored = new Set([
    ...TITLE_KEYS,
    ...DESCRIPTION_KEYS,
    ...URL_KEYS,
    "ok",
    "error",
    "renderer",
    "render",
    "component",
    "html",
    "markup",
    "ui",
    "layout",
  ]);
  return Object.entries(root)
    .filter(([key, value]) => !ignored.has(key) && scalar(value) !== undefined)
    .slice(0, 12)
    .map(([key, value]) => ({
      label: humanize(key),
      value: cellValue(key, value),
      mono: /id|hash|version|uri|path|query|code/i.test(key),
    }));
}

function resourcesFromPayload(
  payload: AdaptiveRecord,
  existing: AdaptiveResource[],
): AdaptiveResource[] {
  const byUri = new Map(existing.map((item) => [item.uri, item]));
  const rootResource = resourceFromRecord(payload, byUri.size);
  if (rootResource) byUri.set(rootResource.uri, rootResource);
  for (const entry of walk(payload)) {
    if (byUri.size >= MAX_RESOURCES) break;
    const item = record(entry.value);
    if (!Object.keys(item).length) continue;
    const resource = resourceFromRecord(item, byUri.size);
    if (resource && !byUri.has(resource.uri)) byUri.set(resource.uri, resource);
  }
  return [...byUri.values()].slice(0, MAX_RESOURCES);
}

function actionFor(tool: ToolCall): string {
  const action = firstString(tool.input ?? {}, [
    "action",
    "operation",
    "query",
    "method",
  ]);
  return action || capabilityForTool(tool.name, tool.input).name;
}

function providerIdentity(toolName: string): {
  provider?: string;
  operation: string;
} {
  const parts = toolName.split("__").filter(Boolean);
  if (parts[0]?.toLowerCase() === "mcp" && parts.length >= 3) {
    return {
      provider: humanize(parts[parts.length - 2]),
      operation: humanize(parts[parts.length - 1]),
    };
  }
  const operation = toolName.includes(".")
    ? toolName.split(".").pop() || toolName
    : toolName;
  return { operation: humanize(operation) };
}

export function isAdaptiveDataToolCall(tool: ToolCall): boolean {
  const capability = capabilityForTool(tool.name, tool.input);
  return (
    capability.renderer === "generic" ||
    ["data", "knowledge", "integrations", "external"].includes(
      capability.family,
    )
  );
}

export function adaptAdaptiveDataTool(
  tool: ToolCall,
): AdaptiveToolView | undefined {
  if (!isAdaptiveDataToolCall(tool)) return undefined;
  const capability = capabilityForTool(tool.name, tool.input);
  const family = (
    ["data", "knowledge", "integrations", "external"].includes(
      capability.family,
    )
      ? capability.family
      : "external"
  ) as AdaptiveToolView["family"];
  const output = unwrapOutput(tool);
  const root = semanticRoot(output.payload);
  const candidates = arrayCandidates(root);
  const timelineCandidate = candidates.find(isTimelineCandidate);
  const timeline = timelineCandidate
    ? timelineFromCandidate(timelineCandidate)
    : [];
  const metrics = metricsFromPayload(root);
  const nonTimelineCandidates = candidates.filter(
    (candidate) =>
      candidate !== timelineCandidate &&
      candidate.key.toLowerCase() !== "metrics",
  );
  const preferredCandidate = nonTimelineCandidates[0];
  const table =
    preferredCandidate &&
    (capability.adapter === "table" ||
      family === "data" ||
      (family === "external" &&
        (preferredCandidate.value as unknown[]).every(
          (entry) => Object.keys(record(entry)).length > 0,
        )))
      ? tableFromCandidate(preferredCandidate, capability)
      : undefined;
  const collection =
    preferredCandidate && !table
      ? collectionFromCandidate(preferredCandidate)
      : [];
  const fields = fieldsFromRoot(root);
  const resources = resourcesFromPayload(root, output.resources);
  const document =
    !table && !collection.length
      ? documentFromPayload(root, output.text)
      : undefined;
  const status: ToolWidgetStatus =
    tool.status === "running"
      ? "running"
      : tool.status === "pending"
        ? "pending"
        : tool.status === "error" || tool.presentation?.isError
          ? "error"
          : Object.keys(root).length ||
              output.text.trim() ||
              resources.length ||
              tool.artifacts?.length
            ? "success"
            : "empty";
  const identity = providerIdentity(tool.name);
  const requestedAction = actionFor(tool);
  const nestedTitle = walk(root).find(
    (entry) =>
      TITLE_KEYS.includes(entry.key) &&
      typeof entry.value === "string" &&
      entry.value.trim(),
  );
  const contentTitle =
    firstString(root, TITLE_KEYS) ||
    clip(redactAdaptiveString(stringValue(nestedTitle?.value)), 180);
  const title =
    family === "external" && identity.provider
      ? `${identity.provider} · ${identity.operation}`
      : family === "knowledge" && contentTitle
        ? contentTitle
        : capability.displayName;
  const errorText =
    firstString(root, ["error", "message"]) ||
    (status === "error" ? output.text : "");
  const limited =
    Boolean(table?.truncated) ||
    (preferredCandidate
      ? (preferredCandidate.value as unknown[]).length > MAX_COLLECTION_ITEMS
      : false) ||
    output.text.length > MAX_DOCUMENT_CHARACTERS;
  const evidenceCount =
    (table?.shown ?? 0) +
    collection.length +
    timeline.length +
    metrics.length +
    resources.length +
    fields.length +
    (document ? 1 : 0);
  const summary =
    status === "running"
      ? `${humanize(requestedAction)} is running`
      : status === "pending"
        ? `${humanize(requestedAction)} is queued`
        : status === "error"
          ? clip(redactAdaptiveString(errorText || "Tool call failed"), 360)
          : table
            ? `${table.shown}${table.total > table.shown ? ` of ${table.total}` : ""} rows`
            : collection.length
              ? `${collection.length} result${collection.length === 1 ? "" : "s"}`
              : metrics.length
                ? `${metrics.length} metric${metrics.length === 1 ? "" : "s"}`
                : resources.length
                  ? `${resources.length} resource${resources.length === 1 ? "" : "s"}`
                  : document
                    ? clip(
                        document.split(/\r?\n/).find(Boolean) ||
                          "Document ready",
                        180,
                      )
                    : fields.length
                      ? `${fields.length} fields`
                      : "No structured result";
  const rawDownload = serializeRaw(output.raw);
  const rawTruncated = rawDownload.length > MAX_RAW_PREVIEW_CHARACTERS;
  const rawPreview = rawTruncated
    ? `${rawDownload.slice(0, MAX_RAW_PREVIEW_CHARACTERS)}\n… preview truncated`
    : rawDownload;

  return {
    capability,
    family,
    title,
    eyebrow:
      family === "external"
        ? identity.provider || "External"
        : humanize(family),
    summary,
    detail: humanize(requestedAction),
    status,
    fields,
    metrics,
    table,
    collection,
    timeline,
    document,
    resources,
    rawPreview,
    rawDownload,
    rawTruncated,
    limited,
    empty: evidenceCount === 0,
  };
}
