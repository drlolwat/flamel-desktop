import { describe, expect, it } from "vitest";
import { resolveExecutionSessionId } from "./sessionIdentity";

describe("resolveExecutionSessionId", () => {
  it("uses the canonical tab identity assigned while a message was queued", () => {
    expect(resolveExecutionSessionId(undefined, "server-session", undefined)).toBe(
      "server-session",
    );
  });

  it("prefers current identity over a stale queued identity", () => {
    expect(
      resolveExecutionSessionId("stale", "current-tab", "current-chat"),
    ).toBe("current-tab");
  });

  it("falls back to runtime and queued identities", () => {
    expect(resolveExecutionSessionId(undefined, undefined, "runtime")).toBe(
      "runtime",
    );
    expect(resolveExecutionSessionId("queued", undefined, undefined)).toBe(
      "queued",
    );
  });

  it("allows an explicit history rewrite to start a new session", () => {
    expect(
      resolveExecutionSessionId("queued", "tab", "chat", true),
    ).toBeUndefined();
  });
});
