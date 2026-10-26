//! Attached data-path C ABI ([attached-data-path.md](../../../docs/algorithms/runtime/attached-data-path.md),
//! `REQ-CORE-107`): a publisher handle and a consumer handle over one real
//! shared-memory segment, exercised through the exported C entry points only —
//! the surface a non-Rust bridge (ros2 first) consumes.
//! REQ: REQ-CORE-107, REQ-CORE-102, REQ-CORE-103.

// Test scaffolding: a setup failure must fail the test loudly rather than thread
// a Result through every fixture step, and the narrowing casts are bounded by
// the fixture constants (16-byte payloads, 8-byte sequence heads).
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation
)]

use std::ffi::c_void;
use std::ptr;

use drm_core::shm::{PosixShm, SegmentName, ShmBackend};
use drm_ffi::attached::{
    drm_attached, drm_attached_close, drm_attached_open, drm_attached_open_aligned,
    drm_attached_publish, drm_attached_read_latest, drm_attached_read_next,
};

/// Success code documented in `drm.h`.
const DRM_OK: i32 = 0;

/// Synthetic handle both sides pass.
const HANDLE: u32 = 91;
/// Payload capacity.
const SAMPLE_MAX: u64 = 16;
/// History depth.
const DEPTH: u64 = 3;
/// Pool size (`>= DEPTH + 1`).
const POOL: u64 = 5;

/// Unique segment name per test process.
fn segment() -> String {
    format!("/drm.attached.abi.{}", std::process::id())
}

/// Opens a handle in the given mode.
///
/// # Safety
/// Test-only helper; pointers come from `c""` literals.
unsafe fn open(mode: u8, name: &std::ffi::CStr) -> Result<*mut drm_attached, i32> {
    let locks = c"/tmp/drm-attached-abi-locks";
    let hashes = [7_u8; 16];
    let mut handle: *mut drm_attached = ptr::null_mut();
    // SAFETY: every pointer is a valid C string / 16-byte array; `out` is writable.
    let rc = unsafe {
        drm_attached_open(
            name.as_ptr(),
            c"drm/attached/abi".as_ptr(),
            locks.as_ptr(),
            hashes.as_ptr(),
            hashes.as_ptr(),
            HANDLE,
            SAMPLE_MAX,
            DEPTH,
            POOL,
            0,
            mode,
            1_000,
            &raw mut handle,
        )
    };
    if rc == DRM_OK { Ok(handle) } else { Err(rc) }
}

/// `REQ-CORE-107` — publish through the attached ABI, then read the same
/// samples back through a consumer handle attached to the same segment.
#[test]
fn attached_abi_publish_then_read() {
    let name = segment();
    let cname = std::ffi::CString::new(name.clone()).expect("segment name");
    std::fs::create_dir_all("/tmp/drm-attached-abi-locks").expect("lock dir");
    let seg = SegmentName::new(&name).expect("segment name");
    let _ = PosixShm.unlink(&seg);

    // SAFETY: test scaffolding over the exported C ABI.
    unsafe {
        let publisher = open(1, &cname).expect("publisher open");

        // Publish three distinguishable samples.
        for n in 0u64..3 {
            let mut payload = [0_u8; SAMPLE_MAX as usize];
            payload[..8].copy_from_slice(&n.to_le_bytes());
            let mut seq = u64::MAX;
            let rc = drm_attached_publish(publisher, payload.as_ptr(), payload.len(), &raw mut seq);
            assert_eq!(rc, DRM_OK, "publish {n}");
            assert_eq!(seq, n, "sequence");
        }

        // A consumer attaches to the live segment. Sequential first, from a fresh
        // cursor: the retained window is replayed oldest-first.
        let consumer = open(0, &cname).expect("consumer open");
        let mut buf = [0_u8; SAMPLE_MAX as usize];
        let mut len = 0usize;
        let mut seen = Vec::new();
        let mut missed = u64::MAX;
        loop {
            let rc = drm_attached_read_next(
                consumer,
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut len,
                &raw mut missed,
            );
            if rc != DRM_OK {
                break;
            }
            seen.push(u64::from_le_bytes(buf[..8].try_into().expect("head")));
        }
        assert_eq!(seen, vec![0, 1, 2], "sequential window");

        // `read_latest` is independent of the cursor: it returns the newest
        // committed sample, and advances the cursor past it.
        let rc = drm_attached_read_latest(consumer, buf.as_mut_ptr(), buf.len(), &raw mut len);
        assert_eq!(rc, DRM_OK, "read_latest");
        assert_eq!(len, SAMPLE_MAX as usize, "payload length");
        assert_eq!(u64::from_le_bytes(buf[..8].try_into().expect("head")), 2);

        assert_eq!(drm_attached_close(consumer), DRM_OK);
        assert_eq!(drm_attached_close(publisher), DRM_OK);
        // Null handles are rejected, not dereferenced.
        assert_ne!(drm_attached_close(ptr::null_mut()), DRM_OK);
    }
    let _ = PosixShm.unlink(&seg);
    let _ = std::mem::size_of::<*mut c_void>();
}

