//! MD5 (RFC 1321) and SHA-1 (FIPS 180-1) streaming hash implementations.
//!
//! Self-contained (no external crypto dependency) so `sys file -md5` / `-sha`
//! work offline; correctness is pinned by the RFC test vectors in the test
//! module below. Both digests support incremental `update()` in arbitrary
//! chunk sizes, so files are hashed streaming (bounded memory).

use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::path::Path;

/// Which digest `sys file -md5 / -sha` computes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HashAlgo {
    Md5,
    Sha1,
}

impl HashAlgo {
    /// The flag spelling, e.g. `-md5`.
    pub fn flag(self) -> &'static str {
        match self {
            Self::Md5 => "-md5",
            Self::Sha1 => "-sha",
        }
    }

    /// Human-readable algorithm name used in error messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
        }
    }

    /// Hex length of the digest (32 for md5, 40 for sha1).
    pub fn hex_len(self) -> usize {
        match self {
            Self::Md5 => 32,
            Self::Sha1 => 40,
        }
    }
}

/// Format bytes as lowercase hex.
fn hex_bytes(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ---------------------------------------------------------------------------
// MD5 (RFC 1321)
// ---------------------------------------------------------------------------

/// Per-round shift amounts (RFC 1321 section 3.4).
const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, //
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, //
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, //
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// Round constants K[i] = floor(abs(sin(i + 1)) * 2^32) (RFC 1321 section 3.4).
const MD5_K: [u32; 64] = [
    0xd76a_a478, 0xe8c7_b756, 0x2420_70db, 0xc1bd_ceee, 0xf57c_0faf, 0x4787_c62a, 0xa830_4613,
    0xfd46_9501, 0x6980_98d8, 0x8b44_f7af, 0xffff_5bb1, 0x895c_d7be, 0x6b90_1122, 0xfd98_7193,
    0xa679_438e, 0x49b4_0821, 0xf61e_2562, 0xc040_b340, 0x265e_5a51, 0xe9b6_c7aa, 0xd62f_105d,
    0x0244_1453, 0xd8a1_e681, 0xe7d3_fbc8, 0x21e1_cde6, 0xc337_07d6, 0xf4d5_0d87, 0x455a_14ed,
    0xa9e3_e905, 0xfcef_a3f8, 0x676f_02d9, 0x8d2a_4c8a, 0xfffa_3942, 0x8771_f681, 0x6d9d_6122,
    0xfde5_380c, 0xa4be_ea44, 0x4bde_cfa9, 0xf6bb_4b60, 0xbebf_bc70, 0x289b_7ec6, 0xeaa1_27fa,
    0xd4ef_3085, 0x0488_1d05, 0xd9d4_d039, 0xe6db_99e5, 0x1fa2_7cf8, 0xc4ac_5665, 0xf429_2244,
    0x432a_ff97, 0xab94_23a7, 0xfc93_a039, 0x655b_59c3, 0x8f0c_cc92, 0xffef_f47d, 0x8584_5dd1,
    0x6fa8_7e4f, 0xfe2c_e6e0, 0xa301_4314, 0x4e08_11a1, 0xf753_7e82, 0xbd3a_f235, 0x2ad7_d2bb,
    0xeb86_d391,
];

/// Streaming MD5 state.
pub struct Md5 {
    state: [u32; 4],
    block: [u8; 64],
    used: usize,
    total: u64,
}

impl Md5 {
    pub fn new() -> Self {
        Self {
            state: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476],
            block: [0; 64],
            used: 0,
            total: 0,
        }
    }

    /// Absorb `data` in arbitrary chunks; may be called repeatedly.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.used > 0 {
            let take = (64 - self.used).min(data.len());
            self.block[self.used..self.used + take].copy_from_slice(&data[..take]);
            self.used += take;
            data = &data[take..];
            if self.used == 64 {
                let b = self.block;
                md5_block(&mut self.state, &b);
                self.used = 0;
            }
        }
        while data.len() >= 64 {
            let mut b = [0u8; 64];
            b.copy_from_slice(&data[..64]);
            md5_block(&mut self.state, &b);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.block[..data.len()].copy_from_slice(data);
            self.used = data.len();
        }
    }

    /// Finish padding and return the 16-byte digest.
    pub fn finalize(mut self) -> [u8; 16] {
        let bit_len = self.total.wrapping_mul(8).to_le_bytes();
        self.block[self.used] = 0x80;
        self.used += 1;
        if self.used > 56 {
            while self.used < 64 {
                self.block[self.used] = 0;
                self.used += 1;
            }
            let b = self.block;
            md5_block(&mut self.state, &b);
            self.used = 0;
        }
        while self.used < 56 {
            self.block[self.used] = 0;
            self.used += 1;
        }
        self.block[56..64].copy_from_slice(&bit_len);
        let b = self.block;
        md5_block(&mut self.state, &b);
        let mut out = [0u8; 16];
        for (i, w) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

/// Compress one 64-byte block into the MD5 state.
fn md5_block(state: &mut [u32; 4], block: &[u8; 64]) {
    let mut m = [0u32; 16];
    for i in 0..16 {
        m[i] = u32::from_le_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let tmp = d;
        d = c;
        c = b;
        b = b.wrapping_add(
            a.wrapping_add(f)
                .wrapping_add(MD5_K[i])
                .wrapping_add(m[g])
                .rotate_left(MD5_S[i]),
        );
        a = tmp;
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

// ---------------------------------------------------------------------------
// SHA-1 (FIPS 180-1)
// ---------------------------------------------------------------------------

/// Streaming SHA-1 state.
pub struct Sha1 {
    state: [u32; 5],
    block: [u8; 64],
    used: usize,
    total: u64,
}

impl Sha1 {
    pub fn new() -> Self {
        Self {
            state: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476, 0xc3d2_e1f0],
            block: [0; 64],
            used: 0,
            total: 0,
        }
    }

    /// Absorb `data` in arbitrary chunks; may be called repeatedly.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.used > 0 {
            let take = (64 - self.used).min(data.len());
            self.block[self.used..self.used + take].copy_from_slice(&data[..take]);
            self.used += take;
            data = &data[take..];
            if self.used == 64 {
                let b = self.block;
                sha1_block(&mut self.state, &b);
                self.used = 0;
            }
        }
        while data.len() >= 64 {
            let mut b = [0u8; 64];
            b.copy_from_slice(&data[..64]);
            sha1_block(&mut self.state, &b);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.block[..data.len()].copy_from_slice(data);
            self.used = data.len();
        }
    }

    /// Finish padding and return the 20-byte digest.
    pub fn finalize(mut self) -> [u8; 20] {
        let bit_len = self.total.wrapping_mul(8).to_be_bytes();
        self.block[self.used] = 0x80;
        self.used += 1;
        if self.used > 56 {
            while self.used < 64 {
                self.block[self.used] = 0;
                self.used += 1;
            }
            let b = self.block;
            sha1_block(&mut self.state, &b);
            self.used = 0;
        }
        while self.used < 56 {
            self.block[self.used] = 0;
            self.used += 1;
        }
        self.block[56..64].copy_from_slice(&bit_len);
        let b = self.block;
        sha1_block(&mut self.state, &b);
        let mut out = [0u8; 20];
        for (i, w) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }
}

