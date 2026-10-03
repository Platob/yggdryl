//! `rust/src/serie/spill.rs`: where a serie's rows live - `resident_size`,
//! `is_spilled` - and the greedy `spill` that moves them to a private,
//! unlinked file mapped back read-only, heaviest leaves first, every read
//! after it reaching the mapping.
//!
//! Every spill here is an explicit `Serie::spill` under a stated bound: this
//! harness never installs a process default, so the doors that settle a
//! column (`from_scalars` among them) stay under the 64 MiB default, which
//! no column here reaches. The auto-spill doors are `rust/tests/spill_doors.rs`'s.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchIterator};
use yggdryl::local::LocalFolder;
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie, SerieReader, SerieValue,
    SortOptions, SpillOptions, StructType, TimeUnit, Timezone, UnionMode,
};

/// The bound that spills every byte.
fn everything() -> SpillOptions {
    SpillOptions::new().with_byte_size(0)
}

/// A required record of `fields`, named `name`.
fn record(name: &str, fields: Vec<Field>) -> Field {
    DataType::from(StructType::from_fields(fields).expect("distinct names")).required_field(name)
}

/// `rows` int64 rows counting up, every seventh absent.
fn prices(rows: usize) -> Serie {
    Serie::from_scalars(
        Field::new("price", DataType::Int64, true),
        (0..rows).map(|index| {
            if index % 7 == 3 {
                Scalar::Null
            } else {
                Scalar::from(index as i64 * 3 - 11)
            }
        }),
    )
    .expect("int64 rows")
}

/// A serie of int64 items per row: row `i` holds `i % 4` items, every
/// fifth row absent.
fn baskets(rows: usize) -> Serie {
    Serie::from_scalars(
        Field::new(
            "basket",
            DataType::serie(Field::new("item", DataType::Int64, false)),
            true,
        ),
        (0..rows).map(|index| {
            if index % 5 == 4 {
                Scalar::Null
            } else {
                Scalar::from_sequence(
                    (0..index % 4).map(|item| Scalar::from((index * 10 + item) as i64)),
                )
            }
        }),
    )
    .expect("serie rows")
}

/// A record of three required children of three weights - `big: int64`,
/// `mid: int32`, `small: int8` - over `rows` rows.
fn weighted(rows: usize) -> Serie {
    let root = record(
        "weighted",
        vec![
            DataType::Int64.required_field("big"),
            DataType::Int32.required_field("mid"),
            DataType::Int8.required_field("small"),
        ],
    );
    Serie::from_scalars(
        root,
        (0..rows).map(|index| {
            Scalar::from_sequence([
                Scalar::from(index as i64),
                Scalar::from(index as i32 * 2),
                Scalar::from((index % 100) as i8),
            ])
        }),
    )
    .expect("weighted rows")
}

/// A record of `venue: utf8` and `count: int64`, the venues three in turn.
fn quotes(rows: usize) -> Serie {
    let root = record(
        "quote",
        vec![
            DataType::utf8().required_field("venue"),
            DataType::Int64.required_field("count"),
        ],
    );
    let venues = ["XNYS", "XNAS", "XLON"];
    Serie::from_scalars(
        root,
        (0..rows).map(|index| {
            Scalar::from_sequence([
                Scalar::from(venues[index % 3]),
                Scalar::from((rows - index) as i64),
            ])
        }),
    )
    .expect("quote rows")
}

/// `column` spilled under `options`, the original left as it was.
fn spilled(column: &Serie, options: &SpillOptions) -> Serie {
    let mut spilled = column.clone();
    spilled
        .spill(options)
        .unwrap_or_else(|error| panic!("{:?}: {error}", column.field().map(Field::name)));
    spilled
}

/// `after` holds `before`'s rows, read every way a caller reads them.
fn assert_same_rows(what: &str, before: &Serie, after: &Serie) {
    assert_eq!(after.len(), before.len(), "{what}");
    assert_eq!(after.null_count(), before.null_count(), "{what}");
    assert_eq!(after.field(), before.field(), "{what}");
    assert_eq!(after.rows(), before.rows(), "{what}");
    for index in 0..before.len() {
        assert_eq!(
            after.scalar(index).expect("a row in range"),
            before.scalar(index).expect("a row in range"),
            "{what}: row {index}"
        );
    }
    assert_eq!(after, before, "{what}");
    assert_eq!(
        after.into_arrow_array().map(|array| array.to_data()),
        before.into_arrow_array().map(|array| array.to_data()),
        "{what}"
    );
}