/// `REQ-CORE-107`, `REQ-CORE-103` — a deployment whose `slot_align` is not the
/// ABI's fixed 8 must be able to create and attach its own segment through the C
/// ABI. `drm_attached_open_aligned` takes the generated alignment, so both sides
/// agree on the stride; the fixed-8 entry point would lay out a different
/// segment size and the aligned consumer would refuse it (I-AT1).
#[test]
fn attached_abi_aligned_segment_round_trip() {
    /// Alignment a `u8[<=N]` deployment declares (`struct_align` of the layout).
    const ALIGN: u64 = 4;
    /// Its own topic handle: the publisher lock is per handle, and the
    /// fixed-alignment test above runs concurrently in another thread.
    const ALIGNED_HANDLE: u32 = HANDLE + 1;
    /// A capacity that is NOT a multiple of 8, so align 4 and align 8 really do
    /// produce different strides (20 vs 24). With a multiple of 8 the two
    /// alignments coincide and the test would prove nothing.
    const CAPACITY: u64 = 20;

    let name = format!("/drm.attached.abi.aligned.{}", std::process::id());
    let cname = std::ffi::CString::new(name.clone()).expect("segment name");
    std::fs::create_dir_all("/tmp/drm-attached-abi-locks").expect("lock dir");
    let seg = SegmentName::new(&name).expect("segment name");
    let _ = PosixShm.unlink(&seg);
    let locks = c"/tmp/drm-attached-abi-locks";
    let hashes = [7_u8; 16];

    // SAFETY: test scaffolding over the exported C ABI; every pointer is valid.
    unsafe {
        let mut publisher: *mut drm_attached = ptr::null_mut();
        let rc = drm_attached_open_aligned(
            cname.as_ptr(),
            c"drm/attached/abi".as_ptr(),
            locks.as_ptr(),
            hashes.as_ptr(),
            hashes.as_ptr(),
            ALIGNED_HANDLE,
            CAPACITY,
            DEPTH,
            POOL,
            0,
            1,
            /*timeout_ms=*/ 1_000,
            ALIGN,
            &raw mut publisher,
        );
        assert_eq!(rc, DRM_OK, "aligned publisher open");

        for n in 0u64..3 {
            let mut payload = [0_u8; CAPACITY as usize];
            payload[..8].copy_from_slice(&n.to_le_bytes());
            let rc =
                drm_attached_publish(publisher, payload.as_ptr(), payload.len(), ptr::null_mut());
            assert_eq!(rc, DRM_OK, "publish {n}");
        }

        // A consumer that knows the same deployment alignment attaches and reads
        // the samples the publisher wrote — the stride agrees.
        let mut consumer: *mut drm_attached = ptr::null_mut();
        let rc = drm_attached_open_aligned(
            cname.as_ptr(),
            c"drm/attached/abi".as_ptr(),
            locks.as_ptr(),
            hashes.as_ptr(),
            hashes.as_ptr(),
            ALIGNED_HANDLE,
            CAPACITY,
            DEPTH,
            POOL,
            0,
            0,
            /*timeout_ms=*/ 1_000,
            ALIGN,
            &raw mut consumer,
        );
        assert_eq!(rc, DRM_OK, "aligned consumer open");

        let mut buf = [0_u8; CAPACITY as usize];
        let mut len = 0usize;
        let mut seen = Vec::new();
        let mut missed = u64::MAX;
        loop {
            let rc = drm_attached_read_next(
                consumer,
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut len,
                &raw mut missed,
            );
            if rc != DRM_OK {
                break;
            }
            seen.push(u64::from_le_bytes(buf[..8].try_into().expect("head")));
        }
        assert_eq!(seen, vec![0, 1, 2], "aligned sequential window");

        // A consumer that assumes the ABI default (8) must be refused rather than
        // handed another slot's bytes.
        let mut wrong: *mut drm_attached = ptr::null_mut();
        let rc = drm_attached_open(
            cname.as_ptr(),
            c"drm/attached/abi".as_ptr(),
            locks.as_ptr(),
            hashes.as_ptr(),
            hashes.as_ptr(),
            ALIGNED_HANDLE,
            CAPACITY,
            DEPTH,
            POOL,
            0,
            0,
            /*timeout_ms=*/ 200,
            &raw mut wrong,
        );
        assert_ne!(
            rc, DRM_OK,
            "a fixed-8 consumer must refuse an align-4 segment"
        );
        assert!(wrong.is_null(), "a refused open leaves the handle null");

        assert_eq!(drm_attached_close(consumer), DRM_OK);
        assert_eq!(drm_attached_close(publisher), DRM_OK);
    }
    let _ = PosixShm.unlink(&seg);
}
