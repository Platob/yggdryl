//! Smoke coverage while the module is being built.

use super::Expression;

#[test]
fn text_round_trips() {
    for text in [
        "ccy = 'EUR' and price > 100",
        "a or b and c",
        "(a or b) and c",
        "not a",
        "x is null",
        "x is not null",
        "x is distinct from y",
        "x in (1, 2, 3)",
        "x between 1 and 10",
        "name like 'a%' escape '\\'",
        "path glob '*.parquet'",
        "&holder.size > 0",
        "&holder.partition['year'] = '2024'",
        "lower(name) = 'x'",
        "cast(x as int32) = 1",
        "try_cast(x as decimal128(9,2)) > decimal128(9,2) '1.50'",
        "case when a then 1 else 2 end",
        "struct(1 as a, 'b' as b)",
        "[1, 2, 3]",
        "{'a': 1}",
        "trade.legs[0]['ccy'] = 'EUR'",
        "-x + 3 * 2 - 1",
        ":since <= ts",
        "\"odd name\" = 1",
    ] {
        let parsed: Expression = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let printed = parsed.to_string();
        let again: Expression = printed
            .parse()
            .unwrap_or_else(|error| panic!("{printed}: {error}"));
        assert_eq!(parsed, again, "{text} printed as {printed}");
    }
}

#[test]
fn statements_round_trip() {
    use super::Statement;

    for text in [
        "select *",
        "select a, b as c where a > 1 order by b desc nulls first limit 10",
        "select lower(name) as name where name is not null",
    ] {
        let parsed: Statement = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let printed = parsed.to_string();
        let again: Statement = printed
            .parse()
            .unwrap_or_else(|error| panic!("{printed}: {error}"));
        assert_eq!(parsed, again, "{text} printed as {printed}");
    }
}

#[test]
fn binds_and_evaluates_rows() {
    use crate::{DataType, Field, Value};

    let schema = Field::new(
        "trades",
        DataType::from_fields([
            Field::new("ccy", DataType::Utf8, true),
            Field::new("price", DataType::decimal128(9, 2).unwrap(), true),
            Field::new("size", DataType::Int32, true),
        ])
        .unwrap(),
        false,
    );
    let bound = "ccy = 'EUR' and price > 100 and size is not null"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert!(bound.is_predicate());
    assert_eq!(bound.column_names(), vec!["ccy", "price", "size"]);

    let row = Value::from_sequence([Value::from("EUR"), Value::Decimal(15_000, 2), Value::I32(5)]);
    assert!(bound.matches(&row).unwrap());

    let other =
        Value::from_sequence([Value::from("USD"), Value::Decimal(15_000, 2), Value::I32(5)]);
    assert!(!bound.matches(&other).unwrap());

    let cheap = Value::from_sequence([Value::from("EUR"), Value::Decimal(5_000, 2), Value::I32(5)]);
    assert!(!bound.matches(&cheap).unwrap());
}

#[test]
fn unknown_is_not_true() {
    use crate::{DataType, Field, Value};

    let schema = Field::new(
        "rows",
        DataType::from_fields([Field::new("a", DataType::Int64, true)]).unwrap(),
        false,
    );
    let bound = "a > 1"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let row = Value::from_sequence([Value::Null]);
    assert_eq!(bound.eval(&row).unwrap(), Value::Null);
    assert!(!bound.matches(&row).unwrap());
}
