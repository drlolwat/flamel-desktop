import { describe, expect, it } from "vitest";
import { isAcceptedDuplicate, mergeInputDelivery, parseInputDelivery } from "./chatInput";

const receipt = { type: "input_delivery", accepted: true, requestId: "input-1", sessionId: "session-1", message: "Focus on tests", delivery: "sent" };

describe("chat input receipts", () => {
  it("accepts only a persisted duplicate matching both identities", () => {
    const duplicate = { ...receipt, code: "request_already_accepted" };
    expect(isAcceptedDuplicate(duplicate, "input-1", "session-1")).toBe(true);
    for (const value of [{}, { ...duplicate, accepted: false }, { ...duplicate, code: "request_id_conflict" }, { ...duplicate, sessionId: "other" }]) {
      expect(isAcceptedDuplicate(value, "input-1", "session-1")).toBe(false);
    }
  });

  it("keeps guidance separate from assistant blocks and rejects another session", () => {
    expect(parseInputDelivery(receipt, "other")).toBeUndefined();
    const parsed = parseInputDelivery(receipt, "session-1")!;
    expect(parsed.message).toBe("Focus on tests");
    const receipts = [parsed];
    expect(mergeInputDelivery(receipts, { ...parsed, delivery: "waiting" })).toBe(receipts);
    expect(mergeInputDelivery(receipts, { ...parsed, delivery: "uncertain" }, true)).toBe(receipts);
    expect(mergeInputDelivery(receipts, parsed)).toBe(receipts);
  });

  it("reconciles uncertain delivery without letting stale hydration replace live receipts", () => {
    const parsed = parseInputDelivery(receipt, "session-1")!;
    const uncertain = [{ ...parsed, delivery: "uncertain" as const }];
    const confirmed = { ...parsed, updatedAt: "2026-09-05T10:00:02Z" };
    expect(mergeInputDelivery(uncertain, confirmed, true)).toEqual([confirmed]);
    const current = [confirmed];
    expect(mergeInputDelivery(current, { ...confirmed, delivery: "failed", updatedAt: "2026-09-05T10:00:01Z" }, true)).toBe(current);
    expect(mergeInputDelivery(current, { ...confirmed, delivery: "waiting" })).toBe(current);
  });
});

it("keeps durable queued-run progress across stale admission responses", () => {
  const queued = parseInputDelivery({ ...receipt, delivery: "queued", updatedAt: "2026-09-08T01:00:00Z" }, "session-1")!;
  const started = parseInputDelivery({ ...receipt, delivery: "queued", runStartedAt: "2026-09-08T02:00:00Z", updatedAt: "2026-09-08T02:00:00Z" }, "session-1")!;
  const lateAdmission = { ...queued, updatedAt: "2026-09-08T02:00:01Z" };
  expect(mergeInputDelivery([lateAdmission], started, true)[0].runStartedAt).toBe(started.runStartedAt);
  const current = mergeInputDelivery([queued], started, true);
  expect(current[0].runStartedAt).toBe(started.runStartedAt);
  expect(mergeInputDelivery(current, queued)).toBe(current);
  expect(mergeInputDelivery(current, { ...queued, updatedAt: undefined })[0].runStartedAt).toBe(started.runStartedAt);
});
