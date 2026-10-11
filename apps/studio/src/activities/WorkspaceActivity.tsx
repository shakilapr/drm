// Workspace activity: read-only hierarchical tree (packages → topics, plus
// nodes, computes, links). Renders the engine snapshot only; zero mutation
// logic (I-ST1, REQ-STUDIO-003).
import type { ModelView } from "../rpc/model";

/** Props for {@link WorkspaceActivity}. */
export interface WorkspaceActivityProps {
  readonly model: ModelView;
}

/** Full hierarchical workspace tree (read-only). */
export function WorkspaceActivity({ model }: WorkspaceActivityProps): React.JSX.Element {
  return (
    <div className="activity workspace-activity" data-testid="workspace-activity">
      <h2>Workspace</h2>
      <ul className="tree">
        {model.packages.map((pkg) => (
          <li key={pkg.name} className="tree-package">
            <span className="tree-label">package {pkg.name}</span>
            <ul>
              {pkg.topics.map((topic) => (
                <li key={`${pkg.name}/${topic.path}`} className="tree-topic">
                  topic {topic.path}
                </li>
              ))}
            </ul>
          </li>
        ))}
        {model.nodes.map((node) => (
          <li key={`node-${node.name}`} className="tree-node">
            node {node.name}
          </li>
        ))}
        {model.computes.map((compute) => (
          <li key={`compute-${compute.name}`} className="tree-compute">
            compute {compute.name}
          </li>
        ))}
        {model.links.map((link) => (
          <li key={`link-${link.id}`} className="tree-link">
            link {link.id}: {link.from} → {link.to}
          </li>
        ))}
      </ul>
    </div>
  );
}
