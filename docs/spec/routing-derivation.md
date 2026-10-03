# Route Derivation — W1 draft (seed of `docs/algorithms/toolchain/route-derivation.md`, M10W1)

Non-normative derived spec; `docs/architecture.md` §Automatic Routing and Explicit
Overrides wins conflicts. REQ: REQ-ROUTE-003 (≥ 2 paths without policy ⇒ verify error).

## Requirements Language

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all capitals.

## Model

- Vertices: compute units + external endpoints. Edges: physical links
  (`transport`, `bandwidth`).
- Topic scope: a topic is **local** iff publisher and every consumer share one
  compute unit; otherwise a forwarding plan exists per publisher:
  `{vertex: {ingress, egress, local}}` per reached vertex (1→N, multi-hop, mixed
  local/remote in one tree).
- Path search: bridge/connectivity analysis (polynomial), never simple-path
  enumeration.

## Selection pipeline (per topic consumer pair)

1. **config-validity** — `ConflictingRouteOverride` before any selection.
2. **deny** — remove denied links.
3. **force** — unique forced link is mandatory; two forced links ⇒ conflict.
4. **prefer** — unordered set; ambiguity inside `prefer` remains a verify error
   (no silent ranking).
5. **automatic** — unique viable path ⇒ route; ≥ 2 ⇒ `AmbiguousRoute`.

## Fixtures (must match architecture examples)

| # | Topology | Config | Expected |
|---|---|---|---|
| F1 | pc, jetson; ethernet+wifi links | none | verify error `AmbiguousRoute` |
| F2 | same | `deny."camera/front" = ["wifi"]` | camera over ethernet; control still ambiguous |
| F3 | pc, robot_pc; ethernet only | none | control/target auto-routed ethernet |

Conflict fixture: `force."diagnostics" = "ethernet"` + `deny."diagnostics" =
["ethernet"]` ⇒ `ConflictingRouteOverride`.

## Open questions (owned by E3/E2)

1. `prefer` naming a link not in the topic's path set — error or dead entry?
2. External endpoint serial links: always leaf edges (no forwarding through)?
3. Multi-hop with `force` on a non-first hop — pipeline applies per hop?
