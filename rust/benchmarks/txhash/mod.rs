pub(crate) mod arrow;
pub(crate) mod values;

/// One instant every case couples: 2023-11-14T22:13:20Z in microseconds.
pub(crate) const INSTANT: i64 = 1_700_000_000_000_000;

/// A deterministic payload of `length` bytes.
pub(crate) fn payload(length: usize) -> Vec<u8> {
    let row = b"{\"id\": 1234567, \"venue\": \"XNAS\", \"price\": \"150.2500\"}\n";
    row.iter().copied().cycle().take(length).collect()
}
