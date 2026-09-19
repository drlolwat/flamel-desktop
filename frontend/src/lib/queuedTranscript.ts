import type { DisplayBlock } from "./messageProcessor";

export type VisibleQueuedMessage = {
  id: string;
  message: string;
  workingDirectory: string;
  createdAt: number;
  imageCount: number;
  serverOwned: boolean;
  interruptActive: boolean;
  admissionState?: "pending" | "accepted" | "failed";
};

type QueuedMessageSource = {
  id: string;
  message: string;
  workingDirectory: string;
  createdAt: number;
  options: {
    images?: unknown[];
    __isContinuation?: boolean;
    __interruptActive?: boolean;
  };
  serverOwned?: boolean;
  admissionState?: "pending" | "accepted" | "failed";
};

export function visibleQueuedMessages(
  queue: readonly QueuedMessageSource[],
): VisibleQueuedMessage[] {
  return queue
    .filter((item) => !item.options.__isContinuation)
    .map((item) => ({
      id: item.id,
      message: item.message,
      workingDirectory: item.workingDirectory,
      createdAt: item.createdAt,
      imageCount: item.options.images?.length ?? 0,
      serverOwned: item.serverOwned === true,
      interruptActive: item.options.__interruptActive === true,
      admissionState: item.admissionState,
    }));
}

export function reconcileQueuedOptimisticBlocks(
  blocks: DisplayBlock[],
  optimisticBlockIds: ReadonlySet<string>,
  interruptActive: boolean,
  queuedRequestId: string,
): DisplayBlock[] {
  return blocks.flatMap(block => {
    if (!optimisticBlockIds.has(block.id)) return [block];
    if (!interruptActive || block.role !== "user") return [];
    return [{ ...block, requestId: queuedRequestId }];
  });
}

export function hasQueuedUserBlock(
  blocks: DisplayBlock[],
  requestId: string | undefined,
  message: string,
): boolean {
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    const block = blocks[index];
    if (block.role !== "user") continue;
    if (requestId && block.requestId === requestId) return true;
    // Older clients did not tag optimistic user blocks with request IDs. Only
    // use the most recent visible user message as the compatibility fallback.
    return block.content === message;
  }
  return false;
}
