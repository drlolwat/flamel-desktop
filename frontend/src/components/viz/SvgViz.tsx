import React, { useMemo } from "react";

const ALLOWED_SVG_TAGS = new Set([
  "svg",
  "g",
  "defs",
  "title",
  "desc",
  "path",
  "rect",
  "circle",
  "ellipse",
  "line",
  "polyline",
  "polygon",
  "text",
  "tspan",
  "lineargradient",
  "radialgradient",
  "stop",
  "clippath",
  "mask",
  "pattern",
  "use",
]);

const ALLOWED_SVG_ATTRIBUTES = new Set([
  "xmlns",
  "viewbox",
  "width",
  "height",
  "x",
  "y",
  "x1",
  "x2",
  "y1",
  "y2",
  "cx",
  "cy",
  "r",
  "rx",
  "ry",
  "d",
  "points",
  "fill",
  "fill-opacity",
  "stroke",
  "stroke-width",
  "stroke-linecap",
  "stroke-linejoin",
  "stroke-opacity",
  "opacity",
  "transform",
  "gradientunits",
  "gradienttransform",
  "offset",
  "stop-color",
  "stop-opacity",
  "clip-path",
  "mask",
  "id",
  "class",
  "font-family",
  "font-size",
  "font-weight",
  "text-anchor",
  "dominant-baseline",
  "role",
  "aria-label",
]);

function fallbackSanitizeSvg(svg: string): string {
  return svg
    .replace(/<script[\s\S]*?<\/script>/gi, "")
    .replace(/<foreignObject[\s\S]*?<\/foreignObject>/gi, "")
    .replace(/\s(?:on\w+|style|href|xlink:href)\s*=\s*("[^"]*"|'[^']*')/gi, "");
}

export function sanitizeSvgMarkup(svg: string): string {
  if (typeof svg !== "string" || !svg.trim()) return "";
  if (
    typeof DOMParser === "undefined" ||
    typeof XMLSerializer === "undefined"
  ) {
    return fallbackSanitizeSvg(svg);
  }

  const document = new DOMParser().parseFromString(svg, "image/svg+xml");
  const root = document.documentElement;
  if (
    root.localName.toLowerCase() !== "svg" ||
    document.querySelector("parsererror")
  ) {
    return "";
  }

  for (const element of Array.from(root.querySelectorAll("*"))) {
    if (!ALLOWED_SVG_TAGS.has(element.localName.toLowerCase())) {
      element.remove();
      continue;
    }
    for (const attribute of Array.from(element.attributes)) {
      const name = attribute.name.toLowerCase();
      const value = attribute.value.trim();
      if (
        !ALLOWED_SVG_ATTRIBUTES.has(name) ||
        /(?:javascript|vbscript|data|https?|file):/i.test(value) ||
        (/url\(/i.test(value) && !/^url\(#[A-Za-z][\w:.-]*\)$/i.test(value))
      ) {
        element.removeAttribute(attribute.name);
      }
    }
  }

  for (const attribute of Array.from(root.attributes)) {
    const name = attribute.name.toLowerCase();
    const value = attribute.value.trim();
    if (
      !ALLOWED_SVG_ATTRIBUTES.has(name) ||
      /(?:javascript|vbscript|data|https?|file):/i.test(value) ||
      (/url\(/i.test(value) && !/^url\(#[A-Za-z][\w:.-]*\)$/i.test(value))
    ) {
      root.removeAttribute(attribute.name);
    }
  }
  return new XMLSerializer().serializeToString(root);
}

interface SvgVizProps {
  svg: string;
  title?: string;
  width?: number;
  height?: number;
}

export default React.memo(function SvgViz({
  svg,
  title,
  width,
  height,
}: SvgVizProps) {
  const cleaned = useMemo(() => sanitizeSvgMarkup(svg), [svg]);

  return (
    <div
      style={{
        background: "var(--surface)",
        border: "1px solid var(--border-soft)",
        borderRadius: "var(--radius-md)",
        padding: "12px 14px",
        margin: "8px 0",
        overflow: "auto",
      }}
    >
      {title && (
        <div
          style={{
            fontSize: 12,
            fontWeight: 600,
            color: "var(--text)",
            marginBottom: 8,
          }}
        >
          {title}
        </div>
      )}
      <div
        style={{
          display: "flex",
          justifyContent: "center",
          maxWidth: width || undefined,
          maxHeight: height || undefined,
        }}
        dangerouslySetInnerHTML={{ __html: cleaned }}
      />
    </div>
  );
});
