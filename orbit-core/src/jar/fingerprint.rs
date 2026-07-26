//! CurseForge-compatible artifact fingerprints.
//!
//! CurseForge's REST schema exposes file fingerprints but does not specify the
//! local calculation. This implementation follows the public Murmur2
//! implementation used by Prism Launcher: bytes 9, 10, 13 and 32 are removed,
//! then MurmurHash2 is evaluated with seed 1.

use std::path::Path;

use crate::error::OrbitError;

const MIX: u32 = 0x5bd1_e995;

pub fn curseforge_fingerprint(bytes: &[u8]) -> u32 {
    let normalized: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|byte| !matches!(byte, 9 | 10 | 13 | 32))
        .collect();
    let mut hash = 1_u32 ^ normalized.len() as u32;
    let mut chunks = normalized.chunks_exact(4);

    for chunk in &mut chunks {
        let mut value = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        value = value.wrapping_mul(MIX);
        value ^= value >> 24;
        value = value.wrapping_mul(MIX);
        hash = hash.wrapping_mul(MIX) ^ value;
    }

    let tail = chunks.remainder();
    match tail.len() {
        3 => {
            hash ^= u32::from(tail[2]) << 16;
            hash ^= u32::from(tail[1]) << 8;
            hash ^= u32::from(tail[0]);
            hash = hash.wrapping_mul(MIX);
        }
        2 => {
            hash ^= u32::from(tail[1]) << 8;
            hash ^= u32::from(tail[0]);
            hash = hash.wrapping_mul(MIX);
        }
        1 => {
            hash ^= u32::from(tail[0]);
            hash = hash.wrapping_mul(MIX);
        }
        _ => {}
    }

    hash ^= hash >> 13;
    hash = hash.wrapping_mul(MIX);
    hash ^ (hash >> 15)
}

pub fn compute_curseforge_fingerprint(path: &Path) -> Result<u32, OrbitError> {
    let bytes = std::fs::read(path)?;
    Ok(curseforge_fingerprint(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_only_the_four_curseforge_whitespace_bytes() {
        assert_eq!(
            curseforge_fingerprint(b"a b\tc\nd\re"),
            curseforge_fingerprint(b"abcde")
        );
        assert_ne!(
            curseforge_fingerprint(b"a\x0bc"),
            curseforge_fingerprint(b"ac")
        );
    }

    #[test]
    fn matches_murmur2_seed_one_golden_vectors() {
        assert_eq!(curseforge_fingerprint(b""), 1_540_447_798);
        assert_eq!(curseforge_fingerprint(b"foo"), 197_930_586);
    }
}
