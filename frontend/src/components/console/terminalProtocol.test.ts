import { describe, it, expect } from 'vitest';
import { encodeTerminalInput, terminalSocketUrl } from './terminalProtocol';

describe('terminal byte transport', () => {
  it('preserves Unicode and terminal control input', () => {
    const input = 'π😀\x03\x1b[A';
    const bytes = Uint8Array.from(atob(encodeTerminalInput(input)), character => character.charCodeAt(0));
    expect(new TextDecoder().decode(bytes)).toBe(input);
  });

  it('uses a one-use ticket and cursor, never a bearer token', () => {
    const url = new URL(terminalSocketUrl('https://code.example.com', 'session/id', 'ticket&one', 42));
    expect(url.protocol).toBe('wss:');
    expect(url.pathname).toBe('/api/terminal/connect/session%2Fid');
    expect(url.searchParams.get('ticket')).toBe('ticket&one');
    expect(url.searchParams.get('afterSequence')).toBe('42');
    expect(url.searchParams.has('token')).toBe(false);
  });
});
