//! Exact decimals: what they hold, how they compare, and how they restate.

use crate::Scalar;

mod representation {
    use super::Scalar;

    #[test]
    fn a_decimal_keeps_the_coefficient_and_the_scale_it_was_given() {
        let price = Scalar::d128(1_050, 2);

        assert_eq!(price.as_d128(), Some((1_050, 2)));
        assert!(price.is_decimal());
        assert_eq!(price.kind(), "d128");
        assert!(!Scalar::from(10.5).is_decimal());
    }

    #[test]
    fn a_decimal_holds_a_fraction_no_double_can_hold() {
        // The point of the variant: one tenth has no finite binary expansion,
        // so a float can only ever hold the nearest double to it.
        let exact = Scalar::d128(1, 1);
        assert_eq!(exact.as_d128(), Some((1, 1)));
        assert_ne!(f64::from(0.1_f32), 0.1_f64);

        // The full 128-bit coefficient survives, which is what a float cannot do.
        assert_eq!(Scalar::d128(i128::MAX, 0).as_d128(), Some((i128::MAX, 0)));
    }

    #[test]
    fn one_renderer_restores_the_exact_plain_text() {
        assert_eq!(
            Scalar::d128(1_050, 2).as_decimal_utf8().as_deref(),
            Some("10.50")
        );
        assert_eq!(
            Scalar::d128(-5, 3).as_decimal_utf8().as_deref(),
            Some("-0.005")
        );
        assert_eq!(
            Scalar::d128(12, -2).as_decimal_utf8().as_deref(),
            Some("1200")
        );
        assert_eq!(
            Scalar::d256(crate::I256::from_i128(1_050), 2)
                .as_decimal_utf8()
                .as_deref(),
            Some("10.50")
        );
    }
}

mod comparison {
    use super::Scalar;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash(value: &Scalar) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn two_spellings_of_one_number_are_one_value() {
        for (left, right) in [
            (Scalar::d128(1_050, 2), Scalar::d128(105, 1)),
            (Scalar::d128(0, 7), Scalar::d128(0, -7)),
            (Scalar::d128(-1_050, 2), Scalar::d128(-105, 1)),
            (Scalar::d128(100, 0), Scalar::d128(1, -2)),
        ] {
            assert_eq!(left, right, "{left:?} == {right:?}");
            assert_eq!(hash(&left), hash(&right), "{left:?} hashes as {right:?}");
        }
    }

    #[test]
    fn decimals_order_by_the_number_they_name() {
        assert!(Scalar::d128(1, 1) < Scalar::d128(2, 1));
        assert!(Scalar::d128(-1, 0) < Scalar::d128(1, 5));
        // 0.001 is smaller than 1 even though its coefficient is not.
        assert!(Scalar::d128(1, 3) < Scalar::d128(1, 0));
        // A coefficient too wide to restate is by that fact the larger one.
        assert!(Scalar::d128(i128::MAX, 0) > Scalar::d128(1, -30));
        assert!(Scalar::d128(i128::MIN, 0) < Scalar::d128(-1, -30));
    }

    #[test]
    fn a_decimal_is_its_own_kind_and_never_an_integer() {
        // A decimal carries a scale, an integer does not, and Arrow keeps them
        // apart too - so equality does not quietly merge them.
        assert_ne!(Scalar::d128(1, 0), Scalar::from(1_i64));
        assert!(Scalar::from(1_i64) < Scalar::d128(1, 0));
    }
}

mod restating {
    use super::Scalar;

    #[test]
    fn restating_adds_digits_freely_and_drops_none() {
        let price = Scalar::d128(1_050, 2);

        assert_eq!(price.decimal_unscaled_at(2), Some(1_050));
        assert_eq!(price.decimal_unscaled_at(4), Some(105_000));
        // 10.50 has a zero to spare, so scale 1 is exact...
        assert_eq!(price.decimal_unscaled_at(1), Some(105));
        // ... and scale 0 is not, because 10.5 is not a whole number.
        assert_eq!(price.decimal_unscaled_at(0), None);

        // A coefficient that would no longer fit is refused, not wrapped.
        assert_eq!(Scalar::d128(i128::MAX, 0).decimal_unscaled_at(1), None);
        // Only a decimal restates at all.
        assert_eq!(Scalar::from(105_i64).decimal_unscaled_at(1), None);
    }
}
