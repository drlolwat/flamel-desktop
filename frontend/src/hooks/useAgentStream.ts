import { useEffect, useRef, useState } from 'react';
import { apiFetch } from '../lib/api';

export interface AgentRun {
  id: string;
  agentConfigId: string;
  status: 'pending' | 'running' | 'done' | 'failed';
  triggerPayload: Record<string, unknown>;
  prUrl?: string;
  error?: string;
  createdAt: string;
  updatedAt: string;
  startedAt?: string;
  finishedAt?: string;
}

export interface AgentRunWithOutput extends AgentRun {
  outputLines: string[];
  currentTool?: string;
}

/** Global agent run state from the SSE stream. */
export function useAgentStream() {
  const [runs, setRuns] = useState<Record<string, AgentRunWithOutput>>({});
  const readerRef = useRef<ReadableStreamDefaultReader<string> | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function connect() {
      try {
        const res = await apiFetch('/api/agent-runs/stream', {
          credentials: 'include',
          headers: { Accept: 'application/x-ndjson' },
        });
        if (!res.ok || !res.body) return;

        const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
        readerRef.current = reader;

        let buf = '';
        while (!cancelled) {
          const { done, value } = await reader.read();
          if (done) break;
          buf += value;
          const lines = buf.split('\n');
          buf = lines.pop() ?? '';
          for (const line of lines) {
            const trimmed = line.trim();
            if (!trimmed) continue;
            try {
              const event = JSON.parse(trimmed);
              handleEvent(event);
            } catch {
              // ignore malformed JSON
            }
          }
        }
      } catch {
        if (!cancelled) {
          // reconnect after 5s on error
          setTimeout(connect, 5000);
        }
      }
    }

    function handleEvent(event: Record<string, unknown>) {
      const type = event.type as string;

      if (type === 'initial') {
        const initialRuns = (event.runs as AgentRun[]) ?? [];
        setRuns(prev => {
          const next = { ...prev };
          for (const run of initialRuns) {
            next[run.id] = { ...run, outputLines: next[run.id]?.outputLines ?? [] };
          }
          return next;
        });
        return;
      }

      const runId = event.runId as string;
      if (!runId) return;

      if (type === 'agent_status') {
        const status = event.status as AgentRun['status'];
        const error = event.error as string | undefined;
        const prUrl = event.prUrl as string | undefined;
        setRuns(prev => ({
          ...prev,
          [runId]: {
            ...(prev[runId] ?? { id: runId, agentConfigId: '', triggerPayload: {}, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString(), outputLines: [] }),
            status,
            ...(error ? { error } : {}),
            ...(prUrl ? { prUrl } : {}),
          },
        }));
      } else if (type === 'agent_output') {
        const line = event.line as string;
        setRuns(prev => {
          const existing = prev[runId];
          if (!existing) return prev;
          const outputLines = [...(existing.outputLines ?? []), line].slice(-100); // keep last 100 lines
          return { ...prev, [runId]: { ...existing, outputLines } };
        });
      } else if (type === 'agent_tool') {
        const tool = event.tool as string;
        setRuns(prev => {
          const existing = prev[runId];
          if (!existing) return prev;
          return { ...prev, [runId]: { ...existing, currentTool: tool } };
        });
      }
    }

    connect();

    return () => {
      cancelled = true;
      readerRef.current?.cancel().catch(() => {});
    };
  }, []);

  return runs;
}
