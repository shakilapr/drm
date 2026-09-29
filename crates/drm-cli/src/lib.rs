//! DRM CLI facade (codebase.md 2.1 layer `facade`): `drm` executable plus the
//! `drm studio --stdio` host; composition of ops crates happens only here.
//! REQ: REQ-CORE-080 (drm clean unlinks only when the exclusive deployment active
//! lock proves no nodes), REQ-STUDIO-003 (every mutation executed and validated by
//! the Rust engine).
//!
//! Body lands W2+; the binary is wired then. `#![forbid(unsafe_code)]` (facade).

#![forbid(unsafe_code)]
