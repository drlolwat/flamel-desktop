import type {
  CommandBlock,
  CommandEvent,
  CommandInputRequest,
  CommandLifecycleStatus,
  CommandOutputChunk,
  CommandOrigin,
  CommandSession,
  CommandStreamKind,
} from '../../lib/consoleTypes';
import { isShellLikeToolName } from '../../lib/toolCapabilities';
import type { NestedAgentEvent, ToolCall } from '../../lib/messageProcessor';

export interface ConsoleLineView {
  id: string;
  sequence: number;
  stream: CommandStreamKind;
  text: string;
  timestamp: string;
  eventKind: CommandEvent['kind'];
  metadata: Record<string, string>;
}

export interface ConsoleBlockView {
  id: string;
  sessionId: string;
  runId?: string | null;
  title: string;
  command?: string | null;
  cwd?: string | null;
  status: CommandLifecycleStatus;
  exitCode?: number | null;
  source: string;
  startedAt?: string | null;
  finishedAt?: string | null;
  metadata: Record<string, string>;
  lines: ConsoleLineView[];
  inputRequests: CommandInputRequest[];
  error?: string | null;
}

const ANSI_ESCAPE_SEQUENCE = /\x1b(?:\][^\x07\x1b]*(?:\x07|\x1b\\)|\[[0-?]*[ -/]*[@-~]|[()][0-2A-Z0-9]|[=>])/g;
const ANSI_SGR_SEQUENCE = /^\x1b\[[0-9;]*m$/;
const NON_PRINTING_CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001a\u001c-\u001f\u007f]/g;

/**
 * Keep color/style SGR codes for rendering, but turn terminal redraw streams
 * into a readable transcript. BuildKit and similar tools use cursor movement,
 * erase commands, and bare carriage returns that otherwise leak into the UI.
 */
export function normalizeConsoleText(text: string, preserveSgr = true): string {
  return text
    .replace(ANSI_ESCAPE_SEQUENCE, sequence => (
      preserveSgr && ANSI_SGR_SEQUENCE.test(sequence) ? sequence : ''
    ))
    .replace(/\r\n/g, '\n')
    .replace(/\r/g, '\n')
    .replace(NON_PRINTING_CONTROL, '');
}

export function plainConsoleText(text: string): string {
  return normalizeConsoleText(text, false);
}

export function consoleOutputLineCount(lines: ConsoleLineView[]): number {
  return lines.reduce((total, line) => {
    const text = plainConsoleText(line.text);
    if (!text) return total;
    const breaks = text.match(/\n/g)?.length ?? 0;
    const endsWithBreak = text.endsWith('\n');
    return total + breaks + (endsWithBreak ? 0 : 1);
  }, 0);
}

interface MutableBlock extends ConsoleBlockView {
  sortSequence: number;
  lineIds: Set<string>;
  inputRequestIds: Set<string>;
}

