//! `rust/src/code.rs`'s listings over the kinds `yggdryl-market` claims:
//! the side and time-in-force listings a logical name prebuilds once the
//! market registers them. The core's own listings are
//! `rust/tests/root/code.rs`'s.

mod datatypes {
    use yggdryl::{DataType, StringEnum};

    #[test]
    fn a_listing_is_a_vocabulary_and_never_a_gate_on_the_value() {
        crate::install::installed();
        // The listing is what a name resolves from, and two readers answer the
        // same members because it is a constant.
        for (name, count) in [
            ("side", yggdryl_market::SIDES.len()),
            ("timeinforce", yggdryl_market::TIMESINFORCE.len()),
        ] {
            let built = StringEnum::from_logical_name(name).unwrap();
            assert_eq!(built.len(), count, "{name}");
            assert_eq!(
                built,
                StringEnum::from_logical_name(name).unwrap(),
                "{name}"
            );
        }
        // Every prebuilt member fits the fixed ASCII width its string listing
        // declares: a side or a time in force column itself stores enum codes
        // and packs nothing, and the listing is the wire values a FIX field
        // of that name holds.
        for (name, dtype) in [
            ("side", DataType::fixed_ascii(8).unwrap()),
            ("timeinforce", DataType::fixed_ascii(8).unwrap()),
        ] {
            StringEnum::from_logical_name(name)
                .unwrap()
                .into_members(&dtype)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
        }
    }

    /// The listings are claimed once, by the market crate: a second
    /// install claims nothing, and another crate's claim of a market
    /// listing is refused naming the first claimant.
    #[test]
    fn a_market_listing_is_claimed_once_by_the_market_crate() {
        crate::install::installed();
        let listed = StringEnum::prebuilt();
        let kinds = yggdryl::market::kinds().len();
        yggdryl_market::install().unwrap();
        assert_eq!(StringEnum::prebuilt(), listed);
        assert_eq!(yggdryl::market::kinds().len(), kinds);
        assert_eq!(StringEnum::prebuilt_values("side"), yggdryl_market::SIDES);
        let refused =
            StringEnum::register_prebuilt("side", yggdryl_market::SIDES, "another").unwrap_err();
        assert!(
            matches!(refused, yggdryl::Error::Conflict { .. }),
            "{refused}"
        );
        assert!(refused.to_string().contains("yggdryl-market"), "{refused}");
    }
}
