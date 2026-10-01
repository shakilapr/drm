/*
 * golden_composites.c — independent C decoder for the hand-authored composite
 * golden vectors (docs/algorithms/canon-codec.md §Test Matrix, work-plan
 * M01W4-E4-T3; REQ-TOOL-040, REQ-TOOL-042, REQ-TOOL-033, REQ-CORE-083).
 *
 * Clean-room: expected byte images and structural expectations are re-derived
 * here from the layout rules (declaration order, natural alignment, u32 count
 * + u32 offset descriptors when > 1 variable member, element-aligned
 * contiguous bodies, zeroed alignment gaps, derived sample_len) — no shared
 * code with the Rust canon implementation.
 *
 * Usage: golden_composites [fixtures-dir]
 */
#include <stdio.h>
#include <string.h>

#define MAX_SAMPLE 256

static unsigned rd_u32(const unsigned char *b, size_t off) {
    return (unsigned)b[off] | ((unsigned)b[off + 1] << 8) | ((unsigned)b[off + 2] << 16) |
           ((unsigned)b[off + 3] << 24);
}

static unsigned long long rd_u64(const unsigned char *b, size_t off) {
    return (unsigned long long)rd_u32(b, off) | ((unsigned long long)rd_u32(b, off + 4) << 32);
}

static unsigned rd_u16(const unsigned char *b, size_t off) {
    return (unsigned)b[off] | ((unsigned)b[off + 1] << 8);
}

static size_t align_up(size_t x, size_t a) { return (x + (a - 1)) & ~(a - 1); }

/* Strict UTF-8 validation (I10): rejects C0/C1 leads, overlong forms,
 * surrogates, > U+10FFFF, and truncated sequences. */
static int utf8_valid(const unsigned char *s, size_t n) {
    size_t i = 0;
    while (i < n) {
        unsigned char c = s[i];
        if (c < 0x80u) {
            i++;
            continue;
        }
        if (c >= 0xC2u && c <= 0xDFu) {
            if (i + 1 >= n || (s[i + 1] & 0xC0u) != 0x80u) {
                return 0;
            }
            i += 2;
            continue;
        }
        if (c == 0xE0u) {
            if (i + 2 >= n || (s[i + 1] & 0xE0u) != 0xA0u || (s[i + 2] & 0xC0u) != 0x80u) {
                return 0;
            }
            i += 3;
            continue;
        }
        if ((c >= 0xE1u && c <= 0xECu) || (c >= 0xEEu && c <= 0xEFu)) {
            if (i + 2 >= n || (s[i + 1] & 0xC0u) != 0x80u || (s[i + 2] & 0xC0u) != 0x80u) {
                return 0;
            }
            i += 3;
            continue;
        }
        if (c == 0xEDu) {
            if (i + 2 >= n || (s[i + 1] & 0xE0u) != 0x80u || (s[i + 2] & 0xC0u) != 0x80u) {
                return 0;
            }
            i += 3;
            continue;
        }
        if (c == 0xF0u) {
            if (i + 3 >= n || (s[i + 1] & 0xF0u) != 0x90u || (s[i + 2] & 0xC0u) != 0x80u ||
                (s[i + 3] & 0xC0u) != 0x80u) {
                return 0;
            }
            i += 4;
            continue;
        }
        if (c >= 0xF1u && c <= 0xF4u) {
            if (i + 3 >= n || (s[i + 1] & 0xC0u) != 0x80u || (s[i + 2] & 0xC0u) != 0x80u ||
                (s[i + 3] & 0xC0u) != 0x80u) {
                return 0;
            }
            i += 4;
            continue;
        }
        return 0;
    }
    return 1;
}

static int check_001(const unsigned char *buf, size_t len) {
    /* {u8 a, u64 b}: a=0x5A, b=0x1122334455667788; a@0, pad 7, b@8. */
    if (len != 16) {
        return 0;
    }
    if (buf[0] != 0x5Au) {
        return 0;
    }
    for (size_t i = 1; i < 8; i++) {
        if (buf[i] != 0u) {
            return 0;
        }
    }
    return rd_u64(buf, 8) == 0x1122334455667788ull;
}