#[test]
fn a_column_built_on_the_heap_is_resident_whole_and_not_spilled() {
    let column = prices(1_024);
    assert!(column.memory_size() > 0);
    assert_eq!(column.resident_size(), column.memory_size());
    assert!(!column.is_spilled());
}

#[test]
fn an_empty_column_holds_no_byte_and_spills_nothing() {
    let empty = Serie::empty(Field::new("price", DataType::Int64, true)).expect("an empty column");
    let after = spilled(&empty, &everything());
    assert_eq!(after.memory_size(), 0);
    assert_eq!(after.resident_size(), 0);
    assert!(!after.is_spilled(), "no byte, so nothing lies in a file");
    assert!(after.is_empty());
}

#[test]
fn a_run_is_never_spilled_and_spills_nothing() {
    let run = Serie::new(vec![
        Scalar::from(1_i64),
        Scalar::Null,
        Scalar::from("XNAS"),
    ]);
    assert!(run.memory_size() > 0);
    assert_eq!(run.resident_size(), run.memory_size());
    assert!(!run.is_spilled());

    let after = spilled(&run, &everything());
    let (Some(after_run), Some(held)) = (after.as_run(), run.as_run()) else {
        panic!("a spilled run is still a run");
    };
    // Untouched: the very values it held, not a copy of them.
    assert_eq!(after_run.as_slice().as_ptr(), held.as_slice().as_ptr());
    assert_eq!(after.resident_size(), run.memory_size());
    assert!(!after.is_spilled());
}

#[test]
fn a_spill_under_a_folder_that_is_not_there_names_it_and_leaves_the_column_as_it_was()
-> yggdryl::Result<()> {
    let folder = LocalFolder::new("/nonexistent/yggdryl-serie-spill-folder")?;
    let column = prices(256);
    let mut target = column.clone();
    let error = target
        .spill(&everything().with_folder(folder.clone()))
        .expect_err("no file is created under a folder that is not there");
    assert!(matches!(error, yggdryl::Error::Io(_)), "{error:?}");
    assert!(
        error.to_string().contains(&folder.url().to_string()),
        "{error}"
    );
    assert!(!target.is_spilled());
    assert_eq!(target.resident_size(), column.resident_size());
    assert_same_rows("a refused spill", &column, &target);

    // A record refused the same way, child by child, is left resident too.
    let records = weighted(256);
    let mut held = records.clone();
    held.spill(
        &everything().with_folder(LocalFolder::new("/nonexistent/yggdryl-serie-spill-folder")?),
    )
    .expect_err("a record's children spill under the same folder");
    assert_eq!(held.resident_size(), records.resident_size());
    assert_same_rows("a refused record spill", &records, &held);
    Ok(())
}

#[test]
fn a_bound_of_zero_spills_a_flat_column_whole_and_every_read_reaches_the_mapping() {
    let column = prices(1_024);
    let after = spilled(&column, &everything());
    assert!(after.is_spilled());
    assert_eq!(after.resident_size(), 0);
    assert_eq!(
        after.memory_size(),
        column.memory_size(),
        "a spill moves bytes, never the measure"
    );
    assert_same_rows("int64", &column, &after);
    // The clone taken before the spill keeps its heap bytes.
    assert!(!column.is_spilled());
    assert_eq!(column.resident_size(), column.memory_size());

    // The leaf's own verb spills the same way and comes back as itself.
    let mut leaf = column.as_int64().expect("an int64 column").clone();
    leaf.spill(&everything()).expect("the leaf spills");
    assert!(leaf.is_spilled());
    assert_eq!(SerieValue::resident_size(&leaf), 0);
    assert_same_rows("the int64 leaf", &column, &leaf.into_serie());
}

