# Security Policy

DRM is real-time robotics middleware; defects can corrupt shared memory between independent processes on a robot.

## Reporting

Report vulnerabilities privately via GitHub Security Advisories ("Report a vulnerability" on the Security tab).

Do not open public issues for suspected vulnerabilities.

## Scope

- `drm-core`: shared-memory lifecycle, hazard slots, crash recovery, epoch handling
- `drm-ffi` / `drm.h`: ABI contract safety
- `drm-router`: framing, transports, parsing of untrusted wire input
- `drm-workspace`: manifest/schema parsing of untrusted input
- DRM Studio: renderer isolation, extension sandboxing, RPC surface (ui-architecture isolation model)

## Response

- Acknowledgment: within 7 days.
- Fix + advisory: P0/P1 correctness or memory-safety defects prioritized per work-plan quality gates; fuzz and sanitizer suites run per PR (core paths) and nightly (codebase §5.2).
- Supported releases: latest tagged release and `main`.
