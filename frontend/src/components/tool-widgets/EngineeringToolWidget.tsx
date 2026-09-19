import React, { useMemo } from "react";
import { api } from "../../lib/api";
import type { CommandEvent } from "../../lib/consoleTypes";
import type { ToolCall } from "../../lib/messageProcessor";
import { useIdeStore } from "../../stores/ideStore";
import { useTerminalStore } from "../../stores/terminalStore";
import { useAppStore } from "../../stores/appStore";
import { toolCallToCommandEvents } from "../console/consoleAdapters";
import { InteractiveConsole } from "../console/InteractiveConsole";
import { Icon } from "../ui/Icons";
import { EmbeddedCodeView } from "./EmbeddedCodeView";
import { EmbeddedDiffView } from "./EmbeddedDiffView";
import { GitActionsResult, gitActionsView } from "./GitActionsResult";
import {
  ToolArtifactActions,
  ToolWidgetFields,
  ToolWidgetInspector,
  ToolWidgetList,
  ToolWidgetSection,
  ToolWidgetState,
  ToolWidgetTable,
  ToolWidgetTabs,
} from "./ToolWidgetPrimitives";
import {
  adaptEngineeringTool,
  decodeEngineeringOutput,
  engineeringInput,
  isEngineeringToolCall,
  type EngineeringResultV1,
  type EngineeringToolView,
  type EngineeringWidgetKind,
} from "./engineeringToolAdapters";
import { ToolWidgetShell, type ToolWidgetSprite } from "./ToolWidgetShell";

function spriteFor(kind: EngineeringWidgetKind): ToolWidgetSprite {
  if (kind === "shell") return "terminal";
  if (kind === "source-control") return "source-control";
  return "workspace";
}

function iconFor(kind: EngineeringWidgetKind) {
  if (kind === "source-control") return <Icon.Diff size={14} />;
  if (kind === "shell") return <Icon.Server size={14} />;
  if (kind === "diagnostics") return <Icon.Check size={14} />;
  if (kind === "code") return <Icon.History size={14} />;
  return <Icon.File size={14} />;
}

function diffStats(view: EngineeringToolView, tool: ToolCall): React.ReactNode {
  const added =
    tool.diffStat?.added ??
    view.diffFiles?.reduce((total, file) => total + (file.additions ?? 0), 0) ??
    0;
  const removed =
    tool.diffStat?.removed ??
    view.diffFiles?.reduce((total, file) => total + (file.deletions ?? 0), 0) ??
    0;

  if (added + removed <= 0) return null;
  return (
    <span
      className="engineering-widget__diff-stat"
      aria-label="Change statistics"
    >
      <span className="embedded-diff__addition">+{added}</span>
      <span className="embedded-diff__deletion">−{removed}</span>
    </span>
  );
}

async function openFileInIde(path: string, fallbackContent = "") {
  try {
    const { content, revision } = await api.fileContent.get(path);
    useIdeStore.getState().openFile(path, content, revision);
  } catch {
    useIdeStore.getState().openFile(path, fallbackContent);
  }
  window.dispatchEvent(
    new CustomEvent("flamel:open-ide", { detail: { path } }),
  );
}

function EvidenceLog({
  lines,
  label = "Evidence",
}: {
  lines: string[];
  label?: string;
}) {
  if (lines.length === 0) return null;
  return (
    <ol className="engineering-widget__evidence" aria-label={label}>
      {lines.map((line, index) => (
        <li key={`${index}-${line.slice(0, 42)}`}>
          <span aria-hidden="true">{String(index + 1).padStart(2, "0")}</span>
          <code>{line}</code>
        </li>
      ))}
    </ol>
  );
}

