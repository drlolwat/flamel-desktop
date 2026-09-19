import React, { useMemo } from 'react';

// ── Types ────────────────────────────────────────────────────────────────────

interface FlowNode {
  id: string;
  label: string;
  x: number;
  y: number;
  w: number;
  h: number;
  layer: number;
}

interface FlowEdge {
  from: string;
  to: string;
  label?: string;
}

interface SeqActor {
  id: string;
  label: string;
  x: number;
}

interface SeqMessage {
  from: string;
  to: string;
  label: string;
  y: number;
  selfCall: boolean;
}

// ── Parser ───────────────────────────────────────────────────────────────────

function parseFlowchart(lines: string[]): { nodes: Map<string, FlowNode>; edges: FlowEdge[] } {
  const nodes = new Map<string, FlowNode>();
  const edges: FlowEdge[] = [];

  function ensureNode(id: string): FlowNode {
    const trimmed = id.trim();
    if (!nodes.has(trimmed)) {
      nodes.set(trimmed, { id: trimmed, label: trimmed, x: 0, y: 0, w: 0, h: 0, layer: -1 });
    }
    return nodes.get(trimmed)!;
  }

  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.toLowerCase() === 'flowchart') continue;

    // Parse: A -> B -> C  or  A -> B: label
    const parts = trimmed.split('->').map(s => s.trim()).filter(Boolean);
    if (parts.length < 2) {
      // Single node declaration
      if (parts.length === 1) ensureNode(parts[0]);
      continue;
    }

    for (let i = 0; i < parts.length - 1; i++) {
      let fromId = parts[i];
      let toRaw = parts[i + 1];
      let edgeLabel: string | undefined;

      // Check for label: "B: some label"
      const colonIdx = toRaw.indexOf(':');
      let toId: string;
      if (colonIdx > 0 && i === parts.length - 2) {
        // Only parse label on the last segment to avoid confusing with chained arrows
        toId = toRaw.slice(0, colonIdx).trim();
        edgeLabel = toRaw.slice(colonIdx + 1).trim();
      } else {
        toId = toRaw;
      }

      ensureNode(fromId);
      ensureNode(toId);
      edges.push({ from: fromId, to: toId, label: edgeLabel });
    }
  }

  return { nodes, edges };
}

function parseSequence(lines: string[]): { actors: SeqActor[]; messages: SeqMessage[] } {
  const actorOrder: string[] = [];
  const actorSet = new Set<string>();
  const messages: SeqMessage[] = [];

  function ensureActor(id: string) {
    const trimmed = id.trim();
    if (!actorSet.has(trimmed)) {
      actorSet.add(trimmed);
      actorOrder.push(trimmed);
    }
  }

  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.toLowerCase() === 'sequence') continue;

    // Parse: Actor1 -> Actor2: Message
    const arrowIdx = trimmed.indexOf('->');
    if (arrowIdx < 0) continue;

    const from = trimmed.slice(0, arrowIdx).trim();
    const rest = trimmed.slice(arrowIdx + 2).trim();

    const colonIdx = rest.indexOf(':');
    let to: string;
    let label: string;
    if (colonIdx >= 0) {
      to = rest.slice(0, colonIdx).trim();
      label = rest.slice(colonIdx + 1).trim();
    } else {
      to = rest;
      label = '';
    }

    ensureActor(from);
    ensureActor(to);
    messages.push({ from, to, label, y: 0, selfCall: from === to });
  }

  const actors: SeqActor[] = actorOrder.map((id, i) => ({ id, label: id, x: i * 160 + 80 }));
  return { actors, messages };
}

// ── Layout ───────────────────────────────────────────────────────────────────

