import { beforeEach, afterEach, describe, it, expect, vi } from "vitest";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("../stores/connectionStore", () => ({
  getApiBase: () => "http://flamel.local.invalid",
}));
import { nativeFetch } from "./runtimeTransport";
const utf8 = new TextEncoder();
beforeEach(() => {
  invoke.mockReset();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
function responder(
  status = 200,
  chunks: (number[] | null)[] = [Array.from(utf8.encode("hello")), null],
) {
  invoke.mockImplementation(async (command: string) => {
    if (command === "runtime_open") return "request-1";
    if (command === "runtime_head")
      return {
        status,
        headers: { "content-type": "text/plain" },
        streamId: "request-1",
      };
    if (command === "runtime_read") return chunks.shift() ?? null;
  });
}
describe("native transport", () => {
  it("streams response bytes and releases native resources at EOF", async () => {
    responder();
    const response = await nativeFetch("/api/me");
    expect(await response.text()).toBe("hello");
    expect(invoke).toHaveBeenCalledWith("runtime_cancel", { id: "request-1" });
    expect(invoke).toHaveBeenCalledWith("runtime_open", {
      request: { version: 1, method: "GET", path: "/api/me", headers: {} },
    });
  });
  it("chunks uploads at 64 KiB and preserves binary bytes", async () => {
    responder();
    const input = new Uint8Array(150_000).fill(201);
    const response = await nativeFetch("/api/upload", {
      method: "POST",
      body: input,
    });
    await response.text();
    const writes = invoke.mock.calls
      .filter(([command, args]) => command === "runtime_write" && !args.done)
      .map(([, args]) => args.chunk as number[]);
    expect(writes.map((chunk) => chunk.length)).toEqual([65536, 65536, 18928]);
    expect(writes.flat()).toEqual(Array.from(input));
  });
  it("does not fall back to a local HTTP endpoint when IPC fails", async () => {
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    invoke.mockRejectedValue(new Error("IPC unavailable"));
    await expect(nativeFetch("/api/me")).rejects.toThrow("IPC unavailable");
    expect(fetch).not.toHaveBeenCalled();
  });
  it("cancels a partially consumed stream", async () => {
    responder();
    const response = await nativeFetch("/api/files");
    await response.body!.cancel();
    expect(invoke).toHaveBeenCalledWith("runtime_cancel", { id: "request-1" });
  });
  it("rejects an already cancelled request before admission", async () => {
    const abort = new AbortController();
    abort.abort();
    await expect(
      nativeFetch("/api/chat", { signal: abort.signal }),
    ).rejects.toMatchObject({ name: "AbortError" });
    expect(invoke).not.toHaveBeenCalled();
  });
  it("handles bodyless status codes", async () => {
    responder(204);
    const response = await nativeFetch("/api/session", { method: "DELETE" });
    expect(response.status).toBe(204);
    expect(response.body).toBeNull();
  });
  it("reports AbortError when cancelled while waiting for response headers", async () => {
    const abort = new AbortController();
    let rejectHead: (reason: unknown) => void;
    invoke.mockImplementation(async (command: string) => {
      if (command === "runtime_open") return "request-1";
      if (command === "runtime_head") {
        return new Promise((_resolve, reject) => {
          rejectHead = reject;
          queueMicrotask(() => abort.abort());
        });
      }
      if (command === "runtime_cancel") rejectHead("Cancelled");
    });
    await expect(nativeFetch("/api/chat", { signal: abort.signal }))
      .rejects.toMatchObject({ name: "AbortError" });
    expect(invoke).toHaveBeenCalledWith("runtime_cancel", { id: "request-1" });
  });
});
