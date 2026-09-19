import React, { useState } from "react";

export function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      onClick={() => {
        navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
      title="Copy"
      style={{
        position: "absolute",
        top: 7,
        right: 8,
        display: "flex",
        alignItems: "center",
        gap: 3,
        padding: "2px 7px",
        borderRadius: "var(--radius-sm)",
        background: copied ? "var(--success-glow)" : "var(--surface-3)",
        border: "1px solid var(--border-soft)",
        color: copied ? "var(--success)" : "var(--text-dim)",
        fontSize: 10,
        fontFamily: "var(--font-mono)",
        cursor: "pointer",
        transition: "all var(--transition-fast)",
        opacity: 0,
      }}
      className="code-copy-btn"
    >
      {copied ? "copied" : "copy"}
    </button>
  );
}

// Stable module-level constant — avoids recreating component refs on every render,
// which would force ReactMarkdown to re-render even when text hasn't changed.
export const MD_COMPONENTS = {
  code({ children, className }: any) {
    const isBlock = className?.includes("language-");
    const lang = className?.replace("language-", "") || "";
    const codeStr = String(children).replace(/\n$/, "");
    if (isBlock) {
      return (
        <div
          style={{ position: "relative", margin: "8px 0" }}
          className="code-block-wrap"
        >
          <style>{`.code-block-wrap:hover .code-copy-btn { opacity: 1; }`}</style>
          {lang && (
            <div
              style={{
                background: "var(--surface-2)",
                borderBottom: "1px solid var(--border-soft)",
                padding: "3px 12px",
                borderRadius: "var(--radius-md) var(--radius-md) 0 0",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
              }}
            >
              <span
                style={{
                  fontSize: 10,
                  color: "var(--text-dim)",
                  fontFamily: "var(--font-mono)",
                  letterSpacing: "0.03em",
                }}
              >
                {lang}
              </span>
            </div>
          )}
          <pre
            style={{
              background: "var(--surface)",
              fontFamily: "var(--font-mono)",
              fontSize: 12,
              borderRadius: lang
                ? "0 0 var(--radius-md) var(--radius-md)"
                : "var(--radius-md)",
              padding: "10px 14px",
              overflowX: "auto",
              border: "1px solid var(--border-soft)",
              borderTop: lang ? "none" : undefined,
              margin: 0,
              position: "relative",
            }}
          >
            <code>{codeStr}</code>
            <CopyButton text={codeStr} />
          </pre>
        </div>
      );
    }
    return (
      <code
        style={{
          background: "var(--surface-2)",
          padding: "1px 5px",
          borderRadius: "var(--radius-sm)",
          fontFamily: "var(--font-mono)",
          fontSize: "0.875em",
          border: "1px solid var(--border-soft)",
        }}
      >
        {children}
      </code>
    );
  },
  p({ children }: any) {
    return <p style={{ marginBottom: 8 }}>{children}</p>;
  },
  ul({ children }: any) {
    return <ul style={{ paddingLeft: 20, marginBottom: 8 }}>{children}</ul>;
  },
  ol({ children }: any) {
    return <ol style={{ paddingLeft: 20, marginBottom: 8 }}>{children}</ol>;
  },
  li({ children }: any) {
    return <li style={{ marginBottom: 3 }}>{children}</li>;
  },
  h1({ children }: any) {
    return (
      <h1
        style={{
          fontSize: 16,
          fontWeight: 700,
          marginBottom: 6,
          marginTop: 14,
          color: "var(--text)",
        }}
      >
        {children}
      </h1>
    );
  },
  h2({ children }: any) {
    return (
      <h2
        style={{
          fontSize: 14,
          fontWeight: 600,
          marginBottom: 5,
          marginTop: 12,
          color: "var(--text)",
        }}
      >
        {children}
      </h2>
    );
  },
  h3({ children }: any) {
    return (
      <h3
        style={{
          fontSize: 13,
          fontWeight: 600,
          marginBottom: 4,
          marginTop: 10,
          color: "var(--text-muted)",
        }}
      >
        {children}
      </h3>
    );
  },
  strong({ children }: any) {
    return (
      <strong style={{ fontWeight: 600, color: "var(--text)" }}>
        {children}
      </strong>
    );
  },
  blockquote({ children }: any) {
    return (
      <blockquote
        style={{
          borderLeft: "3px solid var(--accent)",
          paddingLeft: 12,
          color: "var(--text-muted)",
          margin: "8px 0",
          fontStyle: "italic",
        }}
      >
        {children}
      </blockquote>
    );
  },
  a({ children, href }: any) {
    const handleClick = (e: React.MouseEvent) => {
      if (!href) return;
      e.preventDefault();
      if ((window as any).electronAPI?.openExternal) {
        (window as any).electronAPI.openExternal(href);
      } else {
        window.open(href, "_blank", "noopener,noreferrer");
      }
    };
    return (
      <a
        href={href}
        onClick={handleClick}
        style={{
          color: "var(--accent)",
          textDecoration: "underline",
          textUnderlineOffset: 2,
          cursor: "pointer",
        }}
      >
        {children}
      </a>
    );
  },
  hr() {
    return (
      <hr
        style={{
          border: "none",
          borderTop: "1px solid var(--border-soft)",
          margin: "12px 0",
        }}
      />
    );
  },
  table({ children }: any) {
    return (
      <div
        style={{
          overflowX: "auto",
          margin: "8px 0",
          borderRadius: "var(--radius-md)",
          border: "1px solid var(--border-soft)",
        }}
      >
        <table
          style={{
            width: "100%",
            borderCollapse: "collapse",
            fontSize: 12,
            fontFamily: "var(--font-mono)",
          }}
        >
          {children}
        </table>
      </div>
    );
  },
  thead({ children }: any) {
    return <thead style={{ background: "var(--surface-2)" }}>{children}</thead>;
  },
  th({ children, style }: any) {
    return (
      <th
        style={{
          ...style,
          padding: "6px 12px",
          borderBottom: "1px solid var(--border-soft)",
          textAlign: "left",
          fontWeight: 600,
          color: "var(--text)",
          fontSize: 11,
          letterSpacing: "0.02em",
          whiteSpace: "nowrap",
        }}
      >
        {children}
      </th>
    );
  },
  td({ children, style }: any) {
    return (
      <td
        style={{
          ...style,
          padding: "5px 12px",
          borderBottom: "1px solid var(--border-soft)",
          color: "var(--text-muted)",
        }}
      >
        {children}
      </td>
    );
  },
  tr({ children }: any) {
    return (
      <tr
        style={{ transition: "background 0.1s" }}
        onMouseEnter={(e) =>
          (e.currentTarget.style.background = "var(--surface)")
        }
        onMouseLeave={(e) => (e.currentTarget.style.background = "")}
      >
        {children}
      </tr>
    );
  },
};

export const THINKING_MD_COMPONENTS = {
  ...MD_COMPONENTS,
  p({ children }: any) {
    return <p style={{ marginBottom: 5 }}>{children}</p>;
  },
  h1({ children }: any) {
    return (
      <div style={{ color: "var(--text-muted)", marginBottom: 4 }}>
        {children}
      </div>
    );
  },
  h2({ children }: any) {
    return (
      <div style={{ color: "var(--text-muted)", marginBottom: 4 }}>
        {children}
      </div>
    );
  },
  h3({ children }: any) {
    return (
      <div style={{ color: "var(--text-muted)", marginBottom: 4 }}>
        {children}
      </div>
    );
  },
  strong({ children }: any) {
    return (
      <strong style={{ color: "var(--text-muted)", fontWeight: 500 }}>
        {children}
      </strong>
    );
  },
};
