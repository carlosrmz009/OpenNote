pub fn fnv(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

pub fn share(name: &str) -> f64 {
    (fnv(name) % 10_000) as f64 / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

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
