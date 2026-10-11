// Run activity: supervisor-driven start/stop/restart plus the running node
// list with pid and status. Sessions are owned by the host's run-session
// supervisor and outlive this window (I-ST10); the renderer only sends typed
// commands and renders engine responses (REQ-STUDIO-018).
import { useQueryClient } from "@tanstack/react-query";
import type { ProcessStatus, RunSessionView, RuntimeStats } from "@drm/studio-protocol";

import type { JsonRpcClient } from "../rpc/client";
import type { ModelView } from "../rpc/model";
import {
  RUN_SESSIONS_KEY,
  RUNTIME_STATS_KEY,
  useRunSessions,
  useRuntimeStats,
} from "../rpc/queries";

/** Props for {@link RunActivity}. */
export interface RunActivityProps {
  readonly model: ModelView;
  readonly client: JsonRpcClient;
  readonly ready: boolean;
}

/** Human label for a node process status. */
function processLabel(status: ProcessStatus): string {
  return typeof status === "string" ? status : `exited(${status.exited})`;
}

/** One supervised session with its node processes. */
function sessionRow(
  session: RunSessionView,
  stats: readonly RuntimeStats[],
  onStop: (id: string) => void,
  onRestart: (id: string) => void,
): React.JSX.Element {
  const byNode = new Map(
    stats.filter((s) => s.session === session.id).map((s) => [s.node, s] as const),
  );
  return (
    <li key={session.id} className="run-session">
      <header className="run-session-header">
        <span className="run-session-id">{session.id}</span>
        <span className="run-session-status">{session.status}</span>
        <span className="run-session-policy">
          {session.leave_running ? "leave running" : "stop on disconnect"}
        </span>
        <button type="button" onClick={() => onStop(session.id)}>
          Stop
        </button>
        <button type="button" onClick={() => onRestart(session.id)}>
          Restart
        </button>
      </header>
      <ul className="run-nodes">
        {session.nodes.map((node) => {
          const observed = byNode.get(node.node);
          return (
            <li key={`${session.id}:${node.node}`} className="run-node">
              <span className="run-node-name">{node.node}</span>
              <span className="run-node-pid">
                pid {observed?.pid ?? node.pid ?? "—"}
              </span>
              <span className="run-node-status">{processLabel(node.status)}</span>
            </li>
          );
        })}
      </ul>
    </li>
  );
}

/** Run configurations and the running/stopped node list. */
export function RunActivity({ model, client, ready }: RunActivityProps): React.JSX.Element {
  const queryClient = useQueryClient();
  const sessions = useRunSessions(client, ready);
  const stats = useRuntimeStats(client, ready);

  const refresh = async (): Promise<void> => {
    await queryClient.invalidateQueries({ queryKey: RUN_SESSIONS_KEY });
    await queryClient.invalidateQueries({ queryKey: RUNTIME_STATS_KEY });
  };

  const start = async (): Promise<void> => {
    await client.request({
      method: "run_start",
      config: { nodes: model.nodes.map((node) => node.name), leave_running: true },
    });
    await refresh();
  };

  const stop = async (session: string): Promise<void> => {
    await client.request({ method: "run_stop", session });
    await refresh();
  };

  const restart = async (session: string): Promise<void> => {
    await client.request({ method: "run_restart", session });
    await refresh();
  };

  const sessionList = sessions.data ?? [];
  const statList = stats.data ?? [];

  return (
    <div className="activity run-activity" data-testid="run-activity">
      <h2>Run</h2>
      <button type="button" disabled={!ready || model.nodes.length === 0} onClick={() => void start()}>
        Start all nodes
      </button>
      {sessionList.length === 0 ? (
        <p className="run-empty">No run sessions.</p>
      ) : (
        <ul className="run-sessions">
          {sessionList.map((session) =>
            sessionRow(session, statList, (id) => void stop(id), (id) => void restart(id)),
          )}
        </ul>
      )}
    </div>
  );
}
