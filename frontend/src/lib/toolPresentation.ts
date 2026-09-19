export type ToolArtifactKind = "image" | "file";

export interface ToolArtifact {
  id: string;
  kind: ToolArtifactKind;
  name: string;
  mediaType: string;
  byteSize: number;
  sha256: string;
  contentUrl: string;
  downloadUrl?: string;
}

export interface ToolResourceLink {
  uri: string;
  name?: string;
  title?: string;
  description?: string;
  mediaType?: string;
  size?: number;
}

export interface ToolMediaReference {
  kind: "image" | "audio";
  mediaType?: string;
  resourceUri?: string;
  alt?: string;
}

export interface ToolResultEnvelopeV1 {
  version: 1;
  text?: string;
  structuredContent?: unknown;
  resources?: ToolResourceLink[];
  media?: ToolMediaReference[];
  truncated?: boolean;
}

export interface ToolTransportDiagnostics {
  reason?: string;
  inlineBudgetBytes?: number;
}

export interface ToolPresentationData {
  version: 1;
  text: string;
  summary: string;
  structuredContent?: unknown;
  resources: ToolResourceLink[];
  media: ToolMediaReference[];
  artifacts: ToolArtifact[];
  transport?: ToolTransportDiagnostics;
  isError: boolean;
  truncated: boolean;
  legacy: boolean;
}

const MAX_STRUCTURED_BYTES = 48 * 1024;
const MAX_COLLECTION_ITEMS = 64;
const MAX_STRING_BYTES = 8 * 1024;
const MAX_OVERSIZED_PREVIEW_CHARS = 16 * 1024;
const MAX_SUMMARY_CHARS = 240;

type UnknownRecord = Record<string, unknown>;

