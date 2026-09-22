//! One hash, so that a piece stays on the side of a split it started on.
//!
//! Every division of data in this crate — the corpus into train and test, the tuner's
//! examples into three — is a hash of a name. The hash therefore decides which music a
//! model is scored against, and the only honest way to report a before and an after is
//! for that decision to be the same both times.
//!
//! The standard library's [`DefaultHasher`] is not that. It is explicitly allowed to
//! change between compiler versions, and this crate has runs measured in weeks that are
//! stopped and resumed across upgrades. A piece that moved from test to train partway
//! through one would quietly contaminate the only number worth reading, and nothing
//! anywhere would say so.
//!
//! So: FNV-1a, written out here, with its published test vector pinned by a test. It is
//! not a good hash for a hash table and does not need to be. It needs to be the same
//! hash in five years.
//!
//! [`DefaultHasher`]: std::collections::hash_map::DefaultHasher

/// FNV-1a, 64-bit.
pub fn fnv(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Where a name falls in a division into `parts`, from 0 to `parts - 1`.
///
/// Ten thousand buckets rather than `parts` directly, so that asking for a third and
/// asking for a fifth put a given name somewhere unrelated rather than somewhere
/// correlated.
pub fn share(name: &str) -> f64 {
    (fnv(name) % 10_000) as f64 / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FNV-1a's published test vectors. If these move, every search that was resumed
    /// has just had its test set changed underneath it.
    #[test]
    fn the_hash_is_the_one_it_has_always_been() {
        assert_eq!(fnv(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv("foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn a_share_is_spread_across_the_range() {
        let shares: Vec<f64> = (0..2000).map(|i| share(&format!("piece-{i}"))).collect();
        assert!(shares.iter().all(|s| (0.0..1.0).contains(s)));
        let below = shares.iter().filter(|s| **s < 0.2).count();
        assert!((300..500).contains(&below), "{below} of 2000 fell in the first fifth");
    }
}
