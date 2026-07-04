//! Pretty `display()` across the scalars, data types and fields — atomic values,
//! serie tables, recursive struct tables, records, and the edge cases (null, empty,
//! truncation by rows, fit-to-screen by columns). The exact-string cases double as a
//! record of how the output looks.

use yggdryl_scalar::yggdryl_dtype::{self as dtype, arrow_schema, DataType};
use yggdryl_scalar::{
    AnyScalar, BinaryScalar, DisplayOptions, Float64Scalar, Int64Scalar, Int64Serie, MapScalar,
    RecordScalar, Scalar, Serie, TypedMapScalar, TypedSerie, TypedStructSerie, UInt8Scalar,
    Utf8Scalar,
};

// ---- atomic scalars ----

#[test]
fn atomic_scalars_display_their_value() {
    assert_eq!(Int64Scalar::new(42).display(), "42");
    assert_eq!(Int64Scalar::new(-7).display(), "-7");
    assert_eq!(Int64Scalar::null().display(), "null");
    assert_eq!(Float64Scalar::new(3.5).display(), "3.5");
    assert_eq!(Utf8Scalar::new("hi".into()).display(), "\"hi\"");
    assert_eq!(Utf8Scalar::null().display(), "null");
    assert_eq!(BinaryScalar::new(vec![1, 2, 255]).display(), "0x0102ff");
    assert_eq!(BinaryScalar::null().display(), "null");
}

// ---- data-type signatures ----

#[test]
fn data_type_signatures_are_recursive() {
    assert_eq!(dtype::Int64Type.display(), "int64");
    assert_eq!(dtype::Utf8Type.display(), "utf8");
    assert_eq!(dtype::NullType.display(), "null");

    let list = dtype::SerieType::new(arrow_schema::DataType::Int64);
    assert_eq!(list.display(), "list<int64>");

    let point = dtype::StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Float64, false),
    ]));
    assert_eq!(point.display(), "struct<x: int64, y: float64>");

    // A list of structs nests both levels.
    let list_of_point = dtype::SerieType::new(point.to_arrow());
    assert_eq!(
        list_of_point.display(),
        "list<struct<x: int64, y: float64>>"
    );

    // An optional is a null-or-value union, shown as `optional<…>`.
    let optional = dtype::OptionalType::new(&dtype::Int64Type);
    assert_eq!(optional.display(), "optional<int64>");
}

#[test]
fn optional_scalar_displays_the_inner_value_not_its_union() {
    // Regression: an optional's storage is a union; displaying it must delegate to the
    // inner scalar (or `null`), never recurse into the union representation.
    use yggdryl_scalar::yggdryl_dtype::Int64Type;
    use yggdryl_scalar::TypedOptionalScalar;

    let some = TypedOptionalScalar::<Int64Type, Int64Scalar>::new(Int64Scalar::new(7));
    assert_eq!(some.display(), "7");
    let none: TypedOptionalScalar<Int64Type, Int64Scalar> = TypedOptionalScalar::null();
    assert_eq!(none.display(), "null");
}

// ---- serie tables ----

#[test]
fn serie_renders_a_table_with_header_and_values() {
    let numbers = Int64Serie::from(vec![1i64, 2, 3]);
    let expected = "\
┌───────┐
│ item  │
│ int64 │
├───────┤
│ 1     │
│ 2     │
│ 3     │
└───────┘";
    assert_eq!(numbers.display(), expected);
    assert_eq!(numbers.field().name(), "item"); // fast field accessor
}

#[test]
fn serie_truncates_past_max_rows_with_a_footer() {
    let many = Int64Serie::from((0..25i64).collect::<Vec<_>>());
    let shown = many.display();
    assert!(shown.contains("│ 0")); // first row present
    assert!(shown.contains("│ 9")); // the tenth row (index 9) present
    assert!(!shown.contains("│ 10 ")); // the eleventh is hidden
    assert!(shown.ends_with("… (15 more)")); // 25 - 10 shown

    // A custom row budget renders more of them.
    let three = many.display_with(DisplayOptions {
        max_rows: 3,
        ..Default::default()
    });
    assert!(three.ends_with("… (22 more)"));
}

