import { apiFetch } from "./api";
import { normalizeWorkflowEvent, type WorkflowEvent } from "./workflowEvents";

export interface StreamCompactSessionOptions {
  queuedPrompt?: boolean;
  recoveryStrategy?: string;
  onEvent: (event: WorkflowEvent) => void;
}

export interface CompactionStreamState {
  completed: boolean;
  failed: boolean;
}

export function initialCompactionStreamState(): CompactionStreamState {
  return { completed: false, failed: false };
}

export function applyCompactionStreamLine(
  line: string,
  state: CompactionStreamState,
  onEvent: (event: WorkflowEvent) => void,
): CompactionStreamState {
  if (!line.trim()) return state;
  const event = normalizeWorkflowEvent(JSON.parse(line));
  if (!event) return state;
  onEvent(event);
  return {
    completed: state.completed || event.kind === "system.compact_complete",
    failed: state.failed || event.kind === "system.compact_failed",
  };
}

export async function streamCompactSession(
  sessionId: string,
  options: StreamCompactSessionOptions,
): Promise<boolean> {
  const response = await apiFetch(`/api/sessions/${sessionId}/compact`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      queuedPrompt: !!options.queuedPrompt,
      recoveryStrategy: options.recoveryStrategy,
    }),
  });
  if (!response.ok) throw new Error(`${response.status} ${await response.text()}`);
  if (!response.body) throw new Error("Compaction stream was empty");

  let state = initialCompactionStreamState();
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() || "";
    for (const line of lines) {
      try {
        state = applyCompactionStreamLine(line, state, options.onEvent);
      } catch {
        // Ignore malformed stream fragments; the next complete line will recover.
      }
    }
  }

  if (buffer.trim()) {
    try {
      state = applyCompactionStreamLine(buffer, state, options.onEvent);
    } catch {
      // Ignore a final malformed fragment.
    }
  }

  return state.completed && !state.failed;
}
