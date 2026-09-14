//! The decimal reader an integration test cannot reach.
//!
//! `from_decimal_text` is the crate-private door every text reader goes
//! through; what a caller can observe lives in `tests/types/decimal.rs`.

mod reading {
    use crate::{DataType, Scalar};

    fn money() -> DataType {
        "decimal128(10, 2)".parse().unwrap()
    }

    #[test]
    fn text_is_restated_exactly_at_the_declared_scale() {
        assert_eq!(
            Scalar::from_decimal_text(&money(), "10.50").unwrap(),
            Scalar::d128(1_050, 2)
        );
        assert_eq!(
            Scalar::from_decimal_text(&money(), "1.05e1").unwrap(),
            Scalar::d128(1_050, 2)
        );
        // A digit the scale cannot hold is refused rather than rounded away,
        // and that refusal is a reading rather than a parse failure, so no
        // other reader is allowed to round it either.
        let refused = Scalar::from_decimal_text(&money(), "1.005").unwrap_err();
        assert!(
            matches!(refused, crate::Error::InvalidRecord { .. }),
            "{refused:?}"
        );
        // Text that is not a decimal at all did not read, so it may be tried
        // by a wider reader.
        let unread = Scalar::from_decimal_text(&money(), "ten").unwrap_err();
        assert!(matches!(unread, crate::Error::Parse { .. }), "{unread:?}");
    }
}
