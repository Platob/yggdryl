//! The version invariant an integration test cannot reach.
//!
//! `rendered_len` is crate-private: it is the length the digest feed writes
//! before a version's canonical text, so it has to agree with what `Display`
//! actually renders for every reachable value. Everything a caller can observe
//! lives in `tests/types/version.rs`.

use crate::Version;

#[test]
fn the_rendered_length_agrees_with_what_display_writes() {
    for text in [
        "0",
        "1",
        "5",
        "5.0.2",
        "5.0.250",
        "1.0.256",
        "255.255.65535",
        "5.0SP2",
        "5.0sp250",
    ] {
        let value: Version = text.parse().unwrap();
        assert_eq!(value.rendered_len(), value.to_string().len(), "{text}");
    }
    for major in [0_u8, 1, 9, 10, 99, 100, 255] {
        for minor in [0_u8, 7, 42, 255] {
            for patch in [0_u16, 1, 250, 1_000, 65_535] {
                let value = Version::new(major, minor, patch);
                assert_eq!(value.rendered_len(), value.to_string().len(), "{value}");
            }
        }
    }
}
