import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { Icon } from '../ui/Icons';
import { TypedRevealText } from '../ui/TypedReveal';
import type { CommandEvent, CommandLifecycleStatus, CommandStreamKind } from '../../lib/consoleTypes';
import { buildConsoleBlocks, commandBlocksToText, consoleOutputLineCount, plainConsoleText, type ConsoleBlockView, type ConsoleLineView } from './consoleAdapters';

interface InteractiveConsoleProps {
  events: CommandEvent[];
  title?: string;
  connected?: boolean;
  height?: number | string;
  compact?: boolean;
  flattenSingleBlock?: boolean;
  commandFirst?: boolean;
  promptCommand?: string;
  onClear?: () => void;
  onInputSubmit?: (input: string) => void | Promise<void>;
}

type StreamFilter = 'all' | CommandStreamKind;

const STREAM_FILTERS: StreamFilter[] = ['all', 'stdout', 'stderr', 'pty', 'system', 'stdin'];
const MAX_RENDERED_LINES_PER_BLOCK = 500;
const CONSOLE_PREFS_KEY = 'flamel.console.preferences.v1';
const CONSOLE_INPUT_HISTORY_KEY = 'flamel.console.inputHistory.v1';
const MAX_INPUT_HISTORY = 80;

interface ConsolePreferences {
  streamFilter?: StreamFilter;
  autoScroll?: boolean;
}

function isStreamFilter(value: unknown): value is StreamFilter {
  return typeof value === 'string' && (STREAM_FILTERS as readonly string[]).includes(value);
}

function readJson<T>(key: string, fallback: T): T {
  try {
    if (typeof localStorage === 'undefined') return fallback;
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) as T : fallback;
  } catch {
    return fallback;
  }
}

function writeJson(key: string, value: unknown) {
  try {
    if (typeof localStorage === 'undefined') return;
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Console preferences are convenience state; quota or privacy errors should never break rendering.
  }
}

function loadConsolePreferences(): ConsolePreferences {
  const prefs = readJson<ConsolePreferences>(CONSOLE_PREFS_KEY, {});
  return {
    streamFilter: isStreamFilter(prefs.streamFilter) ? prefs.streamFilter : undefined,
    autoScroll: typeof prefs.autoScroll === 'boolean' ? prefs.autoScroll : undefined,
  };
}

function loadInputHistory(): string[] {
  // Old releases persisted stdin, which can contain passwords. Clear that
  // legacy key and keep any convenience history in this component only.
  try { localStorage.removeItem(CONSOLE_INPUT_HISTORY_KEY); } catch { /* storage may be unavailable */ }
  return [];
}

