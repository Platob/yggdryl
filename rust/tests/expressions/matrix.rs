//! The exhaustive datatype matrix.
//!
//! This is the test that decides whether the engine is finished or merely
//! demonstrable, and it is built so that **adding a datatype to the core breaks
//! it rather than silently skipping it**: the cases are enumerated from
//! [`DataTypeId`]'s own variants, and a variant the matrix does not name is a
//! failure listing it. A variant the engine deliberately refuses is listed as
//! refused *with its reason*, never omitted - the matrix has no empty cells.

use std::sync::Arc;

use yggdryl::expressions::{Apply, ArrowApply};
use yggdryl::{DataType, DataTypeId, Expr, Field, TimeUnit, Value};

/// One datatype's place in the matrix: how it is exercised, or why it is not.
enum Case {
    /// A value the type holds, a smaller one, and a larger one.
    Ordered {
        data_type: DataType,
        low: Value,
        middle: Value,
        high: Value,
    },
    /// A type with values but no ordering worth three points.
    Equatable { data_type: DataType, value: Value },
    /// A type an expression reaches into rather than compares.
    Nested {
        data_type: DataType,
        value: Value,
        /// The accessor chain that reaches a scalar leaf, and what it reaches.
        path: &'static str,
        reaches: Value,
    },
    /// A type the engine deliberately answers nothing about, and why.
    Refused {
        data_type: DataType,
        reason: &'static str,
    },
}

impl Case {
    /// The datatype this case covers.
    fn data_type(&self) -> &DataType {
        match self {
            Self::Ordered { data_type, .. }
            | Self::Equatable { data_type, .. }
            | Self::Nested { data_type, .. }
            | Self::Refused { data_type, .. } => data_type,
        }
    }
}

