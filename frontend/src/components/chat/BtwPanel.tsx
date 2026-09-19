import { useEffect, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { apiFetch } from '../../lib/api';
import { Icon } from '../ui/Icons';

interface BtwPanelProps {
  question: string;
  sessionId?: string;
  workingDirectory?: string;
  onClose: () => void;
}

export default function BtwPanel({ question, sessionId, workingDirectory, onClose }: BtwPanelProps) {
  const [text, setText] = useState('');
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const readerRef = useRef<ReadableStreamDefaultReader<Uint8Array> | null>(null);

  useEffect(() => {
    let cancelled = false;
    setText('');
    setDone(false);
    setError(null);

    (async () => {
      try {
        const res = await apiFetch('/api/btw', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'include',
          body: JSON.stringify({ question, sessionId, workingDirectory }),
        });
        if (!res.ok || !res.body) {
          setError('Failed to start btw request');
          return;
        }

        const reader = res.body.getReader();
        readerRef.current = reader;
        const dec = new TextDecoder();
        let buf = '';

        while (!cancelled) {
          const { done: streamDone, value } = await reader.read();
          if (streamDone) break;
          buf += dec.decode(value, { stream: true });
          const lines = buf.split('\n');
          buf = lines.pop() ?? '';
          for (const line of lines) {
            if (!line.startsWith('data: ')) continue;
            try {
              const msg = JSON.parse(line.slice(6));
              if (msg.type === 'text') setText(t => t + msg.text);
              if (msg.type === 'done') { setDone(true); break; }
            } catch { /* ignore */ }
          }
        }
      } catch (e: unknown) {
        if (!cancelled) setError(e instanceof Error ? e.message : 'Unknown error');
      }
    })();

    return () => {
      cancelled = true;
      readerRef.current?.cancel().catch(() => {});
    };
  }, [question, sessionId, workingDirectory]);

  return (
    <div style={{
      position: 'relative',
      margin: '0 0 8px',
      border: '1px solid var(--warning, #f59e0b)',
      borderRadius: 'var(--radius-md)',
      background: 'color-mix(in srgb, var(--warning, #f59e0b) 5%, var(--surface))',
      overflow: 'hidden',
    }}>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 6,
        padding: '5px 10px 5px 8px',
        borderBottom: '1px solid color-mix(in srgb, var(--warning, #f59e0b) 20%, transparent)',
      }}>
        <span style={{ fontSize: 10, fontWeight: 600, color: 'var(--warning, #f59e0b)', letterSpacing: '0.05em', flexShrink: 0 }}>
          /btw
        </span>
        <span style={{
          fontSize: 11, color: 'var(--text-dim)', flex: 1,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>
          {question}
        </span>
        {!done && !error && (
          <span style={{
            width: 6, height: 6, borderRadius: '50%',
            background: 'var(--warning, #f59e0b)',
            flexShrink: 0,
            animation: 'pulse 1.4s ease-in-out infinite',
          }} />
        )}
        <button
          onClick={onClose}
          style={{
            padding: 2, background: 'transparent', border: 'none',
            color: 'var(--text-dim)', cursor: 'pointer', flexShrink: 0,
            display: 'flex', alignItems: 'center',
          }}
          title="Dismiss"
        >
          <Icon.Close size={12} />
        </button>
      </div>

      {/* Answer body */}
      <div style={{ padding: '8px 12px 10px', fontSize: 13, lineHeight: 1.6, minHeight: 32 }}>
        {error ? (
          <span style={{ color: 'var(--error)', fontSize: 12 }}>{error}</span>
        ) : text ? (
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
        ) : (
          <span style={{ color: 'var(--text-dim)', fontSize: 12 }}>Thinking…</span>
        )}
      </div>
    </div>
  );
}
