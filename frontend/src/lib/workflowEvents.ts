import type { PendingQuestion } from "@flamel/shared";

export type WorkflowEventSource =
  | "backend"
  | "legacy_marker"
  | "chat_stream"
  | "tool_result"
  | "command_event"
  | "project_file";

export type WorkflowEventKind =
  | "plan.created"
  | "plan.updated"
  | "plan.renamed"
  | "plan.loaded"
  | "plan.finished"
  | "plan.archived"
  | "plan.execution_updated"
  | "plan.item_updated"
  | "plan.items_added"
  | "plan.item_deleted"
  | "agent.run_created"
  | "agent.batch_created"
  | "system.thinking"
  | "system.compacting"
  | "system.compact_complete"
  | "system.compact_failed"
  | "recovery.notice"
  | "question.requested"
  | "question.resolved"
  | "question.cancelled"
  | "command.event"
  | "tool.status"
  | "file.updated"
  | "file.created"
  | "file.deleted"
  | "file.moved"
  | "file.tree_changed"
  | "file.refresh_requested";

export type WorkflowEvent<TPayload = any> = {
  type: "workflow_event";
  kind: WorkflowEventKind;
  source: WorkflowEventSource | string;
  payload: TPayload;
  legacyMarker?: string;
  raw?: unknown;
};

export type ParsedAgentMarker = {
  output: string;
  runId?: string;
  runIds?: string[];
  legacyMarker?: "AGENT_RUN" | "AGENT_BATCH";
};

const PLAN_MARKER_LABELS: Record<string, string> = {
  PLAN_CREATED: "Plan artifact created",
  PLAN_UPDATED: "Plan artifact updated",
  PLAN_GET: "Plan artifact loaded",
  PLAN_RENAMED: "Plan artifact renamed",
  PLAN_FINISHED: "Plan marked finished",
  PLAN_ARCHIVED: "Plan archived",
  PLAN_EXECUTION_UPDATED: "Plan execution updated",
  ITEM_UPDATED: "Plan item updated",
  ITEMS_ADDED: "Plan items added",
  ITEM_DELETED: "Plan item deleted",
};

const PLAN_MARKER_KINDS: Record<string, WorkflowEventKind> = {
  PLAN_CREATED: "plan.created",
  PLAN_UPDATED: "plan.updated",
  PLAN_GET: "plan.loaded",
  PLAN_RENAMED: "plan.renamed",
  PLAN_FINISHED: "plan.finished",
  PLAN_ARCHIVED: "plan.archived",
  PLAN_EXECUTION_UPDATED: "plan.execution_updated",
  ITEM_UPDATED: "plan.item_updated",
  ITEMS_ADDED: "plan.items_added",
  ITEM_DELETED: "plan.item_deleted",
};

export const PLAN_TOOL_OUTPUT_RE =
  /^(PLAN_CREATED|PLAN_UPDATED|PLAN_GET|PLAN_RENAMED|PLAN_FINISHED|PLAN_ARCHIVED|PLAN_EXECUTION_UPDATED|ITEM_UPDATED|ITEMS_ADDED|ITEM_DELETED):/;

export function isPlanToolMarkerOutput(output: string | undefined): boolean {
  return !!output && PLAN_TOOL_OUTPUT_RE.test(output.trimStart());
}

export function planToolOutputLabel(output: string | undefined): string {
  const marker = output?.trimStart().match(PLAN_TOOL_OUTPUT_RE)?.[1];
  return marker ? (PLAN_MARKER_LABELS[marker] ?? "Plan panel updated") : "Plan panel updated";
}

export function planMetaFromMarker(json: any) {
  const meta: any = {};
  for (const key of [
    "artifact",
    "versions",
    "finished",
    "executionStatus",
    "approvalStatus",
    "activeItemId",
    "approvalRequestedAt",
    "approvedAt",
    "executionStartedAt",
    "executionUpdatedAt",
  ]) {
    if (Object.prototype.hasOwnProperty.call(json, key)) meta[key] = json[key];
  }
  return meta;
}

export function planToolAction(name: string, input: any): string | null {
  if (name === "plan" || name.endsWith("__plan")) {
    return (input?.action as string) || null;
  }
  const bare = name.includes("__") ? name.split("__").pop()! : name;
  const match = bare.match(/^flamel_plan_(.+)$/);
  return match ? match[1] : null;
}

