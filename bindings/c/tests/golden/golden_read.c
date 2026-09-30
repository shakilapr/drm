/*
 * Golden primitive reader - independent C11 decoder for the frozen canon-codec
 * primitive vectors (canon-codec.md, work-plan M01W3-E4-T4). NOT generated and
 * NOT linked against drm-core: the frozen vector list is re-declared here as an
 * independent oracle, and every fixture is (a) hex-decoded, (b) compared byte
 * for byte against a C-assembled little-endian encoding, and (c) decoded back
 * to a value and compared. A generator or codec regression cannot mask itself.
 *
 * REQ: REQ-TOOL-043 (canonical little-endian), REQ-TOOL-044 (standard sizes).
 *
 * Usage: golden_read <fixtures-dir>
 * Build/run (strict): cc -std=c11 -Wall -Wextra -Wpedantic -Werror -o golden_read golden_read.c
 *                     ./golden_read ../../tests/fixtures/golden/primitives
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define GOLDEN_DIR_DEFAULT "./tests/fixtures/golden/primitives"
#define NAME_MAX_LEN 64

enum gtype { GT_U8, GT_I8, GT_U16, GT_I16, GT_U32, GT_I32, GT_F32, GT_BOOL };

struct golden_vec {
    const char *stem; /* NNN_<type>_<case> */
    enum gtype ty;
    uint8_t width;
    union {
        uint8_t u8;
        int8_t i8;
        uint16_t u16;
        int16_t i16;
        uint32_t u32;
        int32_t i32;
        float f32;
        int b;
    } v;
};

/* Frozen vector list (canon-codec.md, work-plan M01W3-E4-T3). Append-only:
 * entries are numbered by fixture stem; never reorder, never renumber. */
static const struct golden_vec VECTORS[] = {
    {"001_u8_zero", GT_U8, 1, {.u8 = 0u}},
    {"002_u8_max_signed", GT_U8, 1, {.u8 = 127u}},
    {"003_u8_max", GT_U8, 1, {.u8 = 255u}},
    {"004_i8_zero", GT_I8, 1, {.i8 = 0}},
    {"005_i8_min", GT_I8, 1, {.i8 = -128}},
    {"006_i8_max", GT_I8, 1, {.i8 = 127}},
    {"007_u16_zero", GT_U16, 2, {.u16 = 0u}},
    {"008_u16_max_signed", GT_U16, 2, {.u16 = 32767u}},
    {"009_u16_max", GT_U16, 2, {.u16 = 65535u}},
    {"010_i16_zero", GT_I16, 2, {.i16 = 0}},
    {"011_i16_min", GT_I16, 2, {.i16 = -32768}},
    {"012_i16_max", GT_I16, 2, {.i16 = 32767}},
    {"013_u32_zero", GT_U32, 4, {.u32 = 0u}},
    {"014_u32_max", GT_U32, 4, {.u32 = 4294967295u}},
    {"015_i32_zero", GT_I32, 4, {.i32 = 0}},
    {"016_i32_min", GT_I32, 4, {.i32 = -2147483648}},
    {"017_i32_max", GT_I32, 4, {.i32 = 2147483647}},
    {"018_f32_zero", GT_F32, 4, {.f32 = 0.0f}},
    {"019_f32_one", GT_F32, 4, {.f32 = 1.0f}},
    {"020_f32_smallest_normal", GT_F32, 4, {.f32 = 1.1754943508222875e-38f}},
    {"021_f32_max", GT_F32, 4, {.f32 = 3.4028234663852886e+38f}},
    {"022_f32_neg_max", GT_F32, 4, {.f32 = -3.4028234663852886e+38f}},
    {"023_bool_false", GT_BOOL, 1, {.b = 0}},
    {"024_bool_true", GT_BOOL, 1, {.b = 1}},
};
#define VECTOR_COUNT (sizeof VECTORS / sizeof VECTORS[0])

/* Width/type sanity: static guards against table typos (REQ-TOOL-044). */
_Static_assert(sizeof(float) == 4, "f32 must be IEEE single");
_Static_assert(sizeof(int32_t) == 4, "i32 must be 4 bytes");
_Static_assert(sizeof(int16_t) == 2, "i16 must be 2 bytes");

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

static int read_hex_file(const char *dir, const char *stem, uint8_t *bytes, uint8_t width) {
    char path[256];
    char hex[64];
    long len;
    size_t got;
    size_t i;
    FILE *f;

    if (snprintf(path, sizeof path, "%s/%s.hex", dir, stem) < 0) {
        return 0;
    }
    f = fopen(path, "rb");
    if (f == NULL) {
        fprintf(stderr, "golden_read: open %s failed\n", path);
        return 0;
    }
    fseek(f, 0, SEEK_END);
    len = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (len < 0 || (size_t)len != (size_t)(2 * width) || (size_t)len >= sizeof hex) {
        fprintf(stderr, "golden_read: %s: bad length %ld (want %u hex chars)\n", path, len,
                2u * width);
        fclose(f);
        return 0;
    }
    got = fread(hex, 1, (size_t)len, f);
    fclose(f);
    if (got != (size_t)len) {
        fprintf(stderr, "golden_read: %s: short read\n", path);
        return 0;
    }
    for (i = 0; i < (size_t)width; i++) {
        uint8_t hi;
        uint8_t lo;
        if (!nibble(hex[2 * i], &hi) || !nibble(hex[2 * i + 1], &lo)) {
            fprintf(stderr, "golden_read: %s: non-hex char\n", path);
            return 0;
        }
        bytes[i] = (uint8_t)((hi << 4) | lo);
    }
    return 1;
}

