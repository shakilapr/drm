// TanStack Query wiring for engine snapshots. The WorkspaceStore holds
// revisioned workspace semantics; high-rate telemetry never enters it
// (ui-arch §Frontend State). Queries are read-only: mutations stay engine
// capability only (I-ST1, REQ-STUDIO-003, REQ-STUDIO-014).
import { useQuery } from "@tanstack/react-query";
import type { UseQueryResult } from "@tanstack/react-query";
import type {
  Diagnostic,
  RunSessionView,
  RuntimeStats,
  VerifySummary,
  WorkspaceSnapshot,
} from "@drm/studio-protocol";

import type { JsonRpcClient } from "./client";

/** Query key for the revisioned workspace snapshot. */
export const SNAPSHOT_KEY = ["workspace", "snapshot"] as const;

/** Query key for the verify summary. */
export const VERIFY_KEY = ["workspace", "verify"] as const;

/** Query key for the supervisor-owned run sessions. */
export const RUN_SESSIONS_KEY = ["run", "sessions"] as const;

/** Query key for the per-node runtime statistics. */
export const RUNTIME_STATS_KEY = ["run", "runtime_stats"] as const;

/** Reads the revisioned snapshot once the host handshake has completed. */
export function useWorkspaceSnapshot(
  client: JsonRpcClient,
  enabled: boolean,
): UseQueryResult<WorkspaceSnapshot> {
  return useQuery({
    queryKey: SNAPSHOT_KEY,
    enabled,
    queryFn: async () => {
      const response = await client.request({ method: "snapshot" });
      if (response.result !== "snapshot") {
        throw new Error(`unexpected snapshot response: ${response.result}`);
      }
      const snapshot: WorkspaceSnapshot = response;
      return snapshot;
    },
  });
}

/** Reads the verify summary; its findings feed the Problems panel. */
export function useVerifySummary(
  client: JsonRpcClient,
  enabled: boolean,
): UseQueryResult<VerifySummary> {
  return useQuery({
    queryKey: VERIFY_KEY,
    enabled,
    queryFn: async () => {
      const response = await client.request({ method: "verify" });
      if (response.result !== "verify") {
        throw new Error(`unexpected verify response: ${response.result}`);
      }
      const summary: VerifySummary = response;
      return summary;
    },
  });
}

/** The problems shown by the Problems panel (empty while loading). */
export function snapshotProblems(snapshot: WorkspaceSnapshot | undefined): Diagnostic[] {
  return snapshot?.problems ?? [];
}

/**
 * Reads the supervisor-owned run sessions (the Run panel's session list). The
 * query is read-only and polls while the panel is mounted; sessions live in the
 * supervisor, never in the renderer (I-ST10, REQ-STUDIO-018).
 */
export function useRunSessions(
  client: JsonRpcClient,
  enabled: boolean,
): UseQueryResult<RunSessionView[]> {
  return useQuery({
    queryKey: RUN_SESSIONS_KEY,
    enabled,
    refetchInterval: 1000,
    queryFn: async () => {
      const response = await client.request({ method: "run_sessions" });
      if (response.result !== "run_sessions") {
        throw new Error(`unexpected run_sessions response: ${response.result}`);
      }
      return response.sessions;
    },
  });
}

/** Reads per-node runtime statistics (`runtimeStats`; REQ-STUDIO-018). */
export function useRuntimeStats(
  client: JsonRpcClient,
  enabled: boolean,
): UseQueryResult<RuntimeStats[]> {
  return useQuery({
    queryKey: RUNTIME_STATS_KEY,
    enabled,
    refetchInterval: 1000,
    queryFn: async () => {
      const response = await client.request({ method: "runtime_stats" });
      if (response.result !== "runtime_stats") {
        throw new Error(`unexpected runtime_stats response: ${response.result}`);
      }
      return response.stats;
    },
  });
}
