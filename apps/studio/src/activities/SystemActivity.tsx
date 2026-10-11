// System activity: read-only computes, links (with derived endpoints), and
// placements. Budgets/routes are engine-derived and surface here once the
// budget/route projections are emitted; the renderer computes none of them
// (I-ST1, REQ-STUDIO-003).
import type { ModelView } from "../rpc/model";

/** Props for {@link SystemActivity}. */
export interface SystemActivityProps {
  readonly model: ModelView;
}

/** Computes, links, and placements (read-only). */
export function SystemActivity({ model }: SystemActivityProps): React.JSX.Element {
  return (
    <div className="activity system-activity" data-testid="system-activity">
      <h2>System</h2>
      <section>
        <h3>Computes</h3>
        <ul className="computes-list">
          {model.computes.map((compute) => (
            <li key={compute.name}>{compute.name}</li>
          ))}
        </ul>
      </section>
      <section>
        <h3>Links</h3>
        <ul className="links-list">
          {model.links.map((link) => (
            <li key={link.id}>
              {link.id}: {link.from} → {link.to}
            </li>
          ))}
        </ul>
      </section>
      <section>
        <h3>Placement</h3>
        <ul className="placement-list">
          {model.placements.map((placement) => (
            <li key={`${placement.node}@${placement.compute}`}>
              {placement.node} @ {placement.compute}
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