export function InteractiveConsole({
  events,
  title = 'Console',
  connected = false,
  height = 240,
  compact = false,
  flattenSingleBlock = false,
  commandFirst = false,
  promptCommand,
  onClear,
  onInputSubmit,
}: InteractiveConsoleProps) {
  const [query, setQuery] = useState('');
  const [streamFilter, setStreamFilter] = useState<StreamFilter>(() => loadConsolePreferences().streamFilter ?? 'all');
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const [autoScroll, setAutoScroll] = useState(() => loadConsolePreferences().autoScroll ?? true);
  const [input, setInput] = useState('');
  const [inputHistory, setInputHistory] = useState<string[]>(loadInputHistory);
  const [historyIndex, setHistoryIndex] = useState<number | null>(null);
  const [copied, setCopied] = useState(false);
  const [outputExpanded, setOutputExpanded] = useState(!commandFirst);
  const scrollRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const blocks = useMemo(() => buildConsoleBlocks(events), [events]);
  const visibleBlocks = useMemo(
    () => filterBlocks(blocks, query, streamFilter),
    [blocks, query, streamFilter],
  );
  const visibleLineCount = visibleBlocks.reduce((sum, block) => sum + consoleOutputLineCount(block.lines), 0);
  const totalLineCount = blocks.reduce((sum, block) => sum + consoleOutputLineCount(block.lines), 0);
  const hasActiveFilter = query.trim().length > 0 || streamFilter !== 'all';
  const counterText = hasActiveFilter
    ? `${visibleLineCount}/${totalLineCount} lines`
    : `${totalLineCount} lines`;
  const flattenedBlock = flattenSingleBlock && visibleBlocks.length === 1 ? visibleBlocks[0] : undefined;
  const bodyCollapsed = !!flattenedBlock && collapsed.has(flattenedBlock.id);
  const headerRevealActive = connected && events.length > 0;

  useEffect(() => {
    writeJson(CONSOLE_PREFS_KEY, { streamFilter, autoScroll });
  }, [autoScroll, streamFilter]);

  useEffect(() => {
    if (!autoScroll) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [autoScroll, visibleLineCount, visibleBlocks.length]);

  function handleScroll() {
    const el = scrollRef.current;
    if (!el) return;
    setAutoScroll(el.scrollHeight - el.scrollTop - el.clientHeight < 28);
  }

  function toggleBlock(id: string) {
    setCollapsed(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function collapseVisibleBlocks() {
    setCollapsed(new Set(visibleBlocks.map(block => block.id)));
  }

  function expandAllBlocks() {
    setCollapsed(new Set());
  }

  function copyVisible() {
    const text = commandBlocksToText(visibleBlocks);
    if (!text) return;
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    }).catch(() => {});
  }

  function rememberInput(value: string) {
    setInputHistory(prev => {
      const next = [value, ...prev.filter(item => item !== value)].slice(0, MAX_INPUT_HISTORY);
      return next;
    });
  }

  async function submitInput(event: FormEvent) {
    event.preventDefault();
    const value = input;
    if (!value.trim() || !onInputSubmit) return;
    setInput('');
    setHistoryIndex(null);
    rememberInput(value);
    await onInputSubmit(value);
  }

  function handleConsoleKeyDown(event: KeyboardEvent<HTMLElement>) {
    const key = event.key.toLowerCase();
    if ((event.metaKey || event.ctrlKey) && key === 'f') {
      event.preventDefault();
      searchRef.current?.focus();
      searchRef.current?.select();
      return;
    }
    if (event.key === 'Escape' && query) {
      event.preventDefault();
      setQuery('');
    }
  }

  function handleInputKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (!inputHistory.length) return;
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      setHistoryIndex(prev => {
        const next = prev === null ? 0 : Math.min(prev + 1, inputHistory.length - 1);
        setInput(inputHistory[next] || '');
        return next;
      });
      return;
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setHistoryIndex(prev => {
        if (prev === null || prev <= 0) {
          setInput('');
          return null;
        }
        const next = prev - 1;
        setInput(inputHistory[next] || '');
        return next;
      });
    }
  }

  const buttonStyle = (active = false): CSSProperties => ({
    minHeight: 22,
    padding: '2px 7px',
    borderRadius: 5,
    border: '1px solid var(--border-soft)',
    background: active ? 'var(--accent-glow)' : 'var(--surface-2)',
    color: active ? 'var(--accent)' : 'var(--text-muted)',
    fontSize: 10,
    fontWeight: active ? 600 : 500,
    cursor: 'pointer',
    flexShrink: 0,
  });

  if (commandFirst && promptCommand) {
    return (
      <CompactCommandConsole
        command={promptCommand}
        blocks={visibleBlocks}
        connected={connected}
        expanded={outputExpanded}
        onToggle={() => setOutputExpanded(expanded => !expanded)}
        query={query}
        onQueryChange={setQuery}
        streamFilter={streamFilter}
        onStreamFilterChange={setStreamFilter}
        onCopy={copyVisible}
        copied={copied}
        height={height}
        hasActiveFilter={hasActiveFilter}
        eventCount={events.length}
      />
    );
  }

  return (
    <section
      onKeyDown={handleConsoleKeyDown}
      style={{
        marginTop: compact ? 6 : 8,
        border: '1px solid var(--border-soft)',
        borderRadius: 7,
        background: 'var(--surface-2)',
        overflow: 'hidden',
        minWidth: 0,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'flex-start',
          justifyContent: 'space-between',
          flexWrap: 'wrap',
          gap: 8,
          padding: compact ? '6px 8px' : '7px 9px',
          borderBottom: bodyCollapsed ? 'none' : '1px solid var(--border-soft)',
          minWidth: 0,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 7, minWidth: 0, maxWidth: '100%', flex: '1 1 220px' }}>
          <span
            title={connected ? 'Connected' : 'Reconnecting'}
            style={{
              width: 7,
              height: 7,
              borderRadius: '50%',
              flexShrink: 0,
              background: connected ? 'var(--success, #22c55e)' : 'var(--text-dim)',
              boxShadow: connected ? '0 0 7px rgba(34,197,94,0.45)' : 'none',
            }}
          />
          <strong
            style={{
              color: 'var(--text)',
              fontSize: compact ? 11 : 12,
              fontWeight: 650,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              minWidth: 72,
              flex: '1 1 auto',
            }}
          >
            <TypedRevealText text={title} active={headerRevealActive} charsPerSecond={520} maxStep={24} snapBacklog={600} />
          </strong>
          <span style={{ color: 'var(--text-dim)', fontSize: 10, whiteSpace: 'nowrap' }}>
            {counterText}
          </span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 4, flexWrap: 'wrap', justifyContent: 'flex-end', minWidth: 0, flex: '0 1 auto' }}>
          <input
            ref={searchRef}
            value={query}
            onChange={event => setQuery(event.target.value)}
            placeholder="Search"
            aria-label="Search console output"
            title="Search console output"
            style={{
              width: compact ? 92 : 'clamp(84px, 20vw, 128px)',
              minWidth: 72,
              maxWidth: '100%',
              height: 22,
              borderRadius: 5,
              border: '1px solid var(--border-soft)',
              background: 'var(--surface)',
              color: 'var(--text)',
              padding: '0 7px',
              fontSize: 10,
              outline: 'none',
            }}
          />
          {STREAM_FILTERS.map(filter => (
            <button
              key={filter}
              type="button"
              onClick={() => setStreamFilter(filter)}
              style={buttonStyle(streamFilter === filter)}
            >
              {filter}
            </button>
          ))}
          <IconButton title="Collapse visible blocks" onClick={collapseVisibleBlocks}>
            <Icon.ChevronRight size={12} />
          </IconButton>
          <IconButton title="Expand all blocks" onClick={expandAllBlocks}>
            <Icon.ChevronDown size={12} />
          </IconButton>
          <IconButton title={copied ? 'Copied visible output' : 'Copy visible output'} onClick={copyVisible}>
            {copied ? <Icon.Check size={12} /> : <Icon.Copy size={12} />}
          </IconButton>
          {onClear && (
            <IconButton title="Clear local output" onClick={onClear}>
              <Icon.Trash size={12} />
            </IconButton>
          )}
        </div>
      </div>

      {!bodyCollapsed && (
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          style={{
            height,
            overflow: 'auto',
            background: 'var(--surface)',
            color: 'var(--text-muted)',
            fontFamily: 'var(--font-mono)',
            fontSize: compact ? 10 : 11,
            lineHeight: 1.45,
            minWidth: 0,
          }}
        >
          {promptCommand && (
            <TerminalPromptLine command={promptCommand} />
          )}
          {visibleBlocks.length === 0 ? (
            <div style={{ padding: promptCommand ? '4px 12px 10px 28px' : '10px 12px', color: 'var(--text-dim)', fontFamily: 'var(--font-sans)', fontSize: 12 }}>
              <TypedRevealText
                text={emptyStateText(events.length, connected, hasActiveFilter)}
                active={connected || hasActiveFilter}
                charsPerSecond={640}
                maxStep={32}
                snapBacklog={900}
              />
            </div>
          ) : flattenedBlock ? (
            <ConsoleBlockBody block={flattenedBlock} query={query} />
          ) : (
            visibleBlocks.map(block => (
              <ConsoleBlock
                key={block.id}
                block={block}
                collapsed={collapsed.has(block.id)}
                onToggle={() => toggleBlock(block.id)}
                query={query}
              />
            ))
          )}
        </div>
      )}

      {!bodyCollapsed && !autoScroll && (
        <button
          type="button"
          onClick={() => setAutoScroll(true)}
          style={{
            width: '100%',
            padding: '5px 8px',
            border: 'none',
            borderTop: '1px solid var(--border-soft)',
            background: 'var(--surface-2)',
            color: 'var(--accent)',
            fontSize: 10,
            cursor: 'pointer',
          }}
        >
          Jump to latest
        </button>
      )}

      {!bodyCollapsed && onInputSubmit && (
        <form
          onSubmit={submitInput}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            padding: '6px 8px',
            borderTop: '1px solid var(--border-soft)',
            background: 'var(--surface-2)',
          }}
        >
          <span style={{ color: 'var(--text-dim)', fontFamily: 'var(--font-mono)', fontSize: 12 }}>$</span>
          <input
            value={input}
            onChange={event => {
              setInput(event.target.value);
              setHistoryIndex(null);
            }}
            onKeyDown={handleInputKeyDown}
            style={{
              flex: 1,
              minWidth: 0,
              border: 'none',
              outline: 'none',
              background: 'transparent',
              color: 'var(--text)',
              fontFamily: 'var(--font-mono)',
              fontSize: 12,
            }}
          />
          <IconButton title="Send input" type="submit">
            <Icon.Send size={12} />
          </IconButton>
        </form>
      )}
    </section>
  );
}