#[test]
fn never_spills_nothing_and_a_bound_at_the_resident_bytes_spills_nothing() {
    let column = prices(1_024);
    let resident = u64::try_from(column.resident_size()).expect("a byte count");
    for options in [
        SpillOptions::new().with_byte_size(SpillOptions::NEVER),
        SpillOptions::new(),
        SpillOptions::new().with_byte_size(resident),
    ] {
        let after = spilled(&column, &options);
        assert!(!after.is_spilled(), "{options:?}");
        assert_eq!(after.resident_size(), column.memory_size(), "{options:?}");
    }
    // One byte under, and the flat column spills whole.
    let after = spilled(&column, &SpillOptions::new().with_byte_size(resident - 1));
    assert!(after.is_spilled());
    assert_eq!(after.resident_size(), 0);
}

#[test]
fn a_record_spills_its_heaviest_child_first_and_stops_under_the_bound() {
    let records = weighted(1_024);
    let size = |name: &str| records.child(name).expect("a child").memory_size();
    let (big, mid, small) = (size("big"), size("mid"), size("small"));
    assert!(big > mid && mid > small, "{big} > {mid} > {small}");
    // A record of required children holds no buffer of its own.
    assert_eq!(records.resident_size(), big + mid + small);

    // Under a bound the two lighter children fit: the heaviest alone spills.
    let bound = mid + small + 1;
    let after = spilled(&records, &SpillOptions::new().with_byte_size(bound as u64));
    let state = |serie: &Serie| {
        ["big", "mid", "small"].map(|name| serie.child(name).expect("a child").is_spilled())
    };
    assert_eq!(state(&after), [true, false, false]);
    assert_eq!(after.resident_size(), mid + small);
    assert!(after.resident_size() <= bound);
    assert!(
        !after.is_spilled(),
        "a record with resident children is not spilled"
    );
    assert_same_rows("the heaviest child spilled", &records, &after);

    // Under one only the lightest fits: the two heaviest spill, in order.
    let bound = small + 1;
    let after = spilled(&records, &SpillOptions::new().with_byte_size(bound as u64));
    assert_eq!(state(&after), [true, true, false]);
    assert_eq!(after.resident_size(), small);
    assert_same_rows("the two heaviest children spilled", &records, &after);

    // Under zero every child spills, and so the record is spilled.
    let after = spilled(&records, &everything());
    assert_eq!(state(&after), [true, true, true]);
    assert_eq!(after.resident_size(), 0);
    assert!(after.is_spilled());
    assert_same_rows("every child spilled", &records, &after);

    // A record already partly spilled spills only what the bound still asks.
    let mut partly = spilled(
        &records,
        &SpillOptions::new().with_byte_size((mid + small) as u64),
    );
    assert_eq!(state(&partly), [true, false, false]);
    partly
        .spill(&SpillOptions::new().with_byte_size(small as u64))
        .expect("the rest spills");
    assert_eq!(state(&partly), [true, true, false]);
    assert_same_rows("spilled twice", &records, &partly);
}

#[test]
fn a_nested_column_spills_its_items_first_and_whole_once_its_own_cut_passes_the_bound() {
    let lists = baskets(1_024);
    let items = lists.items().expect("the items").memory_size();
    let own = lists.memory_size() - items;
    assert!(items > own && own > 0, "{items} > {own}");

    // A bound the cut and validity fit under: the items alone spill.
    let after = spilled(&lists, &SpillOptions::new().with_byte_size(own as u64));
    assert!(after.items().expect("the items").is_spilled());
    assert_eq!(after.resident_size(), own);
    assert!(!after.is_spilled());
    assert_same_rows("the items spilled", &lists, &after);

    // A bound the cut alone passes: the whole column spills, the mapped
    // items written again into the one new file beside the cut.
    let mut whole = after.clone();
    whole
        .spill(&SpillOptions::new().with_byte_size(own as u64 - 1))
        .expect("the whole column spills");
    assert!(whole.is_spilled());
    assert_eq!(whole.resident_size(), 0);
    assert!(whole.items().expect("the items").is_spilled());
    assert_same_rows("the whole column spilled", &lists, &whole);
}

#[test]
fn a_clone_of_a_spilled_column_is_spilled() {
    let after = spilled(&prices(1_024), &everything());
    let clone = after.clone();
    assert!(clone.is_spilled());
    assert_eq!(clone.resident_size(), 0);
    assert_same_rows("a clone", &after, &clone);
    // The clone outlives the column it was taken from: the mapping is the
    // buffers' own.
    let expected = after.rows().into_owned();
    drop(after);
    assert_eq!(clone.rows().into_owned(), expected);
}

