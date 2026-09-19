import { afterEach, describe, expect, it, vi } from "vitest";
import { createStreamBatcher, scheduleStreamFlush, cancelStreamFlush } from "./streamBatcher";

describe("stream batching", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("coalesces items until the scheduled flush fires", () => {
    vi.useFakeTimers();
    const batches: number[][] = [];
    const batcher = createStreamBatcher<number>((items) => batches.push(items), { delayMs: 33 });

    batcher.push(1);
    batcher.push(2);
    expect(batches).toEqual([]);

    vi.advanceTimersByTime(33);
    expect(batches).toEqual([[1, 2]]);
    expect(batcher.pendingCount()).toBe(0);
  });

  it("flushes immediately once the max batch size is reached", () => {
    vi.useFakeTimers();
    const batches: number[][] = [];
    const batcher = createStreamBatcher<number>((items) => batches.push(items), { delayMs: 1000, maxBatchSize: 3 });

    batcher.pushMany([1, 2]);
    expect(batches).toEqual([]);
    batcher.push(3);

    expect(batches).toEqual([[1, 2, 3]]);
    expect(batcher.pendingCount()).toBe(0);
  });

  it("can cancel scheduled work", () => {
    vi.useFakeTimers();
    const batches: number[][] = [];
    const batcher = createStreamBatcher<number>((items) => batches.push(items), { delayMs: 33 });

    batcher.push(1);
    batcher.cancel();
    vi.advanceTimersByTime(33);

    expect(batches).toEqual([]);
    expect(batcher.pendingCount()).toBe(0);
  });

  it("reuses an existing flush handle", () => {
    vi.useFakeTimers();
    const flush = vi.fn();
    let handle = scheduleStreamFlush(null, flush, 33);
    const sameHandle = scheduleStreamFlush(handle, flush, 33);

    expect(sameHandle).toBe(handle);
    vi.advanceTimersByTime(33);
    expect(flush).toHaveBeenCalledTimes(1);

    handle = cancelStreamFlush(sameHandle);
    expect(handle).toBeNull();
  });
});