/* Assemble the canonical little-endian encoding from the frozen value. */
static void encode(const struct golden_vec *g, uint8_t out[4]) {
    switch (g->ty) {
    case GT_U8:
        out[0] = g->v.u8;
        return;
    case GT_I8:
        out[0] = (uint8_t)g->v.i8;
        return;
    case GT_BOOL:
        out[0] = g->v.b != 0 ? 1u : 0u;
        return;
    case GT_U16:
        out[0] = (uint8_t)(g->v.u16 & 0xFFu);
        out[1] = (uint8_t)(g->v.u16 >> 8);
        return;
    case GT_I16:
        out[0] = (uint8_t)((uint16_t)g->v.i16 & 0xFFu);
        out[1] = (uint8_t)((uint16_t)g->v.i16 >> 8);
        return;
    case GT_U32:
        out[0] = (uint8_t)(g->v.u32 & 0xFFu);
        out[1] = (uint8_t)((g->v.u32 >> 8) & 0xFFu);
        out[2] = (uint8_t)((g->v.u32 >> 16) & 0xFFu);
        out[3] = (uint8_t)(g->v.u32 >> 24);
        return;
    case GT_I32:
        out[0] = (uint8_t)((uint32_t)g->v.i32 & 0xFFu);
        out[1] = (uint8_t)(((uint32_t)g->v.i32 >> 8) & 0xFFu);
        out[2] = (uint8_t)(((uint32_t)g->v.i32 >> 16) & 0xFFu);
        out[3] = (uint8_t)((uint32_t)g->v.i32 >> 24);
        return;
    case GT_F32: {
        uint32_t bits;
        memcpy(&bits, &g->v.f32, sizeof bits); /* IEEE-754 bit pattern */
        out[0] = (uint8_t)(bits & 0xFFu);
        out[1] = (uint8_t)((bits >> 8) & 0xFFu);
        out[2] = (uint8_t)((bits >> 16) & 0xFFu);
        out[3] = (uint8_t)(bits >> 24);
        return;
    }
    }
}

/* Independent decode: bytes -> value -> compare with the frozen value. */
static int decode_matches(const struct golden_vec *g, const uint8_t *bytes) {
    switch (g->ty) {
    case GT_U8:
        return bytes[0] == g->v.u8;
    case GT_I8:
        return (int8_t)bytes[0] == g->v.i8;
    case GT_BOOL:
        return bytes[0] == (g->v.b != 0 ? 1u : 0u);
    case GT_U16:
        return (uint16_t)(bytes[0] | (bytes[1] << 8)) == g->v.u16;
    case GT_I16:
        return (int16_t)(bytes[0] | (bytes[1] << 8)) == g->v.i16;
    case GT_U32: {
        uint32_t got = (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) | ((uint32_t)bytes[2] << 16) |
                       ((uint32_t)bytes[3] << 24);
        return got == g->v.u32;
    }
    case GT_I32: {
        uint32_t raw = (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) | ((uint32_t)bytes[2] << 16) |
                       ((uint32_t)bytes[3] << 24);
        return (int32_t)raw == g->v.i32;
    }
    case GT_F32: {
        uint32_t bits = (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) |
                        ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
        float f;
        memcpy(&f, &bits, sizeof f);
        return f == g->v.f32;
    }
    }
    return 0;
}

int main(int argc, char **argv) {
    const char *dir = argc > 1 ? argv[1] : GOLDEN_DIR_DEFAULT;
    size_t pass = 0;
    size_t i;

    for (i = 0; i < VECTOR_COUNT; i++) {
        const struct golden_vec *g = &VECTORS[i];
        uint8_t file_bytes[4];
        uint8_t expected[4];
        const struct golden_vec *chk = g; /* avoid unused warning under pedantic */

        if (!read_hex_file(dir, g->stem, file_bytes, g->width)) {
            return 1;
        }
        memset(expected, 0, sizeof expected);
        encode(g, expected);
        if (memcmp(file_bytes, expected, g->width) != 0) {
            fprintf(stderr,
                    "golden_read: %s: bytes differ (file %02x%02x%02x%02x, want "
                    "%02x%02x%02x%02x)\n",
                    g->stem, file_bytes[0], file_bytes[1], file_bytes[2], file_bytes[3],
                    expected[0], expected[1], expected[2], expected[3]);
            return 1;
        }
        if (!decode_matches(chk, file_bytes)) {
            fprintf(stderr, "golden_read: %s: decoded value differs from frozen oracle\n", g->stem);
            return 1;
        }
        pass++;
    }
    printf("golden_read: %zu/%zu vectors match (dir %s)\n", pass, VECTOR_COUNT, dir);
    return 0;
}