#[test]
fn a_write_brings_a_spilled_column_back_to_the_heap_and_reads_what_it_wrote() {
    let column = prices(1_024);

    let mut set = spilled(&column, &everything());
    set.set(3, Scalar::from(99_i64)).expect("a row in range");
    assert!(!set.is_spilled());
    assert!(set.resident_size() > 0);
    assert_eq!(set.resident_size(), set.memory_size());
    assert_eq!(set.scalar(3).expect("row 3"), Scalar::from(99_i64));
    for index in (0..column.len()).filter(|index| *index != 3) {
        assert_eq!(
            set.scalar(index).expect("a row"),
            column.scalar(index).expect("a row")
        );
    }

    let mut pushed = spilled(&column, &everything());
    pushed
        .push(Scalar::from(-1_i64))
        .expect("a row the field takes");
    assert!(!pushed.is_spilled());
    assert!(pushed.resident_size() > 0);
    assert_eq!(pushed.len(), column.len() + 1);
    assert_eq!(
        pushed.scalar(column.len()).expect("the pushed row"),
        Scalar::from(-1_i64)
    );
    assert_eq!(&pushed.rows()[..column.len()], &column.rows()[..]);

    // A text column, whose offsets and bytes are two buffers, the same way.
    let symbols = Serie::from_scalars(
        Field::new("symbol", DataType::utf8(), true),
        (0..256).map(|index| Scalar::from(format!("SYM{index}"))),
    )
    .expect("utf8 rows");
    let mut written = spilled(&symbols, &everything());
    assert!(written.is_spilled());
    written.set(0, Scalar::from("ZZZ")).expect("a row in range");
    assert!(!written.is_spilled());
    assert_eq!(written.scalar(0).expect("row 0"), Scalar::from("ZZZ"));
    assert_eq!(&written.rows()[1..], &symbols.rows()[1..]);
}

#[test]
fn every_write_on_a_spilled_column_answers_what_it_answers_on_the_heap_and_lands_there() {
    type Write = fn(&mut Serie, &Serie);
    let writes: [(&str, Write); 9] = [
        ("insert", |serie, _| {
            serie.insert(2, Scalar::from(5_i64)).expect("insert")
        }),
        ("remove", |serie, _| {
            serie.remove(2).expect("remove");
        }),
        ("pop", |serie, _| {
            serie.pop().expect("pop");
        }),
        ("truncate", |serie, _| serie.truncate(10).expect("truncate")),
        ("resize", |serie, _| {
            serie.resize(2_000, Scalar::Null).expect("resize")
        }),
        ("extend_from_serie", |serie, other| {
            serie.extend_from_serie(other).expect("extend")
        }),
        ("as_sorted", |serie, _| {
            serie.as_sorted(SortOptions::default()).expect("as_sorted");
        }),
        ("as_reversed", |serie, _| {
            serie.as_reversed().expect("as_reversed");
        }),
        ("as_unique", |serie, _| {
            serie.as_unique().expect("as_unique");
        }),
    ];
    let column = prices(1_024);
    let after = spilled(&column, &everything());
    for (what, write) in writes {
        let mut heap = column.clone();
        write(&mut heap, &column);
        let mut mapped = after.clone();
        write(&mut mapped, &column);
        assert!(!mapped.is_spilled(), "{what}");
        assert_eq!(mapped.resident_size(), mapped.memory_size(), "{what}");
        assert_same_rows(what, &heap, &mapped);
    }
    // The spilled column the writes were cloned from is still spilled.
    assert!(after.is_spilled());

    // A record's row write reaches every child, each back on the heap.
    let records = weighted(512);
    let row = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i32), Scalar::from(3_i8)]);
    let mut heap = records.clone();
    heap.set(1, row.clone()).expect("a row in range");
    let mut mapped = spilled(&records, &everything());
    mapped.set(1, row).expect("a row in range");
    for name in ["big", "mid", "small"] {
        assert!(!mapped.child(name).expect("a child").is_spilled(), "{name}");
    }
    assert_eq!(mapped.resident_size(), mapped.memory_size());
    assert_same_rows("a record row set", &heap, &mapped);
}

