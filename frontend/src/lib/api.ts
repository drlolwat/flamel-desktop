import { isNativeLocal, nativeFetch } from './runtimeTransport';
import type {
  ChatRequest,
  FileNode,
  FileRevision,
  FileWriteRequest,
  GitBranchSummary,
  GitCommitSummary,
  GitDiffFile,
  GitDiffMode,
  GitOperationRequest,
  GitOperationResult,
  GitStatus,
  McpServerConfig,
  Project,
  SavedPrompt,
  SessionMeta,
  UsageStats,
} from "@flamel/shared";
import { getApiBase } from "../stores/connectionStore";
import { getAuthToken } from "../stores/authTokenStore";

export type FlamelAssistantAction =
  | "assistant.status"
  | "assistant.update_activation"
  | "assistant.reset_activation"
  | "brain.inspect"
  | "brain.remember"
  | "brain.revise"
  | "brain.forget"
  | "brain.export"
  | "capability.snapshot"
  | "automation.list"
  | "automation.history"
  | "automation.pause"
  | "automation.resume"
  | "automation.run"
  | "automation.retry"
  | "automation.delete"
  | "job.list"
  | "job.events"
  | "job.pause"
  | "job.resume"
  | "job.cancel"
  | "job.rerun"
  | "job.approve"
  | "diagnostics.status"
  | "diagnostics.recover";

export interface FlamelAssistantOperationResponse<T = unknown> {
  contractVersion: number;
  surfaceContract: "flamel-assistant.v1";
  action: FlamelAssistantAction;
  ok: boolean;
  state: string;
  approvalRequired: boolean;
  mutated: boolean;
  replayed: boolean;
  summary: {
    text: string;
    card: {
      title: string;
      action: string;
      status: string;
      approvalRequired: boolean;
    };
  };
  result?: T;
  error?: string;
}

function normalizeHeaders(headers?: HeadersInit): Record<string, string> {
  if (!headers) return {};
  if (headers instanceof Headers) {
    const out: Record<string, string> = {};
    headers.forEach((value, key) => {
      out[key] = value;
    });
    return out;
  }
  if (Array.isArray(headers)) {
    return Object.fromEntries(
      headers.map(([key, value]) => [key, String(value)]),
    );
  }
  return Object.fromEntries(
    Object.entries(headers).map(([key, value]) => [key, String(value)]),
  );
}

export function errorMessageFromUnknown(
  error: unknown,
  fallback = "Unknown error",
): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    for (const key of ["message", "error", "statusText"]) {
      const value = record[key];
      if (typeof value === "string" && value.trim()) return value;
    }
    try {
      const serialized = JSON.stringify(error);
      if (serialized && serialized !== "{}") return serialized;
    } catch {
      // Ignore serialization failures and use the fallback below.
    }
  }
  return fallback;
}

function isLoopbackHttpBase(base: string): boolean {
  try {
    const url = new URL(base);
    return (
      url.protocol === "http:" &&
      (url.hostname === "127.0.0.1" || url.hostname === "localhost")
    );
  } catch {
    return false;
  }
}

function shouldUseDesktopNdjsonBridge(
  path: string,
  base: string,
  options: RequestInit,
): boolean {
  const api = window.electronAPI;
  if (!api?.streamNdjson || !api.cancelNdjsonStream || !api.onNdjsonStream)
    return false;
  if (import.meta.env.VITE_MODE !== "local" || !isLoopbackHttpBase(base))
    return false;
  if (options.body !== undefined && typeof options.body !== "string")
    return false;
  const method = (options.method || "GET").toUpperCase();
  if (method === "POST") {
    return (
      path === "/api/chat" || /^\/api\/sessions\/[^/]+\/compact$/.test(path)
    );
  }
  if (method !== "GET") return false;
  return (
    /^\/api\/sessions\/[^/]+\/stream$/.test(path) ||
    /^\/api\/console\/sessions\/[^/]+\/stream(?:\?.*)?$/.test(path) ||
    /^\/api\/console\/runners\/[^/]+\/stream(?:\?.*)?$/.test(path)
  );
}

function standardApiFetch(
  url: string,
  requestOptions: RequestInit,
): Promise<Response> {
  return fetch(url, { credentials: "include", ...requestOptions });
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException
    ? error.name === "AbortError"
    : error instanceof Error && error.name === "AbortError";
}

