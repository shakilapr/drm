//! Canonical C ABI crate (codebase.md 2.1 layer `runtime`): `extern "C"` exports
//! over drm-core; `bindings/c/include/drm/drm.h` is the single source of truth.
//! Fail-stop: a Rust panic here terminates the node process (no `Internal` status).
//! REQ: REQ-BIND-007 (one status taxonomy mirrored 1:1 to C codes), REQ-BIND-002
//! (explicit close; per-handle thread-safety).
//!
//! Unsafe is permitted here (codebase.md 3.2); exports land W2+ per
//! `docs/algorithms/bindings/c-abi-surface.md`.

#[cfg(test)]
mod tests {
    /// REQ: REQ-BIND-007 — placeholder compile smoke for the W1 skeleton.
    #[test]
    fn skeleton_compiles() {
        let s = drm_types::Status::Busy;
        assert_eq!(s.code(), -11);
    }

    /// REQ: REQ-BIND-007 — I-A4 exhaustive 1:1 map: the `DRM_OK`/`DRM_ERR_*`
    /// codes parsed from `drm.h` equal the `Status::code()` set exactly, in
    /// both directions. `status_codes()` is an exhaustive `match` (no
    /// wildcard), so a new `Status` variant without a header row breaks this
    /// test at compile time; a header-only addition breaks the set compare.
    #[test]
    fn drm_h_status_map_exhaustive() {
        const HEADER: &str = include_str!("../../../bindings/c/include/drm/drm.h");
        let mut c_codes: Vec<i32> = Vec::new();
        for line in HEADER.lines() {
            let Some((name, value)) = line.trim().split_once('=') else {
                continue;
            };
            let name = name.trim();
            if !(name.starts_with("DRM_OK") || name.starts_with("DRM_ERR_")) {
                continue;
            }
            let code: i32 = value
                .trim()
                .trim_end_matches(',')
                .parse()
                .unwrap_or_else(|e| unreachable!("drm.h `{name}`: {e:?}"));
            assert!(
                !c_codes.contains(&code),
                "duplicate drm.h code {code} ({name})"
            );
            c_codes.push(code);
        }

        let rust_codes = status_codes();
        assert_eq!(c_codes.len(), rust_codes.len(), "drm.h vs Status count");
        for c in &rust_codes {
            assert!(c_codes.contains(c), "Status code {c} missing from drm.h");
        }
        for c in &c_codes {
            assert!(rust_codes.contains(c), "drm.h code {c} missing from Status");
        }
    }

    /// Exhaustive (no wildcard) so adding a `Status` variant without a header
    /// row fails compilation of this test.
    fn status_codes() -> Vec<i32> {
        use drm_types::Status as S;
        let all = [
            S::Ok,
            S::InvalidArg,
            S::NotFound,
            S::AlreadyExists,
            S::Permission,
            S::QueueFull,
            S::Gap,
            S::Stale,
            S::WrongAbi,
            S::PoolExhausted,
            S::Retry,
            S::Busy,
            S::NoSample,
            S::InvalidBorrow,
            S::GenerationExhausted,
            S::InvalidData,
            S::InvalidConfig,
            S::NotReady,
            S::LockFailed,
            S::SequenceExhausted,
            S::EpochExhausted,
            S::RingStampExhausted,
        ];
        let mut codes = Vec::new();
        for s in all {
            // Or-pattern without wildcard: exhaustive against future variants.
            codes.push(match s {
                S::Ok
                | S::InvalidArg
                | S::NotFound
                | S::AlreadyExists
                | S::Permission
                | S::QueueFull
                | S::Gap
                | S::Stale
                | S::WrongAbi
                | S::PoolExhausted
                | S::Retry
                | S::Busy
                | S::NoSample
                | S::InvalidBorrow
                | S::GenerationExhausted
                | S::InvalidData
                | S::InvalidConfig
                | S::NotReady
                | S::LockFailed
                | S::SequenceExhausted
                | S::EpochExhausted
                | S::RingStampExhausted => s.code(),
            });
        }
        codes
    }
}
