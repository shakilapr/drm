/*
 * Golden composite reader - independent C11 two-region decoder for the frozen
 * canon-codec composite vectors (canon-codec.md I1/I9, work-plan M01W4-E4-T3).
 * NOT generated, NOT linked against drm-core: per fixture this reader
 * (a) hex-decodes the committed bytes, (b) compares byte for byte against a
 * C-assembled little-endian encoding built from hand-declared frozen values,
 * and (c) structurally decodes: re-derives canonical body packing from the
 * descriptor counts, validates counts/offsets/sample_len/UTF-8 (I10), and
 * compares every extracted value against the frozen oracle.
 *
 * REQ: REQ-TOOL-042 (two-region layout), REQ-TOOL-040 (element-aligned
 * bodies), REQ-TOOL-001 (structural bounds).
 *
 * Usage: golden_composites <fixtures-dir>
 * Build/run (strict): cc -std=c11 -Wall -Wextra -Wpedantic -Werror -o golden_composites
 * golden_composites.c
 *                     ./golden_composites ../../tests/fixtures/golden/composites
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define GOLDEN_DIR_DEFAULT "./tests/fixtures/golden/composites"
#define MAX_SAMPLE 64

static uint16_t rd16(const uint8_t *b, size_t off) {
    return (uint16_t)((uint16_t)b[off] | ((uint16_t)b[off + 1] << 8));
}

static uint32_t rd32(const uint8_t *b, size_t off) {
    return (uint32_t)b[off] | ((uint32_t)b[off + 1] << 8) | ((uint32_t)b[off + 2] << 16) |
           ((uint32_t)b[off + 3] << 24);
}

static uint64_t rd64(const uint8_t *b, size_t off) {
    return (uint64_t)rd32(b, off) | ((uint64_t)rd32(b, off + 4) << 32);
}

/* Canonical align_up (a is a power of two). */
static size_t align_up(size_t x, size_t a) { return (x + (a - 1)) & ~((size_t)a - 1); }

/* Strict UTF-8 validation (I10): rejects C0/C1 leads, overlong forms,
 * surrogates, > U+10FFFF, and truncated sequences. */