function CompactCommandConsole({
  command,
  blocks,
  connected,
  expanded,
  onToggle,
  query,
  onQueryChange,
  streamFilter,
  onStreamFilterChange,
  onCopy,
  copied,
  height,
  hasActiveFilter,
  eventCount,
}: {
  command: string;
  blocks: ConsoleBlockView[];
  connected: boolean;
  expanded: boolean;
  onToggle: () => void;
  query: string;
  onQueryChange: (value: string) => void;
  streamFilter: StreamFilter;
  onStreamFilterChange: (value: StreamFilter) => void;
  onCopy: () => void;
  copied: boolean;
  height: number | string;
  hasActiveFilter: boolean;
  eventCount: number;
}) {
  const singleBlock = blocks.length === 1 ? blocks[0] : undefined;
  const status = blocks.find(block => block.status === 'failed' || block.status === 'timed_out')?.status
    ?? blocks.find(block => block.status === 'running' || block.status === 'waiting_for_input')?.status
    ?? blocks.at(-1)?.status
    ?? (connected ? 'running' : 'pending');
  const lineCount = blocks.reduce((sum, block) => sum + consoleOutputLineCount(block.lines), 0);
  const metadata = [
    `${lineCount} ${lineCount === 1 ? 'line' : 'lines'}`,
    typeof singleBlock?.exitCode === 'number' ? `exit ${singleBlock.exitCode}` : null,
  ].filter(Boolean).join(' · ');

  return (
    <section
      style={{
        marginTop: 4,
        border: '1px solid var(--border-soft)',
        borderRadius: 6,
        background: 'var(--surface)',
        overflow: 'hidden',
        minWidth: 0,
      }}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={expanded}
        title={expanded ? 'Hide command output' : 'Show command output'}
        style={{
          width: '100%',
          display: 'grid',
          gridTemplateColumns: '12px minmax(0, 1fr) auto 14px',
          alignItems: 'start',
          gap: 8,
          padding: '7px 9px',
          border: 'none',
          background: expanded ? 'var(--surface-2)' : 'transparent',
          color: 'var(--text)',
          cursor: 'pointer',
          textAlign: 'left',
          minWidth: 0,
        }}
      >
        <span style={{ paddingTop: 4 }}><StatusPill status={status} /></span>
        <code
          title={command}
          style={{
            display: '-webkit-box',
            WebkitBoxOrient: 'vertical',
            WebkitLineClamp: 3,
            overflow: 'hidden',
            whiteSpace: 'pre-wrap',
            overflowWrap: 'anywhere',
            color: 'var(--text)',
            fontFamily: 'var(--font-mono)',
            fontSize: 12,
            lineHeight: 1.45,
            letterSpacing: 0,
            minWidth: 0,
          }}
        >
          <span style={{ color: 'var(--accent)', fontWeight: 700 }}>$ </span>{command}
        </code>
        <span style={{ color: 'var(--text-dim)', fontSize: 10, whiteSpace: 'nowrap', paddingTop: 2 }}>
          {metadata}
        </span>
        <span style={{ display: 'flex', color: 'var(--text-dim)', paddingTop: 2 }}>
          {expanded ? <Icon.ChevronDown size={13} /> : <Icon.ChevronRight size={13} />}
        </span>
      </button>

      {expanded && (
        <>
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              padding: '5px 8px',
              borderTop: '1px solid var(--border-soft)',
              borderBottom: '1px solid var(--border-soft)',
              background: 'var(--surface-2)',
              minWidth: 0,
            }}
          >
            <input
              value={query}
              onChange={event => onQueryChange(event.target.value)}
              placeholder="Search output"
              aria-label="Search command output"
              style={{
                flex: '1 1 140px',
                minWidth: 80,
                height: 24,
                borderRadius: 4,
                border: '1px solid var(--border-soft)',
                background: 'var(--surface)',
                color: 'var(--text)',
                padding: '0 7px',
                fontSize: 10,
                outline: 'none',
              }}
            />
            <select
              value={streamFilter}
              onChange={event => onStreamFilterChange(event.target.value as StreamFilter)}
              aria-label="Filter command output stream"
              title="Filter output stream"
              style={{
                height: 24,
                borderRadius: 4,
                border: '1px solid var(--border-soft)',
                background: 'var(--surface)',
                color: 'var(--text-muted)',
                padding: '0 5px',
                fontSize: 10,
              }}
            >
              {STREAM_FILTERS.map(filter => <option key={filter} value={filter}>{filter}</option>)}
            </select>
            <IconButton title={copied ? 'Copied output' : 'Copy output'} onClick={onCopy}>
              {copied ? <Icon.Check size={12} /> : <Icon.Copy size={12} />}
            </IconButton>
          </div>
          <div
            style={{
              height,
              overflow: 'auto',
              background: 'var(--surface)',
              color: 'var(--text-muted)',
              fontFamily: 'var(--font-mono)',
              fontSize: 10,
              lineHeight: 1.45,
              minWidth: 0,
            }}
          >
            {blocks.length > 0 ? (
              blocks.map(block => (
                <div key={block.id} style={{ borderBottom: blocks.length > 1 ? '1px solid var(--border-soft)' : undefined }}>
                  {blocks.length > 1 && (
                    <div style={{ padding: '5px 9px 3px', color: 'var(--text-dim)', fontSize: 10, overflowWrap: 'anywhere' }}>
                      {block.command || block.title}
                    </div>
                  )}
                  <ConsoleBlockBody block={block} query={query} />
                </div>
              ))
            ) : (
              <div style={{ padding: '10px 12px', color: 'var(--text-dim)', fontFamily: 'var(--font-sans)', fontSize: 12 }}>
                {emptyStateText(eventCount, connected, hasActiveFilter)}
              </div>
            )}
          </div>
        </>
      )}
    </section>
  );
}

