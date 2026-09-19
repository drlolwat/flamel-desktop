import React, { useMemo } from "react";
import type { ToolCall } from "../../lib/messageProcessor";
import {
  assistantFirstString,
  assistantNestedRecord,
  assistantRecord,
  assistantStringValue,
  type UnknownRecord,
} from "../../lib/assistantPresentation";
import {
  normalizePlanInputItems,
  parsePlanMarkerOutput,
  planToolOutputLabel,
  stripAgentMarkerHeaders,
} from "../../lib/workflowEvents";
import { useAppStore } from "../../stores/appStore";
import { Icon } from "../ui/Icons";
import {
  ToolArtifactActions,
  ToolWidgetFields,
  ToolWidgetInspector,
  ToolWidgetList,
  ToolWidgetSection,
  ToolWidgetState,
  ToolWidgetTimeline,
  type ToolTimelineItem,
} from "./ToolWidgetPrimitives";
import { ToolWidgetShell, type ToolWidgetSprite } from "./ToolWidgetShell";
import {
  workflowAction,
  workflowWidgetKind,
  type WorkflowWidgetKind,
} from "./toolWidgetRouting";
export { isWorkflowToolCall, workflowWidgetKind } from "./toolWidgetRouting";

type WorkflowStatus = "pending" | "running" | "success" | "error" | "empty";

interface WorkflowListItem {
  id: string;
  primary: string;
  secondary?: string;
  trailing?: string;
  status?: WorkflowStatus;
}

export interface AgentRunNode {
  id: string;
  parentId?: string;
  title: string;
  persona?: string;
  status: WorkflowStatus;
  detail?: string;
  children: AgentRunNode[];
}

export interface WorkflowToolView {
  kind: WorkflowWidgetKind;
  action: string;
  title: string;
  summary: string;
  detail?: string;
  status: WorkflowStatus;
  statusLabel?: string;
  approvalRequired: boolean;
  mutated: boolean;
  replayed: boolean;
  contractVersion?: number;
  contractKnown: boolean;
  fields: Array<{ label: string; value: React.ReactNode; mono?: boolean }>;
  items: WorkflowListItem[];
  timeline: ToolTimelineItem[];
  agentTree: AgentRunNode[];
  progress?: { done: number; total: number };
  result: UnknownRecord;
  rawOutput?: unknown;
}