static int utf8_valid(const uint8_t *s, size_t n) {
    size_t i = 0;
    while (i < n) {
        uint8_t c = s[i];
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

static int nibble(char c, uint8_t *out) {
    if (c >= '0' && c <= '9') {
        *out = (uint8_t)(c - '0');
        return 1;
    }
    if (c >= 'a' && c <= 'f') {
        *out = (uint8_t)(c - 'a' + 10);
        return 1;
    }
    return 0; /* strict lowercase hex only */
}

static int read_hex_file(const char *dir, const char *stem, uint8_t *bytes, size_t width) {
    char path[256];
    char hex[2 * MAX_SAMPLE];
    long len;
    size_t got;
    size_t i;
    FILE *f;

    if (snprintf(path, sizeof path, "%s/%s.hex", dir, stem) < 0) {
        return 0;
    }
    f = fopen(path, "rb");
    if (f == NULL) {
        fprintf(stderr, "golden_composites: open %s failed\n", path);
        return 0;
    }
    fseek(f, 0, SEEK_END);
    len = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (len < 0 || (size_t)len != 2 * width || (size_t)len >= sizeof hex) {
        fprintf(stderr, "golden_composites: %s: bad length %ld (want %zu hex chars)\n", path, len,
                2 * width);
        fclose(f);
        return 0;
    }
    got = fread(hex, 1, (size_t)len, f);
    fclose(f);
    if (got != (size_t)len) {
        fprintf(stderr, "golden_composites: %s: short read\n", path);
        return 0;
    }
    for (i = 0; i < width; i++) {
        uint8_t hi;
        uint8_t lo;
        if (!nibble(hex[2 * i], &hi) || !nibble(hex[2 * i + 1], &lo)) {
            fprintf(stderr, "golden_composites: %s: non-hex char\n", path);
            return 0;
        }
        bytes[i] = (uint8_t)((hi << 4) | lo);
    }
    return 1;
}

static void put32(uint8_t *out, size_t off, uint32_t v) {
    out[off] = (uint8_t)(v & 0xFFu);
    out[off + 1] = (uint8_t)((v >> 8) & 0xFFu);
    out[off + 2] = (uint8_t)((v >> 16) & 0xFFu);
    out[off + 3] = (uint8_t)(v >> 24);
}

static void put64(uint8_t *out, size_t off, uint64_t v) {
    put32(out, off, (uint32_t)(v & 0xFFFFFFFFu));
    put32(out, off + 4, (uint32_t)(v >> 32));
}

/* C01: {u8 a, u64 b} - a=0x5A, b=0x1122334455667788; a@0, pad 7, b@8. */
static int check_001(const uint8_t *b, size_t len) {
    uint8_t want[16];
    memset(want, 0, sizeof want);
    want[0] = 0x5Au;
    put64(want, 8, 0x1122334455667788ull);
    if (len != 16 || memcmp(b, want, 16) != 0) {
        return 0;
    }
    return b[0] == 0x5Au && rd64(b, 8) == 0x1122334455667788ull;
}

/* C02: {u16 t, string[8] s, u32[<=4] v} - t=4242, s="abc" (3), v=[1,2].
 * Descriptors 2x8 @4..20; offsets re-derived from counts: s@20,
 * v = align_up(20+3, 4) = 24; sample_len = 24+8 = 32. */
static int check_002(const uint8_t *b, size_t len) {
    uint32_t count_s;
    uint32_t count_v;
    uint32_t off_s_stored;
    uint32_t off_v_stored;
    size_t off_s;
    size_t off_v;
    size_t sample;
    uint8_t want[32];

    if (len != 32) {
        return 0;
    }
    if (rd16(b, 0) != 4242u) {
        return 0;
    }
    count_s = rd32(b, 4);
    off_s_stored = rd32(b, 8);
    count_v = rd32(b, 12);
    off_v_stored = rd32(b, 16);
    if (count_s > 8u || count_v > 4u) {
        return 0;
    }
    off_s = align_up((size_t)20, 1);
    off_v = align_up((size_t)off_s + count_s, 4);
    sample = off_v + (size_t)count_v * 4;
    if ((size_t)off_s_stored != off_s || (size_t)off_v_stored != off_v || sample != len) {
        return 0;
    }
    if (!utf8_valid(b + off_s, count_s)) {
        return 0;
    }
    if (count_s != 3u || memcmp(b + off_s, "abc", 3) != 0) {
        return 0;
    }
    if (count_v != 2u || rd32(b, off_v) != 1u || rd32(b, off_v + 4) != 2u) {
        return 0;
    }
    memset(want, 0, sizeof want);
    want[0] = 0x92u;
    want[1] = 0x10u;
    put32(want, 4, 3u);
    put32(want, 8, (uint32_t)off_s);
    put32(want, 12, 2u);
    put32(want, 16, (uint32_t)off_v);
    memcpy(want + 20, "abc", 3);
    put32(want, 24, 1u);
    put32(want, 28, 2u);
    return memcmp(b, want, 32) == 0;
}

/* C03: string[16] - "hello" with UTF-8 e-acute (6 bytes); len@0, body@4. */
static int check_003(const uint8_t *b, size_t len) {
    static const uint8_t WANT[6] = {0x68u, 0xC3u, 0xA9u, 0x6Cu, 0x6Cu, 0x6Fu};
    uint32_t count;
    uint8_t want[10];

    if (len != 10) {
        return 0;
    }
    count = rd32(b, 0);
    if (count > 16u || count != 6u || (size_t)4 + count != len) {
        return 0;
    }
    if (!utf8_valid(b + 4, count) || memcmp(b + 4, WANT, 6) != 0) {
        return 0;
    }
    memset(want, 0, sizeof want);
    put32(want, 0, 6u);
    memcpy(want + 4, WANT, 6);
    return memcmp(b, want, 10) == 0;
}

/* C04: {f32 x, u64[<=2] v} - x=1.5, v=[7]; single variable member: descriptor
 * is the bare u32 count @4 (no offset field); body element-aligned @8. */
static int check_004(const uint8_t *b, size_t len) {
    uint32_t count;
    size_t off_v;
    size_t sample;
    uint8_t want[16];

    if (len != 16) {
        return 0;
    }
    if (rd32(b, 0) != 0x3FC00000u) { /* 1.5f bit pattern */
        return 0;
    }
    count = rd32(b, 4);
    if (count > 2u) {
        return 0;
    }
    off_v = align_up((size_t)8, 8);
    sample = off_v + (size_t)count * 8;
    if (off_v != 8 || sample != len) {
        return 0;
    }
    if (count != 1u || rd64(b, off_v) != 7u) {
        return 0;
    }
    memset(want, 0, sizeof want);
    want[0] = 0x00u;
    want[1] = 0x00u;
    want[2] = 0xC0u;
    want[3] = 0x3Fu;
    put32(want, 4, 1u);
    put64(want, 8, 7u);
    return memcmp(b, want, 16) == 0;
}

/* C05: {u8 a, bool[2] f, bytes[<=4] b} - a=7, f=[1,0], b empty (count 0);
 * descriptor is the bare count @4; body @8 with zero tail bytes. */
static int check_005(const uint8_t *b, size_t len) {
    uint32_t count;
    size_t off_b;
    size_t sample;
    uint8_t want[8];

    if (len != 8) {
        return 0;
    }
    if (b[0] != 7u || b[1] != 1u || b[2] != 0u) {
        return 0;
    }
    count = rd32(b, 4);
    if (count > 4u) {
        return 0;
    }
    off_b = align_up((size_t)8, 1);
    sample = off_b + count;
    if (off_b != 8 || sample != len) {
        return 0;
    }
    if (count != 0u) {
        return 0;
    }
    memset(want, 0, sizeof want);
    want[0] = 7u;
    want[1] = 1u;
    return memcmp(b, want, 8) == 0;
}

/* C06: {u32 n, string[4] s, bytes[<=4] b, u16 k, u32[<=2] w} - n=1, s="xy",
 * b=[9], k=5, w=[1]; three variable members: descriptors 3x8 @8..32,
 * offsets derived s=32, b=32+2=34, w=align_up(34+1,4)=36; sample_len=40. */
static int check_006(const uint8_t *b, size_t len) {
    uint32_t count_s;
    uint32_t count_b;
    uint32_t count_w;
    size_t off_s;
    size_t off_b;
    size_t off_w;
    size_t sample;
    uint8_t want[40];

    if (len != 40) {
        return 0;
    }
    if (rd32(b, 0) != 1u || rd16(b, 4) != 5u) {
        return 0;
    }
    count_s = rd32(b, 8);
    count_b = rd32(b, 16);
    count_w = rd32(b, 24);
    if (count_s > 4u || count_b > 4u || count_w > 2u) {
        return 0;
    }
    off_s = align_up((size_t)32, 1);
    off_b = off_s + count_s;
    off_w = align_up((size_t)off_b + count_b, 4);
    sample = off_w + (size_t)count_w * 4;
    if ((size_t)rd32(b, 12) != off_s || (size_t)rd32(b, 20) != off_b ||
        (size_t)rd32(b, 28) != off_w || sample != len) {
        return 0;
    }
    if (!utf8_valid(b + off_s, count_s)) {
        return 0;
    }
    if (count_s != 2u || memcmp(b + off_s, "xy", 2) != 0) {
        return 0;
    }
    if (count_b != 1u || b[off_b] != 9u) {
        return 0;
    }
    if (count_w != 1u || rd32(b, off_w) != 1u) {
        return 0;
    }
    memset(want, 0, sizeof want);
    put32(want, 0, 1u);
    want[4] = 5u;
    put32(want, 8, 2u);
    put32(want, 12, (uint32_t)off_s);
    put32(want, 16, 1u);
    put32(want, 20, (uint32_t)off_b);
    put32(want, 24, 1u);
    put32(want, 28, (uint32_t)off_w);
    memcpy(want + 32, "xy", 2);
    want[34] = 9u;
    put32(want, 36, 1u);
    return memcmp(b, want, 40) == 0;
}

struct comp_vec {
    const char *stem;
    size_t len;
    int (*check)(const uint8_t *, size_t);
};

/* Frozen fixture list - append-only, numbered by stem; never reorder. */
static const struct comp_vec VECTORS[] = {
    {"001_u8_u64_padded", 16, check_001}, {"002_two_region_str_bounded", 32, check_002},
    {"003_string16_utf8", 10, check_003}, {"004_bounded_u64_aligned", 16, check_004},
    {"005_empty_bodied", 8, check_005},   {"006_three_variable_order", 40, check_006},
};
#define VECTOR_COUNT (sizeof VECTORS / sizeof VECTORS[0])

int main(int argc, char **argv) {
    const char *dir = argc > 1 ? argv[1] : GOLDEN_DIR_DEFAULT;
    size_t pass = 0;
    size_t i;

    for (i = 0; i < VECTOR_COUNT; i++) {
        const struct comp_vec *g = &VECTORS[i];
        uint8_t bytes[MAX_SAMPLE];

        if (!read_hex_file(dir, g->stem, bytes, g->len)) {
            return 1;
        }
        if (!g->check(bytes, g->len)) {
            fprintf(stderr, "golden_composites: %s: failed frozen oracle\n", g->stem);
            return 1;
        }
        pass++;
    }
    printf("golden_composites: %zu/%zu vectors match (dir %s)\n", pass, VECTOR_COUNT, dir);
    return 0;
}
