import React from 'react';

interface Metric {
  label: string;
  value: string;
  delta?: string;
  trend?: 'up' | 'down' | 'flat';
  sparkline?: number[];
}

function MiniSparkline({ data }: { data: number[] }) {
  if (data.length < 2) return null;
  const min = Math.min(...data);
  const max = Math.max(...data);
  const range = max - min || 1;
  const h = 24;
  const w = 60;
  const step = w / (data.length - 1);

  const points = data.map((v, i) => `${i * step},${h - ((v - min) / range) * h}`).join(' ');

  return (
    <svg width={w} height={h} style={{ display: 'block' }}>
      <polyline
        points={points}
        fill="none"
        stroke="var(--accent)"
        strokeWidth={1.5}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}

export default React.memo(function MetricViz({ metrics }: { metrics: Metric[] }) {
  const safeMetrics = Array.isArray(metrics) ? metrics : [];
  const trendColor = (t?: string) =>
    t === 'up' ? 'var(--success)' : t === 'down' ? 'var(--error)' : 'var(--text-muted)';

  const trendArrow = (t?: string) =>
    t === 'up' ? '\u2191' : t === 'down' ? '\u2193' : '';

  return (
    <div style={{
      display: 'flex',
      flexWrap: 'wrap',
      gap: 8,
      margin: '8px 0',
    }}>
      {safeMetrics.map((m, i) => (
        <div key={i} style={{
          flex: '1 1 140px',
          minWidth: 120,
          background: 'var(--surface)',
          border: '1px solid var(--border-soft)',
          borderRadius: 'var(--radius-md)',
          padding: '12px 14px',
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
        }}>
          <div style={{ fontSize: 10, color: 'var(--text-muted)', letterSpacing: '0.04em', textTransform: 'uppercase' }}>
            {m.label}
          </div>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
            <span style={{ fontSize: 22, fontWeight: 700, color: 'var(--text)', fontFamily: 'var(--font-mono)' }}>
              {m.value}
            </span>
            {m.delta && (
              <span style={{ fontSize: 11, fontWeight: 500, color: trendColor(m.trend) }}>
                {trendArrow(m.trend)} {m.delta}
              </span>
            )}
          </div>
          {m.sparkline && m.sparkline.length >= 2 && <MiniSparkline data={m.sparkline} />}
        </div>
      ))}
    </div>
  );
});