#[test]
fn a_slice_of_a_spilled_primitive_or_text_stays_spilled_while_a_slice_of_a_spilled_serie_does_not()
{
    // A primitive and a text column slice their buffers where they lie.
    let venues = quotes(1_024).child("venue").expect("a text child").clone();
    for column in [prices(1_024), venues] {
        let after = spilled(&column, &everything());
        let cut = after.slice(10, 100).expect("a slice in range");
        assert!(cut.is_spilled(), "a slice reads the same mapping");
        assert_eq!(cut.resident_size(), 0);
        assert_same_rows(
            "a flat slice",
            &column.slice(10, 100).expect("a slice"),
            &cut,
        );
    }

    // A serie's slice rebases its offsets onto the heap: the items stay
    // mapped, the cut does not, so the slice is resident in part and not
    // spilled.
    let lists = baskets(1_024);
    let after = spilled(&lists, &everything());
    assert!(after.is_spilled());
    let cut = after.slice(10, 100).expect("a slice in range");
    assert!(!cut.is_spilled());
    assert!(cut.resident_size() > 0);
    assert!(cut.items().expect("the items").is_spilled());
    assert_same_rows(
        "a serie slice",
        &lists.slice(10, 100).expect("a slice"),
        &cut,
    );
}

/// One column of every layout a serie holds, each a few rows with an
/// absent one where its field allows.
fn layouts() -> Vec<(&'static str, Serie)> {
    let build = |field: Field, rows: Vec<Scalar>| {
        let name = field.name().to_owned();
        Serie::from_scalars(field, rows).unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let union_of = |mode: UnionMode| {
        Field::new(
            "member",
            DataType::union(
                [
                    (0, Field::new("id", DataType::Int64, false)),
                    (1, Field::new("symbol", DataType::utf8(), true)),
                ],
                mode,
            )
            .expect("two members"),
            true,
        )
    };
    let member =
        |type_id: i64, payload: Scalar| Scalar::from_sequence([Scalar::from(type_id), payload]);
    let union_rows = || {
        vec![
            member(0, Scalar::from(1_i64)),
            member(1, Scalar::from("AAPL")),
            member(1, Scalar::Null),
            member(0, Scalar::from(2_i64)),
        ]
    };
    let int_item = || Field::new("item", DataType::Int64, true);
    let int_runs = |count: usize| {
        (0..count)
            .map(|index| {
                if index % 4 == 2 {
                    Scalar::Null
                } else {
                    Scalar::from_sequence([Scalar::from(index as i64), Scalar::Null])
                }
            })
            .collect::<Vec<_>>()
    };
    vec![
        (
            "int64",
            build(
                Field::new("int64", DataType::Int64, true),
                vec![Scalar::from(1_i64), Scalar::Null, Scalar::from(i64::MIN)],
            ),
        ),
        (
            "float64",
            build(
                Field::new("float64", DataType::Float64, true),
                vec![Scalar::from(1.5_f64), Scalar::Null, Scalar::from(-0.25_f64)],
            ),
        ),
        (
            "decimal128(12, 4)",
            build(
                Field::new(
                    "decimal",
                    DataType::decimal128(12, 4).expect("a decimal"),
                    true,
                ),
                vec![Scalar::from(12_i64), Scalar::Null, Scalar::from(-3_i64)],
            ),
        ),
        (
            "datetime64(us, UTC)",
            build(
                Field::new(
                    "ts",
                    DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).expect("a datetime"),
                    true,
                ),
                vec![
                    Scalar::from(1_700_000_000_000_000_i64),
                    Scalar::Null,
                    Scalar::from(0_i64),
                ],
            ),
        ),
        (
            "boolean",
            build(
                Field::new("flag", DataType::Boolean, true),
                vec![Scalar::from(true), Scalar::Null, Scalar::from(false)],
            ),
        ),
        (
            "utf8",
            build(
                Field::new("utf8", DataType::utf8(), true),
                vec![Scalar::from("alpha"), Scalar::Null, Scalar::from("")],
            ),
        ),
        (
            "large_utf8",
            build(
                Field::new("large", DataType::large_utf8(), true),
                vec![Scalar::from("alpha"), Scalar::Null, Scalar::from("omega")],
            ),
        ),
        (
            "utf8_view",
            build(
                Field::new("view", DataType::utf8_view(), true),
                vec![
                    Scalar::from("inline"),
                    Scalar::Null,
                    Scalar::from("a run longer than twelve bytes"),
                ],
            ),
        ),
        (
            "fixed_utf8(4)",
            build(
                Field::new("fixed", DataType::fixed_utf8(4).expect("a width"), true),
                vec![Scalar::from("AAPL"), Scalar::Null, Scalar::from("MSFT")],
            ),
        ),
        (
            "sized_utf8(8)",
            build(
                Field::new("sized", DataType::sized_utf8(8).expect("a maximum"), true),
                vec![Scalar::from("XNAS"), Scalar::Null, Scalar::from("XLONDON")],
            ),
        ),
        (
            "binary",
            build(
                Field::new("binary", DataType::binary(), true),
                vec![
                    Scalar::from(b"ab".to_vec()),
                    Scalar::Null,
                    Scalar::from(b"".to_vec()),
                ],
            ),
        ),
        (
            "variant",
            build(
                Field::new("payload", DataType::Variant, true),
                vec![
                    Scalar::Variant(Scalar::from(12_i64).into_variant().expect("a variant")),
                    Scalar::Null,
                    Scalar::Variant(Scalar::from("AAPL").into_variant().expect("a variant")),
                ],
            ),
        ),
        (
            "struct",
            build(
                Field::new(
                    "pair",
                    DataType::from(
                        StructType::from_fields([
                            Field::new("a", DataType::Int64, true),
                            Field::new("b", DataType::utf8(), true),
                        ])
                        .expect("two children"),
                    ),
                    true,
                ),
                vec![
                    Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("x")]),
                    Scalar::Null,
                    Scalar::from_sequence([Scalar::Null, Scalar::from("z")]),
                ],
            ),
        ),
        ("serie", baskets(9)),
        (
            "large_serie",
            build(
                Field::new("large_serie", DataType::large_serie(int_item()), true),
                int_runs(6),
            ),
        ),
        (
            "fixed_size_serie(2)",
            build(
                Field::new(
                    "fixed_serie",
                    DataType::fixed_size_serie(int_item(), 2).expect("a width"),
                    true,
                ),
                int_runs(6),
            ),
        ),
        (
            "map<utf8, int64>",
            build(
                Field::new(
                    "tags",
                    DataType::map_of(DataType::utf8(), DataType::Int64, false).expect("a map"),
                    true,
                ),
                vec![
                    Scalar::from_mapping([
                        (Scalar::from("a"), Scalar::from(1_i64)),
                        (Scalar::from("b"), Scalar::Null),
                    ])
                    .expect("a mapping"),
                    Scalar::Null,
                    Scalar::from_mapping([(Scalar::from("c"), Scalar::from(3_i64))])
                        .expect("a mapping"),
                ],
            ),
        ),
        (
            "dictionary<int32, utf8>",
            build(
                Field::new(
                    "venue",
                    DataType::dictionary(DataType::Int32, DataType::utf8()).expect("a dictionary"),
                    true,
                ),
                vec![
                    Scalar::from("XNAS"),
                    Scalar::Null,
                    Scalar::from("XNYS"),
                    Scalar::from("XNAS"),
                ],
            ),
        ),
        (
            "run_end<int32, int64>",
            build(
                Field::new(
                    "runs",
                    DataType::run_end_encoded(
                        DataType::Int32.required_field("run_ends"),
                        Field::new("values", DataType::Int64, true),
                    )
                    .expect("a run-end encoding"),
                    true,
                ),
                vec![
                    Scalar::from(7_i64),
                    Scalar::from(7_i64),
                    Scalar::Null,
                    Scalar::from(9_i64),
                    Scalar::from(9_i64),
                ],
            ),
        ),
        (
            "sparse_union",
            build(union_of(UnionMode::Sparse), union_rows()),
        ),
        (
            "dense_union",
            build(union_of(UnionMode::Dense), union_rows()),
        ),
    ]
}