function partialInput(tool: ToolCall): UnknownRecord {
  const input = tool.input ?? {};
  const raw = tool.partialInput?.trim();
  if (!raw) return input;
  try {
    return { ...assistantRecord(JSON.parse(raw)), ...input };
  } catch {
    const recovered: UnknownRecord = {};
    for (const key of [
      "action",
      "title",
      "description",
      "objective",
      "task_name",
      "subagent_type",
      "plan_id",
      "job_id",
      "work_id",
      "name",
    ]) {
      const match = raw.match(
        new RegExp(`"${key}"\\s*:\\s*"((?:[^"\\\\]|\\\\.)*)`),
      );
      if (match?.[1])
        recovered[key] = match[1].replace(/\\(["\\/bfnrt])/g, "$1");
    }
    return { ...recovered, ...input };
  }
}

function parseJson(value?: string): unknown {
  if (!value?.trim()) return undefined;
  try {
    return JSON.parse(value);
  } catch {
    return undefined;
  }
}

function unwrapOutput(tool: ToolCall): {
  payload: UnknownRecord;
  raw?: unknown;
  text: string;
} {
  const marker = parsePlanMarkerOutput(tool.output);
  if (marker) {
    return {
      payload: assistantRecord(marker.payload),
      raw: marker.raw,
      text: planToolOutputLabel(tool.output),
    };
  }

  const parsed = parseJson(stripAgentMarkerHeaders(tool.output ?? "", tool.name).output);
  const parsedRecord = assistantRecord(parsed);
  const structured = assistantRecord(
    parsedRecord.structuredContent ?? parsedRecord.structured_content,
  );
  const content = Array.isArray(parsedRecord.content)
    ? parsedRecord.content
    : [];
  const contentText = content
    .map((block) => assistantStringValue(assistantRecord(block).text))
    .filter(Boolean)
    .join("\n");
  let nested = parsed;
  if (contentText.trim().startsWith("{")) {
    nested = parseJson(contentText) ?? parsed;
  }
  const nestedRecord = assistantRecord(nested);
  const payload =
    Object.keys(assistantRecord(nestedRecord.structuredContent)).length > 0
      ? assistantRecord(nestedRecord.structuredContent)
      : Object.keys(structured).length > 0
        ? structured
        : Object.keys(nestedRecord).length > 0
          ? nestedRecord
          : assistantRecord(tool.presentation?.structuredContent);
  return {
    payload,
    raw: parsed,
    text:
      assistantStringValue(payload.text) ||
      assistantStringValue(payload.message) ||
      assistantStringValue(payload.summary) ||
      contentText ||
      tool.presentation?.text ||
      tool.output ||
      "",
  };
}

function toolStatus(
  tool: ToolCall,
  ok: boolean | undefined,
  approvalRequired: boolean,
  hasEvidence: boolean,
): WorkflowStatus {
  if (tool.status === "running") return "running";
  if (tool.status === "pending" || approvalRequired) return "pending";
  if (tool.status === "error" || ok === false || tool.presentation?.isError)
    return "error";
  return hasEvidence ? "success" : "empty";
}

function findRecord(payload: UnknownRecord, keys: string[]): UnknownRecord {
  for (const key of keys) {
    const candidate = assistantRecord(payload[key]);
    if (Object.keys(candidate).length) return candidate;
  }
  return payload;
}

function firstArray(payload: UnknownRecord, keys: string[]): unknown[] {
  for (const key of keys) {
    const value = payload[key];
    if (Array.isArray(value)) return value;
  }
  for (const value of Object.values(payload)) {
    const nested = assistantRecord(value);
    if (!Object.keys(nested).length) continue;
    const candidate = firstArray(nested, keys);
    if (candidate.length) return candidate;
  }
  return [];
}

function itemStatus(value: unknown): WorkflowStatus {
  const status = assistantFirstString(assistantRecord(value), [
    "status",
    "state",
    "executionStatus",
  ]).toLowerCase();
  if (/error|fail|block|cancel|revoke|timeout|skipped/.test(status)) return "error";
  if (/running|active|progress/.test(status)) return "running";
  if (/pending|todo|queue|wait|approval/.test(status)) return "pending";
  if (/done|complete|success|ready|installed|connected|approved/.test(status)) {
    return "success";
  }
  return "empty";
}

function listItems(values: unknown[], fallback: string): WorkflowListItem[] {
  return values.slice(0, 60).map((value, index) => {
    const record = assistantRecord(value);
    return {
      id:
        assistantFirstString(record, [
          "id",
          "item_id",
          "run_id",
          "task_id",
          "artifact_id",
          "name",
        ]) || `${fallback}-${index}`,
      primary:
        assistantFirstString(record, [
          "text",
          "title",
          "name",
          "summary",
          "objective",
          "content",
          "description",
        ]) || `${fallback} ${index + 1}`,
      secondary:
        assistantFirstString(record, [
          "detail",
          "message",
          "owner",
          "path",
          "scope",
          "version",
        ]) || undefined,
      trailing:
        assistantFirstString(record, [
          "status",
          "state",
          "position",
          "duration",
          "reasoning_floor",
        ]) || undefined,
      status: itemStatus(record),
    };
  });
}

function timelineItems(values: unknown[]): ToolTimelineItem[] {
  return values.slice(0, 50).map((value, index) => {
    const record = assistantRecord(value);
    return {
      id:
        assistantFirstString(record, ["event_id", "run_id", "id"]) ||
        `workflow-event-${index}`,
      title:
        assistantFirstString(record, [
          "title",
          "summary",
          "message",
          "type",
          "kind",
          "status",
        ]) || `Event ${index + 1}`,
      detail:
        assistantFirstString(record, [
          "detail",
          "error",
          "description",
          "objective",
        ]) || undefined,
      meta:
        assistantFirstString(record, [
          "occurred_at",
          "started_at",
          "completed_at",
          "timestamp",
          "duration",
        ]) || undefined,
      status: itemStatus(record),
    };
  });
}

function eventRunId(event: UnknownRecord): string {
  return assistantFirstString(event, ["run_id", "runId", "id"]);
}

export function buildAgentRunTree(
  events: Array<Record<string, unknown>>,
  tool: ToolCall,
): AgentRunNode[] {
  const nodes = new Map<string, AgentRunNode>();
  const payload = unwrapOutput(tool).payload;
  const statusQuery = /(?:^|__)AgentBatchStatus$/.test(tool.name);
  const ensure = (id: string, event?: UnknownRecord) => {
    let node = nodes.get(id);
    if (!node) {
      node = {
        id,
        parentId:
          assistantFirstString(event ?? {}, ["parent_run_id", "parentRunId"]) ||
          undefined,
        title:
          assistantFirstString(event ?? {}, [
            "task_name",
            "description",
            "title",
            "name",
          ]) || id,
        persona:
          assistantFirstString(event ?? {}, [
            "subagent_type",
            "persona",
            "agent",
          ]) || undefined,
        status: "running",
        detail: undefined,
        children: [],
      };
      nodes.set(id, node);
    }
    return node;
  };

  // A status lookup reports recorded children; it is not itself a child run.
  // This also restores the tree when realtime events were missed on reconnect.
  if (Array.isArray(payload.agents)) {
    for (const raw of payload.agents) {
      const agent = assistantRecord(raw);
      const id = eventRunId(agent);
      if (!id) continue;
      const node = ensure(id, agent);
      node.status = itemStatus(agent);
      node.detail = assistantFirstString(agent, ["error", "output"]);
    }
    return [...nodes.values()];
  }

  for (const rawEvent of events.slice(0, 500)) {
    const event = assistantRecord(rawEvent);
    const type = assistantFirstString(event, ["type", "kind"]).toLowerCase();
    const runId = type === "batch_agent_done"
      ? assistantFirstString(event, ["child_run_id", "run_id"])
      : eventRunId(event);
    const batchAgents = Array.isArray(event.agents) ? event.agents : [];
    if (runId && batchAgents.length) {
      const batchNode = ensure(runId, event);
      batchNode.title =
        assistantFirstString(event, ["title", "description", "name"]) ||
        "Agent batch";
      for (const [index, rawAgent] of batchAgents.entries()) {
        const agent = assistantRecord(rawAgent);
        const agentId =
          assistantFirstString(agent, ["run_id", "runId", "agent_id", "id"]) ||
          `${runId}-agent-${index}`;
        const agentNode = ensure(agentId, agent);
        agentNode.parentId =
          assistantFirstString(agent, ["parent_run_id", "parentRunId"]) ||
          runId;
        agentNode.status = itemStatus(agent);
        if (agentNode.status === "empty") agentNode.status = "pending";
      }
    }
    if (!runId) continue;
    const node = ensure(runId, event);
    node.parentId =
      assistantFirstString(event, ["parent_run_id", "parentRunId"]) ||
      node.parentId;
    node.title =
      assistantFirstString(event, [
        "task_name",
        "description",
        "title",
        "name",
      ]) || node.title;
    node.persona =
      assistantFirstString(event, ["subagent_type", "persona", "agent"]) ||
      node.persona;
    if (type === "batch_agent_done") {
      node.status = itemStatus(event);
    } else if (type === "batch_done") {
      // Child outcomes, not the existence of a "done" notification, determine
      // whether a batch succeeded. Progress below counts children only.
      node.status = Number(event.failed || 0) + Number(event.timed_out || 0) + Number(event.skipped || 0) > 0 ? "error" : "empty";
    } else if (/error|failed/.test(type) || event.is_error === true || event.ok === false) node.status = "error";
    else if (type === "nested_done") node.status = event.ok === false ? "error" : "success";
    else if (node.status !== "success" && node.status !== "error" && /start|assistant|tool/.test(type)) node.status = "running";
    node.detail =
      assistantFirstString(event, ["summary", "message", "error", "detail", "output"]) ||
      node.detail;
  }

  if (!nodes.size && !statusQuery && !/(?:^|__)AgentBatch$/.test(tool.name)) {
    const input = partialInput(tool);
    const id = tool.runId || tool.id;
    nodes.set(id, {
      id,
      title:
        assistantFirstString(input, ["task_name", "description", "message"]) ||
        (workflowWidgetKind(tool) === "agent" ? "Delegated task" : "Agent run"),
      persona:
        assistantFirstString(input, ["subagent_type", "persona", "agent"]) ||
        undefined,
      status:
        tool.status === "error"
          ? "error"
          : tool.status === "done"
            ? "success"
            : tool.status === "unknown"
              ? "empty"
            : "running",
      detail: tool.output
        ? stripAgentMarkerHeaders(tool.output, tool.name).output
        : undefined,
      children: [],
    });
  }

  const roots: AgentRunNode[] = [];
  for (const node of nodes.values()) {
    const parent = node.parentId ? nodes.get(node.parentId) : undefined;
    // Corrupt/untrusted parent links must not form a cycle in the renderer.
    let ancestor = parent;
    const visited = new Set([node.id]);
    while (ancestor && !visited.has(ancestor.id)) {
      visited.add(ancestor.id);
      ancestor = ancestor.parentId ? nodes.get(ancestor.parentId) : undefined;
    }
    if (parent && !ancestor) parent.children.push(node);
    else roots.push(node);
  }
  return roots.slice(0, 50);
}

function flattenAgentTree(nodes: AgentRunNode[]): AgentRunNode[] {
  return nodes.flatMap((node) => [node, ...flattenAgentTree(node.children)]);
}

function planItems(
  input: UnknownRecord,
  payload: UnknownRecord,
): WorkflowListItem[] {
  const plan = findRecord(payload, ["plan", "artifact"]);
  const outputItems = firstArray(plan, ["items"]);
  const values = outputItems.length
    ? outputItems
    : normalizePlanInputItems(input.items);
  return listItems(values, "Plan item");
}

function completion(
  items: WorkflowListItem[],
): { done: number; total: number } | undefined {
  if (!items.length) return undefined;
  return {
    done: items.filter((item) => item.status === "success").length,
    total: items.length,
  };
}

function field(
  label: string,
  value: unknown,
  mono = false,
): { label: string; value: React.ReactNode; mono?: boolean } | undefined {
  const text = assistantStringValue(value).trim();
  return text ? { label, value: text, mono } : undefined;
}

function namedListValue(value: unknown, keys: string[]): string {
  return (
    assistantStringValue(value) ||
    assistantFirstString(assistantRecord(value), keys)
  );
}

function contractFields(
  payload: UnknownRecord,
  kindRecord: UnknownRecord,
  kind: WorkflowWidgetKind,
): Array<{ label: string; value: React.ReactNode; mono?: boolean }> {
  if (kind === "agent") return []; // State is already shown in the header and each child.
  const values = [
    field("Owner", kindRecord.owner),
    field("Schedule", kindRecord.schedule ?? kindRecord.cron, true),
    field("Timezone", kindRecord.timezone),
    field("Next run", kindRecord.next_run_at ?? kindRecord.nextRunAt, true),
    field("Repository", kindRecord.repo ?? kindRecord.repository, true),
    field("Scope", kindRecord.scope),
    field("Version", kindRecord.version, true),
    field(
      "Signature",
      kindRecord.signature ?? kindRecord.signature_status,
      true,
    ),
    field(
      "Hash",
      kindRecord.sha256 ?? kindRecord.hash ?? kindRecord.digest,
      true,
    ),
    field("State", kindRecord.status ?? kindRecord.state),
  ].filter(
    (
      value,
    ): value is { label: string; value: React.ReactNode; mono?: boolean } =>
      Boolean(value),
  );
  if (kind === "assistant" && !values.length) {
    const assistant = assistantNestedRecord(payload, ["assistant", "profile"]);
    const model = field("Model", assistant.model ?? assistant.provider, true);
    if (model) values.push(model);
  }
  return values;
}

function titleFor(
  kind: WorkflowWidgetKind,
  action: string,
  input: UnknownRecord,
  payload: UnknownRecord,
  kindRecord: UnknownRecord,
): string {
  if (kind === "agent" && action === "AgentBatchStatus") return "Agent status";
  const card = assistantRecord(assistantRecord(payload.summary).card);
  const identity =
    assistantFirstString(kindRecord, [
      "title",
      "name",
      "objective",
      "content",
      "display_name",
    ]) ||
    assistantFirstString(input, [
      "title",
      "name",
      "objective",
      "task_name",
      "description",
    ]);
  const cardTitle = assistantFirstString(card, ["title"]);
  const preferred =
    kind === "assistant" ? cardTitle || identity : identity || cardTitle;
  if (preferred) return preferred;
  const fallback: Record<WorkflowWidgetKind, string> = {
    assistant: "Flamel Assistant",
    automation: "Automation",
    job: "Job",
    plan: "Plan",
    work: "Engineering work",
    agent: "Agent run",
    memory: "Memory",
    skill: "Skill",
    connection: "Connection",
  };
  return fallback[kind] + (action ? ` · ${action.replace(/_/g, " ")}` : "");
}

function openCanonicalSurface(kind: WorkflowWidgetKind, record: UnknownRecord) {
  if (kind === "plan") {
    useAppStore.getState().setRightPanel("plan");
    useAppStore.getState().setRightPanelOpen(true);
    return;
  }
  const view =
    kind === "memory"
      ? "memory"
      : kind === "automation"
        ? "automations"
        : kind === "job"
          ? "jobs"
          : kind === "assistant"
            ? "overview"
            : kind === "skill" || kind === "connection"
              ? "integrations"
              : undefined;
  if (!view) return;
  const objectId = assistantFirstString(record, [
    "id",
    "job_id",
    "profile_id",
    "assistant_id",
  ]);
  const tab =
    view === "integrations"
      ? `integrations:${kind === "skill" ? "skills" : "connected"}`
      : ["assistant", view, objectId].filter(Boolean).join(":");
  window.dispatchEvent(
    new CustomEvent("flamel:open-settings", {
      detail: { tab },
    }),
  );
}

function supportsCanonicalSurface(kind: WorkflowWidgetKind): boolean {
  return [
    "plan",
    "assistant",
    "memory",
    "automation",
    "job",
    "skill",
    "connection",
  ].includes(kind);
}

export function adaptWorkflowTool(
  tool: ToolCall,
): WorkflowToolView | undefined {
  const kind = workflowWidgetKind(tool);
  if (!kind) return undefined;
  const input = partialInput(tool);
  const action = workflowAction(tool);
  const output = unwrapOutput(tool);
  const payload = output.payload;
  const result = findRecord(payload, ["result", "data"]);
  const kindRecord = findRecord(result, [
    "assistant",
    "profile",
    "plan",
    "artifact",
    "automation",
    "job",
    "run",
    "work",
    "item",
    "record",
    "skill",
    "pack",
    "connection",
  ]);
  const summaryRecord = assistantRecord(payload.summary);
  const approvalRequired =
    payload.approvalRequired === true ||
    payload.approval_required === true ||
    assistantRecord(summaryRecord.card).approvalRequired === true;
  const mutated = payload.mutated === true;
  const replayed = payload.replayed === true;
  const ok = typeof payload.ok === "boolean" ? payload.ok : undefined;
  const contractVersion =
    typeof payload.contractVersion === "number"
      ? payload.contractVersion
      : typeof payload.contract_version === "number"
        ? payload.contract_version
        : undefined;
  const contractKnown =
    contractVersion === undefined ||
    (contractVersion === 1 &&
      (!payload.surfaceContract ||
        payload.surfaceContract === "flamel-assistant.v1"));
  let items: WorkflowListItem[] = [];
  let timeline: ToolTimelineItem[] = [];
  let agentTree: AgentRunNode[] = [];
  let progress: WorkflowToolView["progress"];

  if (kind === "plan") {
    items = planItems(input, payload);
    progress = completion(items);
  } else if (kind === "agent") {
    agentTree = buildAgentRunTree(tool.nestedEvents ?? [], tool);
    const agentNodes = flattenAgentTree(agentTree).filter((node) => !node.children.length);
    if (agentNodes.length) progress = {
      done: agentNodes.filter((node) => node.status === "success").length,
      total: agentNodes.length,
    };
  } else {
    const preferredKeys = [
      kind === "memory" ? "records" : "",
      kind === "automation" ? "profiles" : "",
      kind === "job" ? "jobs" : "",
      kind === "skill" ? "skills" : "",
      kind === "connection" ? "connections" : "",
      "items",
    ].filter(Boolean);
    const itemValues =
      kind === "work"
        ? [
            ...firstArray(result, ["dependencies"]),
            ...firstArray(result, ["artifacts"]),
          ]
        : firstArray(result, preferredKeys);
    items = listItems(itemValues, kind);
    const eventValues = firstArray(result, [
      "events",
      "history",
      "timeline",
      "runs",
      "evidence",
      "logs",
    ]);
    timeline = timelineItems(eventValues);
    const explicitDone = Number(
      kindRecord.completed ??
        kindRecord.done ??
        result.completed ??
        result.done,
    );
    const explicitTotal = Number(
      kindRecord.total ?? result.total ?? kindRecord.steps_total,
    );
    if (
      Number.isFinite(explicitDone) &&
      Number.isFinite(explicitTotal) &&
      explicitTotal > 0
    ) {
      progress = { done: explicitDone, total: explicitTotal };
    } else {
      progress = completion(items);
    }
  }

  const hasEvidence =
    Boolean(output.text) ||
    Object.keys(payload).length > 0 ||
    items.length > 0 ||
    timeline.length > 0 ||
    agentTree.length > 0 ||
    approvalRequired ||
    mutated ||
    replayed;
  const batchStatusQuery = kind === "agent" && /(?:^|__)AgentBatch(?:Status)?$/.test(tool.name);
  const batchReceipt = batchStatusQuery && (tool.status === "done" || tool.status === "error");
  const unconfirmed = (kind === "agent" && tool.status === "unknown") || (batchReceipt && (!Array.isArray(payload.agents) || !payload.agents.length || payload.status === "unknown" || payload.status === "not_found"));
  const status = tool.status === "error" ? "error" : unconfirmed ? "empty" : batchReceipt ? itemStatus(payload) : toolStatus(tool, ok, approvalRequired, hasEvidence);
  const contractSummary =
    status === "error"
      ? assistantFirstString(payload, ["error", "message", "text"]) ||
        assistantStringValue(summaryRecord.text) ||
        output.text.split(/\r?\n/).find(Boolean) ||
        ""
      : assistantStringValue(summaryRecord.text) ||
        assistantFirstString(payload, ["message", "text", "error"]) ||
        output.text.split(/\r?\n/).find(Boolean) ||
        "";
  const agentLeaves = flattenAgentTree(agentTree).filter((node) => !node.children.length);
  const agentFailures = agentLeaves.filter((node) => node.status === "error").length;
  const summary = unconfirmed ? assistantFirstString(payload,["message"]) || "No agent completion has been confirmed" : kind === "agent" && agentFailures > 0 ? `${agentLeaves.filter((node) => node.status === "success").length} succeeded · ${agentFailures} failed` : approvalRequired
    ? "Approval required before this change"
    : replayed
      ? contractSummary || "Idempotent replay returned the original receipt"
      : status === "running"
        ? `${action.replace(/_/g, " ")} is running`
        : status === "pending"
          ? `${action.replace(/_/g, " ")} is queued`
          : status === "error"
            ? contractSummary || "Workflow action failed"
            : progress
              ? `${progress.done} of ${progress.total} complete`
              : contractSummary ||
                (mutated ? "Change confirmed" : "Operation complete");
  const fields = contractFields(result, kindRecord, kind);
  if (kind === "skill") {
    const triggers = firstArray(kindRecord, ["triggers"])
      .map((value) =>
        namedListValue(value, ["name", "trigger", "description", "pattern"]),
      )
      .filter(Boolean)
      .slice(0, 12)
      .join(", ");
    const provenance =
      assistantFirstString(kindRecord, [
        "provenance",
        "derived_from",
        "persona",
        "source",
      ]) || "";
    if (triggers) fields.push({ label: "Triggers", value: triggers });
    if (provenance) fields.push({ label: "Provenance", value: provenance });
    const active = field(
      "Projection",
      kindRecord.active === true
        ? "active"
        : kindRecord.enabled === false
          ? "disabled"
          : kindRecord.projection_state,
    );
    if (active) fields.push(active);
  }
  if (kind === "connection") {
    const permissions = firstArray(kindRecord, ["permissions", "scopes"])
      .map((value) =>
        namedListValue(value, ["name", "scope", "permission", "description"]),
      )
      .filter(Boolean)
      .slice(0, 20)
      .join(", ");
    if (permissions) fields.push({ label: "Permissions", value: permissions });
  }
  if (contractVersion !== undefined) {
    fields.push({
      label: "Contract",
      value: contractKnown
        ? `v${contractVersion}`
        : `v${contractVersion} · unsupported`,
      mono: true,
    });
  }

  return {
    kind,
    action,
    title: titleFor(kind, action, input, payload, kindRecord),
    summary,
    detail: kind === "agent" ? undefined : action.replace(/_/g, " "),
    status,
    statusLabel: unconfirmed ? "Unconfirmed" : approvalRequired
      ? "Preview"
      : replayed
        ? "Replayed"
        : status === "running"
          ? "Live"
          : undefined,
    approvalRequired,
    mutated,
    replayed,
    contractVersion,
    contractKnown,
    fields,
    items,
    timeline,
    agentTree,
    progress,
    result,
    rawOutput: output.raw,
  };
}

function spriteFor(kind: WorkflowWidgetKind): ToolWidgetSprite {
  if (kind === "skill" || kind === "connection") return "integrations";
  if (kind === "memory") return "data";
  return "assistant-workflow";
}

function iconFor(kind: WorkflowWidgetKind) {
  if (kind === "agent") return <Icon.User size={14} />;
  if (kind === "memory") return <Icon.History size={14} />;
  if (kind === "plan" || kind === "work") return <Icon.Plan size={14} />;
  if (kind === "skill" || kind === "connection")
    return <Icon.Prompts size={14} />;
  return <Icon.Clock size={14} />;
}

function AgentBranch({
  node,
  level = 0,
}: {
  node: AgentRunNode;
  level?: number;
}) {
  return (
    <li
      data-status={node.status}
      style={{ "--agent-level": level } as React.CSSProperties}
    >
      <span className="workflow-widget__agent-rail" aria-hidden="true" />
      <div className="workflow-widget__agent-copy">
        <strong>{node.title}</strong>
        {node.persona ? <span>{node.persona}</span> : null}
        {node.detail ? <details className="workflow-widget__agent-result"><summary>View result</summary><pre>{node.detail}</pre></details> : null}
      </div>
      <span>{{empty:"Unconfirmed",success:"Finished",error:"Failed",running:"Running",pending:"Waiting"}[node.status]}</span>
      {node.children.length ? (
        <ol>
          {node.children.map((child) => (
            <AgentBranch key={child.id} node={child} level={level + 1} />
          ))}
        </ol>
      ) : null}
    </li>
  );
}

function ProgressReceipt({
  progress,
}: {
  progress: NonNullable<WorkflowToolView["progress"]>;
}) {
  const percentage = Math.round(
    (Math.max(0, progress.done) / Math.max(1, progress.total)) * 100,
  );
  return (
    <div className="workflow-widget__progress" aria-label="Progress">
      <div>
        <span>Progress</span>
        <strong>
          {progress.done} / {progress.total}
        </strong>
      </div>
      <span>
        <span style={{ width: `${Math.min(100, percentage)}%` }} />
      </span>
    </div>
  );
}

function WorkflowBody({
  tool,
  view,
}: {
  tool: ToolCall;
  view: WorkflowToolView;
}) {
  return (
    <>
      {view.approvalRequired ? (
        <ToolWidgetState
          status="pending"
          title="Read-only preview"
          detail="No mutation has run. Confirm through the canonical approval surface."
        />
      ) : null}
      {!view.contractKnown ? (
        <ToolWidgetState
          status="error"
          title="Unknown response contract"
          detail="The receipt is shown conservatively; inspect raw data for the complete payload."
        />
      ) : null}
      {view.kind === "agent" && view.statusLabel === "Unconfirmed" ? <ToolWidgetState status="empty" title="Completion unconfirmed" detail={view.summary} /> : null}
      {view.progress && (view.kind !== "agent" || view.status === "running" || view.status === "pending") ? <ProgressReceipt progress={view.progress} /> : null}
      {view.fields.length ? <ToolWidgetFields fields={view.fields} /> : null}
      {view.items.length ? (
        <ToolWidgetSection
          title={
            view.kind === "plan"
              ? "Plan items"
              : view.kind === "skill"
                ? "Skill records"
                : "Related records"
          }
        >
          <ToolWidgetList
            items={view.items.map((item) => ({
              id: item.id,
              primary: item.primary,
              secondary: item.secondary,
              trailing: item.trailing,
            }))}
          />
        </ToolWidgetSection>
      ) : null}
      {view.timeline.length ? (
        <ToolWidgetSection title="Timeline">
          <ToolWidgetTimeline items={view.timeline} />
        </ToolWidgetSection>
      ) : null}
      {view.agentTree.length ? (
        <ToolWidgetSection title="Execution tree">
          <ol className="workflow-widget__agent-tree">
            {view.agentTree.map((node) => (
              <AgentBranch key={node.id} node={node} />
            ))}
          </ol>
        </ToolWidgetSection>
      ) : null}
      {tool.artifacts?.length ? (
        <ToolWidgetSection title="Artifacts">
          <ToolArtifactActions artifacts={tool.artifacts} />
        </ToolWidgetSection>
      ) : null}
      {supportsCanonicalSurface(view.kind) ? (
        <div className="workflow-widget__actions">
          <button
            type="button"
            onClick={() => openCanonicalSurface(view.kind, view.result)}
          >
            {view.kind === "plan"
              ? "Open Plan panel"
              : "Open canonical surface"}
          </button>
        </div>
      ) : null}
      {!view.approvalRequired &&
      view.statusLabel !== "Unconfirmed" &&
      !view.fields.length &&
      !view.items.length &&
      !view.timeline.length &&
      !view.agentTree.length &&
      !tool.artifacts?.length ? (
        <ToolWidgetState
          status={view.status === "success" ? "empty" : view.status}
          title={
            view.status === "running"
              ? "Collecting workflow evidence"
              : "No structured receipt"
          }
          detail="The provider payload remains available in the inspector."
        />
      ) : null}
    </>
  );
}

export function WorkflowToolWidget({
  tool,
  defaultExpanded,
}: {
  tool: ToolCall;
  defaultExpanded?: boolean;
}) {
  const view = useMemo(() => adaptWorkflowTool(tool), [tool]);
  if (!view) return null;

  return (
    <ToolWidgetShell
      title={view.title}
      eyebrow={view.kind}
      summary={view.summary}
      detail={view.detail}
      status={view.status}
      statusLabel={view.statusLabel}
      icon={iconFor(view.kind)}
      sprite={spriteFor(view.kind)}
      trailing={
        view.replayed ? "idempotent" : view.mutated ? "confirmed" : undefined
      }
      defaultExpanded={defaultExpanded ?? false}
      inspector={
        <ToolWidgetInspector
          input={partialInput(tool)}
          output={tool.output}
          raw={{
            presentation: tool.presentation,
            nestedEvents: tool.nestedEvents,
            contractVersion: view.contractVersion,
          }}
        />
      }
    >
      <WorkflowBody tool={tool} view={view} />
    </ToolWidgetShell>
  );
}
