// Nodes activity: read-only node list with each node's publish/consume
// interface. Renders the engine snapshot only (I-ST1, REQ-STUDIO-003).
import type { ModelView } from "../rpc/model";

/** Props for {@link NodesActivity}. */
export interface NodesActivityProps {
  readonly model: ModelView;
}

/** Renders a comma-separated interface list, or an em dash when empty. */
function interfaceList(items: readonly string[]): string {
  return items.length === 0 ? "—" : items.join(", ");
}

/** Node list grouped by nothing but name (read-only). */
export function NodesActivity({ model }: NodesActivityProps): React.JSX.Element {
  return (
    <div className="activity nodes-activity" data-testid="nodes-activity">
      <h2>Nodes</h2>
      <ul className="nodes-list">
        {model.nodes.map((node) => (
          <li key={node.name} className="node">
            <span className="node-name">{node.name}</span>
            <dl className="node-interface">
              <dt>publishes</dt>
              <dd>{interfaceList(node.publishes)}</dd>
              <dt>consumes</dt>
              <dd>{interfaceList(node.consumes)}</dd>
            </dl>
          </li>
        ))}
      </ul>
    </div>
  );
}
