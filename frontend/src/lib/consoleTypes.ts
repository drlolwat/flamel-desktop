export type CommandSourceKind =
  | 'mcp_shell'
  | 'runner'
  | 'deploy'
  | 'provider'
  | 'agent'
  | 'pty'
  | 'system';

export type CommandStreamKind = 'stdout' | 'stderr' | 'system' | 'stdin' | 'pty';

export type CommandLifecycleStatus =
  | 'unknown'
  | 'pending'
  | 'running'
  | 'waiting_for_input'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | 'timed_out'
  | 'closed';

export type CommandEventKind =
  | 'session_started'
  | 'command_started'
  | 'output_chunk'
  | 'input_requested'
  | 'input_sent'
  | 'command_finished'
  | 'session_closed'
  | 'error';

export interface CommandOrigin {
  kind: CommandSourceKind;
  runner_id?: string | null;
  task_id?: string | null;
  tool_call_id?: string | null;
  agent_run_id?: string | null;
  label?: string | null;
}

export interface CommandSession {
  id: string;
  origin: CommandOrigin;
  status: CommandLifecycleStatus;
  created_at: string;
  closed_at?: string | null;
  cwd?: string | null;
  metadata?: Record<string, string>;
}

export interface CommandRun {
  id: string;
  session_id: string;
  origin: CommandOrigin;
  status: CommandLifecycleStatus;
  started_at: string;
  finished_at?: string | null;
  command?: string | null;
  cwd?: string | null;
  exit_code?: number | null;
  metadata?: Record<string, string>;
}

export interface CommandBlock {
  id: string;
  session_id: string;
  run_id: string;
  origin: CommandOrigin;
  status: CommandLifecycleStatus;
  started_at: string;
  finished_at?: string | null;
  title?: string | null;
  command?: string | null;
  exit_code?: number | null;
  metadata?: Record<string, string>;
}

export interface CommandOutputChunk {
  id: string;
  session_id: string;
  run_id: string;
  block_id: string;
  sequence: number;
  stream: CommandStreamKind;
  text: string;
  received_at: string;
  byte_len?: number | null;
  metadata?: Record<string, string>;
}

export interface CommandInputRequest {
  id: string;
  session_id: string;
  run_id: string;
  block_id: string;
  requested_at: string;
  prompt?: string | null;
  secret?: boolean;
  metadata?: Record<string, string>;
}

export interface CommandEvent {
  event_id: string;
  kind: CommandEventKind;
  session_id: string;
  run_id?: string | null;
  block_id?: string | null;
  sequence: number;
  timestamp: string;
  session?: CommandSession | null;
  run?: CommandRun | null;
  block?: CommandBlock | null;
  chunk?: CommandOutputChunk | null;
  input_request?: CommandInputRequest | null;
  error?: string | null;
}
