import React, { useEffect, useMemo, useState } from "react";
import { apiFetch } from "../../lib/api";
import type { ToolCall } from "../../lib/messageProcessor";
import type {
  ToolArtifact,
  ToolResourceLink,
} from "../../lib/toolPresentation";
import { shortToolName } from "../../lib/toolCapabilities";
export { isMediaToolCall } from "./toolWidgetRouting";
import { Icon } from "../ui/Icons";
import {
  ToolWidgetFields,
  ToolWidgetInspector,
  ToolWidgetSection,
  ToolWidgetState,
} from "./ToolWidgetPrimitives";
import { ToolWidgetShell } from "./ToolWidgetShell";

type MediaKind = "image" | "audio" | "video" | "file";
type MediaSource = "artifact" | "resource" | "workspace";

export interface MediaPreviewItem {
  id: string;
  name: string;
  kind: MediaKind;
  mediaType: string;
  source: MediaSource;
  url?: string;
  downloadUrl?: string;
  path?: string;
  byteSize?: number;
  alt?: string;
}

export interface MediaToolView {
  title: string;
  summary: string;
  detail?: string;
  status: "pending" | "running" | "success" | "error" | "empty";
  prompt?: string;
  outputHint?: string;
  spriteSheet: boolean;
  items: MediaPreviewItem[];
}

type JsonRecord = Record<string, unknown>;

const IMAGE_PATH_RE =
  /(?:^|[\s"'\x60([{<])((?:\/[^\s"'\x60<>()\[\]{}]+?)\.(?:png|jpe?g|webp|gif|avif))(?=$|[\s"'\x60)\]}>.,;:])/gim;

function asRecord(value: unknown): JsonRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : undefined;
}

function stringValue(
  record: JsonRecord | undefined,
  ...keys: string[]
): string | undefined {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return undefined;
}

function safeWebUrl(value: unknown): string | undefined {
  if (typeof value !== "string" || !value || value.length > 4096)
    return undefined;
  if (value.startsWith("/")) return value;
  if (/^https?:\/\//i.test(value) || /^blob:/i.test(value)) return value;
  return undefined;
}

function kindFor(
  mediaType: string,
  fallback: "image" | "file" = "file",
): MediaKind {
  const lower = mediaType.toLowerCase();
  if (lower.startsWith("image/")) return "image";
  if (lower.startsWith("audio/")) return "audio";
  if (lower.startsWith("video/")) return "video";
  return fallback;
}

function outputRecord(tool: ToolCall): JsonRecord | undefined {
  const structured = asRecord(tool.presentation?.structuredContent);
  if (structured) return structured;
  if (!tool.output) return undefined;
  try {
    const parsed = asRecord(JSON.parse(tool.output));
    return (
      asRecord(parsed?.structuredContent) ??
      asRecord(parsed?.structured_content) ??
      asRecord(parsed?.data) ??
      parsed
    );
  } catch {
    return undefined;
  }
}

function artifactItem(artifact: ToolArtifact): MediaPreviewItem {
  return {
    id: artifact.id || artifact.contentUrl,
    name: artifact.name,
    kind: kindFor(artifact.mediaType, artifact.kind),
    mediaType: artifact.mediaType,
    source: "artifact",
    url: safeWebUrl(artifact.contentUrl),
    downloadUrl: safeWebUrl(artifact.downloadUrl ?? artifact.contentUrl),
    byteSize: artifact.byteSize,
  };
}

function resourceItem(
  resource: ToolResourceLink,
  index: number,
): MediaPreviewItem {
  const mediaType = resource.mediaType ?? "application/octet-stream";
  return {
    id: resource.uri || `resource-${index}`,
    name: resource.name ?? resource.title ?? `Resource ${index + 1}`,
    kind: kindFor(mediaType),
    mediaType,
    source: "resource",
    url: safeWebUrl(resource.uri),
    downloadUrl: safeWebUrl(resource.uri),
    byteSize: resource.size,
  };
}

function legacyPaths(output?: string): string[] {
  if (!output) return [];
  const paths: string[] = [];
  for (const match of output.matchAll(IMAGE_PATH_RE)) {
    const path = match[1].replace(/[),.;:]+$/, "");
    if (!paths.includes(path)) paths.push(path);
  }
  return paths.slice(0, 24);
}

