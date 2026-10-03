//! Clean-room BLAKE3 (unkeyed hash mode, 32-byte output) from the public
//! specification. Zero dependencies, zero heap allocation: the chaining-value
//! stack is a fixed array. Used for schema/layout/contract fingerprints
//! (docs/algorithms/toolchain/workspace-model.md §`contract_fingerprint`).
//! REQ: REQ-TOOL-023 (schema hash), REQ-CORE-085 (fingerprint = hash of
//! ABI/schema/layout inputs), REQ-CORE-068 (schema/layout hash).

const OUT_LEN: usize = 32;
const BLOCK_LEN: usize = 64;
const CHUNK_LEN: usize = 1024;

const CHUNK_START: u32 = 1;
const CHUNK_END: u32 = 2;
const PARENT: u32 = 4;
const ROOT: u32 = 8;

const IV: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

const MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

/// Max tree depth: 2^54 chunks cover the 2^64-byte input bound.
const MAX_DEPTH: usize = 54;

#[inline]
fn g(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

#[inline]
fn round(s: &mut [u32; 16], m: &[u32; 16]) {
    g(s, 0, 4, 8, 12, m[0], m[1]);
    g(s, 1, 5, 9, 13, m[2], m[3]);
    g(s, 2, 6, 10, 14, m[4], m[5]);
    g(s, 3, 7, 11, 15, m[6], m[7]);
    g(s, 0, 5, 10, 15, m[8], m[9]);
    g(s, 1, 6, 11, 12, m[10], m[11]);
    g(s, 2, 7, 8, 13, m[12], m[13]);
    g(s, 3, 4, 9, 14, m[14], m[15]);
}

fn compress(
    cv: &[u32; 8],
    block: &[u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
) -> [u32; 16] {
    // Counter split into low/high 32-bit words (truncation intended).
    #[allow(clippy::cast_possible_truncation)]
    let (lo, hi) = (counter as u32, (counter >> 32) as u32);
    let mut s = [
        cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7], IV[0], IV[1], IV[2], IV[3], lo, hi,
        block_len, flags,
    ];
    let mut m = *block;
    for r in 0..7 {
        round(&mut s, &m);
        if r < 6 {
            let mut p = [0_u32; 16];
            for (i, &src) in MSG_PERMUTATION.iter().enumerate() {
                p[i] = m[src];
            }
            m = p;
        }
    }
    for i in 0..8 {
        s[i] ^= s[i + 8];
        s[i + 8] ^= cv[i];
    }
    s
}

fn words_from_block(b: &[u8; BLOCK_LEN]) -> [u32; 16] {
    let mut w = [0_u32; 16];
    for (i, c) in b.chunks_exact(4).enumerate() {
        w[i] = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    }
    w
}

fn first8(w: &[u32; 16]) -> [u32; 8] {
    [w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7]]
}

/// Pending node output: root or non-root finalization is deferred.
#[derive(Clone, Copy)]
struct Output {
    cv: [u32; 8],
    block: [u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
}

impl Output {
    fn chaining_value(&self) -> [u32; 8] {
        first8(&compress(
            &self.cv,
            &self.block,
            self.counter,
            self.block_len,
            self.flags,
        ))
    }

    fn root_bytes(&self) -> [u8; OUT_LEN] {
        let w = compress(&self.cv, &self.block, 0, self.block_len, self.flags | ROOT);
        let mut out = [0_u8; OUT_LEN];
        for (i, word) in w.iter().take(8).enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }
}

#[derive(Clone, Copy)]
struct ChunkState {
    cv: [u32; 8],
    counter: u64,
    block: [u8; BLOCK_LEN],
    block_len: usize,
    blocks_compressed: u8,
}

impl ChunkState {
    const fn new(counter: u64) -> Self {
        Self {
            cv: IV,
            counter,
            block: [0; BLOCK_LEN],
            block_len: 0,
            blocks_compressed: 0,
        }
    }

    fn len(&self) -> usize {
        usize::from(self.blocks_compressed) * BLOCK_LEN + self.block_len
    }

    const fn start_flag(&self) -> u32 {
        if self.blocks_compressed == 0 {
            CHUNK_START
        } else {
            0
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.block_len == BLOCK_LEN {
                let w = words_from_block(&self.block);
                self.cv = first8(&compress(&self.cv, &w, self.counter, 64, self.start_flag()));
                self.blocks_compressed += 1;
                self.block = [0; BLOCK_LEN];
                self.block_len = 0;
            }
            let take = (BLOCK_LEN - self.block_len).min(input.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&input[..take]);
            self.block_len += take;
            input = &input[take..];
        }
    }