#[test]
fn every_layout_spills_under_a_bound_of_zero_and_reads_back_its_rows() {
    for (name, column) in layouts() {
        assert!(!column.is_spilled(), "{name}: built on the heap");
        assert!(column.memory_size() > 0, "{name}");
        let after = spilled(&column, &everything());
        assert!(after.is_spilled(), "{name}: spilled");
        assert_eq!(after.resident_size(), 0, "{name}");
        assert_eq!(
            after.memory_size(),
            column.memory_size(),
            "{name}: the measure is kept"
        );
        assert_same_rows(name, &column, &after);
        assert!(
            !column.is_spilled(),
            "{name}: the original keeps its heap bytes"
        );
    }
}

#[test]
fn a_null_column_holds_no_byte_so_a_spill_leaves_it_resident_and_readable() {
    let column = Serie::from_scalars(
        Field::new("nothing", DataType::Null, true),
        vec![Scalar::Null; 5],
    )
    .expect("null rows");
    assert_eq!(column.memory_size(), 0);
    let after = spilled(&column, &everything());
    assert!(!after.is_spilled());
    assert_eq!(after.resident_size(), 0);
    assert_same_rows("null", &column, &after);
}

#[cfg(unix)]
#[test]
fn a_spill_under_a_stated_folder_leaves_no_file_in_it() -> yggdryl::Result<()> {
    let dir = std::env::temp_dir().join(format!("yggdryl-serie-spill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let options = everything().with_folder(LocalFolder::new(&dir)?);

    let column = quotes(512);
    let after = spilled(&column, &options);
    assert!(after.is_spilled());
    let listed = std::fs::read_dir(&dir)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    assert!(
        listed.is_empty(),
        "the spill file is unlinked as it opens: {listed:?}"
    );
    // Unlinked, and every row still reads through the mapping.
    assert_same_rows("under a stated folder", &column, &after);

    drop(after);
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn a_chunked_serie_spills_its_heaviest_chunks_whole_first() {
    let field = Field::new("price", DataType::Int64, true);
    let chunks = [prices(1_024), prices(256), prices(8)];
    let sizes = chunks.clone().map(|chunk| chunk.memory_size());
    let chunked = ChunkedSerie::from_series(Some(&field), chunks, ArrowCastOptions::new())
        .expect("three chunks");
    assert_eq!(chunked.resident_size(), sizes.iter().sum::<usize>());
    assert!(!chunked.is_spilled());

    let state = |chunked: &ChunkedSerie| {
        [0, 1, 2].map(|index| chunked.chunk(index).expect("a chunk").is_spilled())
    };
    let mut held = chunked.clone();
    held.spill(&SpillOptions::new().with_byte_size((sizes[1] + sizes[2]) as u64))
        .expect("the heaviest chunk spills");
    assert_eq!(state(&held), [true, false, false]);
    assert_eq!(held.resident_size(), sizes[1] + sizes[2]);
    assert!(!held.is_spilled());
    assert_eq!(held.rows(), chunked.rows());

    let mut held = chunked.clone();
    held.spill(&SpillOptions::new().with_byte_size(sizes[2] as u64))
        .expect("the two heaviest chunks spill");
    assert_eq!(state(&held), [true, true, false]);
    assert_eq!(held.resident_size(), sizes[2]);

    let mut held = chunked.clone();
    held.spill(&everything()).expect("every chunk spills");
    assert_eq!(state(&held), [true, true, true]);
    assert_eq!(held.resident_size(), 0);
    assert!(held.is_spilled());
    assert_eq!(held.memory_size(), chunked.memory_size());
    assert_eq!(held.rows(), chunked.rows());
    assert_same_rows(
        "the join of spilled chunks",
        &chunked.into_serie().expect("one column"),
        &held.into_serie().expect("one column"),
    );

    // Never, and the original, stay resident.
    let mut never = chunked.clone();
    never
        .spill(&SpillOptions::new().with_byte_size(SpillOptions::NEVER))
        .expect("nothing spills");
    assert_eq!(state(&never), [false, false, false]);
    assert_eq!(state(&chunked), [false, false, false]);
}

#[test]
fn a_window_reads_where_the_rows_of_the_serie_it_views_lie() {
    let column = prices(1_024);
    let window = column.window(10, 100).expect("a window in range");
    assert!(!window.is_spilled());
    assert_eq!(window.resident_size(), window.memory_size());
    assert!(window.resident_size() > 0);

    let mut after = spilled(&column, &everything());
    let window = after.window(10, 100).expect("a window in range");
    assert!(window.is_spilled());
    assert_eq!(window.resident_size(), 0);
    assert_eq!(
        window.rows(),
        column.window(10, 100).expect("a window").rows()
    );

    let window = after.window_mut(10, 100).expect("a window in range");
    assert!(window.is_spilled());
    assert_eq!(window.resident_size(), 0);

    // A run's window is never spilled, its rows all resident.
    let run = Serie::new(vec![
        Scalar::from(1_i64),
        Scalar::from(2_i64),
        Scalar::from(3_i64),
    ]);
    let window = run.window(1, 2).expect("a window in range");
    assert!(!window.is_spilled());
    assert_eq!(window.resident_size(), window.memory_size());
}

#[test]
fn a_held_reader_spills_the_records_it_holds_and_yields_them_spilled() {
    let column = quotes(512);
    let mut reader = SerieReader::from_serie(column.clone()).expect("a held reader");
    assert_eq!(reader.resident_size(), column.resident_size());
    assert!(!reader.is_spilled());

    reader.spill(&everything()).expect("the held record spills");
    assert!(reader.is_spilled());
    assert_eq!(reader.resident_size(), 0);
    let record = reader
        .next()
        .expect("one record")
        .expect("a held record yields");
    assert!(record.is_spilled());
    assert_same_rows("a held reader's record", &column, &record);
    // Drained, it holds nothing and is not spilled.
    assert!(reader.next().is_none());
    assert_eq!(reader.resident_size(), 0);
    assert!(!reader.is_spilled());

    // A non-record column is the one child of the record it is read as.
    let mut reader = SerieReader::from_serie(prices(256)).expect("a held reader");
    reader.spill(&everything()).expect("the held record spills");
    assert!(reader.is_spilled());

    // A stream holds no landed batch between pulls: nothing to answer or
    // spill.
    let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "price",
        arrow_schema::DataType::Int64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int64Array::from((0..256).collect::<Vec<i64>>()))],
    )
    .expect("a batch");
    let mut stream = SerieReader::from_arrow_reader(
        None,
        Box::new(RecordBatchIterator::new([Ok(batch)], schema)),
        ArrowCastOptions::new(),
    )
    .expect("a stream");
    stream
        .spill(&everything())
        .expect("a stream spills nothing");
    assert_eq!(stream.resident_size(), 0);
    assert!(!stream.is_spilled());
    let landed = stream.next().expect("one batch").expect("a landed batch");
    assert!(!landed.is_spilled());
}