export function normalizePlanInputItems(value: unknown): any[] {
  let items = value;
  if (typeof items === "string") {
    try {
      items = JSON.parse(items);
    } catch {
      return [];
    }
  }
  if (!Array.isArray(items)) return [];

  return items.map((raw, index) => {
    let item = raw;
    if (typeof item === "string") {
      try {
        item = JSON.parse(item);
      } catch {
        item = { text: item };
      }
    }
    if (!item || typeof item !== "object") item = { text: String(item ?? "") };
    const record = item as Record<string, any>;
    const text = [record.text, record.content, record.step, record.title, record.task]
      .find(candidate => typeof candidate === "string" && candidate.trim());
    const section = [record.section, record.phase, record.group]
      .find(candidate => typeof candidate === "string" && candidate.trim());
    const rawStatus = String(record.status || "todo").toLowerCase();
    const status = rawStatus === "done" || rawStatus === "completed"
      ? "done"
      : rawStatus === "in_progress" || rawStatus === "inprogress" || rawStatus === "active"
        ? "in_progress"
        : rawStatus === "blocked"
          ? "blocked"
          : "todo";
    return {
      ...record,
      id: typeof record.id === "string" && record.id ? record.id : `pending_${index}`,
      text: text || `Plan item ${index + 1}`,
      status,
      position: typeof record.position === "number" ? record.position : index,
      notes: typeof record.notes === "string"
        ? record.notes
        : typeof record.details === "string"
          ? record.details
          : undefined,
      section,
    };
  });
}

export function parsePartialPlanInput(raw: string): {
  action?: string;
  title?: string;
  description?: string;
  items: any[];
} | null {
  const actionMatch = raw.match(/"action"\s*:\s*"([^"]+)"/);
  const action = actionMatch?.[1];
  if (!action) return null;

  const titleMatch = raw.match(/"title"\s*:\s*"((?:[^"\\]|\\.)*)"/);
  const title = titleMatch?.[1];
  const descMatch = raw.match(/"description"\s*:\s*"((?:[^"\\]|\\.)*)"/);
  const description = descMatch?.[1];

  const itemsIdx = raw.indexOf('"items"');
  if (itemsIdx < 0) return { action, title, description, items: [] };
  const bracketStart = raw.indexOf("[", itemsIdx);
  if (bracketStart < 0) return { action, title, description, items: [] };

  const items: any[] = [];
  let depth = 0;
  let objStart = -1;
  let inString = false;
  let escaped = false;

  for (let i = bracketStart + 1; i < raw.length; i++) {
    const ch = raw[i];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (ch === "\\" && inString) {
      escaped = true;
      continue;
    }
    if (ch === '"') {
      inString = !inString;
      continue;
    }
    if (inString) continue;
    if (ch === "{") {
      if (depth === 0) objStart = i;
      depth++;
    } else if (ch === "}") {
      depth--;
      if (depth === 0 && objStart >= 0) {
        try {
          items.push(JSON.parse(raw.slice(objStart, i + 1)));
        } catch {}
        objStart = -1;
      }
    }
  }

  return { action, title, description, items: normalizePlanInputItems(items) };
}

export function parsePlanMarkerOutput(output: string | undefined): WorkflowEvent | null {
  if (!output) return null;
  const trimmed = output.trimStart();
  const match = trimmed.match(PLAN_TOOL_OUTPUT_RE);
  const marker = match?.[1];
  if (!marker) return null;
  const firstLine = trimmed.slice(`${marker}:`.length).split("\n")[0];
  try {
    return {
      type: "workflow_event",
      kind: PLAN_MARKER_KINDS[marker],
      source: "legacy_marker",
      legacyMarker: marker,
      payload: JSON.parse(firstLine),
      raw: output,
    };
  } catch {
    return null;
  }
}

export function stripAgentMarkerHeaders(
  output: string,
  toolName?: string,
): ParsedAgentMarker {
  const isAgent = toolName === "Agent" || toolName === "mcp__flamel__Agent";
  const isBatch = toolName === "AgentBatch" || toolName === "mcp__flamel__AgentBatch";

  if ((!toolName || isAgent) && output.startsWith("AGENT_RUN:")) {
    const newlineIdx = output.indexOf("\n");
    const runId = output
      .slice("AGENT_RUN:".length, newlineIdx > 0 ? newlineIdx : output.length)
      .trim();
    return {
      output: newlineIdx > 0 ? output.slice(newlineIdx + 1) : "",
      runId,
      legacyMarker: "AGENT_RUN",
    };
  }

  if ((!toolName || isBatch) && output.startsWith("AGENT_BATCH:")) {
    const lines = output.split("\n");
    let skipLines = 0;
    let runId: string | undefined;
    let runIds: string[] | undefined;
    for (const line of lines) {
      if (line.startsWith("AGENT_BATCH:")) {
        runId = line.slice("AGENT_BATCH:".length).trim();
        skipLines++;
      } else if (line.startsWith("AGENT_RUNS:")) {
        runIds = line
          .slice("AGENT_RUNS:".length)
          .split(",")
          .map((id) => id.trim())
          .filter(Boolean);
        skipLines++;
      } else {
        break;
      }
    }
    return {
      output: lines.slice(skipLines).join("\n"),
      runId,
      runIds,
      legacyMarker: "AGENT_BATCH",
    };
  }

  return { output };
}