static int check_002(const unsigned char *buf, size_t len) {
    /* {u16 t, string[8] s, u32[<=4] v}: t=4242, s="abc" (3), v=[1,2].
     * Descriptors 2x8 @4..20; offsets derived: s@20, v=align_up(20+3,4)=24;
     * sample_len = 24+2*4 = 32; gap byte @23 zeroed. */
    if (len != 32) {
        return 0;
    }
    if (rd_u16(buf, 0) != 4242u || buf[2] != 0u || buf[3] != 0u) {
        return 0;
    }
    unsigned count_s = rd_u32(buf, 4);
    unsigned count_v = rd_u32(buf, 12);
    if (count_s > 8u || count_v > 4u) {
        return 0;
    }
    size_t off_s = align_up((size_t)20, 1);
    size_t off_v = align_up((size_t)20 + count_s, 4);
    if (rd_u32(buf, 8) != (unsigned)off_s || rd_u32(buf, 16) != (unsigned)off_v) {
        return 0;
    }
    if (off_v + (size_t)count_v * 4 != len) {
        return 0;
    }
    if (count_s != 3u || count_v != 2u) {
        return 0;
    }
    if (!utf8_valid(buf + off_s, count_s) || memcmp(buf + off_s, "abc", 3) != 0) {
        return 0;
    }
    if (buf[23] != 0u) {
        return 0;
    }
    return rd_u32(buf, off_v) == 1u && rd_u32(buf, off_v + 4) == 2u;
}

static int check_003(const unsigned char *buf, size_t len) {
    /* string[16]: "héllo" (6 UTF-8 bytes); len@0, body@4..10. */
    static const unsigned char WANT[6] = {0x68u, 0xC3u, 0xA9u, 0x6Cu, 0x6Cu, 0x6Fu};
    if (len != 10) {
        return 0;
    }
    unsigned count = rd_u32(buf, 0);
    if (count > 16u || (size_t)4 + count != len) {
        return 0;
    }
    if (count != 6u) {
        return 0;
    }
    if (!utf8_valid(buf + 4, count)) {
        return 0;
    }
    return memcmp(buf + 4, WANT, 6) == 0;
}

static int check_004(const unsigned char *buf, size_t len) {
    /* {f32 x, u64[<=2] v}: x=1.0, v=[7]; single-variable descriptor is the
     * bare u32 count @4 (no offset field); body element-aligned @8. */
    if (len != 16) {
        return 0;
    }
    if (rd_u32(buf, 0) != 0x3FC00000u) { /* 1.5f bit pattern */
        return 0;
    }
    unsigned count = rd_u32(buf, 4);
    if (count > 2u) {
        return 0;
    }
    size_t off_v = align_up((size_t)8, 8);
    if (off_v != 8u || (size_t)off_v + count * 8 != len) {
        return 0;
    }
    return count == 1u && rd_u64(buf, off_v) == 7ull;
}

static int check_005(const unsigned char *buf, size_t len) {
    /* {u8 a, bool[2] f, bytes[<=4] b}: a=7, f=[1,0], b empty (count 0);
     * descriptor is the bare count @4; body @8, zero tail bytes. */
    if (len != 8) {
        return 0;
    }
    if (buf[0] != 7u || buf[1] != 1u || buf[2] != 0u || buf[3] != 0u) {
        return 0;
    }
    unsigned count = rd_u32(buf, 4);
    if (count > 4u) {
        return 0;
    }
    size_t off_b = align_up((size_t)8, 1);
    if (off_b != 8u || (size_t)off_b + count != len) {
        return 0;
    }
    return count == 0u;
}

