// TanStack Query wiring for engine snapshots. The WorkspaceStore holds
// revisioned workspace semantics; high-rate telemetry never enters it
// (ui-arch §Frontend State). Queries are read-only: mutations stay engine
// capability only (I-ST1, REQ-STUDIO-003, REQ-STUDIO-014).
import { useQuery } from "@tanstack/react-query";
import type { UseQueryResult } from "@tanstack/react-query";
import type { Diagnostic, VerifySummary, WorkspaceSnapshot } from "@drm/studio-protocol";

import type { JsonRpcClient } from "./client";

/** Query key for the revisioned workspace snapshot. */
export const SNAPSHOT_KEY = ["workspace", "snapshot"] as const;

/** Query key for the verify summary. */
export const VERIFY_KEY = ["workspace", "verify"] as const;

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
