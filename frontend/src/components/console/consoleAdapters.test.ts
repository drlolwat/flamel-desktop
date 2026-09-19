import { describe, expect, it } from 'vitest';
import {
  consoleOutputLineCount,
  latestRunningConsoleTool,
  normalizeConsoleText,
  plainConsoleText,
  toolCallToCommandEvents,
  type ConsoleLineView,
} from './consoleAdapters';

function line(text: string): ConsoleLineView {
  return {
    id: crypto.randomUUID(),
    sequence: 1,
    stream: 'stdout',
    text,
    timestamp: '2026-07-10T00:00:00Z',
    eventKind: 'output_chunk',
    metadata: {},
  };
}

describe('consoleOutputLineCount', () => {
  it('counts physical rows inside one transport chunk', () => {
    expect(consoleOutputLineCount([line('build start\ncompiled\nfinished\n')])).toBe(3);
  });

  it('counts unterminated final rows and multiple chunks', () => {
    expect(consoleOutputLineCount([line('first\nsecond'), line('third')])).toBe(3);
  });

  it('handles CRLF output and ignores empty chunks', () => {
    expect(consoleOutputLineCount([line('one\r\ntwo\r\n'), line('')])).toBe(2);
  });

  it('counts visible rows instead of terminal control sequences', () => {
    const buildkit = '\x1b[1A\x1b[1A\x1b[?25lstep one\rstep two\n\x1b[?25h';
    expect(consoleOutputLineCount([line(buildkit)])).toBe(2);
  });
});

it('does not invent success or a completion timestamp for unknown shell state', () => {
  const events = toolCallToCommandEvents({ id: 'uncertain', name: 'shell', input: { command: 'npm test' }, status: 'unknown' });
  expect(events.some(event => event.kind === 'command_finished')).toBe(false);
  expect(events.find(event => event.run)?.run?.status).toBe('unknown');
  expect(events.find(event => event.run)?.run?.finished_at).toBeNull();
});

describe('normalizeConsoleText', () => {
  it('removes cursor and visibility controls while preserving ANSI colors', () => {
    const buildkit = '\x1b[1A\x1b[?25l\x1b[34m=> building\x1b[0m\x1b[?25h';
    expect(normalizeConsoleText(buildkit)).toBe('\x1b[34m=> building\x1b[0m');
    expect(plainConsoleText(buildkit)).toBe('=> building');
  });

  it('turns carriage-return progress updates into transcript rows', () => {
    expect(normalizeConsoleText('building 10%\rbuilding 20%\rbuilding 30%')).toBe(
      'building 10%\nbuilding 20%\nbuilding 30%',
    );
  });
});

describe('latestRunningConsoleTool', () => {
  it('selects the newest active shell tool and ignores completed commands', () => {
    const tools = [
      { id: 'done', name: 'shell', input: {}, status: 'done' as const },
      { id: 'file', name: 'file_read', input: {}, status: 'running' as const },
      { id: 'first', name: 'exec_command', input: {}, status: 'running' as const },
      { id: 'latest', name: 'mcp__flamel__shell', input: {}, status: 'pending' as const },
    ];

    expect(latestRunningConsoleTool(tools)?.id).toBe('latest');
  });

  it('returns undefined when no console is active', () => {
    expect(latestRunningConsoleTool([
      { id: 'done', name: 'shell', input: {}, status: 'done', output: 'ok' },
    ])).toBeUndefined();
  });
});