static int check_006(const unsigned char *buf, size_t len) {
    /* {u32 n, string[4] s, bytes[<=4] b, u16 k, u32[<=2] w}: n=1, s="xy" (2),
     * b=[9] (1), k=5, w=[1] (1). Fixed: n@0, k@4, pad @6..8; descriptors
     * 3x8 @8..32; offsets derived s=32, b=32+2=34, w=align_up(34+1,4)=36;
     * gap byte @35 zeroed; sample_len = 36+4 = 40. */
    if (len != 40) {
        return 0;
    }
    if (rd_u32(buf, 0) != 1u || rd_u16(buf, 4) != 5u || buf[6] != 0u || buf[7] != 0u) {
        return 0;
    }
    unsigned count_s = rd_u32(buf, 8);
    unsigned count_b = rd_u32(buf, 16);
    unsigned count_w = rd_u32(buf, 24);
    if (count_s > 4u || count_b > 4u || count_w > 2u) {
        return 0;
    }
    size_t off_s = align_up((size_t)32, 1);
    size_t off_b = off_s + count_s;
    size_t off_w = align_up((size_t)off_b + count_b, 4);
    if (rd_u32(buf, 12) != (unsigned)off_s || rd_u32(buf, 20) != (unsigned)off_b ||
        rd_u32(buf, 28) != (unsigned)off_w) {
        return 0;
    }
    if (off_w + (size_t)count_w * 4 != len) {
        return 0;
    }
    if (count_s != 2u || count_b != 1u || count_w != 1u) {
        return 0;
    }
    if (!utf8_valid(buf + off_s, count_s) || memcmp(buf + off_s, "xy", 2) != 0) {
        return 0;
    }
    if (buf[off_b] != 9u || buf[35] != 0u) {
        return 0;
    }
    return rd_u32(buf, off_w) == 1u;
}

static int check_007(const unsigned char *buf, size_t len) {
    /* {enum{off,on} e, u32 x}: e=1, x=0x12345678; e@0, pad 3, x@4. */
    if (len != 8) {
        return 0;
    }
    if (buf[0] != 1u || buf[1] != 0u || buf[2] != 0u || buf[3] != 0u) {
        return 0;
    }
    return rd_u32(buf, 4) == 0x12345678u;
}

static int check_008(const unsigned char *buf, size_t len) {
    /* {u8[4] f, string[4] s}: f=[1,2,3,4]@0, bare count @4 (=4), body "wxyz"@8. */
    if (len != 12) {
        return 0;
    }
    if (memcmp(buf, "\x01\x02\x03\x04", 4) != 0) {
        return 0;
    }
    if (rd_u32(buf, 4) != 4u) {
        return 0;
    }
    return memcmp(buf + 8, "wxyz", 4) == 0;
}

static int check_009(const unsigned char *buf, size_t len) {
    /* {u16 a, u16[<=3] v}: a=0x1234, v empty (count 0); a@0, pad 2, count @4,
     * body element-aligned but empty — sample_len == header_end == 8. */
    if (len != 8) {
        return 0;
    }
    if (rd_u16(buf, 0) != 0x1234u || buf[2] != 0u || buf[3] != 0u) {
        return 0;
    }
    unsigned count = rd_u32(buf, 4);
    if (count > 3u || count != 0u) {
        return 0;
    }
    return align_up((size_t)8, 2) == 8u && 8u + count * 2u == len;
}