/// Compress one 64-byte block into the SHA-1 state.
fn sha1_block(state: &mut [u32; 5], block: &[u8; 64]) {
    let mut w = [0u32; 80];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    let (mut a, mut b, mut c, mut d, mut e) = (state[0], state[1], state[2], state[3], state[4]);
    for i in 0..80 {
        let (f, k) = match i {
            0..=19 => ((b & c) | (!b & d), 0x5a82_7999u32),
            20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
            _ => (b ^ c ^ d, 0xca62_c1d6),
        };
        let temp = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(k)
            .wrapping_add(w[i]);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = temp;
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
}

// ---------------------------------------------------------------------------
// File hashing
// ---------------------------------------------------------------------------

/// Stream-hash `path` and return the lowercase hex digest (bounded memory:
/// 64 KiB chunks regardless of the file size).
pub fn hash_file(algo: HashAlgo, path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut buf = [0u8; 65536];
    match algo {
        HashAlgo::Md5 => {
            let mut h = Md5::new();
            loop {
                let n = file
                    .read(&mut buf)
                    .with_context(|| format!("cannot read {}", path.display()))?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(hex_bytes(&h.finalize()))
        }
        HashAlgo::Sha1 => {
            let mut h = Sha1::new();
            loop {
                let n = file
                    .read(&mut buf)
                    .with_context(|| format!("cannot read {}", path.display()))?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(hex_bytes(&h.finalize()))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn md5_hex(data: &[u8]) -> String {
        let mut h = Md5::new();
        h.update(data);
        hex_bytes(&h.finalize())
    }

    fn sha1_hex(data: &[u8]) -> String {
        let mut h = Sha1::new();
        h.update(data);
        hex_bytes(&h.finalize())
    }

    /// Absorb `data` in 1..=17-byte pieces to prove chunk-boundary safety.
    fn md5_hex_chunked(data: &[u8]) -> String {
        let mut h = Md5::new();
        for chunk in data.chunks(17) {
            h.update(chunk);
        }
        hex_bytes(&h.finalize())
    }

    fn sha1_hex_chunked(data: &[u8]) -> String {
        let mut h = Sha1::new();
        for chunk in data.chunks(13) {
            h.update(chunk);
        }
        hex_bytes(&h.finalize())
    }

    #[test]
    fn md5_rfc1321_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"a"), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(md5_hex(b"message digest"), "f96b697d7cb7938d525a2f31aaf161d0");
        assert_eq!(
            md5_hex(b"abcdefghijklmnopqrstuvwxyz"),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
        assert_eq!(
            md5_hex(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
        assert_eq!(
            md5_hex(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }

    #[test]
    fn md5_million_a() {
        // RFC 1321 Appendix A.5: 1,000,000 x 'a' — crosses many block boundaries.
        let data = vec![b'a'; 1_000_000];
        assert_eq!(md5_hex(&data), "7707d6ae4e027c70eea2a935c2296f21");
        // Chunked streaming must agree with one-shot.
        assert_eq!(md5_hex_chunked(&data), "7707d6ae4e027c70eea2a935c2296f21");
    }

    #[test]
    fn md5_chunked_matches_oneshot() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let once = md5_hex(&data);
        for size in [1usize, 3, 7, 63, 64, 65, 100, 1000] {
            let mut h = Md5::new();
            for chunk in data.chunks(size) {
                h.update(chunk);
            }
            assert_eq!(hex_bytes(&h.finalize()), once, "chunk size {size}");
        }
    }

    #[test]
    fn sha1_fips_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"The quick brown fox jumps over the lazy dog"),
            "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
        );
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn sha1_million_a() {
        // FIPS 180-1 / NIST test: 1,000,000 x 'a'.
        let data = vec![b'a'; 1_000_000];
        assert_eq!(sha1_hex(&data), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
        assert_eq!(sha1_hex_chunked(&data), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
    }

    #[test]
    fn sha1_chunked_matches_oneshot() {
        let data: Vec<u8> = (0..2000u32).map(|i| (i % 223) as u8).collect();
        let once = sha1_hex(&data);
        for size in [1usize, 3, 7, 63, 64, 65, 128, 2000] {
            let mut h = Sha1::new();
            for chunk in data.chunks(size) {
                h.update(chunk);
            }
            assert_eq!(hex_bytes(&h.finalize()), once, "chunk size {size}");
        }
    }

    #[test]
    fn hash_file_reads_streaming() {
        let base = std::env::temp_dir().join(format!("sys_hash_{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let f = base.join("t.bin");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
        fs::write(&f, &data).unwrap();
        assert_eq!(
            hash_file(HashAlgo::Md5, &f).unwrap(),
            md5_hex(&data),
            "md5 of on-disk file"
        );
        assert_eq!(
            hash_file(HashAlgo::Sha1, &f).unwrap(),
            sha1_hex(&data),
            "sha1 of on-disk file"
        );
        assert!(hash_file(HashAlgo::Md5, &base.join("nope.bin")).is_err());
        fs::remove_dir_all(&base).unwrap();
    }
}