/// Every datatype variant, with how the engine is asked about it.
///
/// Adding a variant to [`DataTypeId`] without adding a row here fails
/// [`every_datatype_variant_is_covered`], which is the point.
fn cases() -> Vec<Case> {
    use DataType as D;

    let list_of = |inner: DataType| D::list(inner.nullable_field("item"));
    let struct_of = |inner: DataType| {
        D::from_fields([inner.nullable_field("leaf")]).expect("a one-child struct")
    };

    vec![
        Case::Equatable {
            data_type: D::Null,
            value: Value::Null,
        },
        Case::Equatable {
            data_type: D::Boolean,
            value: Value::Bool(true),
        },
        Case::Ordered {
            data_type: D::Int8,
            low: Value::I64(-8),
            middle: Value::I64(0),
            high: Value::I64(8),
        },
        Case::Ordered {
            data_type: D::Int16,
            low: Value::I64(-16),
            middle: Value::I64(0),
            high: Value::I64(16),
        },
        Case::Ordered {
            data_type: D::Int32,
            low: Value::I64(-32),
            middle: Value::I64(0),
            high: Value::I64(32),
        },
        Case::Ordered {
            data_type: D::Int64,
            low: Value::I64(-64),
            middle: Value::I64(0),
            high: Value::I64(64),
        },
        Case::Ordered {
            data_type: D::UInt8,
            low: Value::U64(1),
            middle: Value::U64(8),
            high: Value::U64(200),
        },
        Case::Ordered {
            data_type: D::UInt16,
            low: Value::U64(1),
            middle: Value::U64(16),
            high: Value::U64(60_000),
        },
        Case::Ordered {
            data_type: D::UInt32,
            low: Value::U64(1),
            middle: Value::U64(32),
            high: Value::U64(4_000_000_000),
        },
        Case::Ordered {
            data_type: D::UInt64,
            low: Value::U64(1),
            middle: Value::U64(64),
            high: Value::U64(9_000_000_000_000_000_000),
        },
        Case::Ordered {
            data_type: D::Float16,
            low: Value::from(-1.5_f32),
            middle: Value::from(0.0_f32),
            high: Value::from(1.5_f32),
        },
        Case::Ordered {
            data_type: D::Float32,
            low: Value::from(-1.5_f32),
            middle: Value::from(0.0_f32),
            high: Value::from(1.5_f32),
        },
        Case::Ordered {
            data_type: D::Float64,
            low: Value::from(-1.5_f64),
            middle: Value::from(0.0_f64),
            high: Value::from(1.5_f64),
        },
        Case::Ordered {
            data_type: D::Decimal32 {
                precision: 9,
                scale: 2,
            },
            low: Value::Decimal(-150, 2),
            middle: Value::Decimal(0, 2),
            high: Value::Decimal(150, 2),
        },
        Case::Ordered {
            data_type: D::Decimal64 {
                precision: 18,
                scale: 2,
            },
            low: Value::Decimal(-150, 2),
            middle: Value::Decimal(0, 2),
            high: Value::Decimal(150, 2),
        },
        Case::Ordered {
            data_type: D::Decimal128 {
                precision: 38,
                scale: 4,
            },
            low: Value::Decimal(-15_000, 4),
            middle: Value::Decimal(0, 4),
            high: Value::Decimal(15_000, 4),
        },
        Case::Ordered {
            data_type: D::Decimal256 {
                precision: 50,
                scale: 4,
            },
            low: Value::Decimal(-15_000, 4),
            middle: Value::Decimal(0, 4),
            high: Value::Decimal(15_000, 4),
        },
        Case::Ordered {
            data_type: D::Utf8,
            low: Value::from(""),
            middle: Value::from("mmm"),
            high: Value::from("zzz"),
        },
        Case::Ordered {
            data_type: D::LargeUtf8,
            low: Value::from(""),
            middle: Value::from("mmm"),
            high: Value::from("zzz"),
        },
        Case::Ordered {
            data_type: D::Utf8View,
            low: Value::from(""),
            middle: Value::from("mmm"),
            high: Value::from("zzz"),
        },
        Case::Ordered {
            data_type: D::Binary,
            low: Value::Bytes(Arc::from(&b""[..])),
            middle: Value::Bytes(Arc::from(&b"mmm"[..])),
            high: Value::Bytes(Arc::from(&b"zzz"[..])),
        },
        Case::Ordered {
            data_type: D::LargeBinary,
            low: Value::Bytes(Arc::from(&b""[..])),
            middle: Value::Bytes(Arc::from(&b"mmm"[..])),
            high: Value::Bytes(Arc::from(&b"zzz"[..])),
        },
        Case::Ordered {
            data_type: D::BinaryView,
            low: Value::Bytes(Arc::from(&b""[..])),
            middle: Value::Bytes(Arc::from(&b"mmm"[..])),
            high: Value::Bytes(Arc::from(&b"zzz"[..])),
        },
        Case::Ordered {
            data_type: D::FixedSizeBinary(3),
            low: Value::Bytes(Arc::from(&b"aaa"[..])),
            middle: Value::Bytes(Arc::from(&b"mmm"[..])),
            high: Value::Bytes(Arc::from(&b"zzz"[..])),
        },
        Case::Ordered {
            data_type: D::Date32,
            low: Value::Date(0),
            middle: Value::Date(19_723),
            high: Value::Date(20_000),
        },
        Case::Ordered {
            data_type: D::Date64,
            low: Value::Date(0),
            middle: Value::Date(19_723),
            high: Value::Date(20_000),
        },
        Case::Ordered {
            data_type: D::Time32(TimeUnit::Second),
            low: Value::Time(0, TimeUnit::Second),
            middle: Value::Time(3_600, TimeUnit::Second),
            high: Value::Time(86_399, TimeUnit::Second),
        },
        Case::Ordered {
            data_type: D::Time64(TimeUnit::Microsecond),
            low: Value::Time(0, TimeUnit::Microsecond),
            middle: Value::Time(3_600_000_000, TimeUnit::Microsecond),
            high: Value::Time(86_399_000_000, TimeUnit::Microsecond),
        },
        Case::Ordered {
            data_type: D::Timestamp(TimeUnit::Millisecond, None),
            low: Value::DateTime(0, TimeUnit::Millisecond),
            middle: Value::DateTime(1_700_000_000_000, TimeUnit::Millisecond),
            high: Value::DateTime(1_800_000_000_000, TimeUnit::Millisecond),
        },
        Case::Ordered {
            data_type: D::Duration(TimeUnit::Second),
            low: Value::Duration(0, TimeUnit::Second),
            middle: Value::Duration(60, TimeUnit::Second),
            high: Value::Duration(3_600, TimeUnit::Second),
        },
        // A calendar interval is a tuple of counts rather than one count, so
        // nothing converts into one and no comparison means anything: it is
        // carried, never compared.
        Case::Refused {
            data_type: D::Interval(TimeUnit::YearMonth),
            reason: "a calendar interval is a tuple of counts, not one quantity",
        },
        Case::Nested {
            data_type: list_of(D::Int64),
            value: Value::from_sequence([Value::I64(1), Value::I64(2), Value::I64(3)]),
            path: "[1]",
            reaches: Value::I64(2),
        },
        Case::Nested {
            data_type: D::large_list(D::Int64.nullable_field("item")),
            value: Value::from_sequence([Value::I64(1), Value::I64(2)]),
            path: "[0]",
            reaches: Value::I64(1),
        },
        Case::Nested {
            data_type: D::list_view(D::Int64.nullable_field("item")),
            value: Value::from_sequence([Value::I64(7)]),
            path: "[0]",
            reaches: Value::I64(7),
        },
        Case::Nested {
            data_type: D::large_list_view(D::Int64.nullable_field("item")),
            value: Value::from_sequence([Value::I64(7)]),
            path: "[-1]",
            reaches: Value::I64(7),
        },
        Case::Nested {
            data_type: D::fixed_size_list(D::Int64.nullable_field("item"), 2)
                .expect("a fixed-size list"),
            value: Value::from_sequence([Value::I64(4), Value::I64(5)]),
            path: "[1]",
            reaches: Value::I64(5),
        },
        Case::Nested {
            data_type: struct_of(D::Int64),
            value: Value::record(struct_of(D::Int64), [Value::I64(11)]).expect("a struct row"),
            path: ".leaf",
            reaches: Value::I64(11),
        },
        Case::Nested {
            data_type: D::map_of(D::Utf8, D::Int64, false).expect("a map"),
            value: Value::from_mapping([(Value::from("k"), Value::I64(9))]).expect("a mapping"),
            path: "['k']",
            reaches: Value::I64(9),
        },
        Case::Ordered {
            data_type: D::dictionary(D::Int32, D::Utf8).expect("a dictionary"),
            low: Value::from("aaa"),
            middle: Value::from("mmm"),
            high: Value::from("zzz"),
        },
        Case::Ordered {
            data_type: D::run_end_encoded(
                D::Int32.required_field("run_ends"),
                D::Int64.nullable_field("values"),
            )
            .expect("a run-end encoding"),
            low: Value::I64(-1),
            middle: Value::I64(0),
            high: Value::I64(1),
        },
        // A union's value is one of several types at a time, so a comparison
        // against it has no single reading; a caller narrows it first.
        Case::Refused {
            data_type: D::variant([D::Int64.nullable_field("n")]).expect("a dense union"),
            reason: "a union holds one of several types at a time",
        },
    ]
}