function layoutFlowchart(nodes: Map<string, FlowNode>, edges: FlowEdge[]) {
  // Simple layered layout using topological ordering
  // Assign layers via longest path from roots
  const inDegree = new Map<string, number>();
  const children = new Map<string, string[]>();

  for (const n of nodes.keys()) {
    inDegree.set(n, 0);
    children.set(n, []);
  }
  for (const e of edges) {
    inDegree.set(e.to, (inDegree.get(e.to) || 0) + 1);
    children.get(e.from)?.push(e.to);
  }

  // BFS from roots (in-degree 0)
  const roots = [...nodes.keys()].filter(n => (inDegree.get(n) || 0) === 0);
  if (roots.length === 0 && nodes.size > 0) {
    // Cycle — just pick first node
    roots.push(nodes.keys().next().value!);
  }

  const queue = [...roots];
  for (const r of roots) {
    nodes.get(r)!.layer = 0;
  }

  while (queue.length > 0) {
    const current = queue.shift()!;
    const currentLayer = nodes.get(current)!.layer;
    for (const child of children.get(current) || []) {
      const childNode = nodes.get(child)!;
      if (childNode.layer < currentLayer + 1) {
        childNode.layer = currentLayer + 1;
        queue.push(child);
      }
    }
  }

  // Handle unvisited nodes (disconnected)
  for (const n of nodes.values()) {
    if (n.layer < 0) n.layer = 0;
  }

  // Measure text (approximate)
  const NODE_PAD_X = 24;
  const NODE_PAD_Y = 12;
  const CHAR_W = 7.5;
  const FONT_H = 14;

  for (const n of nodes.values()) {
    n.w = Math.max(60, n.label.length * CHAR_W + NODE_PAD_X * 2);
    n.h = FONT_H + NODE_PAD_Y * 2;
  }

  // Group by layer and position
  const layers = new Map<number, FlowNode[]>();
  for (const n of nodes.values()) {
    if (!layers.has(n.layer)) layers.set(n.layer, []);
    layers.get(n.layer)!.push(n);
  }

  const LAYER_GAP = 80;
  const NODE_GAP = 32;

  let y = 20;
  for (const layerIdx of [...layers.keys()].sort((a, b) => a - b)) {
    const layerNodes = layers.get(layerIdx)!;
    const totalWidth = layerNodes.reduce((s, n) => s + n.w, 0) + (layerNodes.length - 1) * NODE_GAP;
    let x = Math.max(20, 300 - totalWidth / 2);

    const maxH = Math.max(...layerNodes.map(n => n.h));
    for (const n of layerNodes) {
      n.x = x;
      n.y = y + (maxH - n.h) / 2;
      x += n.w + NODE_GAP;
    }
    y += maxH + LAYER_GAP;
  }
}

// ── SVG Renderers ────────────────────────────────────────────────────────────

function FlowchartSvg({ nodes, edges }: { nodes: FlowNode[]; edges: FlowEdge[] }) {
  const nodeMap = new Map(nodes.map(n => [n.id, n]));

  // Calculate viewBox
  const maxX = Math.max(100, ...nodes.map(n => n.x + n.w)) + 20;
  const maxY = Math.max(100, ...nodes.map(n => n.y + n.h)) + 20;

  return (
    <svg width="100%" viewBox={`0 0 ${maxX} ${maxY}`} style={{ display: 'block' }}>
      <defs>
        <marker id="arrow" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
          <path d="M0,0 L8,3 L0,6" fill="var(--text-muted)" />
        </marker>
      </defs>

      {/* Edges */}
      {edges.map((e, i) => {
        const from = nodeMap.get(e.from);
        const to = nodeMap.get(e.to);
        if (!from || !to) return null;

        const x1 = from.x + from.w / 2;
        const y1 = from.y + from.h;
        const x2 = to.x + to.w / 2;
        const y2 = to.y;

        // Simple bezier curve
        const midY = (y1 + y2) / 2;

        return (
          <g key={i}>
            <path
              d={`M${x1},${y1} C${x1},${midY} ${x2},${midY} ${x2},${y2}`}
              fill="none"
              stroke="var(--text-muted)"
              strokeWidth={1.5}
              markerEnd="url(#arrow)"
              opacity={0.6}
            />
            {e.label && (
              <text
                x={(x1 + x2) / 2}
                y={midY - 4}
                textAnchor="middle"
                fontSize={10}
                fill="var(--text-muted)"
              >
                {e.label}
              </text>
            )}
          </g>
        );
      })}

      {/* Nodes */}
      {nodes.map(n => (
        <g key={n.id}>
          <rect
            x={n.x}
            y={n.y}
            width={n.w}
            height={n.h}
            rx={6}
            fill="var(--surface-2)"
            stroke="var(--accent)"
            strokeWidth={1.5}
            opacity={0.9}
          />
          <text
            x={n.x + n.w / 2}
            y={n.y + n.h / 2 + 1}
            textAnchor="middle"
            dominantBaseline="middle"
            fontSize={12}
            fill="var(--text)"
            fontFamily="var(--font-mono)"
          >
            {n.label}
          </text>
        </g>
      ))}
    </svg>
  );
}

