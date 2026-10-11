;; Sample Studio extension (work-plan M22W4-E3-T1). It exercises the
;; `studio-extension-api` host surface: `drm_ext_run` makes one host `call`,
;; and `drm_ext_teardown` makes one teardown `call`. The module is plain WAT so
;; the E2E suite needs no wasm toolchain. REQ: REQ-STUDIO-021.
(module
  (import "drm_studio_api" "call" (func $call (param i32) (result i32)))
  (func (export "drm_ext_run")
    (drop (call $call (i32.const 7))))
  (func (export "drm_ext_teardown")
    (drop (call $call (i32.const -1)))))
