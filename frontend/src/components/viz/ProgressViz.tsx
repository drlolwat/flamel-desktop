import React from 'react';

interface ProgressBar {
  label: string;
  value: number;
  max?: number;
  color?: string;
  format?: 'percent' | 'fraction' | 'value';
}

function formatValue(bar: ProgressBar, pct: number): string {
  const max = bar.max ?? 100;
  switch (bar.format) {
    case 'fraction': return `${bar.value}/${max}`;
    case 'value': return `${bar.value}`;
    default: return `${Math.round(pct)}%`;
  }
}

export default React.memo(function ProgressViz({ bars, title }: { bars: ProgressBar[]; title?: string }) {
  const safeBars = Array.isArray(bars) ? bars : [];
  return (
    <div style={{
      background: 'var(--surface)',
      border: '1px solid var(--border-soft)',
      borderRadius: 'var(--radius-md)',
      padding: '12px 14px',
      margin: '8px 0',
    }}>
      {title && (
        <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--text)', marginBottom: 10 }}>
          {title}
        </div>
      )}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {safeBars.map((bar, i) => {
          const max = bar.max ?? 100;
          const pct = Math.min(100, Math.max(0, (bar.value / max) * 100));
          const color = bar.color || 'var(--accent)';

          return (
            <div key={i}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                <span style={{ fontSize: 11, color: 'var(--text)' }}>{bar.label}</span>
                <span style={{ fontSize: 11, color: 'var(--text-muted)', fontFamily: 'var(--font-mono)' }}>
                  {formatValue(bar, pct)}
                </span>
              </div>
              <div style={{
                height: 8,
                background: 'var(--surface-2)',
                borderRadius: 4,
                overflow: 'hidden',
              }}>
                <div style={{
                  height: '100%',
                  width: `${pct}%`,
                  background: color,
                  borderRadius: 4,
                  transition: 'width 0.3s ease',
                }} />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
});