function SequenceSvg({ actors, messages }: { actors: SeqActor[]; messages: SeqMessage[] }) {
  const HEADER_H = 40;
  const MSG_GAP = 44;
  const ACTOR_W = 100;
  const PAD = 20;

  // Position messages
  const positionedMessages = messages.map((m, i) => ({ ...m, y: HEADER_H + 30 + i * MSG_GAP }));

  const totalHeight = HEADER_H + 30 + messages.length * MSG_GAP + 40;
  const totalWidth = actors.length > 0 ? actors[actors.length - 1].x + ACTOR_W / 2 + PAD : 200;

  const actorMap = new Map(actors.map(a => [a.id, a]));

  return (
    <svg width="100%" viewBox={`0 0 ${totalWidth} ${totalHeight}`} style={{ display: 'block' }}>
      <defs>
        <marker id="seq-arrow" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
          <path d="M0,0 L8,3 L0,6" fill="var(--text-muted)" />
        </marker>
      </defs>

      {/* Actor headers + lifelines */}
      {actors.map(a => (
        <g key={a.id}>
          {/* Header box */}
          <rect
            x={a.x - ACTOR_W / 2}
            y={8}
            width={ACTOR_W}
            height={28}
            rx={4}
            fill="var(--surface-2)"
            stroke="var(--accent)"
            strokeWidth={1.5}
          />
          <text
            x={a.x}
            y={26}
            textAnchor="middle"
            fontSize={11}
            fontWeight={600}
            fill="var(--text)"
            fontFamily="var(--font-mono)"
          >
            {a.label}
          </text>
          {/* Lifeline */}
          <line
            x1={a.x}
            y1={HEADER_H}
            x2={a.x}
            y2={totalHeight - 10}
            stroke="var(--border-soft)"
            strokeWidth={1}
            strokeDasharray="4 3"
          />
        </g>
      ))}

      {/* Messages */}
      {positionedMessages.map((m, i) => {
        const fromActor = actorMap.get(m.from);
        const toActor = actorMap.get(m.to);
        if (!fromActor || !toActor) return null;

        if (m.selfCall) {
          // Self-call: loop arrow
          const x = fromActor.x;
          const loopW = 30;
          return (
            <g key={i}>
              <path
                d={`M${x},${m.y} L${x + loopW},${m.y} L${x + loopW},${m.y + 20} L${x + 4},${m.y + 20}`}
                fill="none"
                stroke="var(--text-muted)"
                strokeWidth={1.5}
                markerEnd="url(#seq-arrow)"
              />
              <text
                x={x + loopW + 6}
                y={m.y + 10}
                fontSize={10}
                fill="var(--text-muted)"
                dominantBaseline="middle"
              >
                {m.label}
              </text>
            </g>
          );
        }

        const x1 = fromActor.x;
        const x2 = toActor.x;
        const dir = x2 > x1 ? 1 : -1;

        return (
          <g key={i}>
            <line
              x1={x1}
              y1={m.y}
              x2={x2 - dir * 8}
              y2={m.y}
              stroke="var(--text-muted)"
              strokeWidth={1.5}
              markerEnd="url(#seq-arrow)"
            />
            <text
              x={(x1 + x2) / 2}
              y={m.y - 6}
              textAnchor="middle"
              fontSize={10}
              fill="var(--text)"
            >
              {m.label}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

// ── Main Component ───────────────────────────────────────────────────────────

interface DiagramVizProps {
  diagram: string;
  title?: string;
}

export default React.memo(function DiagramViz({ diagram, title }: DiagramVizProps) {
  const rendered = useMemo(() => {
    const safeDiagram = typeof diagram === 'string' ? diagram : '';
    const lines = safeDiagram.trim().split('\n');
    const type = (lines[0] || '').trim().toLowerCase();
    const contentLines = lines.slice(1);

    if (type === 'flowchart') {
      const { nodes, edges } = parseFlowchart(contentLines);
      layoutFlowchart(nodes, edges);
      return <FlowchartSvg nodes={[...nodes.values()]} edges={edges} />;
    }

    if (type === 'sequence') {
      const { actors, messages } = parseSequence(contentLines);
      return <SequenceSvg actors={actors} messages={messages} />;
    }

    return <div style={{ color: 'var(--error)', padding: 12, fontSize: 12 }}>Unknown diagram type: {type}</div>;
  }, [diagram]);

  return (
    <div style={{
      background: 'var(--surface)',
      border: '1px solid var(--border-soft)',
      borderRadius: 'var(--radius-md)',
      padding: '12px 14px',
      margin: '8px 0',
      overflow: 'auto',
    }}>
      {title && (
        <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--text)', marginBottom: 8 }}>
          {title}
        </div>
      )}
      {rendered}
    </div>
  );
});