export function normalizeWorkflowEvent(raw: any): WorkflowEvent | null {
  if (!raw || typeof raw !== "object") return null;
  if (raw.type === "workflow_event") {
    const event = raw.event && typeof raw.event === "object" ? raw.event : raw;
    if (typeof event.kind === "string") {
      return {
        type: "workflow_event",
        kind: event.kind as WorkflowEventKind,
        source: event.source ?? "backend",
        payload: event.payload ?? {},
        legacyMarker: event.legacyMarker ?? event.legacy_marker,
        raw,
      };
    }
  }
  if (raw.type === "system_status") {
    if (raw.status === "thinking") {
      return { type: "workflow_event", kind: "system.thinking", source: "chat_stream", payload: raw, raw };
    }
    if (raw.status === "compacting") {
      return { type: "workflow_event", kind: "system.compacting", source: "chat_stream", payload: raw, raw };
    }
    if (raw.status === "compact_complete") {
      return { type: "workflow_event", kind: "system.compact_complete", source: "chat_stream", payload: raw, raw };
    }
    if (raw.status === "compact_failed") {
      return { type: "workflow_event", kind: "system.compact_failed", source: "chat_stream", payload: raw, raw };
    }
  }
  if (raw.type === "recovery") {
    return { type: "workflow_event", kind: "recovery.notice", source: "chat_stream", payload: raw, raw };
  }
  if (raw.type === "ask_question") {
    return { type: "workflow_event", kind: "question.requested", source: "chat_stream", payload: raw, raw };
  }
  if (raw.type === "question_resolved") {
    return { type: "workflow_event", kind: "question.resolved", source: "chat_stream", payload: raw, raw };
  }
  if (raw.type === "question_cancelled") {
    return { type: "workflow_event", kind: "question.cancelled", source: "chat_stream", payload: raw, raw };
  }
  if (raw.type === "command_event") {
    return { type: "workflow_event", kind: "command.event", source: "command_event", payload: raw.event, raw };
  }
  if (raw.type === "status") {
    return { type: "workflow_event", kind: "tool.status", source: "chat_stream", payload: raw, raw };
  }
  return null;
}

export function pendingQuestionFromWorkflowEvent(
  event: WorkflowEvent | null,
): PendingQuestion | null {
  if (event?.kind !== "question.requested") return null;
  const payload = event.payload;
  if (
    typeof payload?.requestId !== "string" ||
    !payload.requestId ||
    typeof payload.question !== "string" ||
    !payload.question.trim()
  ) {
    return null;
  }
  const choices = Array.isArray(payload.choices)
    ? (payload.choices as unknown[])
        .map((choice) => {
          if (typeof choice === "string") return choice.trim();
          if (
            choice &&
            typeof choice === "object" &&
            typeof (choice as { label?: unknown }).label === "string"
          ) {
            const label = (choice as { label: string }).label.trim();
            if (!label) return null;
            const description =
              typeof (choice as { description?: unknown }).description === "string"
                ? (choice as { description: string }).description.trim()
                : undefined;
            return { label, description: description || undefined };
          }
          return null;
        })
        .filter(
          (
            choice,
          ): choice is string | { label: string; description: string | undefined } =>
            choice !== null,
        )
    : undefined;
  return {
    requestId: payload.requestId,
    sessionId:
      typeof payload.sessionId === "string" ? payload.sessionId : undefined,
    runId: typeof payload.runId === "string" ? payload.runId : undefined,
    status: payload.status === "pending" ? "pending" : undefined,
    question: payload.question,
    choices,
    allowFreeText:
      typeof payload.allowFreeText === "boolean"
        ? payload.allowFreeText
        : undefined,
    multiSelect:
      typeof payload.multiSelect === "boolean" ? payload.multiSelect : false,
    previewMarkdown:
      typeof payload.previewMarkdown === "string"
        ? payload.previewMarkdown
        : undefined,
  };
}

export function completedQuestionRequestId(
  event: WorkflowEvent | null,
): string | null {
  if (
    event?.kind !== "question.resolved" &&
    event?.kind !== "question.cancelled"
  ) {
    return null;
  }
  return typeof event.payload?.requestId === "string" && event.payload.requestId
    ? event.payload.requestId
    : null;
}
