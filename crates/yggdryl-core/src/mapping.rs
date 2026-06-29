//! A small, dependency-free codec for component maps (`BTreeMap<String, String>`).
//!
//! The component map is the shared `from_mapping` / `to_mapping` shape every value
//! type renders to. These helpers give such a map a canonical byte form so any
//! mapping-backed type gets `to_bytes` / `from_bytes` for free, without pulling in
//! a serialization framework. The layout is little-endian, length-prefixed:
//!
//! ```text
//! [u32 entry-count] then, per entry: [u32 key-len][key bytes][u32 val-len][val bytes]
//! ```
//!
//! The 32-bit prefixes cap the codec at `u32::MAX` entries and `u32::MAX` (4 GiB)
//! bytes per key or value — far beyond any realistic component map.

use std::collections::BTreeMap;

/// Encodes a component map into its canonical, length-prefixed byte form.
pub fn encode_pairs(pairs: &BTreeMap<String, String>) -> Vec<u8> {
    debug_assert!(
        pairs.len() <= u32::MAX as usize,
        "mapping has too many entries to encode (max {})",
        u32::MAX
    );
    let mut out = Vec::new();
    out.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
    for (key, value) in pairs {
        debug_assert!(
            key.len() <= u32::MAX as usize && value.len() <= u32::MAX as usize,
            "mapping key/value exceeds 4 GiB and cannot be length-prefixed"
        );
        out.extend_from_slice(&(key.len() as u32).to_le_bytes());
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

/// Encodes bytes as a lowercase hex string (the byte-safe textual form a scalar
/// uses for its `to_mapping` value, since component maps hold only strings).
pub fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble"));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).expect("nibble"));
    }
    out
}

/// Decodes the lowercase hex string produced by [`encode_hex`].
pub fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("hex string has an odd number of digits".to_string());
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex digit {:?}", pair[0] as char))?;
        let lo = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex digit {:?}", pair[1] as char))?;
        out.push((hi << 4 | lo) as u8);
    }
    Ok(out)
}

/// Decodes the byte form produced by [`encode_pairs`] back into a component map.
pub fn decode_pairs(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let mut cursor = 0usize;

    fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, String> {
        let end = *cursor + 4;
        let slice = bytes
            .get(*cursor..end)
            .ok_or_else(|| "truncated length prefix".to_string())?;
        *cursor = end;
        Ok(u32::from_le_bytes(slice.try_into().expect("4-byte slice")))
    }

    fn take_str(bytes: &[u8], cursor: &mut usize, len: usize) -> Result<String, String> {
        let end = *cursor + len;
        let slice = bytes
            .get(*cursor..end)
            .ok_or_else(|| "truncated value".to_string())?;
        *cursor = end;
        std::str::from_utf8(slice)
            .map(str::to_owned)
            .map_err(|_| "mapping bytes were not valid UTF-8".to_string())
    }

    let count = take_u32(bytes, &mut cursor)? as usize;
    let mut pairs = BTreeMap::new();
    for _ in 0..count {
        let key_len = take_u32(bytes, &mut cursor)? as usize;
        let key = take_str(bytes, &mut cursor, key_len)?;
        let val_len = take_u32(bytes, &mut cursor)? as usize;
        let value = take_str(bytes, &mut cursor, val_len)?;
        pairs.insert(key, value);
    }
    if cursor != bytes.len() {
        return Err(format!(
            "{} trailing byte(s) after mapping",
            bytes.len() - cursor
        ));
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_mapping() {
        let mut map = BTreeMap::new();
        map.insert("name".to_string(), "id".to_string());
        map.insert("type".to_string(), "binary".to_string());
        let bytes = encode_pairs(&map);
        assert_eq!(decode_pairs(&bytes).unwrap(), map);
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut bytes = encode_pairs(&BTreeMap::new());
        bytes.push(0);
        assert!(decode_pairs(&bytes).is_err());
    }
}