async function desktopNdjsonFetch(
  url: string,
  options: RequestInit,
  headers: Record<string, string>,
): Promise<Response> {
  const api = window.electronAPI!;
  const streamId = crypto.randomUUID();
  let controllerRef: ReadableStreamDefaultController<Uint8Array> | null = null;
  let settled = false;
  let unlisten: (() => void) | undefined;
  let abortHandler: (() => void) | undefined;

  const cleanup = () => {
    if (unlisten) {
      unlisten();
      unlisten = undefined;
    }
    if (abortHandler && options.signal) {
      options.signal.removeEventListener("abort", abortHandler);
      abortHandler = undefined;
    }
  };
  const closeStream = () => {
    if (settled) return;
    settled = true;
    controllerRef?.close();
    cleanup();
  };
  const failStream = (error: unknown) => {
    if (settled) return;
    settled = true;
    controllerRef?.error(
      error instanceof Error
        ? error
        : new Error(errorMessageFromUnknown(error, "Desktop stream failed")),
    );
    cleanup();
  };

  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controllerRef = controller;
    },
    cancel() {
      settled = true;
      cleanup();
      return api.cancelNdjsonStream!(streamId).catch(() => {});
    },
  });

  if (options.signal?.aborted) {
    throw new DOMException("The operation was aborted.", "AbortError");
  }

  unlisten = await api.onNdjsonStream!((event) => {
    if (event.streamId !== streamId || settled) return;
    if (event.kind === "chunk") {
      controllerRef?.enqueue(new Uint8Array(event.chunk || []));
    } else if (event.kind === "done") {
      closeStream();
    } else if (event.kind === "error") {
      failStream(new Error(event.error || "Desktop stream failed"));
    }
  });

  abortHandler = () => {
    api.cancelNdjsonStream!(streamId).catch(() => {});
    failStream(new DOMException("The operation was aborted.", "AbortError"));
  };
  options.signal?.addEventListener("abort", abortHandler, { once: true });

  let started: { status: number; ok: boolean; statusText: string };
  try {
    started = await api.streamNdjson!({
      streamId,
      url,
      method: (options.method || "GET").toUpperCase(),
      headers,
      body: typeof options.body === "string" ? options.body : undefined,
    });
  } catch (error) {
    cleanup();
    throw new Error(
      errorMessageFromUnknown(error, "Could not start Flamel Desktop stream"),
    );
  }

  return new Response(stream, {
    status: started.status,
    statusText: started.statusText,
    headers: { "content-type": "application/x-ndjson" },
  });
}

/**
 * fetch() wrapper that prepends the connection base URL.
 * Adds Authorization: Bearer when we have a stored JWT for this server — needed
 * for cross-origin requests from web.flamel.dev where SameSite=Lax blocks cookies.
 */
export function apiFetch(
  path: string,
  options?: RequestInit,
): Promise<Response> {
  if (isNativeLocal()) return nativeFetch(path, options);
  const base = getApiBase();
  const token = base ? getAuthToken(base) : undefined;
  const authHeader: Record<string, string> = token
    ? { Authorization: `Bearer ${token}` }
    : {};
  const headers = { ...authHeader, ...normalizeHeaders(options?.headers) };
  const requestOptions: RequestInit = { ...options, headers };
  const url = base + path;
  if (shouldUseDesktopNdjsonBridge(path, base, requestOptions)) {
    return desktopNdjsonFetch(url, requestOptions, headers).catch((error) => {
      if (isAbortError(error)) throw error;
      console.warn(
        "Flamel Desktop stream bridge failed; falling back to fetch:",
        errorMessageFromUnknown(error, "Bridge failed"),
      );
      return standardApiFetch(url, requestOptions);
    });
  }
  return standardApiFetch(url, requestOptions);
}

async function req<T>(path: string, options?: RequestInit): Promise<T> {
  const res = await apiFetch(path, {
    headers: { "Content-Type": "application/json", ...options?.headers },
    ...options,
  });
  if (!res.ok) {
    const text = await res.text();
    let message = text;
    try {
      const payload = JSON.parse(text) as {
        error?: unknown;
        message?: unknown;
      };
      if (typeof payload.error === "string") message = payload.error;
      else if (typeof payload.message === "string") message = payload.message;
    } catch {
      // Plain-text server errors are already suitable for display.
    }
    throw new Error(
      message ? `${res.status}: ${message}` : `Request failed (${res.status})`,
    );
  }
  return res.json();
}

async function reqVoid(path: string, options?: RequestInit): Promise<void> {
  const res = await apiFetch(path, {
    headers: { "Content-Type": "application/json", ...options?.headers },
    ...options,
  });
  if (!res.ok) {
    const message = await res.text();
    throw new Error(message || `Request failed (${res.status})`);
  }
}

