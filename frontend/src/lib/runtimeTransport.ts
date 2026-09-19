import { invoke } from "@tauri-apps/api/core";
import { getApiBase } from "../stores/connectionStore";

export const LOCAL_BASE = "http://flamel.local.invalid";
export function isNativeLocal(): boolean {
  return (
    getApiBase() === LOCAL_BASE ||
    (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window)
  );
}
const CHUNK = 64 * 1024;
interface Head {
  status: number;
  headers: Record<string, string>;
  streamId: string;
}

/** Pull-based IPC keeps downloads and live streams bounded by consumer demand. */
export async function nativeFetch(
  path: string,
  options: RequestInit = {},
): Promise<Response> {
  if (options.signal?.aborted) throw new DOMException("Aborted", "AbortError");
  const headers = new Headers(options.headers);
  // Request provides FormData's multipart boundary and handles all standard body types.
  const request = new Request(LOCAL_BASE + path, { ...options, headers });
  const id = await invoke<string>("runtime_open", {
    request: {
      version: 1,
      method: request.method,
      path,
      headers: Object.fromEntries(request.headers),
    },
  });
  let closed = false;
  const cancel = async () => {
    if (closed) return;
    closed = true;
    options.signal?.removeEventListener("abort", onAbort);
    await invoke("runtime_cancel", { id });
  };
  let controller: ReadableStreamDefaultController<Uint8Array> | undefined;
  const onAbort = () => {
    controller?.error(new DOMException("Aborted", "AbortError"));
    void cancel().catch(() => {});
  };
  options.signal?.addEventListener("abort", onAbort, { once: true });
  if (options.signal?.aborted) onAbort();
  const upload = async () => {
    const reader = request.body?.getReader();
    try {
      if (reader)
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          for (let offset = 0; offset < value.length; offset += CHUNK) {
            await invoke("runtime_write", {
              id,
              chunk: Array.from(value.subarray(offset, offset + CHUNK)),
              done: false,
            });
          }
        }
      await invoke("runtime_write", { id, chunk: [], done: true });
    } finally {
      reader?.releaseLock();
    }
  };
  try {
    const [head] = await Promise.all([
      invoke<Head>("runtime_head", { id }),
      upload(),
    ]);
    if (options.signal?.aborted) throw new DOMException("Aborted", "AbortError");
    const body = new ReadableStream<Uint8Array>({
      start(value) {
        controller = value;
      },
      async pull(value) {
        try {
          const chunk = await invoke<number[] | null>("runtime_read", { id });
          if (closed) return;
          if (chunk === null) {
            value.close();
            await cancel();
          } else value.enqueue(Uint8Array.from(chunk));
        } catch (error) {
          if (!closed) value.error(error);
          await cancel();
        }
      },
      cancel,
    });
    if ([204, 205, 304].includes(head.status) || request.method === "HEAD") {
      await body.cancel();
      return new Response(null, { status: head.status, headers: head.headers });
    }
    return new Response(body, { status: head.status, headers: head.headers });
  } catch (error) {
    await cancel().catch(() => {});
    if (options.signal?.aborted) throw new DOMException("Aborted", "AbortError");
    throw error;
  }
}

export type RuntimeSocket = Pick<
  WebSocket,
  | "readyState"
  | "bufferedAmount"
  | "onopen"
  | "onclose"
  | "onmessage"
  | "onerror"
  | "send"
  | "close"
>;
class NativeSocket implements RuntimeSocket {
  readyState: number = WebSocket.CONNECTING;
  bufferedAmount = 0;
  onopen: WebSocket["onopen"] = null;
  onclose: WebSocket["onclose"] = null;
  onmessage: WebSocket["onmessage"] = null;
  onerror: WebSocket["onerror"] = null;
  private id?: string;
  private queue = Promise.resolve();
  constructor(path: string) {
    void this.connect(path);
  }
  private async connect(path: string) {
    try {
      this.id = await invoke<string>("runtime_socket_open", { path });
      if (this.readyState === WebSocket.CLOSED) {
        await invoke("runtime_cancel", { id: this.id });
        return;
      }
      this.readyState = WebSocket.OPEN;
      this.onopen?.call(this as unknown as WebSocket, new Event("open"));
      const decoder = new TextDecoder();
      let pending = "";
      while (this.readyState === WebSocket.OPEN) {
        const chunk = await invoke<number[] | null>("runtime_read", {
          id: this.id,
        });
        if (chunk === null) break;
        pending += decoder.decode(Uint8Array.from(chunk), { stream: true });
        if (pending.length > 50 * 1024 * 1024)
          throw new Error("Socket message exceeds limit");
        let newline: number;
        while ((newline = pending.indexOf("\n")) >= 0) {
          const data = pending.slice(0, newline);
          pending = pending.slice(newline + 1);
          this.onmessage?.call(
            this as unknown as WebSocket,
            new MessageEvent("message", { data }),
          );
        }
      }
    } catch {
      if (this.readyState !== WebSocket.CLOSED)
        this.onerror?.call(this as unknown as WebSocket, new Event("error"));
    } finally {
      this.close();
    }
  }
  send(data: string | ArrayBufferLike | Blob | ArrayBufferView) {
    if (this.readyState !== WebSocket.OPEN || typeof data !== "string")
      throw new Error("Native socket is not ready for a text message");
    const size = new TextEncoder().encode(data).length;
    if (this.bufferedAmount + size > 1024 * 1024)
      throw new Error("Socket send queue is full");
    this.bufferedAmount += size;
    this.queue = this.queue
      .then(() =>
        invoke<void>("runtime_socket_send", { id: this.id, message: data }),
      )
      .catch(() => this.close())
      .finally(() => {
        this.bufferedAmount -= size;
      });
  }
  close() {
    if (this.readyState === WebSocket.CLOSED) return;
    this.readyState = WebSocket.CLOSED;
    if (this.id) void invoke("runtime_cancel", { id: this.id });
    this.onclose?.call(this as unknown as WebSocket, new CloseEvent("close"));
  }
}
export function runtimeSocket(url: string | URL): RuntimeSocket {
  const parsed = new URL(url);
  return isNativeLocal()
    ? new NativeSocket(parsed.pathname + parsed.search)
    : new WebSocket(url);
}