export function buildConsoleBlocks(events: CommandEvent[]): ConsoleBlockView[] {
  const sessions = new Map<string, CommandSession>();
  const blocks = new Map<string, MutableBlock>();
  const runToBlock = new Map<string, string>();

  const sorted = [...events].sort((a, b) => a.sequence - b.sequence);
  for (const event of sorted) {
    if (event.session) sessions.set(event.session.id, event.session);
    if (event.block) runToBlock.set(event.block.run_id, event.block.id);

    if (event.kind === 'command_started' && event.block) {
      upsertBlock(blocks, sessions, event.block.id, event.sequence, {
        block: event.block,
        run: event.run ?? undefined,
      });
      continue;
    }

    if (event.kind === 'command_finished' && event.block) {
      upsertBlock(blocks, sessions, event.block.id, event.sequence, {
        block: event.block,
        run: event.run ?? undefined,
      });
      continue;
    }

    if ((event.kind === 'output_chunk' || event.kind === 'input_sent') && event.chunk) {
      const blockId = event.chunk.block_id || event.block_id || runToBlock.get(event.chunk.run_id) || event.chunk.run_id;
      const block = upsertBlock(blocks, sessions, blockId, event.sequence, {
        chunk: event.chunk,
      });
      if (!block.lineIds.has(event.chunk.id)) {
        block.lineIds.add(event.chunk.id);
        block.lines.push({
          id: event.chunk.id,
          sequence: event.chunk.sequence,
          stream: event.chunk.stream,
          text: normalizeConsoleText(event.chunk.text),
          timestamp: event.chunk.received_at || event.timestamp,
          eventKind: event.kind,
          metadata: event.chunk.metadata ?? {},
        });
      }
      continue;
    }

    if (event.kind === 'input_requested' && event.input_request) {
      const blockId = event.input_request.block_id || event.block_id || event.input_request.run_id;
      const block = upsertBlock(blocks, sessions, blockId, event.sequence, {
        inputRequest: event.input_request,
      });
      if (!block.inputRequestIds.has(event.input_request.id)) {
        block.inputRequestIds.add(event.input_request.id);
        block.inputRequests.push(event.input_request);
      }
      continue;
    }

    if (event.kind === 'error') {
      const blockId = event.block_id || event.run_id || event.session_id;
      const block = upsertBlock(blocks, sessions, blockId, event.sequence, {
        sessionId: event.session_id,
      });
      block.status = 'failed';
      block.error = event.error || 'Command error';
    }
  }

  return [...blocks.values()]
    .map(block => ({
      ...block,
      lines: [...block.lines].sort((a, b) => a.sequence - b.sequence),
      inputRequests: [...block.inputRequests].sort((a, b) => a.requested_at.localeCompare(b.requested_at)),
    }))
    .sort((a, b) => a.sortSequence - b.sortSequence)
    .map(({ sortSequence: _sortSequence, lineIds: _lineIds, inputRequestIds: _inputRequestIds, ...block }) => block);
}

export function commandBlocksToText(blocks: ConsoleBlockView[]): string {
  return blocks
    .map(block => {
      const header = [`$ ${block.title}`];
      if (block.cwd) header.push(`cwd: ${block.cwd}`);
      if (block.status) header.push(`status: ${block.status}`);
      const lines = block.lines.map(line => plainConsoleText(line.text)).join('');
      return `${header.join(' | ')}\n${lines}`.trimEnd();
    })
    .filter(Boolean)
    .join('\n\n');
}

export function isShellLikeToolCall(tool: ToolCall): boolean {
  if (isShellLikeToolName(tool.name)) return true;

  const action = typeof tool.input?.action === 'string' ? tool.input.action.toLowerCase() : '';
  const hasCommandInput =
    typeof tool.input?.command === 'string' ||
    typeof tool.input?.cmd === 'string' ||
    Array.isArray(tool.input?.commands);
  return hasCommandInput && ['exec', 'run', 'start', 'multi', 'write', 'read', 'kill', 'status'].includes(action);
}

export function latestRunningConsoleTool(tools: ToolCall[]): ToolCall | undefined {
  for (let index = tools.length - 1; index >= 0; index -= 1) {
    const tool = tools[index];
    if (
      tool &&
      (tool.status === 'running' || tool.status === 'pending') &&
      isShellLikeToolCall(tool)
    ) {
      return tool;
    }
  }
  return undefined;
}