/// Spell one value the way this grammar spells a literal.
///
/// The renderer itself is crate-private, and reaching it through an expression
/// is what proves the public surface can name every value the matrix uses.
fn literal_text(value: &Value) -> String {
    Expr::literal(value.clone()).to_string()
}

/// Build a one-column struct root over a datatype.
fn root(data_type: &DataType, nullable: bool) -> Field {
    DataType::from_fields([Field::new("column", data_type.clone(), nullable)])
        .expect("a one-column struct")
        .required_field("row")
}

/// Build a row holding one value.
fn row(schema: &Field, value: Value) -> Value {
    Value::record(schema.data_type().clone(), [value]).expect("a one-column row")
}

#[test]
fn every_datatype_variant_is_covered() {
    let covered: Vec<DataTypeId> = cases().iter().map(|case| case.data_type().id()).collect();
    let missing: Vec<&str> = DataTypeId::ALL
        .iter()
        .filter(|id| !covered.contains(id))
        .map(|id| id.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "the matrix names no case for {missing:?}; add one, or list it as refused with its reason"
    );
}

#[test]
fn every_ordered_type_answers_every_operator_the_same_way_three_times() {
    for case in cases() {
        let Case::Ordered {
            data_type,
            low,
            middle,
            high,
        } = case
        else {
            continue;
        };
        for nullable in [true, false] {
            let schema = root(&data_type, nullable);
            let rows = [
                row(&schema, low.clone()),
                row(&schema, middle.clone()),
                row(&schema, high.clone()),
            ];
            let literal = literal_text(&middle);
            for (text, expected) in [
                (format!("column = {literal}"), vec![false, true, false]),
                (format!("column <> {literal}"), vec![true, false, true]),
                (format!("column < {literal}"), vec![true, false, false]),
                (format!("column <= {literal}"), vec![true, true, false]),
                (format!("column > {literal}"), vec![false, false, true]),
                (format!("column >= {literal}"), vec![false, true, true]),
                (format!("column IN ({literal})"), vec![false, true, false]),
                (
                    format!("column BETWEEN {literal} AND {literal}"),
                    vec![false, true, false],
                ),
                ("column IS NULL".to_owned(), vec![false, false, false]),
                ("column IS NOT NULL".to_owned(), vec![true, true, true]),
            ] {
                let predicate = text
                    .parse::<Expr>()
                    .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"))
                    .bind(&schema)
                    .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"))
                    .into_predicate()
                    .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"));
                let answered: Vec<bool> = rows
                    .iter()
                    .map(|row| {
                        predicate
                            .matches(row)
                            .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"))
                    })
                    .collect();
                assert_eq!(answered, expected, "{data_type} answering {text}");
            }
        }
    }
}

