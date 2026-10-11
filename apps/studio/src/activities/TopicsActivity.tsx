// Topics activity: read-only list/table of declared topics (topic, package,
// schema hash). Renders the engine snapshot only (I-ST1, REQ-STUDIO-003).
import { allTopics } from "../rpc/model";
import type { ModelView } from "../rpc/model";

/** Props for {@link TopicsActivity}. */
export interface TopicsActivityProps {
  readonly model: ModelView;
}

/** Searchable-in-place topic table (read-only). */
export function TopicsActivity({ model }: TopicsActivityProps): React.JSX.Element {
  const topics = allTopics(model);
  return (
    <div className="activity topics-activity" data-testid="topics-activity">
      <h2>Topics</h2>
      <table className="topics-table">
        <thead>
          <tr>
            <th scope="col">Topic</th>
            <th scope="col">Package</th>
            <th scope="col">Schema hash</th>
          </tr>
        </thead>
        <tbody>
          {topics.map((topic) => (
            <tr key={topic.path}>
              <td>{topic.path}</td>
              <td>{topic.package}</td>
              <td>
                <code>{topic.schemaHash}</code>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
