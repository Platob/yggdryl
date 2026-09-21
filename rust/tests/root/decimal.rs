//! `rust/src/decimal.rs`: the decimal text reader every door reads through.

mod reading {
    use yggdryl::internals::decimal::from_decimal_text;
    use yggdryl::{DataType, Scalar};

    fn money() -> DataType {
        "decimal128(10, 2)".parse().unwrap()
    }

    /// The empty text is no spelling: the empty-cell rule sits above this
    /// reader, on the doors, and the reader itself keeps refusing it.
    #[test]
    fn the_empty_text_is_no_decimal_spelling() {
        assert!(from_decimal_text(&money(), "").is_err());
    }

    #[test]
    fn text_is_restated_exactly_at_the_declared_scale() {
        assert_eq!(
            from_decimal_text(&money(), "10.50").unwrap(),
            Scalar::d128(1_050, 2)
        );
        assert_eq!(
            from_decimal_text(&money(), "1.05e1").unwrap(),
            Scalar::d128(1_050, 2)
        );
        // A digit the scale cannot hold is refused rather than rounded away,
        // and that refusal is a reading rather than a parse failure, so no
        // other reader is allowed to round it either.
        let refused = from_decimal_text(&money(), "1.005").unwrap_err();
        assert!(
            matches!(refused, yggdryl::Error::InvalidRecord { .. }),
            "{refused:?}"
        );
        // Text that is not a decimal at all did not read, so it may be tried
        // by a wider reader.
        let unread = from_decimal_text(&money(), "ten").unwrap_err();
        assert!(matches!(unread, yggdryl::Error::Parse { .. }), "{unread:?}");
    }
}