export function toolCallToCommandEvents(tool: ToolCall, options: { title?: string; source?: string } = {}): CommandEvent[] {
  const timestamp = new Date().toISOString();
  const sessionId = stableSyntheticId('tool-session', tool.runId || tool.id || tool.name);
  const runId = stableSyntheticId('tool-run', tool.id || tool.runId || tool.name);
  const blockId = stableSyntheticId('tool-block', tool.id || runId);
  const status = toolStatusToCommandStatus(tool.status);
  const finished = status === 'succeeded' || status === 'failed';
  const command = commandTextFromTool(tool);
  const title = options.title || command || tool.name;
  const origin: CommandOrigin = {
    kind: 'mcp_shell',
    tool_call_id: tool.id,
    label: options.source || tool.name,
  };
  const metadata = metadataFromUnknown({
    tool: tool.name,
    action: tool.input?.action,
    source: options.source || 'chat_tool',
  });
  let sequence = 1;
  const events: CommandEvent[] = [];
  const session: CommandSession = {
    id: sessionId,
    origin,
    status: status === 'succeeded' || status === 'failed' ? 'closed' : status,
    created_at: timestamp,
    closed_at: finished ? timestamp : null,
    cwd: typeof tool.input?.cwd === 'string' ? tool.input.cwd : null,
    metadata,
  };
  const run = {
    id: runId,
    session_id: sessionId,
    origin,
    status,
    started_at: timestamp,
    finished_at: finished ? timestamp : null,
    command,
    cwd: typeof tool.input?.cwd === 'string' ? tool.input.cwd : null,
    exit_code: status === 'failed' ? 1 : status === 'succeeded' ? 0 : null,
    metadata,
  };
  const block: CommandBlock = {
    id: blockId,
    session_id: sessionId,
    run_id: runId,
    origin,
    status,
    started_at: timestamp,
    finished_at: run.finished_at,
    title,
    command,
    exit_code: run.exit_code,
    metadata,
  };

  events.push({
    event_id: `${sessionId}-session-started`,
    kind: 'session_started',
    session_id: sessionId,
    run_id: runId,
    block_id: blockId,
    sequence: sequence++,
    timestamp,
    session,
  });
  events.push({
    event_id: `${runId}-command-started`,
    kind: 'command_started',
    session_id: sessionId,
    run_id: runId,
    block_id: blockId,
    sequence: sequence++,
    timestamp,
    session,
    run,
    block,
  });

  if (tool.output) {
    const chunkSequence = sequence++;
    events.push({
      event_id: `${runId}-output-0`,
      kind: 'output_chunk',
      session_id: sessionId,
      run_id: runId,
      block_id: blockId,
      sequence: chunkSequence,
      timestamp,
      chunk: {
        id: `${runId}-chunk-0`,
        session_id: sessionId,
        run_id: runId,
        block_id: blockId,
        sequence: chunkSequence,
        stream: status === 'failed' ? 'stderr' : 'stdout',
        text: tool.output,
        received_at: timestamp,
        byte_len: tool.output.length,
        metadata,
      },
    });
  }

  if (status === 'failed' && !tool.output) {
    events.push({
      event_id: `${runId}-error`,
      kind: 'error',
      session_id: sessionId,
      run_id: runId,
      block_id: blockId,
      sequence: sequence++,
      timestamp,
      error: 'Command failed',
    });
  }

  if (finished) {
    events.push({
      event_id: `${runId}-command-finished`,
      kind: 'command_finished',
      session_id: sessionId,
      run_id: runId,
      block_id: blockId,
      sequence: sequence++,
      timestamp,
      session,
      run,
      block,
    });
  }

  return events;
}

