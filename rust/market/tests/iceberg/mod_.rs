//! `rust/src/iceberg/mod.rs` over the kinds `yggdryl-market` claims: an
//! enum column's Iceberg bound is the int its column stores. The core's own
//! enum leaf, `State`, is `rust/tests/iceberg/mod_.rs`'s.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::DataType;
    use yggdryl::iceberg::PrimitiveType;

    #[test]
    fn an_enum_bound_is_the_int_its_column_stores() {
        use yggdryl::Scalar;
        use yggdryl::internals::iceberg_value::{is_portable, single_to_value, single_value};
        use yggdryl_market::{MarketDataKind, Side, TimeInForce};

        crate::install::installed();
        // An enum column stores its member's code, so its bounds are Iceberg
        // ints a planner compares in code order and never the name a text
        // codec writes.
        let members: [(DataType, Scalar, u16); 5] = [
            (
                MarketDataKind::dtype(),
                Scalar::from(MarketDataKind::Order),
                10,
            ),
            (
                MarketDataKind::dtype(),
                Scalar::from(MarketDataKind::Unknown),
                0,
            ),
            (Side::dtype(), Scalar::from(Side::Buy), 1),
            (Side::dtype(), Scalar::from(Side::SellUnd), 17),
            (
                TimeInForce::dtype(),
                Scalar::from(TimeInForce::GoodTillCancel),
                2,
            ),
        ];
        for (dtype, exact, code) in members {
            assert!(is_portable(&dtype), "{dtype}");
            assert_eq!(
                PrimitiveType::from_dtype(&dtype).unwrap(),
                PrimitiveType::Int
            );
            let bytes = single_value(&exact, &dtype).expect("an enum member encodes a bound");
            // Whatever width the column stores, an Iceberg int is four bytes.
            assert_eq!(bytes, i32::from(code).to_le_bytes(), "{exact:?}");
            assert_eq!(single_to_value(&bytes, &dtype), Some(exact), "{dtype}");
        }
        // The code of no member reads as no bound rather than as a member.
        assert_eq!(
            single_to_value(&26_i32.to_le_bytes(), &MarketDataKind::dtype()),
            None
        );
        assert_eq!(single_to_value(&18_i32.to_le_bytes(), &Side::dtype()), None);
    }
}
