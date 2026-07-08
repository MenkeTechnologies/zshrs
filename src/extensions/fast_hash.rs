//! Fast, dependency-free hasher for the shell's internal name→node tables.
//!
//! # Why
//!
//! zsh's hash tables (parameters, functions, aliases, …) are keyed by short
//! identifier strings and looked up constantly — every variable read/write,
//! every command dispatch. zshrs backs them with `std::collections::HashMap`,
//! whose default hasher is SipHash-1-3: cryptographically DoS-resistant, but
//! ~2–3× slower than necessary for short internal keys that never come from
//! an adversary choosing collisions. Profiling a string/param workload showed
//! HashMap hashing/probing as the single largest cost (dwarfing everything
//! else). C's `hashtable.c` uses a trivial multiply-hash; this restores that
//! choice.
//!
//! [`FxBuildHasher`] is the FxHash algorithm (as used by rustc and Firefox):
//! a rotate + XOR + multiply per machine word. Not DoS-resistant, which is
//! irrelevant for process-internal symbol tables. Use via [`FastMap`], a drop-in
//! `HashMap` type alias.

use std::hash::{BuildHasherDefault, Hasher};

/// FxHash constant (golden-ratio-derived odd multiplier), matching rustc_hash.
const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// FxHash hasher — rotate/xor/multiply per word. Fast for short keys.
#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, i: u64) {
        self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(K);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        // Consume 8 bytes at a time, then the tail. Byte order doesn't matter
        // for an internal table as long as it's consistent within a run.
        while bytes.len() >= 8 {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&bytes[..8]);
            self.add(u64::from_ne_bytes(buf));
            bytes = &bytes[8..];
        }
        if !bytes.is_empty() {
            let mut buf = [0u8; 8];
            buf[..bytes.len()].copy_from_slice(bytes);
            self.add(u64::from_ne_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // Final avalanche so the low bits (used for bucket selection) mix.
        self.hash.wrapping_mul(K).rotate_left(26)
    }
}

/// `BuildHasher` producing [`FxHasher`]s.
pub type FxBuildHasher = BuildHasherDefault<FxHasher>;

/// Drop-in `HashMap` alias using the fast hasher. Same API as
/// `std::collections::HashMap`; construct with `FastMap::default()`.
pub type FastMap<K, V> = std::collections::HashMap<K, V, FxBuildHasher>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::BuildHasher;

    #[test]
    fn distinct_keys_hash_distinctly_enough() {
        let bh = FxBuildHasher::default();
        let names = ["PATH", "IFS", "HOME", "x", "_p9k__x", "PWD", "a", "bb"];
        let hashes: Vec<u64> = names
            .iter()
            .map(|n| {
                let mut h = bh.build_hasher();
                h.write(n.as_bytes());
                h.finish()
            })
            .collect();
        // No accidental all-equal collisions across a typical name set.
        for i in 0..hashes.len() {
            for j in i + 1..hashes.len() {
                assert_ne!(hashes[i], hashes[j], "{} vs {}", names[i], names[j]);
            }
        }
    }

    #[test]
    fn fastmap_roundtrips() {
        let mut m: FastMap<String, i32> = FastMap::default();
        m.insert("PATH".into(), 1);
        m.insert("HOME".into(), 2);
        assert_eq!(m.get("PATH"), Some(&1));
        assert_eq!(m.get("HOME"), Some(&2));
        assert_eq!(m.get("nope"), None);
        assert_eq!(m.len(), 2);
    }
}
