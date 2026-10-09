//! `rust/src/scalar.rs` over the enum leaves `yggdryl-market` claims: a
//! kind's value ranks where the market claims it, which orders it among
//! every other kind of value. The sweep over every core variant is
//! `rust/tests/root/scalar.rs`'s.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Scalar;
    use yggdryl::internals::scalar::value_rank;
    use yggdryl_market::Side;

    #[test]
    fn a_market_kind_ranks_where_it_is_claimed() {
        crate::install::installed();
        // The rank is wire-visible: it orders dictionary values.
        let side = Scalar::from(Side::new("1").unwrap());
        assert_eq!(value_rank(&side), 29, "{side:?}");
        // A side sorts after every value of a lower rank and before every
        // value of a higher one.
        let mut sorted = [
            side.clone(),
            Scalar::from_sequence([]),
            Scalar::from("a"),
            Scalar::Null,
        ];
        sorted.sort();
        assert_eq!(
            sorted.iter().map(value_rank).collect::<Vec<_>>(),
            [0, 5, 11, 29]
        );
    }
}