export class FileRevisionConflictError extends Error {
  constructor(public readonly current: FileRevision) {
    super("File changed on disk after it was opened");
    this.name = "FileRevisionConflictError";
  }
}

async function saveFile(
  request: FileWriteRequest,
): Promise<{ ok: boolean; file: FileRevision }> {
  const response = await apiFetch("/api/file-content", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(request),
  });
  const payload = (await response.json().catch(() => ({}))) as {
    ok?: boolean;
    file?: FileRevision;
    current?: FileRevision;
    code?: string;
    error?: string;
  };
  if (
    response.status === 409 &&
    payload.code === "revision_conflict" &&
    payload.current
  ) {
    throw new FileRevisionConflictError(payload.current);
  }
  if (!response.ok || !payload.file) {
    throw new Error(payload.error || `Request failed (${response.status})`);
  }
  return { ok: payload.ok === true, file: payload.file };
}

export const api = {
  login: (token: string) =>
    req("/login", { method: "POST", body: JSON.stringify({ token }) }),
  logout: () => req("/logout", { method: "POST" }),

  projects: (): Promise<Project[]> => req("/api/projects"),

  sessions: {
    list: (q?: string): Promise<SessionMeta[]> =>
      req(`/api/sessions${q ? `?q=${encodeURIComponent(q)}` : ""}`),
    get: (id: string): Promise<SessionMeta & { shareToken?: string }> =>
      req(`/api/sessions/${id}`),
    bootstrap: (
      id: string,
      options?: Pick<RequestInit, "signal">,
    ): Promise<import("@flamel/shared").SessionBootstrap> =>
      req(`/api/sessions/${id}/bootstrap`, options),
    setActivePlan: (id: string, activePlanId?: string): Promise<SessionMeta> =>
      req(`/api/sessions/${id}/active-plan`, {
        method: "PUT",
        body: JSON.stringify({ activePlanId: activePlanId || null }),
      }),
    update: (id: string, data: Partial<SessionMeta>): Promise<SessionMeta> =>
      req(`/api/sessions/${id}`, { method: "PUT", body: JSON.stringify(data) }),
    delete: (id: string): Promise<void> =>
      req(`/api/sessions/${id}`, { method: "DELETE" }),
    share: (id: string): Promise<{ shareToken: string; shareUrl: string }> =>
      req(`/api/sessions/${id}/share`, { method: "POST" }),
    revokeShare: (id: string): Promise<void> =>
      req(`/api/sessions/${id}/share`, { method: "DELETE" }),
    truncate: (id: string, keepUserMessageCount: number): Promise<void> =>
      req(`/api/sessions/${id}/truncate`, {
        method: "POST",
        body: JSON.stringify({ keepUserMessageCount }),
      }),
  },

  chat: {
    send: (_body: ChatRequest): EventSource => {
      // We use fetch for NDJSON streaming
      return null as any; // handled by useChat hook
    },
    abort: (requestId: string): Promise<void> =>
      req(`/api/abort/${requestId}`, { method: "POST" }),
    permission: (requestId: string, approved: boolean): Promise<void> =>
      req(`/api/permission/${requestId}`, {
        method: "POST",
        body: JSON.stringify({ approved }),
      }),
    ask: {
      answer: (requestId: string, answer: string): Promise<void> =>
        req(`/api/ask/${requestId}`, {
          method: "POST",
          body: JSON.stringify({ answer }),
        }),
      cancel: (requestId: string): Promise<void> =>
        req(`/api/ask/${requestId}/cancel`, { method: "POST" }),
    },
  },

  prompts: {
    list: (): Promise<SavedPrompt[]> => req("/api/prompts"),
    create: (title: string, content: string): Promise<SavedPrompt> =>
      req("/api/prompts", {
        method: "POST",
        body: JSON.stringify({ title, content }),
      }),
    delete: (id: string): Promise<void> =>
      req(`/api/prompts/${id}`, { method: "DELETE" }),
    use: (id: string): Promise<void> =>
      req(`/api/prompts/${id}/use`, { method: "POST" }),
  },

  usage: (): Promise<UsageStats> => req("/api/usage"),

  files: (
    path: string,
    showHidden = false,
  ): Promise<{ path: string; tree: FileNode[] }> =>
    req(
      `/api/files?path=${encodeURIComponent(path)}&show_hidden=${showHidden}`,
    ),

  fileContent: {
    get: (path: string): Promise<FileRevision> =>
      req(`/api/file-content?path=${encodeURIComponent(path)}`),
    save: (
      path: string,
      content: string,
      expectedRevision?: string,
    ): Promise<{ ok: boolean; file: FileRevision }> =>
      saveFile({ path, content, expectedRevision }),
  },

  git: {
    status: (path: string, signal?: AbortSignal): Promise<GitStatus> =>
      req(`/api/git/status?path=${encodeURIComponent(path)}`, { signal }),
    diff: (
      path: string,
      file?: string,
      mode?: GitDiffMode,
    ): Promise<{ diff: string }> => {
      const params = new URLSearchParams({ path });
      if (file) params.set("file", file);
      if (mode) params.set("mode", mode);
      return req(`/api/git/diff?${params.toString()}`);
    },
    structuredDiff: (
      path: string,
      file?: string,
      mode?: GitDiffMode,
    ): Promise<{ files: GitDiffFile[] }> => {
      const params = new URLSearchParams({ path });
      if (file) params.set("file", file);
      if (mode) params.set("mode", mode);
      return req(`/api/git/structured-diff?${params.toString()}`);
    },
    file: (
      path: string,
      file: string,
      version: "head" | "index" = "head",
    ): Promise<{ content: string }> =>
      req(
        `/api/git/file?path=${encodeURIComponent(path)}&file=${encodeURIComponent(file)}&version=${version}`,
      ),
    history: (
      path: string,
      options?: { limit?: number; skip?: number; file?: string },
    ): Promise<{ commits: GitCommitSummary[] }> => {
      const params = new URLSearchParams({ path });
      if (options?.limit) params.set("limit", String(options.limit));
      if (options?.skip) params.set("skip", String(options.skip));
      if (options?.file) params.set("file", options.file);
      return req(`/api/git/history?${params.toString()}`);
    },
    branches: (path: string): Promise<{ branches: GitBranchSummary[] }> =>
      req(`/api/git/branches?path=${encodeURIComponent(path)}`),
    operate: (operation: GitOperationRequest): Promise<GitOperationResult> =>
      req("/api/git/operation", {
        method: "POST",
        body: JSON.stringify(operation),
      }),
  },

  terminal: {
    start: (
      cwd: string,
      cols: number,
      rows: number,
    ): Promise<{ session_id: string; command: string; cwd: string }> =>
      req("/api/terminal/start", {
        method: "POST",
        body: JSON.stringify({ cwd, cols, rows }),
      }),
    input: (id: string, data: string): Promise<void> =>
      reqVoid(`/api/terminal/${encodeURIComponent(id)}/input`, {
        method: "POST",
        body: JSON.stringify({ data }),
      }),
    resize: (id: string, cols: number, rows: number): Promise<void> =>
      reqVoid(`/api/terminal/${encodeURIComponent(id)}/resize`, {
        method: "POST",
        body: JSON.stringify({ cols, rows }),
      }),
    status: (
      id: string,
    ): Promise<{
      running: boolean;
      exited: boolean;
      cwd: string;
      command: string;
    }> => req(`/api/terminal/${encodeURIComponent(id)}/status`),
    close: (id: string): Promise<void> =>
      reqVoid(`/api/terminal/${encodeURIComponent(id)}`, { method: "DELETE" }),
  },

  mcp: {
    list: (projectPath?: string): Promise<McpServerConfig[]> =>
      req(
        `/api/mcp${projectPath ? `?projectPath=${encodeURIComponent(projectPath)}` : ""}`,
      ),
    create: (config: McpServerConfig): Promise<McpServerConfig> =>
      req("/api/mcp", { method: "POST", body: JSON.stringify(config) }),
    update: (
      id: string,
      data: Partial<McpServerConfig>,
    ): Promise<McpServerConfig> =>
      req(`/api/mcp/${id}`, { method: "PUT", body: JSON.stringify(data) }),
    delete: (id: string): Promise<void> =>
      req(`/api/mcp/${id}`, { method: "DELETE" }),
  },

  assistant: {
    operation: <T = unknown>(
      action: FlamelAssistantAction,
      argumentsValue: Record<string, unknown> = {},
      options: { confirm?: boolean; idempotencyKey?: string } = {},
    ): Promise<FlamelAssistantOperationResponse<T>> =>
      req("/api/internal/flamel-assistant/operation", {
        method: "POST",
        body: JSON.stringify({
          action,
          arguments: argumentsValue,
          confirm: options.confirm ?? false,
          ...(options.idempotencyKey
            ? { idempotency_key: options.idempotencyKey }
            : {}),
        }),
      }),
  },

  capabilities: {
    catalog: (
      projectPath?: string,
    ): Promise<import("@flamel/shared").CapabilityCatalogResponse> =>
      req(
        `/api/capabilities/catalog${projectPath ? `?projectPath=${encodeURIComponent(projectPath)}` : ""}`,
      ),
    create: (
      entry: Partial<import("@flamel/shared").CapabilityCatalogEntry>,
    ): Promise<import("@flamel/shared").CapabilityCatalogEntry> =>
      req("/api/capabilities/catalog", {
        method: "POST",
        body: JSON.stringify(entry),
      }),
    connect: (
      id: string,
      projectPath?: string,
    ): Promise<{ ok: boolean; status: string }> =>
      req(
        `/api/capabilities/${id}/connect${projectPath ? `?projectPath=${encodeURIComponent(projectPath)}` : ""}`,
        { method: "POST" },
      ),
    disconnect: (id: string, projectPath?: string): Promise<{ ok: boolean }> =>
      req(
        `/api/capabilities/${id}/connect${projectPath ? `?projectPath=${encodeURIComponent(projectPath)}` : ""}`,
        { method: "DELETE" },
      ),
    test: (
      id: string,
      projectPath?: string,
    ): Promise<{ ok: boolean; status: string; error?: string }> =>
      req(
        `/api/capabilities/${id}/test${projectPath ? `?projectPath=${encodeURIComponent(projectPath)}` : ""}`,
        { method: "POST" },
      ),
    tools: (
      id: string,
    ): Promise<{ tools: import("@flamel/shared").CapabilityTool[] }> =>
      req(`/api/capabilities/${id}/tools`),
    setTool: (
      id: string,
      tool: string,
      enabled: boolean,
    ): Promise<{ ok: boolean; enabled: boolean }> =>
      req(`/api/capabilities/${id}/tools/${encodeURIComponent(tool)}`, {
        method: "PUT",
        body: JSON.stringify({ enabled }),
      }),
    marketplace: (): Promise<{
      entries: import("@flamel/shared").MarketplaceCapability[];
      adminManaged: boolean;
    }> => req("/api/capabilities/marketplace"),
    install: (id: string): Promise<{ ok: boolean; capabilityId: string }> =>
      req(`/api/capabilities/marketplace/${id}/install`, { method: "POST" }),
    packs: (): Promise<{
      packs: import("@flamel/shared").CapabilityPackInstallation[];
      catalogSource: "capabilities";
      signedOnly: true;
    }> => req("/api/capabilities/packs"),
    installPack: (
      envelope: Record<string, unknown>,
    ): Promise<{
      ok: boolean;
      pack: import("@flamel/shared").CapabilityPackInstallation;
    }> =>
      req("/api/capabilities/packs", {
        method: "POST",
        body: JSON.stringify(envelope),
      }),
    rollbackPack: (
      id: string,
    ): Promise<{
      ok: boolean;
      pack: import("@flamel/shared").CapabilityPackInstallation;
    }> =>
      req(`/api/capabilities/packs/${encodeURIComponent(id)}/rollback`, {
        method: "POST",
      }),
    uninstallPack: (
      id: string,
    ): Promise<{
      ok: boolean;
      pack: import("@flamel/shared").CapabilityPackInstallation;
    }> =>
      req(`/api/capabilities/packs/${encodeURIComponent(id)}`, {
        method: "DELETE",
      }),
    skills: (
      options: {
        query?: string;
        projectPath?: string;
        provider?: "codex" | "claude";
      } = {},
    ): Promise<{
      skills: import("@flamel/shared").GovernedSkill[];
      instructionOnly: true;
      reviewRequired: true;
    }> => {
      const params = new URLSearchParams();
      if (options.query) params.set("query", options.query);
      if (options.projectPath) params.set("projectPath", options.projectPath);
      if (options.provider) params.set("provider", options.provider);
      const suffix = params.size ? `?${params.toString()}` : "";
      return req(`/api/capabilities/skills${suffix}`);
    },
    inspectSkill: (
      id: string,
      projectPath?: string,
    ): Promise<import("@flamel/shared").GovernedSkill> =>
      req(
        `/api/capabilities/skills/${encodeURIComponent(id)}${projectPath ? `?projectPath=${encodeURIComponent(projectPath)}` : ""}`,
      ),
    proposeSkill: (
      slug: string,
      skill: import("@flamel/shared").GovernedSkillVersionInput,
    ): Promise<{ skill: import("@flamel/shared").GovernedSkill }> =>
      req("/api/capabilities/skills", {
        method: "POST",
        body: JSON.stringify({ slug, skill }),
      }),
    reviewSkill: (
      id: string,
      version: string,
      action: "approve" | "reject",
    ): Promise<{ skill: import("@flamel/shared").GovernedSkill }> =>
      req(`/api/capabilities/skills/${encodeURIComponent(id)}/review`, {
        method: "POST",
        body: JSON.stringify({ version, action }),
      }),
    addSkillVersion: (
      id: string,
      skill: import("@flamel/shared").GovernedSkillVersionInput,
    ): Promise<{ skill: import("@flamel/shared").GovernedSkill }> =>
      req(`/api/capabilities/skills/${encodeURIComponent(id)}/versions`, {
        method: "POST",
        body: JSON.stringify(skill),
      }),
    installSkill: (
      id: string,
      options: {
        version?: string;
        projectPath?: string;
        providerProjectionEnabled?: boolean;
      } = {},
    ): Promise<{
      skill: import("@flamel/shared").GovernedSkill;
      snapshots: Record<
        string,
        import("@flamel/shared").SkillProjectionSnapshot
      >;
    }> =>
      req(`/api/capabilities/skills/${encodeURIComponent(id)}/install`, {
        method: "POST",
        body: JSON.stringify(options),
      }),
    rollbackSkill: (
      id: string,
      projectPath?: string,
    ): Promise<{
      skill: import("@flamel/shared").GovernedSkill;
      snapshots: Record<
        string,
        import("@flamel/shared").SkillProjectionSnapshot
      >;
    }> =>
      req(`/api/capabilities/skills/${encodeURIComponent(id)}/rollback`, {
        method: "POST",
        body: JSON.stringify({ projectPath }),
      }),
    setSkillState: (
      id: string,
      options: {
        enabled: boolean;
        projectPath?: string;
        providerProjectionEnabled?: boolean;
      },
    ): Promise<{
      ok: boolean;
      snapshots: Record<
        string,
        import("@flamel/shared").SkillProjectionSnapshot
      >;
    }> =>
      req(`/api/capabilities/skills/${encodeURIComponent(id)}`, {
        method: "PATCH",
        body: JSON.stringify(options),
      }),
    uninstallSkill: (
      id: string,
      projectPath?: string,
    ): Promise<{
      ok: boolean;
      snapshots: Record<
        string,
        import("@flamel/shared").SkillProjectionSnapshot
      >;
    }> =>
      req(`/api/capabilities/skills/${encodeURIComponent(id)}`, {
        method: "DELETE",
        body: JSON.stringify({ projectPath }),
      }),
    metrics: (): Promise<{
      calls: number;
      failures: number;
      successRate: number;
      averageLatencyMs: number;
    }> => req("/api/capabilities/metrics"),
    setCredential: (
      id: string,
      value: string,
      kind = "token",
    ): Promise<{ ok: boolean; credentialRef: string }> =>
      req(`/api/capabilities/${id}/credential`, {
        method: "PUT",
        body: JSON.stringify({ value, kind }),
      }),
    deleteCredential: (id: string): Promise<{ ok: boolean }> =>
      req(`/api/capabilities/${id}/credential`, { method: "DELETE" }),
    audit: (): Promise<{
      events: Array<{
        id: string;
        capabilityId?: string;
        eventType: string;
        decision: string;
        toolName?: string;
        summary: string;
        metadata: Record<string, unknown>;
        createdAt: string;
      }>;
    }> => req("/api/capabilities/audit?limit=100"),
  },

  work: {
    list: (
      projectPath: string,
      query?: string,
    ): Promise<{ items: import("@flamel/shared").WorkItem[] }> =>
      req(
        `/api/work-items?projectPath=${encodeURIComponent(projectPath)}${query ? `&query=${encodeURIComponent(query)}` : ""}`,
      ),
    create: (
      item: Partial<import("@flamel/shared").WorkItem> & {
        projectPath: string;
        title: string;
      },
    ): Promise<import("@flamel/shared").WorkItem> =>
      req("/api/work-items", { method: "POST", body: JSON.stringify(item) }),
    update: (
      id: string,
      item: Partial<import("@flamel/shared").WorkItem>,
    ): Promise<import("@flamel/shared").WorkItem> =>
      req(`/api/work-items/${id}`, {
        method: "PUT",
        body: JSON.stringify(item),
      }),
    delete: (id: string): Promise<{ ok: boolean }> =>
      req(`/api/work-items/${id}`, { method: "DELETE" }),
  },

  memories: {
    list: (
      projectPath?: string,
    ): Promise<{
      memories: Array<{
        id: string;
        projectPath: string;
        content: string;
        tags: string[];
        provenance: Record<string, unknown>;
        confidence: number;
        sourceKind: string;
        sourceId?: string | null;
        pinned: boolean;
        supersededBy?: string | null;
        retention: string;
        expiresAt?: number | null;
        updatedAt: number;
      }>;
    }> =>
      req(
        `/api/memories${projectPath ? `?projectPath=${encodeURIComponent(projectPath)}` : ""}`,
      ),
    pin: (id: string, pinned: boolean): Promise<{ ok: boolean }> =>
      req(`/api/memories/${id}`, {
        method: "PUT",
        body: JSON.stringify({ pinned }),
      }),
    delete: (id: string): Promise<{ ok: boolean }> =>
      req(`/api/memories/${id}`, { method: "DELETE" }),
  },

  plans: {
    list: (
      sessionId?: string,
      projectPath?: string,
    ): Promise<import("@flamel/shared").PlanMeta[]> => {
      const params = new URLSearchParams();
      if (sessionId) params.set("sessionId", sessionId);
      if (projectPath) params.set("projectPath", projectPath);
      const qs = params.toString();
      return req(`/api/plans${qs ? `?${qs}` : ""}`);
    },
    get: (id: string): Promise<import("@flamel/shared").Plan> =>
      req(`/api/plans/${id}`),
  },

  todos: {
    list: (projectPath: string): Promise<import("@flamel/shared").TodoItem[]> =>
      req(`/api/todos?projectPath=${encodeURIComponent(projectPath)}`),
    create: (
      text: string,
      projectPath: string,
    ): Promise<import("@flamel/shared").TodoItem> =>
      req("/api/todos", {
        method: "POST",
        body: JSON.stringify({ text, projectPath }),
      }),
    update: (
      id: string,
      patch: { done?: boolean; text?: string },
    ): Promise<import("@flamel/shared").TodoItem> =>
      req(`/api/todos/${id}`, { method: "PUT", body: JSON.stringify(patch) }),
    delete: (id: string): Promise<void> =>
      req(`/api/todos/${id}`, { method: "DELETE" }),
    clearDone: (projectPath: string): Promise<void> =>
      req(`/api/todos/done?projectPath=${encodeURIComponent(projectPath)}`, {
        method: "DELETE",
      }),
  },

  share: {
    get: (token: string): Promise<{ session: SessionMeta; share: any }> =>
      req(`/share/${token}`),
  },

  gitAccounts: {
    list: (): Promise<import("@flamel/shared").GitAccount[]> =>
      req("/api/git-accounts"),
    create: (data: {
      type: string;
      label: string;
      baseUrl?: string;
      token: string;
    }): Promise<import("@flamel/shared").GitAccount> =>
      req("/api/git-accounts", { method: "POST", body: JSON.stringify(data) }),
    delete: (id: string): Promise<void> =>
      req(`/api/git-accounts/${id}`, { method: "DELETE" }),
  },

  gitIntegration: {
    repoInfo: (
      projectPath: string,
    ): Promise<{
      branch: string;
      owner?: string;
      repo?: string;
      hasAccount: boolean;
    }> => req(`/api/git/repo?projectPath=${encodeURIComponent(projectPath)}`),
    issues: (
      projectPath: string,
      state = "open",
    ): Promise<import("@flamel/shared").GitIssue[]> =>
      req(
        `/api/git/issues?projectPath=${encodeURIComponent(projectPath)}&state=${state}`,
      ),
    issue: (
      projectPath: string,
      number: number,
    ): Promise<import("@flamel/shared").GitIssue> =>
      req(
        `/api/git/issues/${number}?projectPath=${encodeURIComponent(projectPath)}`,
      ),
    pulls: (
      projectPath: string,
      state = "open",
    ): Promise<import("@flamel/shared").GitPullRequest[]> =>
      req(
        `/api/git/pulls?projectPath=${encodeURIComponent(projectPath)}&state=${state}`,
      ),
    pull: (
      projectPath: string,
      number: number,
    ): Promise<import("@flamel/shared").GitPullRequest> =>
      req(
        `/api/git/pulls/${number}?projectPath=${encodeURIComponent(projectPath)}`,
      ),
    createPull: (data: {
      projectPath: string;
      title: string;
      description?: string;
      headBranch: string;
      baseBranch: string;
      draft?: boolean;
    }): Promise<import("@flamel/shared").GitPullRequest> =>
      req("/api/git/pulls", { method: "POST", body: JSON.stringify(data) }),
  },

  runners: {
    list: (): Promise<any[]> => req("/api/runners"),
    get: (id: string): Promise<any> => req(`/api/runners/${id}`),
    create: (data: {
      name: string;
      ssh_host?: string;
      ssh_user?: string;
      ssh_key_path?: string;
      ssh_password?: string;
      ssh_key?: string;
      notes?: string;
    }): Promise<any> =>
      req("/api/runners", { method: "POST", body: JSON.stringify(data) }),
    update: (
      id: string,
      data: Partial<{
        name: string;
        ssh_host: string | null;
        ssh_user: string | null;
        ssh_key_path: string | null;
        ssh_password: string | null;
        ssh_key: string | null;
        notes: string | null;
      }>,
    ): Promise<any> =>
      req(`/api/runners/${id}`, { method: "PUT", body: JSON.stringify(data) }),
    delete: (id: string): Promise<void> =>
      req(`/api/runners/${id}`, { method: "DELETE" }),
    regenerateToken: (id: string): Promise<{ token: string }> =>
      req(`/api/runners/${id}/regenerate-token`, { method: "POST" }),
    browse: (id: string, path?: string): Promise<any[]> =>
      req(
        `/api/runners/${id}/browse${path ? `?path=${encodeURIComponent(path)}` : ""}`,
      ),
    deploy: (id: string): Promise<any> =>
      req(`/api/runners/${id}/deploy`, { method: "POST" }),
  },

  projectRunners: {
    list: (projectPath: string): Promise<{ mode: string; runners: any[] }> =>
      req(
        `/api/project-runners?project_path=${encodeURIComponent(projectPath)}`,
      ),
    setMode: (projectPath: string, mode: string): Promise<any> =>
      req("/api/project-runners/mode", {
        method: "PUT",
        body: JSON.stringify({ project_path: projectPath, mode }),
      }),
    add: (
      projectPath: string,
      runnerId: string,
      priority?: number,
    ): Promise<any> =>
      req("/api/project-runners", {
        method: "POST",
        body: JSON.stringify({
          project_path: projectPath,
          runner_id: runnerId,
          priority,
        }),
      }),
    remove: (projectPath: string, runnerId: string): Promise<any> =>
      req("/api/project-runners", {
        method: "DELETE",
        body: JSON.stringify({
          project_path: projectPath,
          runner_id: runnerId,
        }),
      }),
  },

  admin: {
    stats: (): Promise<any> => req("/api/admin/stats"),
    users: (): Promise<any[]> => req("/api/admin/users"),
    updateUser: (id: string, data: { role: string }): Promise<any> =>
      req(`/api/admin/users/${id}`, {
        method: "PUT",
        body: JSON.stringify(data),
      }),
    deleteUser: (id: string): Promise<void> =>
      req(`/api/admin/users/${id}`, { method: "DELETE" }),
    runners: (): Promise<any[]> => req("/api/admin/runners"),
  },

  agents: {
    list: (): Promise<import("@flamel/shared").AgentConfig[]> =>
      req("/api/agent-configs"),
    create: (
      data: Partial<import("@flamel/shared").AgentConfig>,
    ): Promise<import("@flamel/shared").AgentConfig> =>
      req("/api/agent-configs", { method: "POST", body: JSON.stringify(data) }),
    update: (
      id: string,
      data: Partial<import("@flamel/shared").AgentConfig>,
    ): Promise<import("@flamel/shared").AgentConfig> =>
      req(`/api/agent-configs/${id}`, {
        method: "PUT",
        body: JSON.stringify(data),
      }),
    delete: (id: string): Promise<void> =>
      req(`/api/agent-configs/${id}`, { method: "DELETE" }),
    runs: (configId?: string): Promise<import("@flamel/shared").AgentRun[]> =>
      req(`/api/agent-runs${configId ? `?configId=${configId}` : ""}`),
    approve: (id: string): Promise<void> =>
      req(`/api/agent-runs/${id}/approve`, { method: "POST" }),
    reject: (id: string): Promise<void> =>
      req(`/api/agent-runs/${id}/reject`, { method: "POST" }),
    kill: (id: string): Promise<void> =>
      req(`/api/agent-runs/${id}/kill`, { method: "POST" }),
  },
};