function ConsoleBlock({
  block,
  collapsed,
  onToggle,
  query,
}: {
  block: ConsoleBlockView;
  collapsed: boolean;
  onToggle: () => void;
  query: string;
}) {
  const revealAnnotations = block.status === 'pending' || block.status === 'running' || block.status === 'waiting_for_input';

  return (
    <div style={{ borderBottom: '1px solid var(--border-soft)' }}>
      <button
        type="button"
        onClick={onToggle}
        style={{
          width: '100%',
          display: 'grid',
          gridTemplateColumns: '14px minmax(0, 1fr) auto',
          alignItems: 'center',
          gap: 6,
          padding: '5px 8px',
          border: 'none',
          background: 'rgba(255,255,255,0.018)',
          color: 'var(--text-muted)',
          cursor: 'pointer',
          textAlign: 'left',
        }}
      >
        <span style={{ display: 'flex', color: 'var(--text-dim)' }}>
          {collapsed ? <Icon.ChevronRight size={11} /> : <Icon.ChevronDown size={11} />}
        </span>
        <span style={{ minWidth: 0, display: 'flex', alignItems: 'center', gap: 6, overflow: 'hidden' }}>
          <StatusPill status={block.status} />
          <span style={{ color: 'var(--text)', fontSize: 12, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 48, flex: '1 1 auto' }}>
            <TypedRevealText text={block.title} active={revealAnnotations} charsPerSecond={520} maxStep={24} snapBacklog={600} />
          </span>
          <span style={{ color: 'var(--text-dim)', fontSize: 10, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0, flex: '0 1 auto' }}>
            <TypedRevealText text={block.source} active={revealAnnotations} charsPerSecond={520} maxStep={24} snapBacklog={600} />
          </span>
        </span>
        <span style={{ color: 'var(--text-dim)', fontSize: 10, whiteSpace: 'nowrap' }}>
          {consoleOutputLineCount(block.lines)} lines{typeof block.exitCode === 'number' ? ` · ${block.exitCode}` : ''}
        </span>
      </button>

      {!collapsed && <ConsoleBlockBody block={block} query={query} />}
    </div>
  );
}