    fn output(&self) -> Output {
        Output {
            cv: self.cv,
            block: words_from_block(&self.block),
            counter: self.counter,
            // block_len <= 64 by construction.
            #[allow(clippy::cast_possible_truncation)]
            block_len: self.block_len as u32,
            flags: self.start_flag() | CHUNK_END,
        }
    }
}

fn parent_output(left: &[u32; 8], right: &[u32; 8]) -> Output {
    let mut block = [0_u32; 16];
    block[..8].copy_from_slice(left);
    block[8..].copy_from_slice(right);
    Output {
        cv: IV,
        block,
        counter: 0,
        block_len: 64,
        flags: PARENT,
    }
}

/// Incremental core of [`blake3`]; private, fixed storage, no heap.
struct Hasher {
    chunk: ChunkState,
    stack: [[u32; 8]; MAX_DEPTH],
    depth: usize,
}

impl Hasher {
    const fn new() -> Self {
        Self {
            chunk: ChunkState::new(0),
            stack: [[0; 8]; MAX_DEPTH],
            depth: 0,
        }
    }

    fn add_chunk_cv(&mut self, mut cv: [u32; 8], mut total_chunks: u64) {
        while total_chunks & 1 == 0 {
            self.depth -= 1;
            cv = parent_output(&self.stack[self.depth], &cv).chaining_value();
            total_chunks >>= 1;
        }
        self.stack[self.depth] = cv;
        self.depth += 1;
    }

    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.chunk.len() == CHUNK_LEN {
                let cv = self.chunk.output().chaining_value();
                let total = self.chunk.counter + 1;
                self.add_chunk_cv(cv, total);
                self.chunk = ChunkState::new(total);
            }
            let take = (CHUNK_LEN - self.chunk.len()).min(input.len());
            self.chunk.update(&input[..take]);
            input = &input[take..];
        }
    }

    /// Digest of everything absorbed so far.
    fn finalize(&self) -> [u8; OUT_LEN] {
        let mut out = self.chunk.output();
        let mut rem = self.depth;
        while rem > 0 {
            rem -= 1;
            out = parent_output(&self.stack[rem], &out.chaining_value());
        }
        out.root_bytes()
    }
}

/// BLAKE3 (hash mode) — 32-byte output. REQ: REQ-TOOL-023.
#[must_use]
pub fn blake3(data: &[u8]) -> [u8; OUT_LEN] {
    let mut h = Hasher::new();
    h.update(data);
    h.finalize()
}

/// BLAKE3 (hash mode) truncated to 128 bits. REQ: REQ-TOOL-023.
#[must_use]
pub fn blake3_128(data: &[u8]) -> [u8; 16] {
    let full = blake3(data);
    let mut out = [0_u8; 16];
    out.copy_from_slice(&full[..16]);
    out
}

#[cfg(test)]
mod tests {
    use super::{Hasher, blake3};

    fn hex(b: &[u8; 32]) -> String {
        use std::fmt::Write;
        b.iter().fold(String::new(), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
    }

    fn pattern(n: usize) -> Vec<u8> {
        // i % 251 is < 256 by construction.
        #[allow(clippy::cast_possible_truncation)]
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    /// REQ: REQ-CORE-085 — official empty-input vector.
    #[test]
    fn empty() {
        assert_eq!(
            hex(&blake3(b"")),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    /// REQ: REQ-CORE-085 — "abc" vector.
    #[test]
    fn abc() {
        assert_eq!(
            hex(&blake3(b"abc")),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
    }

    /// REQ: REQ-CORE-085 — official i%251 vectors: single/multi chunk boundaries.
    #[test]
    fn official_pattern_vectors() {
        let cases = [
            (
                1,
                "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
            ),
            (
                1023,
                "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11",
            ),
            (
                1024,
                "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
            ),
            (
                1025,
                "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
            ),
            (
                2048,
                "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a",
            ),
            (
                31744,
                "62b6960e1a44bcc1eb1a611a8d6235b6b4b78f32e7abc4fb4c6cdcce94895c47",
            ),
        ];
        for (n, want) in cases {
            assert_eq!(hex(&blake3(&pattern(n))), want, "len {n}");
        }
    }

    /// REQ: REQ-CORE-085 — incremental == one-shot for every split, >1 MiB tree.
    #[test]
    fn incremental_matches_oneshot() {
        let data = pattern((1 << 20) + 12_345);
        let want = blake3(&data);
        for step in [1_usize, 7, 63, 64, 65, 1000, 1024, 1025, 4096, 99_999] {
            let mut h = Hasher::new();
            for c in data.chunks(step) {
                h.update(c);
            }
            assert_eq!(h.finalize(), want, "step {step}");
        }
        for n in [
            0_usize, 1, 63, 64, 65, 1023, 1024, 1025, 2047, 2048, 2049, 3072, 5000,
        ] {
            let d = pattern(n);
            let mut h = Hasher::new();
            let (a, b) = d.split_at(n / 3);
            h.update(a);
            h.update(b);
            assert_eq!(h.finalize(), blake3(&d), "len {n}");
        }
    }
}