function ResultState({ view }: { view: EngineeringToolView }) {
  if (view.status === "running" && !view.outputText) {
    return (
      <ToolWidgetState
        status="running"
        title="Collecting evidence"
        detail="The card will update in place as results arrive."
      />
    );
  }
  if (view.status === "pending") {
    return (
      <ToolWidgetState
        status="pending"
        title="Waiting to start"
        detail="Input is preserved while the provider prepares the call."
      />
    );
  }
  if (view.status === "error" && !view.outputText) {
    return (
      <ToolWidgetState
        status="error"
        title="No diagnostic output returned"
        detail="Inspect the raw result for provider-specific error metadata."
      />
    );
  }
  if (view.status === "empty") {
    return (
      <ToolWidgetState
        status="empty"
        title="No matching evidence"
        detail="The operation completed without a displayable result."
      />
    );
  }
  return null;
}

const MAX_STRUCTURED_ROWS = 32;

type ResultRecord = Record<string, unknown>;

function resultRecord(value: unknown): ResultRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as ResultRecord)
    : undefined;
}

function resultRows(value: unknown): ResultRecord[] {
  return Array.isArray(value)
    ? value
        .map(resultRecord)
        .filter((row): row is ResultRecord => Boolean(row))
        .slice(0, MAX_STRUCTURED_ROWS)
    : [];
}

function resultText(record: ResultRecord | undefined, key: string): string | undefined {
  const value = record?.[key];
  return typeof value === "string" ? value : undefined;
}