function ConsoleBlockBody({ block, query }: { block: ConsoleBlockView; query: string }) {
  const queryActive = query.trim().length > 0;
  const shouldWindowLines = !queryActive && block.lines.length > MAX_RENDERED_LINES_PER_BLOCK;
  const visibleLines = shouldWindowLines ? block.lines.slice(-MAX_RENDERED_LINES_PER_BLOCK) : block.lines;
  const hiddenLineCount = shouldWindowLines ? block.lines.length - visibleLines.length : 0;

  return (
    <div style={{ padding: '4px 0 5px' }}>
      {block.error && (
        <div style={{ padding: '2px 8px 4px 28px', color: 'var(--error, #f87171)', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
          <TypedRevealText text={block.error} active charsPerSecond={680} maxStep={36} snapBacklog={900} />
        </div>
      )}
      {hiddenLineCount > 0 && (
        <div style={{ padding: '2px 8px 4px 28px', color: 'var(--text-dim)', fontFamily: 'var(--font-sans)', fontSize: 10 }}>
          <TypedRevealText text={`${hiddenLineCount} earlier lines clipped locally`} active charsPerSecond={640} maxStep={32} snapBacklog={900} />
        </div>
      )}
      {visibleLines.map(line => (
        <ConsoleLine key={line.id} line={line} query={query} />
      ))}
      {block.inputRequests.map(input => (
        <div key={input.id} style={{ padding: '2px 8px 2px 28px', color: 'var(--warning, #eab308)', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
          <TypedRevealText text={input.prompt || 'Waiting for input'} active charsPerSecond={680} maxStep={36} snapBacklog={900} />
        </div>
      ))}
    </div>
  );
}

function TerminalPromptLine({ command }: { command: string }) {
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '28px minmax(0, 1fr)',
        gap: 8,
        padding: '9px 12px 7px',
        borderBottom: '1px solid var(--border-soft)',
        color: 'var(--text)',
        minWidth: 0,
      }}
    >
      <span style={{ color: 'var(--accent)', fontWeight: 800, whiteSpace: 'nowrap' }}>$&gt;</span>
      <span style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word', minWidth: 0 }}>
        {command}
      </span>
    </div>
  );
}

function ConsoleLine({ line, query }: { line: ConsoleLineView; query: string }) {
  const time = line.timestamp
    ? new Date(line.timestamp).toLocaleTimeString([], { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })
    : '';
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '54px 44px minmax(0, 1fr)',
        gap: 7,
        padding: '1px 8px',
        color: streamColor(line.stream),
        minWidth: 0,
      }}
    >
      <span style={{ color: 'var(--text-dim)', whiteSpace: 'nowrap' }}>{time}</span>
      <span style={{ color: streamColor(line.stream), whiteSpace: 'nowrap', opacity: 0.85 }}>{line.stream}</span>
      <span style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word', minWidth: 0 }}>
        <AnsiText text={line.text} highlight={query} />
      </span>
    </div>
  );
}

