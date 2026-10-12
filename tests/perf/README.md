# tests/perf — performance fixtures

Test packages and nodes for the system performance matrix
([`cargo xtask perf-matrix`](../../xtask/README.md); methodology in
[bench-methodology.md](../../docs/spec/bench-methodology.md) §System perf matrix).

## Layout

`workspace/` is a normal DRM workspace (`drm build` accepts it) holding the
payload classes the matrix measures:

| Topic               | Class                      | Declared `@history` / `@pool_size` / `@overflow` |
| ------------------- | -------------------------- | ------------------------------------------------ |
| `sensor/imu`        | 64 B fixed (IMU-shaped)    | `8` / `12` / `overwrite`                         |
| `sensor/image`      | 1 MiB fixed (frame-shaped) | `latest` / `2` / `overwrite`                     |
| `control/command`   | 32 B fixed, backpressure   | `latest` / `2` / `reject`                        |
| `control/telemetry` | 4 KiB-bounded variable     | `4` / `6` / `overwrite`                          |
| `control/pose`      | 128 B fixed, deep history  | `16` / `20` / `overwrite`                        |

Nodes (`camera`, `controller`, `monitor`) declare publish/consume wiring at the
same fidelity as `examples/mixed_robot`: the manifest build command asserts the
generated include tree and deployment metadata exist. Node sources are smoke
sources; the matrix drives the declared topic layout through the runtime.