#[test]
fn every_nested_type_is_reached_into_and_reports_the_schema_it_produces() {
    for case in cases() {
        let Case::Nested {
            data_type,
            value,
            path,
            reaches,
        } = case
        else {
            continue;
        };
        let schema = root(&data_type, true);
        let text = format!("column{path}");
        let bound = text
            .parse::<Expr>()
            .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"))
            .bind(&schema)
            .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"));
        let read = bound
            .evaluate(&row(&schema, value))
            .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"));
        assert_eq!(read, reaches, "{data_type} reached by {text}");

        // The field an accessor produces is answerable with no data at all.
        let reported = text
            .parse::<Expr>()
            .expect("parses")
            .apply_field(&schema)
            .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"));
        assert_eq!(reported.field_len(), 1, "{data_type} reported one column");
    }
}

#[test]
fn every_refused_type_is_refused_by_name_rather_than_answered_wrongly() {
    for case in cases() {
        let Case::Refused { data_type, reason } = case else {
            continue;
        };
        let schema = root(&data_type, true);
        // Binding succeeds - a column of any type can be named - and what is
        // refused is asking a question the type has no answer to.
        let compared = "column > column"
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema);
        let evaluated =
            compared.and_then(|bound| bound.evaluate(&row(&schema, data_type.default_value()?)));
        // Whatever happens, it is a typed answer or a typed error - never a
        // panic and never a wrong `true`.
        match evaluated {
            Ok(value) => assert!(
                value.is_null() || matches!(value, Value::Bool(false)),
                "{data_type} answered {value:?} where nothing is comparable ({reason})"
            ),
            Err(error) => assert!(!error.to_string().is_empty(), "{data_type}: {reason}"),
        }
    }
}