#[test]
fn serie_null_and_empty_display_distinctly() {
    assert_eq!(Int64Serie::null().display(), "null");
    // The empty serie is a header with no rows (not null).
    let empty = Int64Serie::from(Vec::<i64>::new()).display();
    assert!(empty.contains("item"));
    assert!(!empty.contains("null"));
    assert!(!empty.contains("more"));
}

#[test]
fn serie_with_a_null_element_shows_null_in_the_cell() {
    let serie = Int64Serie::from(vec![Some(1), None, Some(3)]);
    let shown = serie.display();
    assert!(shown.contains("│ null"));
    assert!(shown.contains("│ 1"));
    assert!(shown.contains("│ 3"));
}

// ---- nested: struct serie is a recursive table ----

fn point_type() -> dtype::StructType {
    dtype::StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("name", arrow_schema::DataType::Utf8, true),
    ]))
}

fn point(x: i64, name: &str) -> RecordScalar {
    RecordScalar::new(
        point_type(),
        vec![
            AnyScalar::from(Int64Scalar::new(x)),
            AnyScalar::from_arrow(Utf8Scalar::new(name.into()).to_arrow_scalar()),
        ],
    )
    .unwrap()
}

#[test]
fn struct_serie_renders_one_column_per_field() {
    let serie = TypedStructSerie::new(point_type(), vec![point(1, "a"), point(2, "b")]);
    let expected = "\
┌───────┬──────┐
│ x     │ name │
│ int64 │ utf8 │
├───────┼──────┤
│ 1     │ \"a\"  │
│ 2     │ \"b\"  │
└───────┴──────┘";
    assert_eq!(serie.display(), expected);
}

#[test]
fn record_renders_a_transposed_field_value_table() {
    let shown = point(42, "zoe").display();
    assert!(shown.contains("field"));
    assert!(shown.contains("value"));
    assert!(shown.contains("x: int64"));
    assert!(shown.contains("name: utf8"));
    assert!(shown.contains("42"));
    assert!(shown.contains("\"zoe\""));
    // A null record collapses to `null`.
    assert_eq!(RecordScalar::null(point_type()).display(), "null");
}

// ---- fit to screen: many columns collapse into a trailing … column ----

#[test]
fn a_wide_struct_serie_fits_the_screen() {
    // Twelve int columns would overflow a narrow width; the tail collapses to `…`.
    let fields: Vec<arrow_schema::Field> = (0..12)
        .map(|i| {
            arrow_schema::Field::new(format!("col{i:02}"), arrow_schema::DataType::Int64, false)
        })
        .collect();
    let wide_type = dtype::StructType::new(arrow_schema::Fields::from(fields.clone()));
    let scalars: Vec<AnyScalar> = (0..12)
        .map(|i| AnyScalar::from(Int64Scalar::new(i)))
        .collect();
    let row = RecordScalar::new(wide_type.clone(), scalars).unwrap();
    let serie = TypedStructSerie::new(wide_type, vec![row]);

    let narrow = serie.display_with(DisplayOptions {
        max_width: 40,
        ..Default::default()
    });
    assert!(narrow.contains("col00"));
    assert!(narrow.contains('…')); // the overflow marker column
    assert!(!narrow.contains("col11")); // the last columns are dropped
                                        // Every rendered line fits the width budget (plus a little for the borders).
    for line in narrow.lines() {
        assert!(line.chars().count() <= 46, "line too wide: {line:?}");
    }
}

// ---- nested: maps, list cells, and recursive containers ----

#[test]
fn map_renders_a_key_value_table() {
    let ranks = TypedMapScalar::new(vec![
        (UInt8Scalar::new(7), Int64Scalar::new(42)),
        (UInt8Scalar::new(9), Int64Scalar::new(100)),
    ])
    .unwrap();
    let expected = "\
┌───────┬───────┐
│ key   │ value │
│ uint8 │ int64 │
├───────┼───────┤
│ 7     │ 42    │
│ 9     │ 100   │
└───────┴───────┘";
    // The typed map and the dynamic map it erases to render identically.
    assert_eq!(ranks.display(), expected);
    assert_eq!(ranks.erase().display(), expected);
}