function resultNumber(record: ResultRecord | undefined, key: string): number | undefined {
  const value = record?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function resultBaseName(path: string): string {
  return path.replace(/\\/g, "/").split("/").filter(Boolean).pop() ?? path;
}

function ResultList({
  label,
  rows,
}: {
  label: string;
  rows: Array<{
    id: string;
    primary: React.ReactNode;
    secondary?: React.ReactNode;
    trailing?: React.ReactNode;
    level?: number;
  }>;
}) {
  if (rows.length === 0) {
    return <ToolWidgetState status="empty" title={"No " + label.toLowerCase()} />;
  }
  return (
    <ol className="engineering-result-list" aria-label={label}>
      {rows.map((row) => (
        <li key={row.id} style={row.level ? { paddingLeft: 8 + row.level * 14 } : undefined}>
          <span className="engineering-result-list__text">
            <span className="engineering-result-list__primary">{row.primary}</span>
            {row.secondary ? (
              <span className="engineering-result-list__secondary">{row.secondary}</span>
            ) : null}
          </span>
          {row.trailing ? (
            <span className="engineering-result-list__trailing">{row.trailing}</span>
          ) : null}
        </li>
      ))}
    </ol>
  );
}

function SourceFilesResult({ result }: { result: EngineeringResultV1 }) {
  const files = resultRows(result.data.files);
  if (files.length === 0) {
    return <ToolWidgetState status="empty" title="No source returned" />;
  }

  const baseNames = files.map((file, index) =>
    resultBaseName(resultText(file, "path") ?? "file-" + (index + 1)),
  );
  const tabs = files.map((file, index) => {
    const path = resultText(file, "path") ?? "file-" + (index + 1);
    const source = resultText(file, "source") ?? "";
    const language = resultText(file, "language");
    const range = resultRecord(file.resolvedRange);
    const start = resultNumber(range, "startLine");
    const end = resultNumber(range, "endLine");
    const total = resultNumber(file, "totalLines");
    const rangeLabel =
      start !== undefined && end !== undefined
        ? "L" + start + "–" + end + (total !== undefined ? " of " + total : "")
        : total !== undefined
          ? total + " lines"
          : undefined;
    const pathParts = path.replace(/\\/g, "/").split("/").filter(Boolean);
    const duplicateBaseName =
      baseNames.filter((candidate) => candidate === baseNames[index]).length >
      1;
    return {
      id: index + "-" + path,
      label: duplicateBaseName
        ? (pathParts.at(-2) ?? baseNames[index])
        : baseNames[index],
      content: (
        <EmbeddedCodeView
          path={path}
          pathLabel={baseNames[index]}
          detail={rangeLabel}
          value={source}
          language={language}
          label={"Source for " + path}
          minimumHeight={120}
          maximumHeight={240}
          onOpenInIde={() => openFileInIde(path, source)}
        />
      ),
    };
  });

  return (
    <div className="engineering-result__source-files">
      {tabs.length === 1 ? (
        tabs[0].content
      ) : (
        <ToolWidgetTabs tabs={tabs} label="Files read" />
      )}
    </div>
  );
}

function PathResults({ result }: { result: EngineeringResultV1 }) {
  const paths = resultRows(result.data.paths);
  return (
    <ToolWidgetSection
      title="Paths"
      description={
        paths.length +
        " bounded workspace result" +
        (paths.length === 1 ? "" : "s") +
        "."
      }
    >
      <ResultList
        label="Workspace paths"
        rows={paths.map((row, index) => {
          const path =
            resultText(row, "path") ??
            resultText(row, "name") ??
            "Result " + (index + 1);
          const score = resultNumber(row, "score");
          return {
            id: index + "-" + path,
            primary: path,
            secondary: resultText(row, "kind"),
            trailing: score !== undefined ? score.toFixed(2) : undefined,
          };
        })}
      />
    </ToolWidgetSection>
  );
}

function MatchResults({ result }: { result: EngineeringResultV1 }) {
  const matches = resultRows(result.data.matches);
  const groups = new Map<string, ResultRecord[]>();
  for (const match of matches) {
    const path = resultText(match, "path") ?? "Unknown file";
    groups.set(path, [...(groups.get(path) ?? []), match]);
  }

  if (groups.size === 0) {
    return <ToolWidgetState status="empty" title="No code matches" />;
  }

  return (
    <section className="engineering-match-groups" aria-label="Code matches by file">
      {[...groups].map(([path, rows], groupIndex) => (
        <details key={path} open={groupIndex === 0}>
          <summary>
            <span title={path}>{path}</span>
            <span>{rows.length}</span>
          </summary>
          <ol>
            {rows.map((row, index) => {
              const line = resultNumber(row, "line");
              const column = resultNumber(row, "column");
              return (
                <li
                  key={(line ?? index) + "-" + (column ?? 0) + "-" + index}
                  data-role={resultText(row, "role")}
                >
                  <span>
                    {line !== undefined
                      ? "L" + line + (column !== undefined ? ":" + column : "")
                      : "—"}
                  </span>
                  <code>{resultText(row, "text") ?? ""}</code>
                </li>
              );
            })}
          </ol>
        </details>
      ))}
    </section>
  );
}

function flattenSymbols(
  symbols: ResultRecord[],
  level = 0,
  output: Array<{ record: ResultRecord; level: number }> = [],
): Array<{ record: ResultRecord; level: number }> {
  for (const symbol of symbols) {
    if (output.length >= MAX_STRUCTURED_ROWS) break;
    output.push({ record: symbol, level });
    const children = [...resultRows(symbol.children), ...resultRows(symbol.methods)];
    if (children.length > 0 && level < 4) {
      flattenSymbols(children, level + 1, output);
    }
  }
  return output;
}

function SymbolResults({
  result,
  symbols = result.data.symbols,
  title = "Symbol outline",
}: {
  result: EngineeringResultV1;
  symbols?: unknown;
  title?: string;
}) {
  const flattened = flattenSymbols(resultRows(symbols));
  return (
    <ToolWidgetSection
      title={title}
      description={
        flattened.length +
        " bounded symbol" +
        (flattened.length === 1 ? "" : "s") +
        "."
      }
    >
      <ResultList
        label={title}
        rows={flattened.map(({ record, level }, index) => {
          const name = resultText(record, "name") ?? "Symbol " + (index + 1);
          const start = resultNumber(record, "startLine");
          const end = resultNumber(record, "endLine");
          return {
            id: index + "-" + name + "-" + (start ?? 0),
            primary: name,
            secondary: resultText(record, "kind"),
            trailing:
              start !== undefined
                ? "L" +
                  start +
                  (end !== undefined && end !== start ? "–" + end : "")
                : undefined,
            level,
          };
        })}
      />
    </ToolWidgetSection>
  );
}

function ImportResults({ result }: { result: EngineeringResultV1 }) {
  const imports = resultRows(result.data.imports);
  return (
    <ToolWidgetSection
      title="Imports"
      description={
        imports.length +
        " bounded import" +
        (imports.length === 1 ? "" : "s") +
        "."
      }
    >
      <ResultList
        label="Imports"
        rows={imports.map((row, index) => {
          const line = resultNumber(row, "line");
          return {
            id: index + "-" + (line ?? 0),
            primary: resultText(row, "text") ?? "Import " + (index + 1),
            secondary: resultText(row, "kind"),
            trailing: line !== undefined ? "L" + line : undefined,
          };
        })}
      />
    </ToolWidgetSection>
  );
}

function ContextResults({ result }: { result: EngineeringResultV1 }) {
  const files = resultRows(result.data.files);
  const task = typeof result.data.task === "string" ? result.data.task : undefined;
  return (
    <ToolWidgetSection title="Relevant files" description={task}>
      <ResultList
        label="Relevant files"
        rows={files.map((row, index) => {
          const path = resultText(row, "path") ?? "File " + (index + 1);
          const symbols = Array.isArray(row.symbols)
            ? row.symbols
                .slice(0, 5)
                .map((symbol) =>
                  typeof symbol === "string"
                    ? symbol
                    : resultText(resultRecord(symbol), "name"),
                )
                .filter(Boolean)
                .join(" · ")
            : "";
          const score = resultNumber(row, "score");
          return {
            id: index + "-" + path,
            primary: path,
            secondary: symbols || resultText(row, "language"),
            trailing: score !== undefined ? score.toFixed(2) : undefined,
          };
        })}
      />
    </ToolWidgetSection>
  );
}

function DependencyResults({ result }: { result: EngineeringResultV1 }) {
  const directions = [
    { key: "outbound", title: "Uses", rows: resultRows(result.data.outbound) },
    { key: "inbound", title: "Used by", rows: resultRows(result.data.inbound) },
  ];
  return (
    <>
      {directions.map(({ key, title, rows }) => (
        <ToolWidgetSection key={key} title={title}>
          <ResultList
            label={title}
            rows={rows.map((row, index) => {
              const from = resultText(row, "from");
              const to = resultText(row, "to");
              const line = resultNumber(row, "line");
              return {
                id: key + "-" + index + "-" + (from ?? "") + "-" + (to ?? ""),
                primary:
                  key === "outbound"
                    ? to ?? "Unknown dependency"
                    : from ?? "Unknown caller",
                secondary: resultText(row, "kind"),
                trailing: line !== undefined ? "L" + line : undefined,
              };
            })}
          />
        </ToolWidgetSection>
      ))}
    </>
  );
}

function StructuredResultBody({ result }: { result: EngineeringResultV1 }) {
  if (result.status === "error") {
    return (
      <ToolWidgetState
        status="error"
        title={result.summary || "Engineering action failed"}
        detail={result.text || "No diagnostic detail was returned."}
      />
    );
  }

  if (result.tool === "file_read") {
    if (result.action === "read" || result.action === "batch") {
      return <SourceFilesResult result={result} />;
    }
    if (result.action === "summary") {
      return (
        <>
          {typeof result.data.purpose === "string" ? (
            <ToolWidgetSection title="Purpose">
              <p className="engineering-result__purpose">{result.data.purpose}</p>
            </ToolWidgetSection>
          ) : null}
          <ImportResults result={result} />
          <SymbolResults result={result} />
        </>
      );
    }
    return <PathResults result={result} />;
  }

  if (result.action === "search" || result.action === "refs") {
    return <MatchResults result={result} />;
  }
  if (result.action === "symbols") return <SymbolResults result={result} />;
  if (result.action === "find") {
    const path = resultText(result.data, "path") ?? "source";
    const source = resultText(result.data, "source") ?? "";
    return (
      <>
        <EmbeddedCodeView
          path={path}
          value={source}
          language={resultText(result.data, "language")}
          label={"Source for " + path}
          minimumHeight={120}
          maximumHeight={420}
          onOpenInIde={() => openFileInIde(path, source)}
        />
        {resultRows(result.data.children).length > 0 ? (
          <SymbolResults
            result={result}
            symbols={result.data.children}
            title="Child symbols"
          />
        ) : null}
      </>
    );
  }
  if (result.action === "imports") return <ImportResults result={result} />;
  if (result.action === "context") return <ContextResults result={result} />;
  if (result.action === "deps") return <DependencyResults result={result} />;
  return <ToolWidgetState status="empty" title="Unsupported engineering result" />;
}

function StructuredResultNotices({ view }: { view: EngineeringToolView }) {
  const warnings = view.structuredResult?.warnings ?? [];
  if (!view.truncated && warnings.length === 0) return null;
  return (
    <ToolWidgetSection
      title={view.truncated ? "Preview clipped" : "Warnings"}
      description={
        view.truncated
          ? "This card keeps bounded evidence. Use the artifact for the complete result."
          : undefined
      }
    >
      {warnings.length > 0 ? (
        <ul className="engineering-result__warnings">
          {warnings.map((warning, index) => (
            <li key={index + "-" + warning}>{warning}</li>
          ))}
        </ul>
      ) : null}
      {view.truncated ? (
        view.artifacts.length > 0 ? (
          <ToolArtifactActions artifacts={view.artifacts} />
        ) : (
          <ToolWidgetState
            status="empty"
            title="Full result unavailable"
            detail="The bounded evidence remains usable, but this result has no valid artifact reference."
          />
        )
      ) : null}
    </ToolWidgetSection>
  );
}

function WorkspaceBody({ view }: { view: EngineeringToolView }) {
  const boundedResult = view.truncated ? (
    <ToolWidgetSection
      title="Preview clipped"
      description="This card keeps a bounded source preview. Use the artifact for the complete result."
    >
      {view.artifacts.length ? (
        <ToolArtifactActions artifacts={view.artifacts} />
      ) : (
        <ToolWidgetState
          status="empty"
          title="Full result unavailable"
          detail="The preserved preview remains usable, but this result has no valid artifact reference."
        />
      )}
    </ToolWidgetSection>
  ) : null;

  if (view.structuredResult) {
    return (
      <>
        {view.fields.length ? <ToolWidgetFields fields={view.fields} /> : null}
        <StructuredResultBody result={view.structuredResult} />
        <StructuredResultNotices view={view} />
      </>
    );
  }

  if (view.diffFiles?.length || view.diffPatch) {
    return (
      <>
        <EmbeddedDiffView
          files={view.diffFiles}
          patch={view.diffPatch}
          onOpenInIde={(file) =>
            openFileInIde(
              file.path,
              file.modified ?? file.original ?? file.patch ?? "",
            )
          }
        />
        {boundedResult}
      </>
    );
  }

  if (view.code && view.path) {
    return (
      <>
        <EmbeddedCodeView
          path={view.path}
          value={view.code}
          onOpenInIde={() => openFileInIde(view.path!, view.code ?? "")}
        />
        {boundedResult}
      </>
    );
  }

  return (
    <>
      {view.fields.length ? <ToolWidgetFields fields={view.fields} /> : null}
      {view.entries.length ? (
        <ToolWidgetList
          items={view.entries.map((entry) => ({
            id: entry.id,
            primary: entry.primary,
            secondary: entry.secondary,
            trailing: entry.trailing,
          }))}
        />
      ) : null}
      <ResultState view={view} />
      {boundedResult}
      {view.status === "error" ? (
        <EvidenceLog lines={view.logLines} label="Workspace error evidence" />
      ) : null}
    </>
  );
}

function CodeBody({ view }: { view: EngineeringToolView }) {
  if (view.structuredResult) {
    return (
      <>
        {view.fields.length ? <ToolWidgetFields fields={view.fields} /> : null}
        <StructuredResultBody result={view.structuredResult} />
        <StructuredResultNotices view={view} />
      </>
    );
  }

  return (
    <>
      {view.fields.length ? <ToolWidgetFields fields={view.fields} /> : null}
      {view.code && view.path ? (
        <EmbeddedCodeView
          path={view.path}
          value={view.code}
          label={`Source for ${view.path}`}
          maximumHeight={420}
          onOpenInIde={() => openFileInIde(view.path!, view.code ?? "")}
        />
      ) : null}
      {view.entries.length ? (
        <ToolWidgetSection
          title="Evidence"
          description="Bounded matches from the requested project scope."
        >
          <ToolWidgetList
            items={view.entries.map((entry) => ({
              id: entry.id,
              primary: entry.primary,
              secondary: entry.secondary,
              trailing: entry.trailing,
            }))}
          />
        </ToolWidgetSection>
      ) : null}
      <ResultState view={view} />
      {!view.entries.length && view.outputText ? (
        <EvidenceLog lines={view.logLines} label="Code navigation evidence" />
      ) : null}
    </>
  );
}

function CheckTable({
  checks,
  caption,
}: {
  checks: EngineeringToolView["checks"];
  caption: string;
}) {
  const rows = checks.map((check) => ({
    id: check.id,
    check: check.name,
    status: (
      <span className="engineering-widget__check" data-status={check.status}>
        <span aria-hidden="true" />
        {check.status}
      </span>
    ),
    duration: check.duration ?? "—",
  }));
  if (!rows.length) return null;

  return (
    <ToolWidgetTable
      caption={caption}
      columns={[
        { key: "check", label: "Check" },
        { key: "status", label: "Status" },
        { key: "duration", label: "Duration", align: "right" },
      ]}
      rows={rows}
    />
  );
}

function SourceControlBody({ view, tool }: { view: EngineeringToolView; tool: ToolCall }) {
  if (view.diffPatch) {
    return <EmbeddedDiffView patch={view.diffPatch} />;
  }
  const actions = gitActionsView(view.outputText, view.action);
  if (actions) {
    const input = engineeringInput(tool);
    const baseUrl = [input.server_url, input.base_url, input.forge_url].find(value => typeof value === 'string');
    return <GitActionsResult result={actions} action={view.action} baseUrl={typeof baseUrl === 'string' ? baseUrl : undefined} />;
  }

  return (
    <>
      {view.fields.length ? <ToolWidgetFields fields={view.fields} /> : null}
      {view.entries.length ? (
        <ToolWidgetList
          items={view.entries.map((entry) => ({
            id: entry.id,
            primary: entry.primary,
            secondary: entry.secondary,
            trailing: entry.trailing,
          }))}
        />
      ) : null}
      <CheckTable checks={view.checks} caption="Pull request checks" />
      <ResultState view={view} />
      {!view.entries.length &&
      !view.checks.length &&
      view.outputText &&
      view.status !== "empty" ? (
        <EvidenceLog lines={view.logLines} label="Source-control evidence" />
      ) : null}
    </>
  );
}

function DiagnosticsBody({ view }: { view: EngineeringToolView }) {
  return (
    <>
      {view.fields.length ? <ToolWidgetFields fields={view.fields} /> : null}
      <CheckTable checks={view.checks} caption="Verification checks" />
      <ResultState view={view} />
      {view.logLines.length ? (
        <ToolWidgetSection
          title={view.status === "error" ? "Failure evidence" : "Bounded log"}
          description="The complete provider payload remains available under raw data."
        >
          <EvidenceLog lines={view.logLines} label="Verification log" />
        </ToolWidgetSection>
      ) : null}
    </>
  );
}

export function selectEngineeringConsoleEvents(
  tool: ToolCall,
  syntheticEvents: CommandEvent[],
): CommandEvent[] {
  const live = tool.commandEvents ?? [];
  const liveHasOutput = live.some(
    (event) =>
      event.kind === "output_chunk" ||
      event.kind === "input_sent" ||
      event.kind === "error",
  );
  return live.length > 0 &&
    (tool.status === "running" || tool.status === "pending" || liveHasOutput)
    ? live
    : syntheticEvents;
}

function ShellBody({
  tool,
  view,
}: {
  tool: ToolCall;
  view: EngineeringToolView;
}) {
  const syntheticEvents = useMemo(
    () =>
      toolCallToCommandEvents(tool, {
        title: view.command || "Shell",
        source: "chat",
      }),
    [tool, view.command],
  );
  const events = useMemo(
    () => selectEngineeringConsoleEvents(tool, syntheticEvents),
    [syntheticEvents, tool],
  );
  const terminalSession = events.flatMap(event => [event.session, event.run, event.block])
    .find(item => typeof item?.metadata?.terminal_session_id === "string");
  const shellResult = resultRecord(decodeEngineeringOutput(tool.presentation?.text ?? tool.output).parsed);
  // Only the supervised shell handle can stand in for missing command events.
  // A legacy MCP session id does not identify an interactive terminal.
  const resultTerminalId = typeof shellResult?.terminal_session_id === "string"
    && shellResult.session_id === `pty:${shellResult.terminal_session_id}`
    && typeof shellResult.command_session_id === "string"
    ? shellResult.terminal_session_id : undefined;
  const terminalSessionId = terminalSession?.metadata?.terminal_session_id || resultTerminalId;
  const projectPath = events.flatMap(event => [event.session, event.run])
    .map(item => item?.cwd).find(path => typeof path === "string")
    || (resultTerminalId && typeof shellResult?.cwd === "string" ? shellResult.cwd : "");

  return (
    <div className="engineering-widget__console">
      {typeof terminalSessionId === "string" && terminalSessionId && (
        <button type="button" className="engineering-widget__terminal-attach" onClick={() => {
          useTerminalStore.getState().attachTerminal(terminalSessionId, projectPath, view.command || "Shell");
          useAppStore.getState().setCenterView("terminal");
        }}>Open terminal</button>
      )}
      <InteractiveConsole
        events={events}
        connected={tool.status !== "error"}
        title={tool.status === "error" ? "Shell error" : "Shell output"}
        height={220}
        compact
        flattenSingleBlock
        commandFirst
        promptCommand={view.command}
      />
    </div>
  );
}

function widgetBody(tool: ToolCall, view: EngineeringToolView) {
  if (view.kind === "shell") return <ShellBody tool={tool} view={view} />;
  if (view.kind === "workspace") return <WorkspaceBody view={view} />;
  if (view.kind === "source-control") return <SourceControlBody view={view} tool={tool} />;
  if (view.kind === "diagnostics") return <DiagnosticsBody view={view} />;
  return <CodeBody view={view} />;
}

export { isEngineeringToolCall };

export function EngineeringToolWidget({
  tool,
  defaultExpanded: defaultExpandedOverride,
}: {
  tool: ToolCall;
  defaultExpanded?: boolean;
}) {
  const view = useMemo(() => adaptEngineeringTool(tool), [tool]);
  if (!view) return null;

  const defaultExpanded = defaultExpandedOverride ?? false;
  const checkSummary =
    view.kind === "diagnostics" && view.checks.length
      ? `${view.checks.filter((check) => check.status === "passed").length}/${view.checks.length}`
      : null;
  const trailing = diffStats(view, tool) ?? checkSummary ?? null;

  return (
    <ToolWidgetShell
      title={view.title}
      eyebrow={view.eyebrow}
      summary={view.summary}
      detail={view.detail}
      status={view.status}
      statusLabel={
        view.kind === "shell" && view.status === "running"
          ? "Live"
          : view.statusLabel
      }
      icon={iconFor(view.kind)}
      sprite={spriteFor(view.kind)}
      trailing={trailing}
      defaultExpanded={defaultExpanded}
      inspector={
        <ToolWidgetInspector
          input={engineeringInput(tool)}
          output={tool.output}
          raw={{
            name: tool.name,
            status: tool.status,
            partialInput: tool.partialInput,
            artifacts: tool.artifacts,
            presentation: tool.presentation,
          }}
        />
      }
    >
      {widgetBody(tool, view)}
    </ToolWidgetShell>
  );
}
