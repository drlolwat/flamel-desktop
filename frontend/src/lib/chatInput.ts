import type { GuidanceAnchor } from "./guidanceTranscript";

export type InputDelivery = {
  requestId: string;
  sessionId: string;
  message: string;
  delivery: "saving" | "waiting" | "sent" | "queued" | "uncertain" | "failed";
  note?: string;
  updatedAt?: string;
  /** Durable evidence that this queued input has started its own run. */
  runStartedAt?: string;
  /** Client-only position; never sent as provider input or execution settings. */
  transcriptAnchor?: GuidanceAnchor;
};

export function isAcceptedDuplicate(value: unknown, requestId: string, sessionId: string): boolean {
  if (!value || typeof value !== "object") return false;
  const receipt = value as Record<string, unknown>;
  return receipt.accepted === true && receipt.code === "request_already_accepted"
    && receipt.requestId === requestId && receipt.sessionId === sessionId;
}

export function parseInputDelivery(value: unknown, sessionId: string): InputDelivery | undefined {
  if (!value || typeof value !== "object") return undefined;
  const input = value as Record<string, unknown>;
  if (input.type !== "input_delivery" || input.accepted !== true || input.sessionId !== sessionId
    || typeof input.requestId !== "string" || typeof input.message !== "string"
    || !["waiting", "sent", "queued", "uncertain", "failed"].includes(String(input.delivery))) return undefined;
  return {
    requestId: input.requestId, sessionId, message: input.message,
    delivery: input.delivery as InputDelivery["delivery"],
    note: typeof input.note === "string" ? input.note : undefined,
    runStartedAt: typeof input.runStartedAt === "string" ? input.runStartedAt : undefined,
    updatedAt: typeof input.updatedAt === "string" ? input.updatedAt : undefined,
  };
}

export function mergeInputDelivery(receipts: InputDelivery[], incoming: InputDelivery, onlyIfMissing = false): InputDelivery[] {
  const index = receipts.findIndex((item) => item.requestId === incoming.requestId);
  if (index < 0) return (onlyIfMissing ? [incoming, ...receipts] : [...receipts, incoming]).slice(-100);
  const existing = receipts[index];
  // Starting a run is irreversible evidence, even if its admission HTTP
  // response was timestamped slightly later than the run's start.
  const withRunProgress = incoming.runStartedAt && !existing.runStartedAt
    ? receipts.map((item, i) => i === index ? { ...item, runStartedAt: incoming.runStartedAt } : item)
    : receipts;
  if (existing.updatedAt && incoming.updatedAt && existing.updatedAt > incoming.updatedAt) return withRunProgress;
  // A hydration response can arrive after a live event. Only a newer server
  // receipt may replace it; local saving/uncertain receipts have no timestamp.
  if (onlyIfMissing && (!incoming.updatedAt || (existing.updatedAt && existing.updatedAt >= incoming.updatedAt))) return withRunProgress;
  if (["sent", "queued", "failed"].includes(existing.delivery) && ["saving", "waiting", "uncertain"].includes(incoming.delivery)) return receipts;
  if (JSON.stringify(existing) === JSON.stringify(incoming)) return receipts;
  return receipts.map((item, i) => i === index ? {
    ...incoming, runStartedAt: existing.runStartedAt ?? incoming.runStartedAt, transcriptAnchor: existing.transcriptAnchor ?? incoming.transcriptAnchor,
  } : item);
}