export function nestedCommandEventsToCommandEvents(
  events: NestedAgentEvent[],
  title = 'Sub-agent command output',
): CommandEvent[] {
  const outputEvents = events
    .map((event, index) => ({ event, index }))
    .filter(({ event }) => event.type === 'command_execution.output_delta' && typeof event.delta === 'string');
  if (outputEvents.length === 0) return [];

  const timestamp = timestampFromEvent(outputEvents[0]?.event) || new Date().toISOString();
  const finishedAt = timestampFromEvent(outputEvents[outputEvents.length - 1]?.event) || timestamp;
  const startedCommand = events.find(event => {
    if (event.type !== 'item.started') return false;
    const item = event.item as Record<string, unknown> | undefined;
    return item?.type === 'command_execution';
  })?.item as Record<string, unknown> | undefined;
  const completedCommand = events.find(event => {
    if (event.type !== 'item.completed') return false;
    const item = event.item as Record<string, unknown> | undefined;
    return item?.type === 'command_execution';
  })?.item as Record<string, unknown> | undefined;
  const hasError = events.some(event => event.type === 'nested_error') || completedCommand?.status === 'failed';
  const hasDone = events.some(event => event.type === 'nested_done') || !!completedCommand;
  const status: CommandLifecycleStatus = hasError ? 'failed' : hasDone ? 'succeeded' : 'running';
  const sessionId = 'nested-agent-console-session';
  const runId = 'nested-agent-console-run';
  const blockId = 'nested-agent-console-block';
  const command =
    (startedCommand?.command as string | undefined) ||
    (startedCommand?.name as string | undefined) ||
    (completedCommand?.command as string | undefined) ||
    title;
  const origin: CommandOrigin = { kind: 'agent', label: 'sub-agent' };
  const metadata = { source: 'nested_agent' };
  let sequence = 1;
  const session: CommandSession = {
    id: sessionId,
    origin,
    status: status === 'running' ? 'running' : 'closed',
    created_at: timestamp,
    closed_at: status === 'running' ? null : finishedAt,
    metadata,
  };
  const run = {
    id: runId,
    session_id: sessionId,
    origin,
    status,
    started_at: timestamp,
    finished_at: status === 'running' ? null : finishedAt,
    command,
    cwd: null,
    exit_code: status === 'failed' ? 1 : status === 'succeeded' ? 0 : null,
    metadata,
  };
  const block: CommandBlock = {
    id: blockId,
    session_id: sessionId,
    run_id: runId,
    origin,
    status,
    started_at: timestamp,
    finished_at: run.finished_at,
    title,
    command,
    exit_code: run.exit_code,
    metadata,
  };
  const consoleEvents: CommandEvent[] = [
    {
      event_id: `${sessionId}-session-started`,
      kind: 'session_started',
      session_id: sessionId,
      run_id: runId,
      block_id: blockId,
      sequence: sequence++,
      timestamp,
      session,
    },
    {
      event_id: `${runId}-command-started`,
      kind: 'command_started',
      session_id: sessionId,
      run_id: runId,
      block_id: blockId,
      sequence: sequence++,
      timestamp,
      session,
      run,
      block,
    },
  ];

  for (const { event, index } of outputEvents) {
    const text = (event.delta as string) || '';
    const chunkSequence = sequence++;
    consoleEvents.push({
      event_id: `${runId}-output-${index}`,
      kind: 'output_chunk',
      session_id: sessionId,
      run_id: runId,
      block_id: blockId,
      sequence: chunkSequence,
      timestamp: timestampFromEvent(event) || timestamp,
      chunk: {
        id: `${runId}-chunk-${index}`,
        session_id: sessionId,
        run_id: runId,
        block_id: blockId,
        sequence: chunkSequence,
        stream: hasError ? 'stderr' : 'stdout',
        text,
        received_at: timestampFromEvent(event) || timestamp,
        byte_len: text.length,
        metadata,
      },
    });
  }

  if (status !== 'running') {
    consoleEvents.push({
      event_id: `${runId}-command-finished`,
      kind: 'command_finished',
      session_id: sessionId,
      run_id: runId,
      block_id: blockId,
      sequence: sequence++,
      timestamp: finishedAt,
      session,
      run,
      block,
    });
  }

  return consoleEvents;
}

