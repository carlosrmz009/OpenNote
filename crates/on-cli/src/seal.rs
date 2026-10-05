#[allow(dead_code)]
pub fn seal(bytes: &[u8], key: u64) -> Vec<u8> {
    let mut state = key;
    let mut out = Vec::with_capacity(bytes.len());
    for chunk in bytes.chunks(8) {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        out.extend(chunk.iter().zip(z.to_le_bytes()).map(|(b, k)| b ^ k));
    }
    out
}