function IconButton({
  children,
  onClick,
  title,
  type = 'button',
}: {
  children: ReactNode;
  onClick?: () => void;
  title: string;
  type?: 'button' | 'submit';
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      title={title}
      style={{
        width: 24,
        height: 22,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        border: 'none',
        borderRadius: 5,
        background: 'transparent',
        color: 'var(--text-dim)',
        cursor: 'pointer',
      }}
    >
      {children}
    </button>
  );
}

function StatusPill({ status }: { status: CommandLifecycleStatus }) {
  return (
    <span
      aria-label={status === 'unknown' ? 'Completion not confirmed' : status.replaceAll('_', ' ')}
      title={status === 'unknown' ? 'Completion not confirmed' : status.replaceAll('_', ' ')}
      style={{
        width: 7,
        height: 7,
        borderRadius: '50%',
        flexShrink: 0,
        background: statusColor(status),
        boxShadow: status === 'running' ? `0 0 6px ${statusColor(status)}` : 'none',
      }}
    />
  );
}

function emptyStateText(eventCount: number, connected: boolean, hasActiveFilter: boolean): string {
  if (eventCount === 0) return connected ? 'Waiting for command output.' : 'Connecting to console stream...';
  if (hasActiveFilter) return 'No output matches the current filters.';
  return connected ? 'Waiting for command output.' : 'No visible console output.';
}

