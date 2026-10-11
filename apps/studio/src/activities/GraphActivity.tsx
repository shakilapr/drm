// Graph activity: a read-only projection of the deployment model's computes
// and derived links. The graph library (React Flow) is isolated behind this
// activity; the projection and layout here stay dependency-free. Graph
// coordinates, zoom, and collapsed state are UI state, never `drm.toml`
// (ui-arch §System Graph, REQ-STUDIO-017). The renderer performs no routing.
import { useMemo } from "react";

import { projectGraph } from "../rpc/graph";
import type { GraphEdge, GraphNode, GraphProjection } from "../rpc/graph";
import type { ModelView } from "../rpc/model";
import { useUiStore } from "../stores/ui";

/** Props for {@link GraphActivity}. */
export interface GraphActivityProps {
  readonly model: ModelView;
}

const NODE_WIDTH = 160;
const NODE_HEIGHT = 64;
const COLUMN_WIDTH = 220;
const ROW_HEIGHT = 120;

/** Deterministic grid position for the `index`-th compute. */
function position(index: number, columns: number): { x: number; y: number } {
  return {
    x: 40 + (index % columns) * COLUMN_WIDTH,
    y: 40 + Math.floor(index / columns) * ROW_HEIGHT,
  };
}

/** Renders one compute node box (placed nodes collapsed to a count). */
function nodeBox(
  node: GraphNode,
  at: { x: number; y: number },
  collapsed: boolean,
): React.JSX.Element {
  const detail = collapsed ? `${node.placed.length} nodes` : node.placed.join(", ") || "—";
  return (
    <g key={node.id} transform={`translate(${at.x}, ${at.y})`} className="graph-node">
      <rect width={NODE_WIDTH} height={NODE_HEIGHT} rx={8} />
      <text x={12} y={26}>
        {node.label}
      </text>
      <text x={12} y={46} className="graph-node-detail">
        {detail}
      </text>
    </g>
  );
}

/** Renders one link edge between two compute boxes. */
function edgeLine(
  edge: GraphEdge,
  positions: ReadonlyMap<string, { x: number; y: number }>,
): React.JSX.Element | null {
  const from = positions.get(edge.from);
  const to = positions.get(edge.to);
  if (!from || !to) {
    return null;
  }
  return (
    <line
      key={edge.id}
      className="graph-edge"
      x1={from.x + NODE_WIDTH}
      y1={from.y + NODE_HEIGHT / 2}
      x2={to.x}
      y2={to.y + NODE_HEIGHT / 2}
    />
  );
}

/** Read-only compute/link graph projection. */
export function GraphActivity({ model }: GraphActivityProps): React.JSX.Element {
  const collapsed = useUiStore((state) => state.graph.collapsed);
  const toggleCollapsed = useUiStore((state) => state.toggleGraphCollapsed);
  const graph: GraphProjection = useMemo(() => projectGraph(model), [model]);
  const columns = Math.max(1, Math.ceil(Math.sqrt(graph.nodes.length)));
  const rows = Math.max(1, Math.ceil(graph.nodes.length / columns));
  const width = 80 + columns * COLUMN_WIDTH;
  const height = 80 + rows * ROW_HEIGHT;
  const positions = useMemo(
    () => new Map(graph.nodes.map((node, index) => [node.id, position(index, columns)] as const)),
    [graph.nodes, columns],
  );

  return (
    <div className="activity graph-activity" data-testid="graph-activity">
      <h2>Graph</h2>
      <p className="graph-hint">Read-only deployment projection (computes + links).</p>
      <button type="button" onClick={toggleCollapsed}>
        {collapsed ? "Expand nodes" : "Collapse nodes"}
      </button>
      <svg
        className="graph-canvas"
        role="img"
        aria-label="Deployment graph"
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
      >
        {graph.edges.map((edge) => edgeLine(edge, positions))}
        {graph.nodes.map((node) => {
          const at = positions.get(node.id);
          return at ? nodeBox(node, at, collapsed) : null;
        })}
      </svg>
    </div>
  );
}
