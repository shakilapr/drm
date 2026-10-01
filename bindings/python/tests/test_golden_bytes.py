"""Golden byte decoder — independent Python struct reader (third decoder).

Re-declares the frozen vector lists (canon-codec.md I1/I9; work-plan
M01W4-E4-T3) and verifies the committed `.hex` fixtures:
24 primitive vectors via `struct.unpack("<fmt", ...)`, 6 composite two-region
vectors via `struct.unpack_from` with structural checks (descriptor counts,
offsets, derived sample_len, zeroed gap bytes, strict UTF-8).

REQ: REQ-TOOL-043, REQ-TOOL-044, REQ-TOOL-040, REQ-TOOL-042.
"""

from __future__ import annotations

import struct
from collections.abc import Callable
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
PRIMS_DIR = REPO / "tests" / "fixtures" / "golden" / "primitives"
COMPS_DIR = REPO / "tests" / "fixtures" / "golden" / "composites"


def read_hex(path: Path) -> bytes:
    """Strict lowercase-hex fixture load."""
    text = path.read_text(encoding="ascii").strip()
    assert text and len(text) % 2 == 0, path
    assert all(c in "0123456789abcdef" for c in text), path
    return bytes.fromhex(text)


def u16(buf: bytes, off: int) -> int:
    return int(struct.unpack_from("<H", buf, off)[0])


def u32(buf: bytes, off: int) -> int:
    return int(struct.unpack_from("<I", buf, off)[0])


def u64(buf: bytes, off: int) -> int:
    return int(struct.unpack_from("<Q", buf, off)[0])


def f32(buf: bytes, off: int) -> float:
    return float(struct.unpack_from("<f", buf, off)[0])


def align_up(x: int, a: int) -> int:
    assert a > 0 and (a & (a - 1)) == 0
    return (x + a - 1) & ~(a - 1)


def utf8_strict(buf: bytes, off: int, n: int) -> str:
    """I10: malformed sequences must raise (never silently replaced)."""
    return buf[off : off + n].decode("utf-8", "strict")


def test_primitives_24() -> None:
    """All 24 primitive vectors: frozen oracle values, byte-exact."""
    # stem -> (struct format, frozen value); re-declared, never generated.
    frozen: dict[str, tuple[str, int | float]] = {
        "001_u8_zero": ("B", 0),
        "002_u8_max_signed": ("B", 127),
        "003_u8_max": ("B", 255),
        "004_i8_zero": ("b", 0),
        "005_i8_min": ("b", -128),
        "006_i8_max": ("b", 127),
        "007_u16_zero": ("H", 0),
        "008_u16_max_signed": ("H", 32767),
        "009_u16_max": ("H", 65535),
        "010_i16_zero": ("h", 0),
        "011_i16_min": ("h", -32768),
        "012_i16_max": ("h", 32767),
        "013_u32_zero": ("I", 0),
        "014_u32_max": ("I", 4294967295),
        "015_i32_zero": ("i", 0),
        "016_i32_min": ("i", -2147483648),
        "017_i32_max": ("i", 2147483647),
        "018_f32_zero": ("f", 0.0),
        "019_f32_one": ("f", 1.0),
        "020_f32_smallest_normal": ("f", 1.1754943508222875e-38),
        "021_f32_max": ("f", 3.4028234663852886e38),
        "022_f32_neg_max": ("f", -3.4028234663852886e38),
        "023_bool_false": ("B", 0),
        "024_bool_true": ("B", 1),
    }
    assert len(frozen) == 24
    for stem, (fmt, want) in frozen.items():
        buf = read_hex(PRIMS_DIR / f"{stem}.hex")
        assert len(buf) == struct.calcsize(f"<{fmt}"), stem
        got = struct.unpack(f"<{fmt}", buf)[0]
        assert got == want, f"{stem}: {got!r} != {want!r}"


def _check_001(buf: bytes) -> None:
    """{u8 a, u64 b}: a=0x5A, b=0x1122334455667788; a@0, pad 7, b@8."""
    assert len(buf) == 16
    assert buf[0] == 0x5A
    assert buf[1:8] == b"\x00" * 7
    assert u64(buf, 8) == 0x1122_3344_5566_7788


