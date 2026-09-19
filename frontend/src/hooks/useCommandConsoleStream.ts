import { useCallback, useEffect, useRef, useState } from 'react';
import { apiFetch } from '../lib/api';
import type { CommandEvent } from '../lib/consoleTypes';
import { createStreamBatcher, type StreamBatcher } from '../lib/streamBatcher';

export interface UseCommandConsoleStreamOptions {
  sessionId?: string | null;
  runnerId?: string | null;
  maxEvents?: number;
}

export function commandConsoleStreamPath(
  source: Pick<UseCommandConsoleStreamOptions, 'sessionId' | 'runnerId'>,
  afterSequence = 0,
  epoch?: string,
): string {
  const base = source.sessionId
    ? `/api/console/sessions/${encodeURIComponent(source.sessionId)}/stream`
    : `/api/console/runners/${encodeURIComponent(source.runnerId!)}/stream`;
  const params = new URLSearchParams();
  if (afterSequence > 0) params.set('afterSequence', String(afterSequence));
  if (epoch) params.set('epoch', epoch);
  return params.size ? `${base}?${params}` : base;
}

export function parseCommandEventLine(line: string): CommandEvent | null {
  const trimmed = line.trim();
  if (!trimmed) return null;
  const payload = JSON.parse(trimmed);
  if (payload.type !== 'command_event' || !payload.event) return null;
  return payload.event as CommandEvent;
}

export function appendCommandEvents(
  events: CommandEvent[],
  incoming: CommandEvent[],
  maxEvents: number,
): CommandEvent[] {
  if (incoming.length === 0) return events;
  const byId = new Map(events.map(event => [event.event_id, event]));
  let changed = false;
  for (const event of incoming) {
    if (byId.has(event.event_id)) continue;
    byId.set(event.event_id, event);
    changed = true;
  }
  if (!changed) return events;
  return Array.from(byId.values()).sort((a, b) => a.sequence - b.sequence).slice(-maxEvents);
}

export function appendCommandEvent(
  events: CommandEvent[],
  event: CommandEvent,
  maxEvents: number,
): CommandEvent[] {
  return appendCommandEvents(events, [event], maxEvents);
}

export function useCommandConsoleStream({
  sessionId,
  runnerId,
  maxEvents = 2000,
}: UseCommandConsoleStreamOptions) {
  const [events, setEvents] = useState<CommandEvent[]>([]);
  const [connected, setConnected] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const readerRef = useRef<ReadableStreamDefaultReader<string> | null>(null);
  const reconnectTimerRef = useRef<number | null>(null);
  const eventBatcherRef = useRef<StreamBatcher<CommandEvent> | null>(null);
  const lastSequenceRef = useRef(0);
  const epochRef = useRef<string | undefined>(undefined);
  const sourceKey = sessionId ? `session:${sessionId}` : runnerId ? `runner:${runnerId}` : '';

  const clear = useCallback(() => {
    eventBatcherRef.current?.cancel();
    setEvents([]);
  }, []);

  useEffect(() => {
    if (!sourceKey) {
      eventBatcherRef.current?.cancel();
      eventBatcherRef.current = null;
      setEvents([]);
      setConnected(false);
      lastSequenceRef.current = 0;
      return;
    }

    let cancelled = false;
    let stopRetrying = false;
    const abort = new AbortController();
    lastSequenceRef.current = 0;
    epochRef.current = undefined;
    setNotice(null);
    eventBatcherRef.current?.cancel();
    const eventBatcher = createStreamBatcher<CommandEvent>(
      batch => setEvents(prev => appendCommandEvents(prev, batch, maxEvents)),
      { delayMs: 33, maxBatchSize: 128 },
    );
    eventBatcherRef.current = eventBatcher;
    setEvents([]);

    function scheduleReconnect() {
      if (cancelled || stopRetrying || reconnectTimerRef.current !== null) return;
      reconnectTimerRef.current = window.setTimeout(() => {
        reconnectTimerRef.current = null;
        connect();
      }, 2000);
    }

    async function connect() {
      try {
        const res = await apiFetch(commandConsoleStreamPath({ sessionId, runnerId }, lastSequenceRef.current, epochRef.current), {
          signal: abort.signal,
          credentials: 'include',
          headers: { Accept: 'application/x-ndjson' },
        });
        if (!res.ok || !res.body) {
          setConnected(false);
          if ([401, 403, 404].includes(res.status)) {
            stopRetrying = true;
            setNotice(res.status === 404 ? 'This live console is no longer available. The server may have restarted.' : 'Sign in to view this console.');
          }
          scheduleReconnect();
          return;
        }

        setConnected(true);
        const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
        readerRef.current = reader;

        let buffer = '';
        while (!cancelled) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += value;
          const lines = buffer.split('\n');
          buffer = lines.pop() ?? '';

          for (const line of lines) {
            try {
              const control = JSON.parse(line);
              if (control.type === 'command_stream_start') {
                epochRef.current = control.epoch;
                if (control.reset) {
                  eventBatcher.cancel();
                  lastSequenceRef.current = 0;
                  setEvents([]);
                  setNotice('Server restarted. Showing output from the new runtime.');
                }
                continue;
              }
              if (control.type === 'command_gap') { setNotice(control.message || 'Some earlier output is no longer available.'); continue; }
              const event = parseCommandEventLine(line);
              if (!event) continue;
              lastSequenceRef.current = Math.max(lastSequenceRef.current, event.sequence || 0);
              eventBatcher.push(event);
            } catch {
              // Ignore malformed stream lines and keep the connection alive.
            }
          }
        }
      } catch {
        // Reconnect below unless the hook was torn down.
      } finally {
        if (!cancelled) {
          eventBatcher.flush();
          setConnected(false);
          scheduleReconnect();
        }
      }
    }

    connect();

    return () => {
      cancelled = true;
      abort.abort();
      setConnected(false);
      if (reconnectTimerRef.current !== null) {
        window.clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
      eventBatcher.cancel();
      if (eventBatcherRef.current === eventBatcher) eventBatcherRef.current = null;
      readerRef.current?.cancel().catch(() => {});
      readerRef.current = null;
    };
  }, [maxEvents, runnerId, sessionId, sourceKey]);

  return { events, connected, clear, notice };
}
