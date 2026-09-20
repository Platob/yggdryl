pub(crate) mod txhash;
pub(crate) mod variant;
pub(crate) mod xxhash;

/// A deterministic payload built outside each measured loop.
pub(crate) fn payload(length: usize) -> Vec<u8> {
    let row = b"{\"id\": 1234567, \"venue\": \"XNAS\", \"price\": \"150.2500\"}\n";
    row.iter().copied().cycle().take(length).collect()
}
