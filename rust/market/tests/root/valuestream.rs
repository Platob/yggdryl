//! `rust/src/valuestream.rs` over the enum leaves `yggdryl-market` claims:
//! a kind's value streams under the byte the market claims it at and reads
//! back as itself, leaf for leaf. Every core leaf's round trip is
//! `rust/tests/root/valuestream.rs`'s.

use yggdryl::{DataType, Scalar, VALUE_STREAM_VERSION};

/// One value of each market kind the value stream carries.
fn corpus() -> Vec<Scalar> {
    let text = |dtype: &str, text: &str| {
        DataType::from_str(dtype)
            .unwrap()
            .required_field("value")
            .scalar(Scalar::from(text))
            .unwrap()
    };
    vec![text("side", "BUY"), text("timeinforce", "GTC")]
}

#[test]
fn every_market_kind_reads_back_as_itself() {
    crate::install::installed();
    for value in corpus() {
        let bytes = value.into_value_bytes();
        assert_eq!(bytes[0], VALUE_STREAM_VERSION, "{value:?}");
        assert_eq!(bytes[1], value.id().as_u8(), "{value:?}");
        let read = Scalar::decode_value_bytes(&bytes)
            .unwrap_or_else(|error| panic!("{value:?} encoded as {bytes:?} refused: {error}"));
        assert_eq!(read, value);
        assert_eq!(
            read.id(),
            value.id(),
            "the leaf travels, not just the value"
        );
        assert_eq!(read.dtype().ok(), value.dtype().ok(), "{value:?}");
        // The stream is the same bytes, cut one leaf per chunk.
        let chunks: Vec<Vec<u8>> = value.encode_value_stream_bytes().collect();
        assert_eq!(chunks.concat(), bytes);
        assert_eq!(Scalar::decode_value_stream_bytes(&chunks).unwrap(), value);
    }
}