#[test]
fn typed_map_display_does_not_recurse_into_its_union_storage() {
    // Regression: a `TypedMapScalar` used the atomic default, whose Arrow bounce had no
    // `Map` arm — it re-wrapped the same map and recursed until the stack overflowed.
    let one = TypedMapScalar::new(vec![(UInt8Scalar::new(1), Int64Scalar::new(2))]).unwrap();
    assert!(one.display().contains("│ 1"));
    assert!(one.display().contains("│ 2"));
    // The null and empty maps stay distinct and never blow up.
    let null: TypedMapScalar<dtype::UInt8Type, dtype::Int64Type, UInt8Scalar, Int64Scalar> =
        TypedMapScalar::null();
    assert_eq!(null.display(), "null");
    let empty: TypedMapScalar<dtype::UInt8Type, dtype::Int64Type, UInt8Scalar, Int64Scalar> =
        TypedMapScalar::default();
    assert!(empty.display().contains("key"));
    assert!(!empty.display().contains("null"));
}

#[test]
fn a_big_map_only_formats_max_rows_but_footers_the_true_total() {
    // The typed map must not materialize every entry to print the first few: only the
    // head is assembled, yet the footer still reports the full count.
    let big = TypedMapScalar::new(
        (0..25u8)
            .map(|k| (UInt8Scalar::new(k), Int64Scalar::new(i64::from(k))))
            .collect(),
    )
    .unwrap();
    let shown = big.display();
    assert!(shown.contains("│ 0")); // first entry present
    assert!(shown.contains("│ 9")); // the tenth entry present
    assert!(shown.ends_with("… (15 more)")); // 25 - 10 rendered
                                             // A custom row budget renders more entries and updates the footer.
    let three = big.display_with(DisplayOptions {
        max_rows: 3,
        ..Default::default()
    });
    assert!(three.ends_with("… (22 more)"));
}

#[test]
fn a_serie_of_lists_renders_each_list_inline() {
    let lists = TypedSerie::new(vec![
        TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]),
        TypedSerie::new(vec![
            Int64Scalar::new(3),
            Int64Scalar::new(4),
            Int64Scalar::new(5),
        ]),
    ]);
    let expected = "\
┌─────────────┐
│ item        │
│ list<int64> │
├─────────────┤
│ [1, 2]      │
│ [3, 4, 5]   │
└─────────────┘";
    assert_eq!(lists.display(), expected);
}

#[test]
fn a_long_list_cell_elides_past_six_elements() {
    let long: Vec<Int64Scalar> = (0..20).map(Int64Scalar::new).collect();
    let serie = TypedSerie::new(vec![TypedSerie::new(long)]);
    let shown = serie.display();
    assert!(shown.contains("[0, 1, 2, 3, 4, 5, …]"), "got: {shown}");
}

#[test]
fn a_struct_serie_renders_nested_list_and_struct_fields_inline() {
    let loc_ty = dtype::StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
    ]));
    let row_ty = dtype::StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("id", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new(
            "tags",
            arrow_schema::DataType::List(std::sync::Arc::new(arrow_schema::Field::new(
                "item",
                arrow_schema::DataType::Utf8,
                true,
            ))),
            true,
        ),
        arrow_schema::Field::new("loc", loc_ty.to_arrow(), true),
    ]));
    let tags = TypedSerie::new(vec![
        Utf8Scalar::new("red".into()),
        Utf8Scalar::new("blue".into()),
    ]);
    let loc = RecordScalar::new(
        loc_ty,
        vec![
            AnyScalar::from(Int64Scalar::new(10)),
            AnyScalar::from(Int64Scalar::new(20)),
        ],
    )
    .unwrap();
    let row = RecordScalar::new(
        row_ty.clone(),
        vec![
            AnyScalar::from(Int64Scalar::new(1)),
            AnyScalar::from_arrow(tags.to_arrow_scalar()),
            AnyScalar::from_arrow(loc.to_arrow_scalar()),
        ],
    )
    .unwrap();
    let serie = TypedStructSerie::new(row_ty, vec![row.clone()]);
    let shown = serie.display();
    // One column per field; the nested list and struct fields render as compact cells.
    assert!(shown.contains("│ [\"red\", \"blue\"]"), "got: {shown}");
    assert!(shown.contains("│ {x: 10, y: 20}"), "got: {shown}");
    assert!(shown.contains("list<utf8>"));
    assert!(shown.contains("struct<x: int64, y: int64>"));

    // The record itself (transposed) shows the same nested cells against their fields.
    let record = row.display();
    assert!(record.contains("tags: list<utf8>"));
    assert!(record.contains("[\"red\", \"blue\"]"));
    assert!(record.contains("loc: struct<x: int64, y: int64>"));
    assert!(record.contains("{x: 10, y: 20}"));
}

