//! `rust/src/expression/filter.rs` over the enum leaves `yggdryl-market`
//! claims: a predicate naming a kind's member keeps the rows its Arrow
//! filter keeps, over the member and over its stored code. The rule over
//! the core's `State` is `rust/tests/expression/filter.rs`'s.

#[test]
fn logical_enum_predicates_keep_native_rows_as_arrow_filters_do_on_the_market_kinds() {
    crate::install::installed();
    use yggdryl::expression::Filter;
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};
    use yggdryl_market::{MarketDataKind, MarketDataType, Side, TimeInForce};

    let members = [
        (
            Scalar::from(MarketDataKind::ALL[0]),
            Scalar::from(MarketDataKind::ALL[1]),
        ),
        (
            Scalar::from(MarketDataType::ALL[0]),
            Scalar::from(MarketDataType::ALL[1]),
        ),
        (Scalar::from(Side::ALL[0]), Scalar::from(Side::ALL[1])),
        (
            Scalar::from(TimeInForce::ALL[0]),
            Scalar::from(TimeInForce::ALL[1]),
        ),
    ];
    for (first, second) in members {
        let dtype = DataType::from_str(first.id().as_str()).unwrap();
        let root =
            DataType::from(StructType::from_fields([Field::new("member", dtype, true)]).unwrap())
                .required_field("rows");
        let rows = [first.clone(), second.clone(), Scalar::Null]
            .map(|value| Scalar::from_sequence([value]));
        let batch = Serie::from_scalars(root.clone(), rows.clone())
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        let first_name = first.as_str().unwrap();
        let second_name = second.as_str().unwrap();
        let stored = Scalar::from_sequence([Scalar::from(first.enum_code().unwrap())]);
        for predicate in [
            format!("member = '{first_name}'"),
            format!("member != '{first_name}'"),
            format!("member in ('{first_name}')"),
            format!("member < '{second_name}'"),
            format!("member is distinct from '{first_name}'"),
        ] {
            let filter: Filter = predicate.parse().unwrap();
            let bound = filter.bind(&root).unwrap();
            assert_eq!(
                bound.matches(&rows[0]).unwrap(),
                bound.matches(&stored).unwrap(),
                "{predicate} over a physical enum code"
            );
            let kept = rows
                .iter()
                .filter(|row| bound.matches(row).unwrap())
                .count();
            assert_eq!(
                kept,
                filter.apply_arrow_batch(&batch).unwrap().num_rows(),
                "{predicate} over {}",
                first.id()
            );
        }
        assert!(
            format!("member = '{first_name}'")
                .parse::<Filter>()
                .unwrap()
                .bind(&root)
                .unwrap()
                .matches(&rows[0])
                .unwrap()
        );
    }
}