def _check_002(buf: bytes) -> None:
    """{u16 t, string[8] s, u32[<=4] v}: t=4242, s="abc" (3), v=[1,2].
    Descriptors 2x8 @4..20; offsets derived s=20, v=align_up(23,4)=24."""
    assert len(buf) == 32
    assert u16(buf, 0) == 4242
    count_s = u32(buf, 4)
    count_v = u32(buf, 12)
    assert count_s <= 8 and count_v <= 4
    off_s = align_up(20, 1)
    off_v = align_up(20 + count_s, 4)
    assert u32(buf, 8) == off_s and u32(buf, 16) == off_v
    assert off_v + count_v * 4 == 32
    assert count_s == 3 and count_v == 2
    assert utf8_strict(buf, off_s, count_s) == "abc"
    assert buf[23] == 0  # gap byte zeroed
    assert u32(buf, off_v) == 1 and u32(buf, off_v + 4) == 2


def _check_003(buf: bytes) -> None:
    """string[16]: "héllo" — 6 UTF-8 bytes (é = C3 A9), byte length counts
    bytes not characters."""
    assert len(buf) == 10
    count = u32(buf, 0)
    assert count <= 16 and 4 + count == len(buf) and count == 6
    assert utf8_strict(buf, 4, count) == "héllo"


def _check_004(buf: bytes) -> None:
    """{f32 x, u64[<=2] v}: x=1.5, v=[7]; bare count descriptor (no offset
    field); body element-aligned @8."""
    assert len(buf) == 16
    assert f32(buf, 0) == 1.5
    count = u32(buf, 4)
    assert count <= 2
    off_v = align_up(8, 8)
    assert off_v == 8 and off_v + count * 8 == len(buf)
    assert count == 1 and u64(buf, off_v) == 7


def _check_005(buf: bytes) -> None:
    """{u8 a, bool[2] f, bytes[<=4] b}: a=7, f=[1,0], b empty (count 0);
    sample_len == header_end == 8."""
    assert len(buf) == 8
    assert buf[0] == 7 and buf[1] == 1 and buf[2] == 0 and buf[3] == 0
    count = u32(buf, 4)
    assert count <= 4
    off_b = align_up(8, 1)
    assert off_b == 8 and off_b + count == len(buf) and count == 0


def _check_006(buf: bytes) -> None:
    """{u32 n, string[4] s, bytes[<=4] b, u16 k, u32[<=2] w}: n=1, s="xy" (2),
    b=[9] (1), k=5, w=[1]. Descriptors 3x8 @8..32; offsets derived s=32,
    b=34, w=36; gap byte @35 zeroed; sample_len = 40."""
    assert len(buf) == 40
    assert u32(buf, 0) == 1 and u16(buf, 4) == 5
    assert buf[6] == 0 and buf[7] == 0
    count_s = u32(buf, 8)
    count_b = u32(buf, 16)
    count_w = u32(buf, 24)
    assert count_s <= 4 and count_b <= 4 and count_w <= 2
    off_s = align_up(32, 1)
    off_b = off_s + count_s
    off_w = align_up(off_b + count_b, 4)
    assert u32(buf, 12) == off_s
    assert u32(buf, 20) == off_b
    assert u32(buf, 28) == off_w
    assert off_w + count_w * 4 == 40
    assert count_s == 2 and count_b == 1 and count_w == 1
    assert utf8_strict(buf, off_s, count_s) == "xy"
    assert buf[off_b] == 9 and buf[35] == 0
    assert u32(buf, off_w) == 1


COMPOSITE_CHECKS: list[tuple[str, Callable[[bytes], None]]] = [
    ("001_u8_u64_padded", _check_001),
    ("002_two_region_str_bounded", _check_002),
    ("003_string16_utf8", _check_003),
    ("004_bounded_u64_aligned", _check_004),
    ("005_empty_bodied", _check_005),
    ("006_three_variable_order", _check_006),
]


def test_composites_6() -> None:
    """All 6 two-region vectors: structural decode + frozen values."""
    assert len(COMPOSITE_CHECKS) == 6
    for stem, check in COMPOSITE_CHECKS:
        buf = read_hex(COMPS_DIR / f"{stem}.hex")
        check(buf)


def test_utf8_malformed_rejected() -> None:
    """I10: strict decode raises on the canonical malformed families."""
    for bad in (b"\xc0\xaf", b"\xe0\x80\x80", b"\xed\xa0\x80", b"\xf5\x80\x80\x80", b"\xc3"):
        try:
            bad.decode("utf-8", "strict")
        except UnicodeDecodeError:
            continue
        raise AssertionError(f"malformed sequence accepted: {bad!r}")