static int check_010(const unsigned char *buf, size_t len) {
    /* {u16 t, string[8] s, u64[<=2] v, bytes[<=4] b}: t=0x1234, s="hi" (2),
     * v=[7] (1), b=[9,10] (2). Descriptors 3x8 @4..28; offsets derived
     * s=28, v=align_up(30,8)=32, b=40; gap 30..32 zeroed; sample_len=42. */
    if (len != 42) {
        return 0;
    }
    if (rd_u16(buf, 0) != 0x1234u || buf[2] != 0u || buf[3] != 0u) {
        return 0;
    }
    unsigned count_s = rd_u32(buf, 4);
    unsigned count_v = rd_u32(buf, 12);
    unsigned count_b = rd_u32(buf, 20);
    if (count_s > 8u || count_v > 2u || count_b > 4u) {
        return 0;
    }
    size_t off_s = align_up((size_t)28, 1);
    size_t off_v = align_up((size_t)off_s + count_s, 8);
    size_t off_b = align_up((size_t)off_v + count_v * 8, 1);
    if (rd_u32(buf, 8) != (unsigned)off_s || rd_u32(buf, 16) != (unsigned)off_v ||
        rd_u32(buf, 24) != (unsigned)off_b) {
        return 0;
    }
    if (off_b + (size_t)count_b != len) {
        return 0;
    }
    if (count_s != 2u || count_v != 1u || count_b != 2u) {
        return 0;
    }
    if (!utf8_valid(buf + off_s, count_s) || memcmp(buf + off_s, "hi", 2) != 0) {
        return 0;
    }
    if (buf[30] != 0u || buf[31] != 0u) {
        return 0;
    }
    if (rd_u64(buf, off_v) != 7ull) {
        return 0;
    }
    return buf[off_b] == 9u && buf[off_b + 1] == 10u;
}

static const struct {
    const char *name;
    int (*check)(const unsigned char *buf, size_t len);
    size_t len;
} FIXTURES[] = {
    {"001_u8_u64_padded", check_001, 16},    {"002_two_region_str_bounded", check_002, 32},
    {"003_string16_utf8", check_003, 10},    {"004_bounded_u64_aligned", check_004, 16},
    {"005_empty_bodied", check_005, 8},      {"006_three_variable_order", check_006, 40},
    {"007_enum_member", check_007, 8},       {"008_fixed_array_string", check_008, 12},
    {"009_empty_bounded_u16", check_009, 8}, {"010_three_var_u64_gap", check_010, 42},
};

static int hexval(char c, unsigned *out) {
    if (c >= '0' && c <= '9') {
        *out = (unsigned)(c - '0');
        return 0;
    }
    if (c >= 'a' && c <= 'f') {
        *out = (unsigned)(c - 'a' + 10);
        return 0;
    }
    return 1; /* strict lowercase hex only */
}

static int load_hex(const char *path, unsigned char *out, size_t cap, size_t *out_len) {
    FILE *f = fopen(path, "rb");
    char hex[2 * MAX_SAMPLE];
    size_t n;
    size_t i;
    if (f == NULL) {
        return 1;
    }
    n = fread(hex, 1, sizeof hex, f);
    fclose(f);
    if (n == 0 || (n % 2) != 0) {
        return 1;
    }
    for (i = 0; i < n / 2; i++) {
        unsigned hi;
        unsigned lo;
        if (hexval(hex[2 * i], &hi) != 0 || hexval(hex[2 * i + 1], &lo) != 0) {
            return 1;
        }
        if (i >= cap) {
            return 1;
        }
        out[i] = (unsigned char)((hi << 4) | lo);
    }
    *out_len = n / 2;
    return 0;
}

int main(int argc, char **argv) {
    const char *dir = (argc > 1) ? argv[1] : "tests/fixtures/golden/composites";
    int pass = 0;
    size_t total = sizeof FIXTURES / sizeof FIXTURES[0];
    for (size_t i = 0; i < total; i++) {
        char path[512];
        unsigned char buf[MAX_SAMPLE];
        size_t len = 0;
        (void)snprintf(path, sizeof path, "%s/%s.hex", dir, FIXTURES[i].name);
        if (load_hex(path, buf, MAX_SAMPLE, &len) != 0) {
            printf("FAIL load %s\n", path);
            continue;
        }
        if (FIXTURES[i].check(buf, len)) {
            pass++;
        } else {
            printf("FAIL %s\n", FIXTURES[i].name);
        }
    }
    printf("golden_composites: %d/%zu vectors match (%s)\n", pass, total, dir);
    return (pass == (int)total) ? 0 : 1;
}