#[test]
fn deeply_nested_lists_collapse_at_the_depth_cap() {
    // A list nested far past the cap must collapse to `[…]`, never overflow the stack.
    use arrow_array::Array;
    let mut nested: std::sync::Arc<dyn Array> =
        std::sync::Arc::new(arrow_array::Int64Array::from(vec![1i64, 2, 3]));
    for _ in 0..8 {
        let field = std::sync::Arc::new(arrow_schema::Field::new(
            "item",
            nested.data_type().clone(),
            true,
        ));
        nested = std::sync::Arc::new(
            arrow_array::ListArray::try_new(
                field,
                arrow_buffer::OffsetBuffer::from_lengths([nested.len()]),
                nested,
                None,
            )
            .unwrap(),
        );
    }
    let serie = Serie::from_arrow(&nested).unwrap();
    let shown = serie.display();
    // The innermost levels collapse to a single `…`, bounding the cell.
    assert!(shown.contains("[[[[[[…]]]]]]"), "got: {shown}");
    // Every cell line stays within the elision budget (no runaway wall of brackets).
    for line in shown.lines() {
        assert!(line.chars().count() <= 46, "line too wide: {line:?}");
    }
}

#[test]
fn a_map_cell_with_a_struct_value_renders_inline() {
    // A map whose value is a struct, reached through the dynamic map from Arrow, shows
    // each entry's struct value compactly.
    use arrow_array::Array;
    let value_struct = arrow_array::StructArray::from(vec![
        (
            std::sync::Arc::new(arrow_schema::Field::new(
                "x",
                arrow_schema::DataType::Int64,
                false,
            )),
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![1i64, 3]))
                as std::sync::Arc<dyn Array>,
        ),
        (
            std::sync::Arc::new(arrow_schema::Field::new(
                "y",
                arrow_schema::DataType::Int64,
                false,
            )),
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![2i64, 4]))
                as std::sync::Arc<dyn Array>,
        ),
    ]);
    let entries_struct = arrow_array::StructArray::from(vec![
        (
            std::sync::Arc::new(arrow_schema::Field::new(
                "key",
                arrow_schema::DataType::UInt8,
                false,
            )),
            std::sync::Arc::new(arrow_array::UInt8Array::from(vec![1u8, 2]))
                as std::sync::Arc<dyn Array>,
        ),
        (
            std::sync::Arc::new(arrow_schema::Field::new(
                "value",
                value_struct.data_type().clone(),
                true,
            )),
            std::sync::Arc::new(value_struct) as std::sync::Arc<dyn Array>,
        ),
    ]);
    let entries_field = std::sync::Arc::new(arrow_schema::Field::new(
        "entries",
        entries_struct.data_type().clone(),
        false,
    ));
    let map_array = arrow_array::MapArray::try_new(
        entries_field,
        arrow_buffer::OffsetBuffer::from_lengths([2usize]),
        entries_struct,
        None,
        false,
    )
    .unwrap();
    let map = MapScalar::from_arrow(&map_array as &dyn Array).unwrap();
    let shown = map.display();
    assert!(shown.contains("│ {x: 1, y: 2}"), "got: {shown}");
    assert!(shown.contains("│ {x: 3, y: 4}"), "got: {shown}");
    assert!(shown.contains("struct<x: int64, y: int64>"));
}
