import type { FlamelAssistantOperationResponse } from "./api";

export type UnknownRecord = Record<string, unknown>;

export interface AssistantListItem {
  id: string;
  title: string;
  subtitle: string;
  status: string;
  detail?: string;
  meta?: string[];
  raw: UnknownRecord;
}

export function assistantRecord(value: unknown): UnknownRecord {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as UnknownRecord)
    : {};
}

export function assistantStringValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return "";
}

export function assistantFirstString(
  value: UnknownRecord,
  keys: string[],
  fallback = "",
): string {
  for (const key of keys) {
    const candidate = assistantStringValue(value[key]).trim();
    if (candidate) return candidate;
  }
  return fallback;
}

export function assistantNestedRecord(
  value: UnknownRecord,
  keys: string[],
): UnknownRecord {
  for (const key of keys) {
    const candidate = assistantRecord(value[key]);
    if (Object.keys(candidate).length) return candidate;
  }
  return {};
}

export function assistantItemsFromResponse(
  response: FlamelAssistantOperationResponse,
  field: string,
): UnknownRecord[] {
  const result = assistantRecord(response.result);
  const value = result[field];
  return Array.isArray(value) ? value.map(assistantRecord) : [];
}

function detailLine(value: unknown): string {
  if (typeof value === "string") return value.trim();
  const entry = assistantRecord(value);
  const timestamp = assistantFirstString(entry, [
    "occurred_at",
    "started_at",
    "completed_at",
    "scheduled_for",
    "updated_at",
    "created_at",
    "timestamp",
  ]);
  const status = assistantFirstString(entry, [
    "status",
    "state",
    "type",
    "kind",
  ]);
  const summary = assistantFirstString(entry, [
    "summary",
    "message",
    "error",
    "name",
    "title",
  ]);
  const id = assistantFirstString(entry, ["run_id", "event_id", "id"]);
  return [timestamp, status, summary, id].filter(Boolean).join(" · ");
}

export function assistantDetailLines(
  response: FlamelAssistantOperationResponse,
  preferredFields: string[],
): string[] {
  const result = assistantRecord(response.result);
  const containers = [
    result,
    assistantRecord(result.profile),
    assistantRecord(result.job),
    assistantRecord(result.run),
  ];
  for (const field of preferredFields) {
    for (const container of containers) {
      const values = container[field];
      if (Array.isArray(values)) {
        return values.map(detailLine).filter(Boolean).slice(0, 50);
      }
    }
  }

  for (const container of containers.slice(1).concat(result)) {
    const line = detailLine(container);
    if (line) return [line];
  }
  return [];
}

export function assistantStateTone(
  status: string,
): "ready" | "warning" | "error" | "neutral" {
  const normalized = status.toLowerCase();
  if (
    [
      "ready",
      "connected",
      "running",
      "active",
      "installed",
      "completed",
      "done",
      "approved",
    ].some((value) => normalized.includes(value))
  ) {
    return "ready";
  }
  if (
    ["failed", "error", "cancelled", "revoked", "blocked", "unavailable"].some(
      (value) => normalized.includes(value),
    )
  ) {
    return "error";
  }
  if (
    [
      "pending",
      "approval",
      "paused",
      "scheduled",
      "configured",
      "needs_auth",
      "waiting",
    ].some((value) => normalized.includes(value))
  ) {
    return "warning";
  }
  return "neutral";
}

export function itemFromAssistantRecord(
  value: UnknownRecord,
  kind: "assistant" | "memory" | "automation" | "job",
): AssistantListItem {
  const run = assistantNestedRecord(value, ["latest_run", "latestRun", "run"]);
  const id = assistantFirstString(
    value,
    [
      "id",
      "assistant_id",
      "assistantId",
      "profile_id",
      "profileId",
      "job_id",
      "jobId",
    ],
    "unknown",
  );
  const status = assistantFirstString(
    run,
    ["status", "state"],
    assistantFirstString(
      value,
      ["status", "state", "connection_status", "connectionStatus"],
      "unknown",
    ),
  );
  if (kind === "memory") {
    const scope = assistantNestedRecord(value, ["scope"]);
    const surface = assistantNestedRecord(value, ["surface"]);
    const provenance = assistantNestedRecord(value, ["provenance"]);
    return {
      id,
      title: assistantFirstString(value, ["content"], "Remembered context"),
      subtitle: [
        assistantFirstString(value, ["kind"], "fact"),
        assistantFirstString(scope, ["kind"], "personal"),
      ].join(" · "),
      status:
        value.superseded_by || value.supersededBy
          ? "superseded"
          : value.pinned
            ? "pinned"
            : "active",
      detail: [
        assistantFirstString(value, ["retention"], "durable"),
        assistantFirstString(provenance, ["source"], "unknown source"),
      ].join(" · "),
      meta: [
        `revision ${assistantFirstString(value, ["revision"], "1")}`,
        assistantFirstString(value, ["assistant_id", "assistantId"]),
        assistantFirstString(surface, ["kind"], "private"),
        assistantFirstString(value, ["access_count", "accessCount"], "0") + " recalls",
        assistantFirstString(value, ["updated_at", "updatedAt"]),
      ].filter(Boolean),
      raw: value,
    };
  }
  if (kind === "automation") {
    return {
      id,
      title: assistantFirstString(
        value,
        ["name", "title"],
        "Unnamed automation",
      ),
      subtitle: assistantFirstString(
        value,
        ["description", "execution_kind", "executionKind"],
        "Flamel automation",
      ),
      status,
      detail: assistantFirstString(
        value,
        ["next_run_at", "nextRunAt", "mode"],
        "No next run reported",
      ),
      meta: [
        assistantFirstString(value, ["timezone"]),
        assistantFirstString(value, ["last_run_at", "lastRunAt"]),
      ].filter(Boolean),
      raw: value,
    };
  }
  if (kind === "job") {
    return {
      id,
      title: assistantFirstString(value, ["title", "name"], "Untitled Job"),
      subtitle: assistantFirstString(
        value,
        ["objective", "repo", "repository"],
        "Durable Flamel work",
      ),
      status,
      detail: assistantFirstString(
        run,
        ["summary", "detail"],
        assistantFirstString(value, ["updated_at", "updatedAt"], ""),
      ),
      meta: [
        assistantFirstString(value, ["owner"]),
        assistantFirstString(run, ["id", "run_id", "runId"]),
      ].filter(Boolean),
      raw: value,
    };
  }
  const config = assistantNestedRecord(value, ["config"]);
  return {
    id,
    title: assistantFirstString(value, ["name", "title"], "Flamel Assistant"),
    subtitle: assistantFirstString(
      config,
      ["model"],
      assistantFirstString(
        value,
        ["model", "provider"],
        "Provider not reported",
      ),
    ),
    status,
    detail: assistantFirstString(
      value,
      ["last_error", "lastError", "workspace"],
      "",
    ),
    meta: [
      assistantFirstString(value, ["chattle_status", "chattleStatus"]),
      assistantFirstString(value, ["updated_at", "updatedAt"]),
    ].filter(Boolean),
    raw: value,
  };
}
