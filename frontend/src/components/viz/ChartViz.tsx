import React from 'react';
import {
  BarChart, Bar, LineChart, Line, AreaChart, Area,
  PieChart, Pie, Cell, ScatterChart, Scatter,
  RadarChart, Radar, PolarGrid, PolarAngleAxis, PolarRadiusAxis,
  XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, Legend,
} from 'recharts';

const DEFAULT_COLORS = [
  'var(--accent)', 'var(--success)', 'var(--warning)', 'var(--error)', '#5bb8d6',
  '#6dd7b4', '#c08b6a', '#a98fd1', '#7f8a8f', 'var(--accent-text)',
];

const TOOLTIP_STYLE = {
  backgroundColor: 'var(--surface-2)',
  border: '1px solid var(--border-soft)',
  borderRadius: 6,
  fontSize: 11,
  color: 'var(--text)',
};

interface ChartVizProps {
  type: string;
  title?: string;
  data: Record<string, any>[];
  xKey: string;
  yKeys: string[];
  colors?: string[];
  stacked?: boolean;
}

export default React.memo(function ChartViz({ type, title, data, xKey, yKeys, colors, stacked }: ChartVizProps) {
  const safeData = Array.isArray(data) ? data : [];
  const safeYKeys = Array.isArray(yKeys) ? yKeys : [];
  const palette = colors?.length ? colors : DEFAULT_COLORS;

  const commonAxisProps = {
    tick: { fill: 'var(--text-muted)', fontSize: 10 },
    axisLine: { stroke: 'var(--border-soft)' },
    tickLine: false,
  };

  const renderCartesian = (ChartComp: any, ShapeComp: any, shapeProps: any = {}) => (
    <ResponsiveContainer width="100%" height={280}>
      <ChartComp data={safeData} margin={{ top: 8, right: 16, left: 0, bottom: 4 }}>
        <CartesianGrid strokeDasharray="3 3" stroke="var(--border-soft)" opacity={0.5} />
        <XAxis dataKey={xKey} {...commonAxisProps} />
        <YAxis {...commonAxisProps} />
        <Tooltip contentStyle={TOOLTIP_STYLE} />
        {safeYKeys.length > 1 && <Legend wrapperStyle={{ fontSize: 11 }} />}
        {safeYKeys.map((key, i) => (
          <ShapeComp
            key={key}
            dataKey={key}
            fill={palette[i % palette.length]}
            stroke={palette[i % palette.length]}
            stackId={stacked ? 'stack' : undefined}
            {...shapeProps}
          />
        ))}
      </ChartComp>
    </ResponsiveContainer>
  );

  const renderPie = () => (
    <ResponsiveContainer width="100%" height={280}>
      <PieChart>
        <Pie
          data={safeData}
          dataKey={safeYKeys[0]}
          nameKey={xKey}
          cx="50%"
          cy="50%"
          outerRadius={100}
          innerRadius={40}
          paddingAngle={2}
          label={({ name, percent }: any) => `${name} ${(percent * 100).toFixed(0)}%`}
          labelLine={false}
          fontSize={10}
        >
          {safeData.map((_, i) => (
            <Cell key={i} fill={palette[i % palette.length]} />
          ))}
        </Pie>
        <Tooltip contentStyle={TOOLTIP_STYLE} />
      </PieChart>
    </ResponsiveContainer>
  );

  const renderScatter = () => (
    <ResponsiveContainer width="100%" height={280}>
      <ScatterChart margin={{ top: 8, right: 16, left: 0, bottom: 4 }}>
        <CartesianGrid strokeDasharray="3 3" stroke="var(--border-soft)" opacity={0.5} />
        <XAxis dataKey={xKey} name={xKey} {...commonAxisProps} />
        <YAxis dataKey={safeYKeys[0]} name={safeYKeys[0]} {...commonAxisProps} />
        <Tooltip contentStyle={TOOLTIP_STYLE} cursor={{ strokeDasharray: '3 3' }} />
        <Scatter data={safeData} fill={palette[0]} />
      </ScatterChart>
    </ResponsiveContainer>
  );

  const renderRadar = () => (
    <ResponsiveContainer width="100%" height={280}>
      <RadarChart data={safeData} cx="50%" cy="50%" outerRadius={90}>
        <PolarGrid stroke="var(--border-soft)" />
        <PolarAngleAxis dataKey={xKey} tick={{ fill: 'var(--text-muted)', fontSize: 10 }} />
        <PolarRadiusAxis tick={{ fill: 'var(--text-muted)', fontSize: 9 }} />
        {safeYKeys.map((key, i) => (
          <Radar
            key={key}
            dataKey={key}
            stroke={palette[i % palette.length]}
            fill={palette[i % palette.length]}
            fillOpacity={0.2}
          />
        ))}
        <Tooltip contentStyle={TOOLTIP_STYLE} />
        {safeYKeys.length > 1 && <Legend wrapperStyle={{ fontSize: 11 }} />}
      </RadarChart>
    </ResponsiveContainer>
  );

  let chart: React.ReactNode;
  switch (type) {
    case 'bar': chart = renderCartesian(BarChart, Bar, { radius: [3, 3, 0, 0] }); break;
    case 'line': chart = renderCartesian(LineChart, Line, { type: 'monotone', dot: false, strokeWidth: 2 }); break;
    case 'area': chart = renderCartesian(AreaChart, Area, { type: 'monotone', fillOpacity: 0.15, strokeWidth: 2 }); break;
    case 'pie': chart = renderPie(); break;
    case 'scatter': chart = renderScatter(); break;
    case 'radar': chart = renderRadar(); break;
    default: chart = <div style={{ color: 'var(--error)', padding: 12 }}>Unsupported chart type: {type}</div>;
  }

  return (
    <div style={{
      background: 'var(--surface)',
      border: '1px solid var(--border-soft)',
      borderRadius: 'var(--radius-md)',
      padding: '12px 8px 4px',
      margin: '8px 0',
    }}>
      {title && (
        <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--text)', marginBottom: 8, paddingLeft: 8 }}>
          {title}
        </div>
      )}
      {chart}
    </div>
  );
});