function upsertBlock(
  blocks: Map<string, MutableBlock>,
  sessions: Map<string, CommandSession>,
  id: string,
  sequence: number,
  data: {
    block?: CommandBlock;
    run?: CommandEvent['run'];
    chunk?: CommandOutputChunk;
    inputRequest?: CommandInputRequest;
    sessionId?: string;
  },
): MutableBlock {
  const sessionId =
    data.block?.session_id || data.run?.session_id || data.chunk?.session_id || data.inputRequest?.session_id || data.sessionId || id;
  const session = sessions.get(sessionId);
  const existing = blocks.get(id);
  const title =
    data.block?.title ||
    data.block?.command ||
    data.run?.command ||
    data.chunk?.metadata?.step ||
    data.chunk?.metadata?.source ||
    session?.origin.label ||
    session?.origin.kind ||
    'Command';

  if (!existing) {
    const metadata = data.block?.metadata ?? data.run?.metadata ?? data.chunk?.metadata ?? data.inputRequest?.metadata ?? session?.metadata ?? {};
    const created: MutableBlock = {
      id,
      sessionId,
      runId: data.block?.run_id || data.run?.id || data.chunk?.run_id || data.inputRequest?.run_id,
      title,
      command: data.block?.command || data.run?.command || title,
      cwd: data.block?.metadata?.cwd || data.run?.cwd || session?.cwd,
      status: data.block?.status || data.run?.status || session?.status || 'running',
      exitCode: data.block?.exit_code ?? data.run?.exit_code,
      source: describeSource(data.block?.origin || data.run?.origin || session?.origin, metadata),
      startedAt: data.block?.started_at || data.run?.started_at || session?.created_at,
      finishedAt: data.block?.finished_at || data.run?.finished_at || session?.closed_at,
      metadata,
      lines: [],
      inputRequests: [],
      error: null,
      sortSequence: sequence,
      lineIds: new Set(),
      inputRequestIds: new Set(),
    };
    blocks.set(id, created);
    return created;
  }

  existing.title = title || existing.title;
  existing.command = data.block?.command || data.run?.command || existing.command;
  existing.cwd = data.run?.cwd || session?.cwd || existing.cwd;
  existing.status = data.block?.status || data.run?.status || session?.status || existing.status;
  existing.exitCode = data.block?.exit_code ?? data.run?.exit_code ?? existing.exitCode;
  existing.startedAt = data.block?.started_at || data.run?.started_at || existing.startedAt;
  existing.finishedAt = data.block?.finished_at || data.run?.finished_at || session?.closed_at || existing.finishedAt;
  existing.metadata = data.block?.metadata ?? data.run?.metadata ?? existing.metadata;
  existing.source = describeSource(data.block?.origin || data.run?.origin || session?.origin, existing.metadata);
  return existing;
}

function toolStatusToCommandStatus(status: ToolCall['status']): CommandLifecycleStatus {
  switch (status) {
    case 'pending':
      return 'pending';
    case 'running':
      return 'running';
    case 'error':
      return 'failed';
    case 'done':
      return 'succeeded';
    default:
      return 'unknown';
  }
}

function commandTextFromTool(tool: ToolCall): string {
  const input = tool.input ?? {};
  if (typeof input.command === 'string' && input.command.trim()) return input.command;
  if (typeof input.cmd === 'string' && input.cmd.trim()) return input.cmd;
  if (Array.isArray(input.commands)) return `${input.action || 'multi'} (${input.commands.length} commands)`;
  if (typeof input.action === 'string' && input.action.trim()) return `${tool.name} ${input.action}`;
  return tool.name;
}

function metadataFromUnknown(values: Record<string, unknown>): Record<string, string> {
  const metadata: Record<string, string> = {};
  for (const [key, value] of Object.entries(values)) {
    if (value === undefined || value === null) continue;
    if (typeof value === 'string') metadata[key] = value;
    else if (typeof value === 'number' || typeof value === 'boolean') metadata[key] = String(value);
  }
  return metadata;
}

function stableSyntheticId(prefix: string, raw: string): string {
  return `${prefix}-${raw.replace(/[^a-zA-Z0-9_-]+/g, '-').slice(0, 80) || 'unknown'}`;
}

function timestampFromEvent(event: NestedAgentEvent | undefined): string | null {
  if (!event) return null;
  const timestamp = event.timestamp ?? event.created_at ?? event.time;
  return typeof timestamp === 'string' && timestamp ? timestamp : null;
}

function describeSource(
  origin: CommandSession['origin'] | undefined,
  metadata: Record<string, string>,
): string {
  if (origin?.label) return origin.label;
  if (metadata.step && metadata.source) return `${metadata.source}:${metadata.step}`;
  if (metadata.source) return metadata.source;
  if (origin?.kind === 'mcp_shell') return 'shell';
  if (origin?.kind === 'pty') return 'terminal';
  if (origin?.kind === 'deploy') return 'deploy';
  return origin?.kind || 'command';
}