function formatBytes(value?: number): string {
  if (!value || value < 1) return "size unavailable";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

function dedupeItems(items: MediaPreviewItem[]): MediaPreviewItem[] {
  const seen = new Set<string>();
  return items.filter((item) => {
    const key = item.path ?? item.url ?? `${item.name}:${item.mediaType}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function buildMediaToolView(tool: ToolCall): MediaToolView {
  const input = tool.input ?? {};
  const output = outputRecord(tool);
  const artifacts = dedupeItems(
    [...(tool.artifacts ?? []), ...(tool.presentation?.artifacts ?? [])].map(
      artifactItem,
    ),
  );
  const resources = (tool.presentation?.resources ?? []).map(resourceItem);
  const media = (tool.presentation?.media ?? []).map(
    (reference, index): MediaPreviewItem => {
      const mediaType = reference.mediaType ?? "application/octet-stream";
      return {
        id: reference.resourceUri ?? `media-${index}`,
        name: reference.alt ?? `Media ${index + 1}`,
        kind: reference.kind,
        mediaType,
        source: "resource",
        url: safeWebUrl(reference.resourceUri),
        downloadUrl: safeWebUrl(reference.resourceUri),
        alt: reference.alt,
      };
    },
  );
  const workspace = legacyPaths(tool.output).map(
    (path, index): MediaPreviewItem => ({
      id: `workspace-${path}`,
      name: path.split("/").pop() || `Image ${index + 1}`,
      kind: "image",
      mediaType: "image/*",
      source: "workspace",
      path,
    }),
  );
  const items = dedupeItems([
    ...artifacts,
    ...media,
    ...resources,
    ...workspace,
  ]);
  const prompt =
    stringValue(input, "prompt", "description", "text") ??
    stringValue(output, "prompt", "revised_prompt");
  const outputHint =
    stringValue(output, "output_hint", "outputHint", "hint") ??
    stringValue(input, "output_hint", "outputHint");
  const action = stringValue(input, "action") ?? shortToolName(tool.name);
  const spriteSheet = /sprite|tileset|pixel[ -]?art|contact[ -]?sheet/i.test(
    [action, prompt, outputHint, ...items.map((item) => item.name)]
      .filter(Boolean)
      .join(" "),
  );
  const status =
    tool.status === "running"
      ? "running"
      : tool.status === "pending"
        ? "pending"
        : tool.status === "error" || tool.presentation?.isError
          ? "error"
          : items.length
            ? "success"
            : "empty";
  const title = shortToolName(tool.name).toLowerCase().includes("view_image")
    ? "Image preview"
    : shortToolName(tool.name).toLowerCase().includes("image")
      ? "Generated media"
      : "Media assets";
  const summary =
    status === "running"
      ? "Rendering media"
      : status === "pending"
        ? "Waiting to render"
        : status === "error"
          ? "Media tool failed"
          : items.length
            ? `${items.length} ${items.length === 1 ? "artifact" : "artifacts"} ready`
            : "No previewable artifact";

  return {
    title,
    summary,
    detail: spriteSheet ? "sprite sheet" : action.replace(/_/g, " "),
    status,
    prompt,
    outputHint,
    spriteSheet,
    items,
  };
}

function useWorkspacePreviews(items: MediaPreviewItem[]) {
  const [urls, setUrls] = useState<Record<string, string>>({});
  const pathKey = items
    .filter((item) => item.source === "workspace" && item.path)
    .map((item) => item.path)
    .join("\n");

  useEffect(() => {
    const paths = pathKey ? pathKey.split("\n") : [];
    let cancelled = false;
    const objectUrls: string[] = [];
    if (!paths.length) {
      setUrls({});
      return;
    }

    Promise.all(
      paths.map(async (path) => {
        try {
          const response = await apiFetch(
            `/api/file-asset?path=${encodeURIComponent(path)}`,
          );
          if (!response.ok) return [path, ""] as const;
          const url = URL.createObjectURL(await response.blob());
          objectUrls.push(url);
          return [path, url] as const;
        } catch {
          return [path, ""] as const;
        }
      }),
    ).then((entries) => {
      if (!cancelled) setUrls(Object.fromEntries(entries));
    });

    return () => {
      cancelled = true;
      objectUrls.forEach((url) => URL.revokeObjectURL(url));
    };
  }, [pathKey]);

  return urls;
}

function MediaActionBar({ item }: { item: MediaPreviewItem }) {
  const copyReference = async () => {
    const value = item.path ?? item.url;
    if (!value) return;
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // Clipboard access is optional; open/save still work without it.
    }
  };
  return (
    <div className="media-widget__actions">
      {item.url ? (
        <a href={item.url} target="_blank" rel="noreferrer">
          <Icon.ExternalLink size={12} />
          Open
        </a>
      ) : null}
      {item.downloadUrl ? (
        <a href={item.downloadUrl} download={item.name}>
          <Icon.Download size={12} />
          Save
        </a>
      ) : null}
      {item.path || item.url ? (
        <button type="button" onClick={copyReference}>
          <Icon.Copy size={12} />
          Copy path
        </button>
      ) : null}
    </div>
  );
}

export function MediaPreviewGallery({
  items,
  spriteSheet = false,
}: {
  items: MediaPreviewItem[];
  spriteSheet?: boolean;
}) {
  const [mode, setMode] = useState<"grid" | "focus">(
    items.length > 1 ? "grid" : "focus",
  );
  const [active, setActive] = useState(0);
  const [zoom, setZoom] = useState(1);
  const [dimensions, setDimensions] = useState<Record<string, string>>({});
  const workspaceUrls = useWorkspacePreviews(items);
  const visible = mode === "focus" ? items.slice(active, active + 1) : items;

  if (!items.length) {
    return (
      <ToolWidgetState
        status="empty"
        title="No managed media"
        detail="The provider did not return a safe artifact URL or workspace image path."
      />
    );
  }

  return (
    <div className="media-widget">
      <div className="media-widget__toolbar">
        <div role="group" aria-label="Preview layout">
          <button
            type="button"
            data-active={mode === "grid"}
            onClick={() => setMode("grid")}
          >
            Grid
          </button>
          <button
            type="button"
            data-active={mode === "focus"}
            onClick={() => setMode("focus")}
          >
            Focus
          </button>
        </div>
        {mode === "focus" && items.length > 1 ? (
          <div className="media-widget__pager">
            <button
              type="button"
              onClick={() =>
                setActive((active - 1 + items.length) % items.length)
              }
            >
              ←
            </button>
            <span>
              {active + 1} / {items.length}
            </span>
            <button
              type="button"
              onClick={() => setActive((active + 1) % items.length)}
            >
              →
            </button>
          </div>
        ) : null}
        {spriteSheet ? (
          <div role="group" aria-label="Sprite zoom">
            {[1, 2, 4].map((value) => (
              <button
                type="button"
                key={value}
                data-active={zoom === value}
                onClick={() => setZoom(value)}
              >
                {value}×
              </button>
            ))}
          </div>
        ) : null}
      </div>

      <div className="media-widget__grid" data-mode={mode}>
        {visible.map((item) => {
          const source =
            item.url ?? (item.path ? workspaceUrls[item.path] : undefined);
          return (
            <article className="media-widget__item" key={item.id}>
              <div
                className="media-widget__preview"
                data-kind={item.kind}
                data-sprite={spriteSheet || undefined}
              >
                {item.kind === "image" && source ? (
                  <img
                    src={source}
                    alt={item.alt ?? item.name}
                    loading="lazy"
                    style={
                      spriteSheet && mode === "focus"
                        ? { width: `${zoom * 100}%` }
                        : undefined
                    }
                    onLoad={(event) => {
                      const image = event.currentTarget;
                      setDimensions((current) => ({
                        ...current,
                        [item.id]: `${image.naturalWidth} × ${image.naturalHeight}`,
                      }));
                    }}
                  />
                ) : item.kind === "audio" && source ? (
                  <audio controls preload="metadata" src={source} />
                ) : item.kind === "video" && source ? (
                  <video controls preload="metadata" src={source} />
                ) : (
                  <div className="media-widget__placeholder">
                    <Icon.File size={18} />
                    <span>
                      {source ? "File artifact" : "Preview unavailable"}
                    </span>
                  </div>
                )}
              </div>
              <div className="media-widget__caption">
                <div>
                  <strong title={item.name}>{item.name}</strong>
                  <span>
                    {[
                      item.mediaType,
                      dimensions[item.id],
                      formatBytes(item.byteSize),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </div>
                <MediaActionBar
                  item={{
                    ...item,
                    url: source ?? item.url,
                    downloadUrl: item.downloadUrl ?? source,
                  }}
                />
              </div>
            </article>
          );
        })}
      </div>
    </div>
  );
}

export function MediaToolWidget({
  tool,
  defaultExpanded,
}: {
  tool: ToolCall;
  defaultExpanded?: boolean;
}) {
  const view = useMemo(() => buildMediaToolView(tool), [tool]);
  const expanded = defaultExpanded ?? false;

  return (
    <ToolWidgetShell
      title={view.title}
      eyebrow="Media"
      summary={view.summary}
      detail={view.detail}
      status={view.status}
      statusLabel={view.status === "running" ? "Rendering" : undefined}
      icon={<Icon.Image size={14} />}
      sprite="media"
      trailing={view.spriteSheet ? "pixel preview" : undefined}
      defaultExpanded={expanded}
      inspector={
        <ToolWidgetInspector
          input={tool.input}
          output={tool.output}
          raw={{
            presentation: tool.presentation,
            artifacts: tool.artifacts,
          }}
        />
      }
    >
      {view.prompt || view.outputHint ? (
        <ToolWidgetSection title="Generation brief">
          <ToolWidgetFields
            fields={[
              ...(view.prompt ? [{ label: "Prompt", value: view.prompt }] : []),
              ...(view.outputHint
                ? [{ label: "Output", value: view.outputHint }]
                : []),
            ]}
          />
        </ToolWidgetSection>
      ) : null}
      <MediaPreviewGallery items={view.items} spriteSheet={view.spriteSheet} />
    </ToolWidgetShell>
  );
}