function filterBlocks(blocks: ConsoleBlockView[], query: string, stream: StreamFilter): ConsoleBlockView[] {
  const q = query.trim().toLowerCase();
  return blocks
    .map(block => {
      const lines = block.lines.filter(line => {
        const streamMatches = stream === 'all' || line.stream === stream;
        const queryMatches = !q || plainConsoleText(line.text).toLowerCase().includes(q);
        return streamMatches && queryMatches;
      });
      const blockMatches = !q || `${block.title} ${block.command ?? ''} ${block.source}`.toLowerCase().includes(q);
      return blockMatches ? { ...block, lines } : lines.length > 0 ? { ...block, lines } : null;
    })
    .filter((block): block is ConsoleBlockView => block !== null);
}

function streamColor(stream: CommandStreamKind): string {
  switch (stream) {
    case 'stderr':
      return 'var(--error, #f87171)';
    case 'stdin':
      return 'var(--accent)';
    case 'system':
      return 'var(--warning, #eab308)';
    default:
      return 'var(--text-muted)';
  }
}

function statusColor(status: CommandLifecycleStatus): string {
  switch (status) {
    case 'running':
    case 'waiting_for_input':
      return 'var(--accent)';
    case 'succeeded':
      return 'var(--success, #22c55e)';
    case 'failed':
    case 'timed_out':
      return 'var(--error, #f87171)';
    case 'cancelled':
      return 'var(--warning, #eab308)';
    default:
      return 'var(--text-dim)';
  }
}

function AnsiText({ text, highlight }: { text: string; highlight?: string }) {
  const parts = useMemo(() => parseAnsi(text), [text]);
  const normalizedHighlight = highlight?.trim().toLowerCase() || '';
  return (
    <>
      {parts.flatMap((part, partIndex) => splitHighlightedText(part.text, normalizedHighlight).map((segment, segmentIndex) => {
        const key = `${partIndex}-${segmentIndex}-${segment.text.length}`;
        if (!segment.match) return <span key={key} style={part.style}>{segment.text}</span>;
        return (
          <mark
            key={key}
            style={{
              ...part.style,
              padding: '0 1px',
              borderRadius: 2,
              background: 'var(--accent-glow)',
              color: part.style.color || 'var(--text)',
            }}
          >
            {segment.text}
          </mark>
        );
      }))}
    </>
  );
}

function splitHighlightedText(text: string, query: string): Array<{ text: string; match: boolean }> {
  if (!query) return [{ text, match: false }];
  const lower = text.toLowerCase();
  const segments: Array<{ text: string; match: boolean }> = [];
  let cursor = 0;
  while (cursor < text.length) {
    const index = lower.indexOf(query, cursor);
    if (index === -1) break;
    if (index > cursor) segments.push({ text: text.slice(cursor, index), match: false });
    segments.push({ text: text.slice(index, index + query.length), match: true });
    cursor = index + query.length;
  }
  if (cursor < text.length) segments.push({ text: text.slice(cursor), match: false });
  return segments.length ? segments : [{ text, match: false }];
}

function parseAnsi(text: string): Array<{ text: string; style: CSSProperties }> {
  const parts: Array<{ text: string; style: CSSProperties }> = [];
  const regex = /\x1b\[([0-9;]*)m/g;
  let style: CSSProperties = {};
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIndex) {
      parts.push({ text: text.slice(lastIndex, match.index), style: { ...style } });
    }
    style = applyAnsiCodes(style, match[1]);
    lastIndex = regex.lastIndex;
  }

  if (lastIndex < text.length) {
    parts.push({ text: text.slice(lastIndex), style });
  }
  return parts.length > 0 ? parts : [{ text, style: {} }];
}

function applyAnsiCodes(current: CSSProperties, rawCodes: string): CSSProperties {
  const codes = rawCodes ? rawCodes.split(';').map(code => Number(code || 0)) : [0];
  let next: CSSProperties = { ...current };
  for (const code of codes) {
    if (code === 0) next = {};
    else if (code === 1) next.fontWeight = 700;
    else if (code === 22) delete next.fontWeight;
    else if (code === 39) delete next.color;
    else if (ANSI_COLORS[code]) next.color = ANSI_COLORS[code];
  }
  return next;
}

const ANSI_COLORS: Record<number, string> = {
  30: '#7f858f',
  31: '#f87171',
  32: '#4ade80',
  33: '#facc15',
  34: '#60a5fa',
  35: '#c084fc',
  36: '#22d3ee',
  37: '#e5e7eb',
  90: '#9ca3af',
  91: '#fca5a5',
  92: '#86efac',
  93: '#fde68a',
  94: '#93c5fd',
  95: '#d8b4fe',
  96: '#67e8f9',
  97: '#ffffff',
};