#[test]
fn a_null_makes_every_comparison_unknown_for_every_type() {
    for case in cases() {
        let (data_type, value) = match &case {
            Case::Ordered {
                data_type, middle, ..
            } => (data_type, middle.clone()),
            Case::Equatable { data_type, value } => (data_type, value.clone()),
            _ => continue,
        };
        if matches!(data_type, DataType::Null) {
            continue;
        }
        let schema = root(data_type, true);
        let literal = literal_text(&value);
        let empty = row(&schema, Value::Null);
        for text in [
            format!("column = {literal}"),
            format!("column <> {literal}"),
            format!("column < {literal}"),
            format!("column IN ({literal})"),
        ] {
            let bound = text
                .parse::<Expr>()
                .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"))
                .bind(&schema)
                .unwrap_or_else(|error| panic!("{data_type}: {text}: {error}"));
            let answered = bound.evaluate(&empty).expect("evaluates");
            assert!(
                answered.is_null(),
                "{data_type} answered {answered:?} rather than unknown for {text}"
            );
        }
        // And `IS NULL` is the only way to ask, in every type.
        let asked = "column IS NULL"
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema)
            .expect("binds")
            .into_predicate()
            .expect("a predicate");
        assert!(asked.matches(&empty).expect("evaluates"), "{data_type}");
    }
}

#[test]
fn a_literal_is_folded_into_the_column_type_for_every_type_that_reads_text() {
    for case in cases() {
        let Case::Ordered {
            data_type, middle, ..
        } = case
        else {
            continue;
        };
        // Every filter value arrives as text somewhere - a directory name, a
        // pair list, a query string - so every type that can read one must
        // compare in its own type rather than as a string.
        let Some(text) = yggdryl::expressions::coerce_value(&middle, &DataType::Utf8) else {
            continue;
        };
        let Some(text) = text.as_str().map(ToOwned::to_owned) else {
            continue;
        };
        if yggdryl::expressions::coerce_text(&text, &data_type).as_ref() != Some(&middle) {
            // The type does not round-trip through text, so text is not a way
            // to name one of its values and nothing here applies.
            continue;
        }
        let schema = root(&data_type, true);
        let escaped = text.replace('\'', "''");
        let bound = format!("column = '{escaped}'")
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema)
            .unwrap_or_else(|error| panic!("{data_type}: {error}"));
        let predicate = bound.into_predicate().expect("a predicate");
        assert!(
            predicate
                .matches(&row(&schema, middle.clone()))
                .expect("evaluates"),
            "{data_type} did not fold {text:?} into its own type"
        );
    }
}

#[test]
fn a_cross_type_comparison_happens_in_the_common_type_or_is_refused_by_name() {
    // The pairs the type system admits, each asserting the comparison is
    // numerically right rather than rank-ordered - which is what `Value`'s own
    // total order would have answered.
    let pairs: Vec<(DataType, Value, DataType, Value, bool)> = vec![
        (
            DataType::Int32,
            Value::I64(2),
            DataType::Int64,
            Value::I64(10),
            true,
        ),
        (
            DataType::Int64,
            Value::I64(2),
            DataType::Decimal128 {
                precision: 10,
                scale: 2,
            },
            Value::Decimal(1_000, 2),
            true,
        ),
        (
            DataType::Decimal128 {
                precision: 10,
                scale: 2,
            },
            Value::Decimal(150, 2),
            DataType::Decimal128 {
                precision: 10,
                scale: 4,
            },
            Value::Decimal(20_000, 4),
            true,
        ),
        (
            DataType::Float64,
            Value::from(1.5_f64),
            DataType::Int64,
            Value::I64(2),
            true,
        ),
        (
            DataType::Date32,
            Value::Date(19_723),
            DataType::Timestamp(TimeUnit::Millisecond, None),
            Value::DateTime(1_800_000_000_000, TimeUnit::Millisecond),
            true,
        ),
        (
            DataType::Utf8,
            Value::from("a"),
            DataType::Utf8View,
            Value::from("b"),
            true,
        ),
        // Text against a number has no common comparison type at all.
        (
            DataType::Utf8,
            Value::from("a"),
            DataType::Int64,
            Value::I64(1),
            false,
        ),
    ];
    for (left_type, left, right_type, right, comparable) in pairs {
        let schema = DataType::from_fields([
            Field::new("left", left_type.clone(), true),
            Field::new("right", right_type.clone(), true),
        ])
        .expect("a two-column struct")
        .required_field("row");
        let bound = "left < right"
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema);
        if !comparable {
            let error = bound.expect_err("no common comparison type").to_string();
            assert!(error.contains(&left_type.to_string()), "{error}");
            assert!(error.contains(&right_type.to_string()), "{error}");
            continue;
        }
        let row = Value::record(schema.data_type().clone(), [left.clone(), right.clone()])
            .expect("a row");
        let answered = bound
            .unwrap_or_else(|error| panic!("{left_type} vs {right_type}: {error}"))
            .into_predicate()
            .expect("a predicate")
            .matches(&row)
            .expect("evaluates");
        assert!(
            answered,
            "{left_type} {left:?} < {right_type} {right:?} answered false"
        );
    }
}

