// Read-only graph projection of the engine's deployment model. The Graph
// activity renders this projection; the graph library (React Flow) is isolated
// behind the Graph activity boundary and consumes the same projection, so the
// projection stays renderer-agnostic. Viewport, zoom, and collapsed state are
// UI state (Zustand), never `drm.toml` (ui-arch §System Graph, REQ-STUDIO-017).
// The renderer computes no routing — it projects the engine's derived links.
import type { ModelView } from "./model";

/** A graph node: a compute unit with its placed nodes (collapsed). */
export interface GraphNode {
  readonly id: string;
  readonly label: string;
  readonly placed: string[];
}

/** A graph edge: a derived deployment link between two compute units. */
export interface GraphEdge {
  readonly id: string;
  readonly from: string;
  readonly to: string;
}

/** A read-only route/link projection of the deployment model. */
export interface GraphProjection {
  readonly nodes: GraphNode[];
  readonly edges: GraphEdge[];
}

/** Projects the deployment model into compute nodes + link edges. */
export function projectGraph(model: ModelView): GraphProjection {
  const placedByCompute = new Map<string, string[]>();
  for (const placement of model.placements) {
    const placed = placedByCompute.get(placement.compute) ?? [];
    placed.push(placement.node);
    placedByCompute.set(placement.compute, placed);
  }
  const nodes = model.computes.map((compute) => ({
    id: compute.name,
    label: compute.name,
    placed: placedByCompute.get(compute.name) ?? [],
  }));
  const edges = model.links.map((link) => ({
    id: link.id,
    from: link.from,
    to: link.to,
  }));
  return { nodes, edges };
}
