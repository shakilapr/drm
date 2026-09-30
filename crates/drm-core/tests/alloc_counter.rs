//! Zero-allocation codec gate (canon-codec I4; work-plan M01W4-E1-T2).
//! `harness = false` binary: single-threaded, counting allocator installed by
//! the `test-count-allocs` feature, allocator deltas observed around a
//! representative steady-state codec block. Run with:
//! `cargo test -p drm-core --features test-count-allocs`
//! REQ: REQ-CORE-082.

use drm_core::canon;

fn expect<T, E: std::fmt::Debug>(r: Result<T, E>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => unreachable!("alloc gate: {what}: {e:?}"),
    }
}

/// REQ: REQ-CORE-082 — codec steady-state block performs zero allocations.
fn main() {
    let before = (
        drm_core::alloc::counter::ALLOCS.load(std::sync::atomic::Ordering::Relaxed),
        drm_core::alloc::counter::DEALLOCS.load(std::sync::atomic::Ordering::Relaxed),
    );

    // Representative steady-state block: bounds-checked primitive
    // encode/decode over a fixed buffer; no heap types on the data path.
    let mut buf = [0u8; 64];
    expect(canon::write_u32(&mut buf, 0, 0x1234_5678), "write u32");
    expect(canon::write_i16(&mut buf, 4, -300), "write i16");
    expect(canon::write_bool(&mut buf, 6, true), "write bool");
    for i in 0..16u16 {
        expect(
            canon::write_u16(&mut buf, 8 + 2 * usize::from(i), u16::from(i) * 3),
            "write u16",
        );
    }
    let u32v = expect(canon::read_u32(&buf, 0), "read u32");
    let i16v = expect(canon::read_i16(&buf, 4), "read i16");
    let bv = expect(canon::read_bool(&buf, 6), "read bool");
    assert_eq!(u32v, 0x1234_5678u32);
    assert_eq!(i16v, -300i16);
    assert!(bv);
    assert_eq!(canon::read_u16(&buf, 8 + 2 * 15), Ok(45u16));

    let after = (
        drm_core::alloc::counter::ALLOCS.load(std::sync::atomic::Ordering::Relaxed),
        drm_core::alloc::counter::DEALLOCS.load(std::sync::atomic::Ordering::Relaxed),
    );
    assert_eq!(after.0 - before.0, 0, "allocations on the codec path");
    assert_eq!(after.1 - before.1, 0, "deallocations on the codec path");
    println!("alloc_counter: codec block allocation-free (0/0)");
}