function isRecord(value: unknown): value is UnknownRecord {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function boundedString(value: unknown, max = MAX_STRING_BYTES): string | undefined {
  if (typeof value !== "string") return undefined;
  if (value.length <= max) return value;
  return value.slice(0, max);
}

function isEncodedPayload(key: string, value: string): boolean {
  const normalizedKey = key.toLowerCase();
  if (normalizedKey === "data" || normalizedKey === "blob") return true;
  return /^data:[^,;]+(?:;[^,]*)?;base64,/i.test(value);
}

function sanitizeStructuredValue(
  value: unknown,
  state: { truncated: boolean },
  depth = 0,
  key = "",
): unknown {
  if (depth > 8) {
    state.truncated = true;
    return "[depth limit]";
  }
  if (value == null || typeof value === "boolean" || typeof value === "number") {
    return value;
  }
  if (typeof value === "string") {
    if (isEncodedPayload(key, value)) {
      state.truncated = true;
      return "[binary payload omitted]";
    }
    if (value.length > MAX_STRING_BYTES) {
      state.truncated = true;
      return value.slice(0, MAX_STRING_BYTES);
    }
    return value;
  }
  if (Array.isArray(value)) {
    if (value.length > MAX_COLLECTION_ITEMS) state.truncated = true;
    return value
      .slice(0, MAX_COLLECTION_ITEMS)
      .map((item) => sanitizeStructuredValue(item, state, depth + 1));
  }
  if (isRecord(value)) {
    const entries = Object.entries(value);
    if (entries.length > MAX_COLLECTION_ITEMS) state.truncated = true;
    const sanitized: UnknownRecord = {};
    for (const [entryKey, entryValue] of entries.slice(0, MAX_COLLECTION_ITEMS)) {
      sanitized[entryKey] = sanitizeStructuredValue(
        entryValue,
        state,
        depth + 1,
        entryKey,
      );
    }
    return sanitized;
  }
  return String(value);
}

function boundedStructuredContent(
  value: unknown,
  state: { truncated: boolean },
): unknown | undefined {
  if (value === undefined) return undefined;
  const sanitized = sanitizeStructuredValue(value, state);
  try {
    if (JSON.stringify(sanitized).length > MAX_STRUCTURED_BYTES) {
      state.truncated = true;
      return undefined;
    }
  } catch {
    state.truncated = true;
    return undefined;
  }
  return sanitized;
}

function safeResourceUri(value: unknown): string | undefined {
  if (typeof value !== "string" || value.length === 0 || value.length > 2_048) {
    return undefined;
  }
  if (/[\u0000-\u001f]/.test(value)) return undefined;
  const lower = value.toLowerCase();
  if (
    lower.startsWith("javascript:") ||
    lower.startsWith("data:") ||
    lower.startsWith("vbscript:")
  ) {
    return undefined;
  }
  if (value.startsWith("/")) return value;
  if (/^(https?|file|flamel|artifact|mcp|skill):\/\//i.test(value)) return value;
  return undefined;
}

function normalizeResource(value: unknown): ToolResourceLink | undefined {
  if (!isRecord(value)) return undefined;
  const uri = safeResourceUri(value.uri);
  if (!uri) return undefined;
  const size =
    typeof value.size === "number" && Number.isFinite(value.size) && value.size >= 0
      ? value.size
      : undefined;
  const name = boundedString(value.name, 256);
  const title = boundedString(value.title, 256);
  const description = boundedString(value.description, 1_024);
  const mediaType = boundedString(value.mediaType ?? value.mimeType ?? value.mime_type, 128);
  return {
    uri,
    ...(name ? { name } : {}),
    ...(title ? { title } : {}),
    ...(description ? { description } : {}),
    ...(mediaType ? { mediaType } : {}),
    ...(size !== undefined ? { size } : {}),
  };
}

function normalizeMedia(value: unknown): ToolMediaReference | undefined {
  if (!isRecord(value)) return undefined;
  const kind = value.kind ?? value.type;
  if (kind !== "image" && kind !== "audio") return undefined;
  const resourceUri = safeResourceUri(value.resourceUri ?? value.resource_uri ?? value.uri);
  const mediaType = boundedString(
    value.mediaType ?? value.mimeType ?? value.mime_type,
    128,
  );
  const alt = boundedString(value.alt, 512);
  return {
    kind,
    ...(mediaType ? { mediaType } : {}),
    ...(resourceUri ? { resourceUri } : {}),
    ...(alt ? { alt } : {}),
  };
}

function normalizeArtifact(value: unknown): ToolArtifact | undefined {
  if (!isRecord(value) || (value.kind !== "image" && value.kind !== "file")) {
    return undefined;
  }
  const contentUrl = safeResourceUri(value.contentUrl);
  if (!contentUrl) return undefined;
  const downloadUrl = safeResourceUri(value.downloadUrl);
  return {
    id: boundedString(value.id, 256) || "",
    kind: value.kind,
    name: boundedString(value.name, 512) || "Artifact",
    mediaType: boundedString(value.mediaType, 128) || "application/octet-stream",
    byteSize:
      typeof value.byteSize === "number" && Number.isFinite(value.byteSize)
        ? Math.max(0, value.byteSize)
        : 0,
    sha256: boundedString(value.sha256, 128) || "",
    contentUrl,
    ...(downloadUrl ? { downloadUrl } : {}),
  };
}

interface OversizedResultPresentation {
  preview: string;
  summary: string;
  artifacts: ToolArtifact[];
  transport: ToolTransportDiagnostics;
  legacy: boolean;
}

function gatewayArtifact(value: unknown): ToolArtifact | undefined {
  if (!isRecord(value)) return undefined;
  const path = boundedString(value.path, 2_048);
  if (!path || /[\u0000-\u001f]/.test(path)) return undefined;

  const normalizedPath = path.replace(/\\/g, "/");
  const marker = "/flamel-mcp-artifacts/";
  const markerIndex = normalizedPath.lastIndexOf(marker);
  if (markerIndex < 0) return undefined;
  const name = normalizedPath.slice(markerIndex + marker.length);
  if (!/^[A-Za-z0-9_-]+\.txt$/.test(name)) return undefined;

  const bytes =
    typeof value.bytes === "number" && Number.isFinite(value.bytes) && value.bytes >= 0
      ? value.bytes
      : 0;
  const mediaType =
    boundedString(value.mediaType ?? value.mimeType ?? value.mime_type, 128) ||
    "text/plain";
  const contentUrl = `/api/mcp-artifacts/${encodeURIComponent(name)}`;

  return {
    id: `mcp-${name}`,
    kind: "file",
    name,
    mediaType,
    byteSize: bytes,
    sha256: "",
    contentUrl,
    downloadUrl: `${contentUrl}?download=1`,
  };
}

function oversizedResultPresentation(text: string): OversizedResultPresentation | undefined {
  if (!text.trimStart().startsWith("{")) return undefined;

  let envelope: UnknownRecord;
  try {
    const parsed = JSON.parse(text);
    if (!isRecord(parsed)) return undefined;
    envelope = parsed;
  } catch {
    return undefined;
  }

  const data = isRecord(envelope.data) ? envelope.data : undefined;
  const preview = boundedString(data?.preview, MAX_OVERSIZED_PREVIEW_CHARS);
  if (preview === undefined) return undefined;

  const current =
    envelope.schema === "flamel.oversized_result.v1" &&
    data?.truncated === true &&
    isRecord(data.original);
  const legacySummary = boundedString(envelope.summary, MAX_SUMMARY_CHARS);
  const legacy =
    !current &&
    Boolean(legacySummary?.match(/^Output exceeded the \d+ byte inline budget$/));
  if (!current && !legacy) return undefined;

  const artifacts = Array.isArray(envelope.artifacts)
    ? envelope.artifacts
        .slice(0, MAX_COLLECTION_ITEMS)
        .map(gatewayArtifact)
        .filter((value): value is ToolArtifact => Boolean(value))
    : [];
  const diagnostics = isRecord(envelope.diagnostics)
    ? envelope.diagnostics
    : undefined;
  const legacyBudget = legacySummary?.match(/^Output exceeded the (\d+) byte inline budget$/);
  const inlineBudgetBytes =
    typeof diagnostics?.inline_budget_bytes === "number" &&
    Number.isFinite(diagnostics.inline_budget_bytes)
      ? diagnostics.inline_budget_bytes
      : legacyBudget
        ? Number(legacyBudget[1])
        : undefined;
  const reason =
    boundedString(diagnostics?.reason, 128) ||
    (legacy ? "inline_budget_exceeded" : undefined);
  const summary =
    current &&
    legacySummary &&
    !legacySummary.startsWith("Output exceeded")
      ? legacySummary
      : "Clipped tool result";

  return {
    preview,
    summary,
    artifacts,
    transport: {
      ...(reason ? { reason } : {}),
      ...(inlineBudgetBytes !== undefined ? { inlineBudgetBytes } : {}),
    },
    legacy,
  };
}

function contentBlocks(content: unknown): unknown[] {
  if (Array.isArray(content)) return content;
  if (isRecord(content) && Array.isArray(content.content)) return content.content;
  return [];
}

function metadataText(block: UnknownRecord): string {
  const type = block.type;
  if (type === "image" || type === "audio") {
    return JSON.stringify({
      type,
      mediaType: block.mediaType ?? block.mimeType ?? block.mime_type,
      delivered_to_model: true,
    });
  }
  if (type === "resource_link") {
    const resource = normalizeResource(block);
    return resource ? JSON.stringify({ type, ...resource }) : "";
  }
  if (type === "resource" && isRecord(block.resource)) {
    const resource = normalizeResource(block.resource);
    const text = boundedString(block.resource.text, MAX_STRING_BYTES);
    return [resource ? JSON.stringify({ type, ...resource }) : "", text || ""]
      .filter(Boolean)
      .join("\n");
  }
  const state = { truncated: false };
  const sanitized = sanitizeStructuredValue(block, state);
  try {
    return JSON.stringify(sanitized);
  } catch {
    return "";
  }
}

export function textFromToolResultContent(content: unknown): string {
  if (typeof content === "string") return content;
  const blocks = contentBlocks(content);
  if (blocks.length > 0) {
    return blocks
      .map((block) => {
        if (isRecord(block) && block.type === "text") {
          return typeof block.text === "string" ? block.text : "";
        }
        return isRecord(block) ? metadataText(block) : "";
      })
      .join("");
  }
  if (content == null) return "";
  const state = { truncated: false };
  try {
    return JSON.stringify(sanitizeStructuredValue(content, state));
  } catch {
    return String(content);
  }
}

function resourcesFromContent(content: unknown): ToolResourceLink[] {
  const resources: ToolResourceLink[] = [];
  for (const block of contentBlocks(content).slice(0, MAX_COLLECTION_ITEMS)) {
    if (!isRecord(block)) continue;
    if (block.type === "resource_link") {
      const resource = normalizeResource(block);
      if (resource) resources.push(resource);
    } else if (block.type === "resource" && isRecord(block.resource)) {
      const resource = normalizeResource(block.resource);
      if (resource) resources.push(resource);
    }
  }
  return resources;
}

function mediaFromContent(content: unknown): ToolMediaReference[] {
  const media: ToolMediaReference[] = [];
  for (const block of contentBlocks(content).slice(0, MAX_COLLECTION_ITEMS)) {
    const normalized = normalizeMedia(block);
    if (normalized) media.push(normalized);
  }
  return media;
}

function dedupeBy<T>(values: T[], key: (value: T) => string): T[] {
  const seen = new Set<string>();
  return values.filter((value) => {
    const identity = key(value);
    if (seen.has(identity)) return false;
    seen.add(identity);
    return true;
  });
}

function summaryForPresentation(
  text: string,
  structuredContent: unknown,
  resources: ToolResourceLink[],
  media: ToolMediaReference[],
): string {
  const firstLine = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find(Boolean);
  if (firstLine) {
    return firstLine.length > MAX_SUMMARY_CHARS
      ? firstLine.slice(0, MAX_SUMMARY_CHARS - 1) + "…"
      : firstLine;
  }
  if (resources.length > 0) {
    return String(resources.length) + " resource" + (resources.length === 1 ? "" : "s");
  }
  if (media.length > 0) {
    return String(media.length) + " media item" + (media.length === 1 ? "" : "s");
  }
  if (structuredContent !== undefined) return "Structured result";
  return "No output";
}

export function normalizeToolPresentation(options: {
  content: unknown;
  envelope?: unknown;
  fallbackText?: string;
  artifacts?: unknown;
  isError?: boolean;
}): ToolPresentationData {
  const { content, envelope, fallbackText, artifacts, isError = false } = options;
  const rawEnvelope =
    isRecord(envelope) && envelope.version === 1 ? envelope : undefined;
  const state = { truncated: rawEnvelope?.truncated === true };

  const contentRecord = isRecord(content) ? content : undefined;
  const structuredCandidate =
    rawEnvelope?.structuredContent ??
    contentRecord?.structuredContent ??
    contentRecord?.structured_content;
  const structuredContent = boundedStructuredContent(structuredCandidate, state);

  if (Array.isArray(rawEnvelope?.resources) && rawEnvelope.resources.length > MAX_COLLECTION_ITEMS) {
    state.truncated = true;
  }
  const envelopeResources = Array.isArray(rawEnvelope?.resources)
    ? rawEnvelope.resources
        .slice(0, MAX_COLLECTION_ITEMS)
        .map(normalizeResource)
        .filter((value): value is ToolResourceLink => Boolean(value))
    : [];
  const resources = dedupeBy(
    [...envelopeResources, ...resourcesFromContent(content)],
    (value) => value.uri,
  ).slice(0, MAX_COLLECTION_ITEMS);

  if (Array.isArray(rawEnvelope?.media) && rawEnvelope.media.length > MAX_COLLECTION_ITEMS) {
    state.truncated = true;
  }
  const envelopeMedia = Array.isArray(rawEnvelope?.media)
    ? rawEnvelope.media
        .slice(0, MAX_COLLECTION_ITEMS)
        .map(normalizeMedia)
        .filter((value): value is ToolMediaReference => Boolean(value))
    : [];
  const inferredMedia = mediaFromContent(content).filter(
    (candidate) =>
      candidate.resourceUri ||
      !envelopeMedia.some(
        (explicit) =>
          explicit.kind === candidate.kind && explicit.mediaType === candidate.mediaType,
      ),
  );
  const media = dedupeBy(
    [...envelopeMedia, ...inferredMedia],
    (value) =>
      value.kind + ":" + (value.resourceUri || "") + ":" + (value.mediaType || ""),
  ).slice(0, MAX_COLLECTION_ITEMS);

  if (Array.isArray(artifacts) && artifacts.length > MAX_COLLECTION_ITEMS) {
    state.truncated = true;
  }
  const normalizedArtifacts = Array.isArray(artifacts)
    ? artifacts
        .slice(0, MAX_COLLECTION_ITEMS)
        .map(normalizeArtifact)
        .filter((value): value is ToolArtifact => Boolean(value))
    : [];

  const sourceText =
    fallbackText ??
    boundedString(rawEnvelope?.text, Number.MAX_SAFE_INTEGER) ??
    textFromToolResultContent(content);
  const oversized = oversizedResultPresentation(sourceText);
  if (oversized) state.truncated = true;
  const text = oversized?.preview ?? sourceText;
  const presentationArtifacts = dedupeBy(
    [...normalizedArtifacts, ...(oversized?.artifacts ?? [])],
    (value) => value.id || value.contentUrl,
  ).slice(0, MAX_COLLECTION_ITEMS);

  return {
    version: 1,
    text,
    summary:
      oversized?.summary ??
      summaryForPresentation(text, structuredContent, resources, media),
    ...(structuredContent !== undefined ? { structuredContent } : {}),
    resources,
    media,
    artifacts: presentationArtifacts,
    ...(oversized ? { transport: oversized.transport } : {}),
    isError,
    truncated: state.truncated,
    legacy: oversized ? oversized.legacy : !rawEnvelope,
  };
}