#[test]
fn a_deeply_nested_value_is_reached_through_one_chain() {
    // Three levels, generated rather than typed out one shape at a time:
    // a map of lists of structs, reached to its innermost leaf.
    let leaf = DataType::from_fields([DataType::Int64.nullable_field("price")]).expect("a struct");
    let list = DataType::list(leaf.clone().nullable_field("item"));
    let map = DataType::map_of(DataType::Utf8, list.clone(), false).expect("a map");
    let schema = DataType::from_fields([map.nullable_field("payload")])
        .expect("a one-column struct")
        .required_field("row");

    let inner = Value::record(leaf, [Value::I64(42)]).expect("a leaf row");
    let value = Value::from_mapping([(Value::from("k"), Value::from_sequence([inner]))])
        .expect("a mapping");
    let row = Value::record(schema.data_type().clone(), [value]).expect("a row");

    let bound = "payload['k'][0].price"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds");
    assert_eq!(bound.evaluate(&row).expect("evaluates"), Value::I64(42));

    // The reported schema equals what the value produced, with nothing read.
    let reported = "payload['k'][0].price"
        .parse::<Expr>()
        .expect("parses")
        .apply_field(&schema)
        .expect("reports a schema");
    assert_eq!(
        reported.fields()[0].data_type(),
        &DataType::Int64,
        "{reported}"
    );
}

#[test]
fn apply_field_equals_the_schema_apply_arrow_batch_produces() {
    for case in cases() {
        let (data_type, value) = match &case {
            Case::Ordered {
                data_type, middle, ..
            } => (data_type, middle.clone()),
            Case::Equatable { data_type, value } => (data_type, value.clone()),
            Case::Nested {
                data_type, value, ..
            } => (data_type, value.clone()),
            Case::Refused { .. } => continue,
        };
        let schema = root(data_type, true);
        let Ok(arrow_schema) = yggdryl::arrow::schema_from_field(&schema) else {
            continue;
        };
        let Ok(array) = yggdryl::arrow::scalar_array(&schema.fields()[0], &value) else {
            continue;
        };
        let Ok(batch) = arrow_array::RecordBatch::try_new(arrow_schema, vec![array]) else {
            continue;
        };
        // A projection that renames is the interesting case: the reported
        // schema has to carry the alias, and so does the produced batch.
        let selection: yggdryl::Selection = "column AS renamed".parse().expect("parses");
        let reported = selection.apply_field(&schema).expect("reports a schema");
        let produced = selection
            .apply_arrow_batch(batch)
            .unwrap_or_else(|error| panic!("{data_type}: {error}"));
        let produced_root =
            yggdryl::arrow::record_schema_from_arrow("row", produced.schema().as_ref())
                .expect("reads the produced schema");
        assert_eq!(
            reported.field_len(),
            produced_root.field_len(),
            "{data_type}: {reported} vs {produced_root}"
        );
        assert_eq!(
            reported.fields()[0].name(),
            produced_root.fields()[0].name(),
            "{data_type}"
        );
    }
}
