import { runtimeSocket, type RuntimeSocket } from '../../lib/runtimeTransport';
import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import '@xterm/xterm/css/xterm.css';
import { api, apiFetch } from '../../lib/api';
import { getApiBase, useConnectionStore } from '../../stores/connectionStore';
import { useTerminalStore } from '../../stores/terminalStore';
import { encodeTerminalInput, terminalSocketUrl } from '../console/terminalProtocol';

interface WorkbenchTerminalProps { terminalId: string; visible: boolean }

export default function WorkbenchTerminal({ terminalId, visible }: WorkbenchTerminalProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const socketRef = useRef<RuntimeSocket | null>(null);
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const [message, setMessage] = useState('Starting terminal…');
  const [ended, setEnded] = useState(false);
  const [search, setSearch] = useState('');
  const projectPath = useTerminalStore(state => state.terminals.find(entry => entry.id === terminalId)?.projectPath);
  const apiBase = useConnectionStore(state => state.getBaseUrl());

  useEffect(() => {
    if (!hostRef.current || projectPath === undefined) return;
    const terminal = new Terminal({
      cursorBlink: true, cursorStyle: 'bar', fontFamily: 'monospace', fontSize: 12,
      lineHeight: 1.25, scrollback: 10_000,
      theme: { background: '#090909', foreground: '#d7d3d0', cursor: '#eb493b', selectionBackground: '#eb493b55' },
    });
    const fit = new FitAddon();
    const searchAddon = new SearchAddon();
    terminal.loadAddon(fit);
    terminal.loadAddon(searchAddon);
    terminal.open(hostRef.current);
    terminalRef.current = terminal;
    fitRef.current = fit;
    searchRef.current = searchAddon;
    let disposed = false;
    let stopped = false;
    let cursor = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
    let attempts = 0;
    const abort = new AbortController();

    function send(payload: Record<string, unknown>): boolean {
      const socket = socketRef.current;
      if (!socket || socket.readyState !== WebSocket.OPEN || socket.bufferedAmount > 262_144) {
        setMessage('Input was not sent: terminal is disconnected or busy.');
        return false;
      }
      socket.send(JSON.stringify(payload));
      return true;
    }
    function fitTerminal() {
      if (disposed || !visibleRef.current) return;
      fit.fit();
      if (socketRef.current?.readyState === WebSocket.OPEN) send({ type: 'resize', cols: terminal.cols, rows: terminal.rows });
    }
    const observer = new ResizeObserver(() => requestAnimationFrame(fitTerminal));
    observer.observe(hostRef.current);
    const input = terminal.onData(data => send({ type: 'input', data: encodeTerminalInput(data) }));

    function scheduleReconnect(id: string) {
      if (disposed || stopped || reconnectTimer) return;
      setMessage('Disconnected — reconnecting. Input is paused.');
      reconnectTimer = setTimeout(() => { reconnectTimer = undefined; void connect(id); }, Math.min(1000 * 2 ** attempts++, 10_000));
    }

    async function connect(id: string) {
      if (disposed || stopped) return;
      try {
        const status = await apiFetch(`/api/terminal/${encodeURIComponent(id)}/status`, { signal: abort.signal });
        if (status.status === 404) {
          stopped = true; setEnded(true); setMessage('This terminal has ended or the server restarted. Start a new terminal to continue.'); return;
        }
        if (status.status === 401 || status.status === 403) {
          stopped = true; setMessage('Sign in again to reconnect to this terminal.'); return;
        }
        if (!status.ok) throw new Error('Terminal status unavailable');
        const response = await apiFetch('/api/realtime/ticket', { method: 'POST', signal: abort.signal });
        if (!response.ok) throw new Error('Terminal authorization failed');
        const { ticket } = await response.json() as { ticket: string };
        if (disposed) return;
        const socket = runtimeSocket(terminalSocketUrl(getApiBase() || window.location.origin, id, ticket, cursor));
        socketRef.current = socket;
        socket.onopen = () => { attempts = 0; setMessage(''); fitTerminal(); };
        socket.onmessage = event => {
          if (disposed) return;
          try {
            const payload = JSON.parse(event.data) as { type?: string; data?: string; sequence?: number; exit_code?: number; message?: string };
            if (payload.type === 'output' && payload.data && (payload.sequence ?? 0) > cursor) {
              cursor = payload.sequence!;
              terminal.write(Uint8Array.from(atob(payload.data), character => character.charCodeAt(0)));
            } else if (payload.type === 'gap') {
              terminal.reset(); setMessage(payload.message || 'Earlier output is no longer available.');
            } else if (payload.type === 'exit' || payload.type === 'missing') {
              stopped = true; setEnded(true);
              setMessage(payload.type === 'missing' ? 'Terminal closed.' : `Process exited${payload.exit_code == null ? '' : ` (${payload.exit_code})`}.`);
            } else if (payload.type === 'input_error') {
              setMessage('Input was rejected. Check the connection before trying again.');
            }
          } catch { setMessage('Invalid terminal frame; reconnect to refresh output.'); }
        };
        socket.onclose = () => { if (socketRef.current === socket) socketRef.current = null; scheduleReconnect(id); };
        socket.onerror = () => socket.close();
      } catch {
        if (!disposed) scheduleReconnect(id);
      }
    }

    void (async () => {
      const record = useTerminalStore.getState().terminals.find(entry => entry.id === terminalId);
      if (!record) return;
      setEnded(false);
      let id = record.sessionId;
      if (!id) {
        try {
          fitTerminal();
          const started = await api.terminal.start(projectPath, terminal.cols, terminal.rows);
          if (disposed) { await api.terminal.close(started.session_id); return; }
          id = started.session_id;
          useTerminalStore.getState().setSessionId(terminalId, id);
        } catch (error) { if (!disposed) setMessage(error instanceof Error ? error.message : 'Could not start terminal'); return; }
      }
      await connect(id);
    })();

    return () => {
      disposed = true;
      abort.abort();
      if (reconnectTimer) clearTimeout(reconnectTimer);
      socketRef.current?.close(); socketRef.current = null;
      observer.disconnect(); input.dispose(); terminal.dispose();
      terminalRef.current = null; fitRef.current = null; searchRef.current = null;
    };
    // Visibility must never dispose the terminal or reset its byte cursor.
  }, [terminalId, projectPath, apiBase]);

  useEffect(() => {
    if (!visible) return;
    requestAnimationFrame(() => {
      const terminal = terminalRef.current;
      fitRef.current?.fit();
      const socket = socketRef.current;
      if (terminal && socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: 'resize', cols: terminal.cols, rows: terminal.rows }));
    });
  }, [visible]);

  return (
    <div style={{ display: visible ? 'flex' : 'none', flexDirection: 'column', height: '100%', minHeight: 0, background: '#090909' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 10px', borderBottom: '1px solid var(--border-soft)' }}>
        <input aria-label="Search terminal output" placeholder="Find in output" value={search} onChange={event => { setSearch(event.target.value); searchRef.current?.findNext(event.target.value); }} onKeyDown={event => { if (event.key === 'Enter') { if (event.shiftKey) searchRef.current?.findPrevious(search); else searchRef.current?.findNext(search); } }} style={{ minWidth: 0, fontSize: 12, background: 'transparent', color: 'var(--text)', border: 'none' }} />
        {ended && <button type="button" onClick={() => projectPath && useTerminalStore.getState().openTerminal(projectPath)}>New terminal</button>}
      </div>
      <div ref={hostRef} style={{ flex: 1, minHeight: 0, padding: '8px 10px' }} />
      {message && <div role="status" style={{ padding: '5px 10px', borderTop: '1px solid var(--border-soft)', color: 'var(--text-muted)', fontSize: 12 }}>{message}</div>}
    </div>
  );
}
