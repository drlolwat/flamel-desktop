export const DEFAULT_STREAM_FLUSH_MS = 33;

export type StreamFlushHandle = ReturnType<typeof setTimeout> | null;

export interface StreamBatcher<T> {
  push: (item: T) => void;
  pushMany: (items: T[]) => void;
  flush: () => void;
  cancel: () => void;
  pendingCount: () => number;
}

interface StreamBatcherOptions {
  delayMs?: number;
  maxBatchSize?: number;
}

export function scheduleStreamFlush(
  handle: StreamFlushHandle,
  flush: () => void,
  delayMs = DEFAULT_STREAM_FLUSH_MS,
): StreamFlushHandle {
  return handle ?? setTimeout(flush, delayMs);
}

export function cancelStreamFlush(handle: StreamFlushHandle): null {
  if (handle) clearTimeout(handle);
  return null;
}

export function createStreamBatcher<T>(
  onFlush: (items: T[]) => void,
  options: StreamBatcherOptions = {},
): StreamBatcher<T> {
  const delayMs = options.delayMs ?? DEFAULT_STREAM_FLUSH_MS;
  const maxBatchSize = Math.max(1, options.maxBatchSize ?? 128);
  let queued: T[] = [];
  let flushHandle: StreamFlushHandle = null;

  const flush = () => {
    flushHandle = cancelStreamFlush(flushHandle);
    if (queued.length === 0) return;
    const items = queued;
    queued = [];
    onFlush(items);
  };

  const schedule = () => {
    flushHandle = scheduleStreamFlush(flushHandle, flush, delayMs);
  };

  return {
    push(item) {
      queued.push(item);
      if (queued.length >= maxBatchSize) flush();
      else schedule();
    },
    pushMany(items) {
      if (items.length === 0) return;
      queued.push(...items);
      if (queued.length >= maxBatchSize) flush();
      else schedule();
    },
    flush,
    cancel() {
      flushHandle = cancelStreamFlush(flushHandle);
      queued = [];
    },
    pendingCount() {
      return queued.length;
    },
  };
}