#[test]
fn a_spilled_column_casts_sorts_takes_and_windows_its_own_rows() {
    let column = prices(1_024);
    let after = spilled(&column, &everything());
    let wide = Field::new("price", DataType::Float64, true);
    assert_eq!(
        after
            .cast(&wide, ArrowCastOptions::new())
            .expect("a cast")
            .rows(),
        column
            .cast(&wide, ArrowCastOptions::new())
            .expect("a cast")
            .rows()
    );
    // A cast onto its own field is the column itself, still mapped.
    let same = after
        .cast(column.field().expect("a field"), ArrowCastOptions::new())
        .expect("the identity");
    assert!(same.is_spilled());

    for options in [SortOptions::default(), SortOptions::descending()] {
        assert_eq!(
            after.into_sorted(options).expect("sorted").rows(),
            column.into_sorted(options).expect("sorted").rows(),
            "{options:?}"
        );
    }
    let picks = Serie::new(vec![
        Scalar::from(1_000_u32),
        Scalar::from(3_u32),
        Scalar::from(0_u32),
    ]);
    assert_eq!(
        after.into_taken(&picks).expect("taken").rows(),
        column.into_taken(&picks).expect("taken").rows()
    );

    let records = quotes(300);
    let after = spilled(&records, &everything());
    assert!(after.is_spilled());
    for sorted in [false, true] {
        let expected = records.window_by("venue", sorted).expect("windows");
        let windows = after.window_by("venue", sorted).expect("windows");
        assert_eq!(windows.len(), expected.len(), "sorted: {sorted}");
        for ((key, window), (expected_key, expected_window)) in windows.iter().zip(expected.iter())
        {
            assert_eq!(key, expected_key, "sorted: {sorted}");
            assert_eq!(window.rows(), expected_window.rows(), "sorted: {sorted}");
        }
    }
}
