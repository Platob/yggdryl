//! Exact decimals: what they hold, how they compare, and how they restate.

use crate::Value;

mod representation {
    use super::Value;

    #[test]
    fn a_decimal_keeps_the_coefficient_and_the_scale_it_was_given() {
        let price = Value::decimal(1_050, 2);

        assert_eq!(price.as_decimal(), Some((1_050, 2)));
        assert!(price.is_decimal());
        assert_eq!(price.kind(), "decimal");
        assert!(!Value::from(10.5).is_decimal());
    }

    #[test]
    fn a_decimal_holds_a_fraction_no_double_can_hold() {
        // The point of the variant: one tenth has no finite binary expansion,
        // so a float can only ever hold the nearest double to it.
        let exact = Value::decimal(1, 1);
        assert_eq!(exact.as_decimal(), Some((1, 1)));
        assert_ne!(f64::from(0.1_f32), 0.1_f64);

        // The full 128-bit coefficient survives, which is what a float cannot do.
        assert_eq!(
            Value::decimal(i128::MAX, 0).as_decimal(),
            Some((i128::MAX, 0))
        );
    }
}

mod comparison {
    use super::Value;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash(value: &Value) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn two_spellings_of_one_number_are_one_value() {
        for (left, right) in [
            (Value::decimal(1_050, 2), Value::decimal(105, 1)),
            (Value::decimal(0, 7), Value::decimal(0, -7)),
            (Value::decimal(-1_050, 2), Value::decimal(-105, 1)),
            (Value::decimal(100, 0), Value::decimal(1, -2)),
        ] {
            assert_eq!(left, right, "{left:?} == {right:?}");
            assert_eq!(hash(&left), hash(&right), "{left:?} hashes as {right:?}");
        }
    }

    #[test]
    fn decimals_order_by_the_number_they_name() {
        assert!(Value::decimal(1, 1) < Value::decimal(2, 1));
        assert!(Value::decimal(-1, 0) < Value::decimal(1, 5));
        // 0.001 is smaller than 1 even though its coefficient is not.
        assert!(Value::decimal(1, 3) < Value::decimal(1, 0));
        // A coefficient too wide to restate is by that fact the larger one.
        assert!(Value::decimal(i128::MAX, 0) > Value::decimal(1, -30));
        assert!(Value::decimal(i128::MIN, 0) < Value::decimal(-1, -30));
    }

    #[test]
    fn a_decimal_is_its_own_kind_and_never_an_integer() {
        // A decimal carries a scale, an integer does not, and Arrow keeps them
        // apart too - so equality does not quietly merge them.
        assert_ne!(Value::decimal(1, 0), Value::from(1_i64));
        assert!(Value::from(1_i64) < Value::decimal(1, 0));
    }
}

mod restating {
    use super::Value;

    #[test]
    fn restating_adds_digits_freely_and_drops_none() {
        let price = Value::decimal(1_050, 2);

        assert_eq!(price.decimal_unscaled_at(2), Some(1_050));
        assert_eq!(price.decimal_unscaled_at(4), Some(105_000));
        // 10.50 has a zero to spare, so scale 1 is exact...
        assert_eq!(price.decimal_unscaled_at(1), Some(105));
        // ... and scale 0 is not, because 10.5 is not a whole number.
        assert_eq!(price.decimal_unscaled_at(0), None);

        // A coefficient that would no longer fit is refused, not wrapped.
        assert_eq!(Value::decimal(i128::MAX, 0).decimal_unscaled_at(1), None);
        // Only a decimal restates at all.
        assert_eq!(Value::from(105_i64).decimal_unscaled_at(1), None);
    }
}
