// Studio shell: read-only inspectors (Workspace/Topics/Nodes/System) and the
// Problems panel, all rendering engine snapshots through TanStack Query. The
// renderer holds zero DRM logic and performs no mutations (I-ST1,
// REQ-STUDIO-003, REQ-STUDIO-014).
import { useEffect, useMemo, useState } from "react";

import { GraphActivity } from "./activities/GraphActivity";
import { NodesActivity } from "./activities/NodesActivity";
import { RunActivity } from "./activities/RunActivity";
import { SystemActivity } from "./activities/SystemActivity";
import { TopicsActivity } from "./activities/TopicsActivity";
import { WorkspaceActivity } from "./activities/WorkspaceActivity";
import { ProblemsPanel } from "./components/ProblemsPanel";
import { JsonRpcClient, StdioTransport } from "./rpc/client";
import { parseModel } from "./rpc/model";
import { snapshotProblems, useWorkspaceSnapshot } from "./rpc/queries";

const client = new JsonRpcClient(new StdioTransport());

/** The read-only inspector activities in v1. */
const ACTIVITIES = [
  { id: "workspace", label: "Workspace" },
  { id: "topics", label: "Topics" },
  { id: "nodes", label: "Nodes" },
  { id: "system", label: "System" },
  { id: "graph", label: "Graph" },
  { id: "run", label: "Run" },
] as const;

type ActivityId = (typeof ACTIVITIES)[number]["id"];

/** Studio shell: handshake, snapshot query, activity switch, Problems panel. */
export function App(): React.JSX.Element {
  const [ready, setReady] = useState(false);
  const [activity, setActivity] = useState<ActivityId>("workspace");

  useEffect(() => {
    let cancelled = false;
    const boot = async (): Promise<void> => {
      await client.handshake("1.0.0");
      await client.request({ method: "open_workspace", path: "." });
      if (!cancelled) {
        setReady(true);
      }
    };
    void boot();
    return () => {
      cancelled = true;
    };
  }, []);

  const snapshot = useWorkspaceSnapshot(client, ready);
  const render = snapshot.data?.model ?? "";
  const model = useMemo(() => parseModel(render), [render]);

  return (
    <main className="app">
      <header className="app-header">
        <h1>DRM Studio</h1>
        <nav className="activities" aria-label="Activities">
          {ACTIVITIES.map((item) => (
            <button
              key={item.id}
              type="button"
              aria-pressed={activity === item.id}
              onClick={() => {
                setActivity(item.id);
              }}
            >
              {item.label}
            </button>
          ))}
        </nav>
      </header>
      <section className="primary-sidebar">
        {activity === "workspace" && <WorkspaceActivity model={model} />}
        {activity === "topics" && <TopicsActivity model={model} />}
        {activity === "nodes" && <NodesActivity model={model} />}
        {activity === "system" && <SystemActivity model={model} />}
        {activity === "graph" && <GraphActivity model={model} />}
        {activity === "run" && <RunActivity model={model} client={client} ready={ready} />}
      </section>
      <footer className="bottom-panel">
        <ProblemsPanel problems={snapshotProblems(snapshot.data)} />
      </footer>
    </main>
  );
}
