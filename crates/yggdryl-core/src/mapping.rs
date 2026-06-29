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

use std::collections::BTreeMap;

/// Encodes a component map into its canonical, length-prefixed byte form.
pub(crate) fn encode_pairs(pairs: &BTreeMap<String, String>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
    for (key, value) in pairs {
        out.extend_from_slice(&(key.len() as u32).to_le_bytes());
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

/// Decodes the byte form produced by [`encode_pairs`] back into a component map.
pub(crate) fn decode_pairs(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
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
