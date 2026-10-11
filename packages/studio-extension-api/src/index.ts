// The only permitted Studio extension surface (codebase.md 2.3 R3). Internals,
// UI stores, and Electron APIs are never exposed here. The wasm guest reaches
// this surface through the `drm_studio_api` import implemented by
// `crates/drm-extension-host`; the host enforces a fuel + memory budget and
// stops a runaway extension deterministically (studio-protocol.md I-ST5).
// REQ: REQ-STUDIO-021.

/** Host import module the wasm guest binds against. REQ: REQ-STUDIO-021. */
export const HOST_API_MODULE = "drm_studio_api";

/** Host import the wasm guest binds against. REQ: REQ-STUDIO-021. */
export const HOST_API_CALL = "call";

/** Guest entry point every extension module exports. REQ: REQ-STUDIO-021. */
export const ENTRY_POINT = "drm_ext_run";

/** Optional guest teardown export, run during deterministic teardown. */
export const TEARDOWN_POINT = "drm_ext_teardown";

/** Fixed extension resource budget. REQ: REQ-STUDIO-004. */
export interface ExtensionBudget {
  /** Instruction (fuel) budget; exhaustion is a deterministic stop. */
  fuel: number;
  /** Linear-memory cap in bytes. */
  maxMemoryBytes: number;
}

/** Default budget: 10M fuel, 16 MiB. REQ: REQ-STUDIO-004. */
export const DEFAULT_BUDGET: ExtensionBudget = {
  fuel: 10_000_000,
  maxMemoryBytes: 16 * 1024 * 1024,
};

/**
 * The `studio-extension-api` host surface an extension may call. It exposes
 * domain capabilities only — never raw machine or shared-memory access.
 * REQ: REQ-STUDIO-021.
 */
export interface StudioApi {
  /** Handle one guest request and return the reply. REQ: REQ-STUDIO-021. */
  call(request: number): number;
}

/** Deterministic reason an extension stopped. REQ: REQ-STUDIO-004. */
export type ExtensionStop =
  | { kind: "fuel_exhausted" }
  | { kind: "memory_limit"; limit: number }
  | { kind: "trap"; message: string };

/** Per-invocation metrics reported by the host. REQ: REQ-STUDIO-021. */
export interface CallOutcome {
  /** Fuel consumed by the invocation. REQ: REQ-STUDIO-004. */
  fuelConsumed: number;
  /** Host `call` invocations made during the invocation. REQ: REQ-STUDIO-021. */
  apiCalls: number;
}

/** Recovery strategy after a mid-call stop. REQ: REQ-STUDIO-022. */
export type RecoveryStrategy = "supervisor_restart" | "fail_closed";

/** The strategy chosen by the M22W4 chaos EXP. REQ: REQ-STUDIO-022. */
export const CHOSEN_RECOVERY: RecoveryStrategy = "supervisor_restart";
