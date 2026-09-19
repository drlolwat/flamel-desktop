import type { CommandEvent } from "./consoleTypes";
import type { ToolCall } from "./messageProcessor";
import { textFromToolResultContent } from "./messageProcessor";
import { applyToolResult, type AppliedToolResult, type ToolResultEventLike } from "./toolResultReducer";

export function shouldStartAssistantBlockAfterToolResult(
  sawToolResult: boolean,
  content: unknown,
  currentTools: ToolCall[],
): boolean {
  if (!sawToolResult) return false;
  if (!Array.isArray(content)) return true;
  const currentIds = new Set(currentTools.map((tool) => tool.id));
  // Codex snapshots repeat earlier tool envelopes after their final receipts.
  // Keep those tools (and their receipts) in this block; disjoint Claude turns
  // still start a fresh block. A snapshot alone never proves tool completion.
  return !content.some(
    (item) => item?.type === "tool_use" && !!item.id && currentIds.has(item.id),
  );
}

export function toolFromAssistantSnapshot(
  envelope: Pick<ToolCall, "id" | "name" | "input">,
  previous?: ToolCall,
): ToolCall {
  // A guidance message/reconnect may create a block boundary within one
  // cumulative provider turn. Preserve the receipt on the earlier call ID.
  return { ...previous, ...envelope, status: previous?.status ?? "running" };
}

export function isCommandEventPayload(payload: unknown): payload is CommandEvent {
  const event = payload as Partial<CommandEvent> | null;
  return (
    !!event &&
    typeof event === "object" &&
    typeof event.event_id === "string" &&
    typeof event.kind === "string"
  );
}

export function commandEventOrigins(event: CommandEvent) {
  return [event.session?.origin, event.run?.origin, event.block?.origin].filter(
    Boolean,
  );
}

export function commandEventToolCallId(event: CommandEvent): string | undefined {
  for (const origin of commandEventOrigins(event)) {
    const toolCallId = origin?.tool_call_id;
    if (typeof toolCallId === "string" && toolCallId) return toolCallId;
  }
  return undefined;
}

export function commandEventLooksShellLike(event: CommandEvent): boolean {
  const origins = commandEventOrigins(event);
  return (
    origins.length === 0 ||
    origins.some(
      (origin) => origin?.kind === "mcp_shell" || origin?.kind === "pty",
    )
  );
}

export function isShellToolName(name: string): boolean {
  const bare = name.split("__").pop() || name;
  return bare === "shell" || bare === "exec_command";
}

export function appendCommandEvent(
  events: CommandEvent[] | undefined,
  event: CommandEvent,
): CommandEvent[] {
  if (events?.some((existing) => existing.event_id === event.event_id))
    return events;
  return [...(events || []), event].sort(
    (a, b) => (a.sequence || 0) - (b.sequence || 0),
  );
}

export function attachPendingCommandEvents(
  tool: ToolCall,
  pending: CommandEvent[],
): ToolCall {
  if (!isShellToolName(tool.name) || pending.length === 0) return tool;
  const matching = pending.filter(
    (event) => commandEventToolCallId(event) === tool.id,
  );
  if (matching.length === 0) return tool;
  for (let index = pending.length - 1; index >= 0; index--) {
    if (commandEventToolCallId(pending[index]) === tool.id)
      pending.splice(index, 1);
  }
  const nextEvents = matching.reduce<CommandEvent[]>(
    (events, event) => appendCommandEvent(events, event),
    tool.commandEvents || [],
  );
  return nextEvents === tool.commandEvents
    ? tool
    : { ...tool, commandEvents: nextEvents };
}

export function attachCommandEventToTool(
  event: CommandEvent,
  currentTools: ToolCall[],
  pending: CommandEvent[],
  updateTool: (toolId: string, updates: Partial<ToolCall>) => void,
): boolean {
  if (!commandEventLooksShellLike(event)) return false;

  const explicitToolId = commandEventToolCallId(event);
  let targetIndex = explicitToolId
    ? currentTools.findIndex((tool) => tool.id === explicitToolId)
    : -1;
  if (targetIndex < 0 && !explicitToolId) {
    for (let i = currentTools.length - 1; i >= 0; i--) {
      const tool = currentTools[i];
      if (
        tool &&
        isShellToolName(tool.name) &&
        (tool.status === "running" || tool.status === "pending")
      ) {
        targetIndex = i;
        break;
      }
    }
  }
  if (targetIndex < 0) {
    // Only explicitly correlated events may wait for a future tool envelope.
    // An unscoped late event must never be pinned to the last completed terminal.
    if (explicitToolId) {
      if (!pending.some((existing) => existing.event_id === event.event_id))
        pending.push(event);
      return true;
    }
    return false;
  }

  const tool = currentTools[targetIndex];
  if (!tool) return true;
  const commandEvents = appendCommandEvent(tool.commandEvents, event);
  if (commandEvents === tool.commandEvents) return true;
  currentTools[targetIndex] = { ...tool, commandEvents };
  updateTool(tool.id, { commandEvents });
  return true;
}

export function applyStreamingToolResult(
  currentTools: ToolCall[],
  result: ToolResultEventLike,
  update: (toolId: string, updates: Partial<ToolCall>) => void,
  bindNestedRunToTool: (runId: string, toolId: string) => void,
  mergedNestedEventsForTool: (
    toolId: string,
    existingEvents: Array<Record<string, unknown>>,
  ) => Array<Record<string, unknown>>,
  findPreviousTool: (toolId: string) => ToolCall | undefined,
  pending: Map<string, ToolResultEventLike>,
): AppliedToolResult | undefined {
  const inCurrentBlock = currentTools.some((tool) => tool.id === result.tool_use_id);
  const previous = inCurrentBlock ? undefined : findPreviousTool(result.tool_use_id);
  const tools = inCurrentBlock ? currentTools : previous ? [previous] : [];
  if (!tools.length) {
    const existing = pending.get(result.tool_use_id);
    if (existing && !existing.is_delta) return undefined;
    // ponytail: only unmatched results are buffered; overflow reconnects to the durable journal.
    if (!existing && pending.size >= 1024) throw new Error("Too many unmatched tool results; reconnecting");
    pending.set(result.tool_use_id, existing && result.is_delta ? {
      ...result,
      content: textFromToolResultContent(existing.content) + textFromToolResultContent(result.content),
    } : result);
    return undefined;
  }
  pending.delete(result.tool_use_id);
  const application = applyToolResult(tools, result, {
    decorate: (nextTool, reduction) => {
      if (!reduction.runId) return nextTool;
      bindNestedRunToTool(reduction.runId, nextTool.id);
      return {
        ...nextTool,
        nestedEvents: mergedNestedEventsForTool(
          nextTool.id,
          nextTool.nestedEvents || [],
        ),
      };
    },
  });
  if (!application?.changed) return application;

  if (inCurrentBlock) currentTools.splice(0, currentTools.length, ...application.tools);
  update(application.tool.id, application.tool);
  return application;
}
