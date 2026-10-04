//! The column side: what holding the buffers buys over boxing the rows.
//!
//! Every case here is paired against the thing it is meant to beat - taking
//! buffers against proving rows, a buffer read against building the value
//! that row would cost, a native write against the field's own contract on
//! the way to the same write - so a number that moves says which side moved.
//! The counts behind the pairs are pinned in `rust/tests/allocations.rs`;
//! these cases are the time those counts buy.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, Int64Array, RecordBatch, StringArray, StructArray, TimestampNanosecondArray,
};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields};
use criterion::{BatchSize, Criterion};
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Selector, Serie, SerieValue,
    SortOptions, StructType, TimeUnit, Timezone, UnionMode,
};

/// Rows per measured column. The smoke corpus keeps `cargo test
/// --all-targets` under a second in a debug build.
const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

/// Legs per order row in the struct-of-serie case.
const LEGS: usize = 4;

/// One non-null 64-bit column field.
fn price_field() -> Field {
    Field::new("price", DataType::Int64, false)
}

/// `ROWS` 64-bit values as Arrow buffers, freshly allocated so the column
/// taken over them holds them alone and its first edit lands in place.
fn price_array() -> ArrayRef {
    Arc::new(Int64Array::from(
        (0..ROWS)
            .map(|index| i64::try_from(index).expect("a row count fits i64"))
            .collect::<Vec<_>>(),
    ))
}

/// `ROWS` 64-bit values as native rows.
fn price_rows() -> Vec<Scalar> {
    (0..ROWS)
        .map(|index| Scalar::from(i64::try_from(index).expect("a row count fits i64")))
        .collect()
}

/// The column over a fresh [`price_array`], holding its buffers alone.
fn price_column() -> Serie {
    Serie::from_arrow_array(Some(&price_field()), price_array(), ArrowCastOptions::new())
        .expect("an int64 column")
}

/// One non-null record root of two leaf columns.
fn quotes_root() -> Field {
    let fields = StructType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("symbol", DataType::utf8(), false),
    ])
    .expect("two named children");
    Field::new("row", DataType::from(fields), false)
}

/// `ROWS` records under [`quotes_root`], as Arrow buffers.
fn quotes_array() -> ArrayRef {
    let fields: Fields = vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]
    .into();
    let ids: ArrayRef = price_array();
    let symbols: ArrayRef = Arc::new(StringArray::from(vec!["AAPL"; ROWS]));
    Arc::new(StructArray::try_new(fields, vec![ids, symbols], None).expect("two equal columns"))
}

/// One non-null record root of an identifier and a serie of int64 legs.
fn orders_root() -> Field {
    let fields = StructType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new(
            "legs",
            DataType::serie(Field::new("item", DataType::Int64, false)),
            false,
        ),
    ])
    .expect("two named children");
    Field::new("order", DataType::from(fields), false)
}

/// One order row: an identifier and [`LEGS`] legs.
fn order_row(id: i64) -> Scalar {
    Scalar::from_sequence([
        Scalar::from(id),
        Scalar::from_sequence((0..LEGS).map(|leg| Scalar::from(id + leg as i64))),
    ])
}

/// `ROWS` orders under [`orders_root`], laid out from proven rows.
fn orders_column() -> Serie {
    Serie::from_scalars(
        orders_root(),
        (0..ROWS).map(|index| order_row(i64::try_from(index).expect("a row count fits i64"))),
    )
    .expect("a record column of series")
}

/// One nullable run-end field of UTF-8 states over `int32` run ends.
fn states_field() -> Field {
    Field::new(
        "state",
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::utf8(), true),
        )
        .expect("an int32 run-end width"),
        true,
    )
}

/// `ROWS` states in runs of eight, laid out from proven rows.
fn states_column() -> Serie {
    Serie::from_scalars(
        states_field(),
        (0..ROWS).map(|index| Scalar::from(if index / 8 % 2 == 0 { "open" } else { "closed" })),
    )
    .expect("a run-end column")
}

/// `ROWS` 64-bit values in a fixed shuffle - a linear congruential walk of
/// the row positions - as a column holding its buffer alone, so a sort has
/// work to do and an in-place sort rewrites a buffer it owns.
fn shuffled_column() -> Serie {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let values: Vec<i64> = (0..ROWS)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            i64::try_from(state >> 33).expect("31 bits fit i64")
        })
        .collect();
    Serie::from_arrow_array(
        Some(&price_field()),
        Arc::new(Int64Array::from(values)),
        ArrowCastOptions::new(),
    )
    .expect("an int64 column")
}

/// `ROWS` venue codes cycling through sixteen, as a UTF-8 column.
fn venues_column() -> Serie {
    const VENUES: [&str; 16] = [
        "XNAS", "XNYS", "XPAR", "XLON", "XETR", "XAMS", "XBRU", "XMIL", "XSWX", "XTKS", "XHKG",
        "XASX", "XTSE", "XMAD", "XSTO", "XCSE",
    ];
    let values: Vec<&str> = (0..ROWS)
        .map(|index| VENUES[(index * 7 + index / 3) % VENUES.len()])
        .collect();
    Serie::from_arrow_array(
        Some(&Field::new("venue", DataType::utf8(), false)),
        Arc::new(StringArray::from(values)),
        ArrowCastOptions::new(),
    )
    .expect("a utf8 column")
}

/// `column` held as eight chunks of equal length sliced out of it.
fn eight_chunks(column: &Serie) -> ChunkedSerie {
    let size = ROWS / 8;
    ChunkedSerie::from_series(
        None,
        (0..8).map(|chunk| column.slice(chunk * size, size).expect("a chunk")),
        ArrowCastOptions::new(),
    )
    .expect("chunks under one field")
}

/// Rows per venue run in [`ticks_column`].
const VENUE_RUN: usize = 100;

/// One non-null record root of a venue, a count and a nanosecond UTC
/// instant.
fn ticks_root() -> Field {
    let fields = StructType::from_fields([
        Field::new("venue", DataType::utf8(), false),
        Field::new("count", DataType::Int64, false),
        Field::new(
            "ts",
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            },
            false,
        ),
    ])
    .expect("three named children");
    Field::new("tick", DataType::from(fields), false)
}

/// The venues a tick names, in key order.
const TICK_VENUES: [&str; 4] = ["XLON", "XNAS", "XNYS", "XPAR"];

/// `ROWS` ticks under [`ticks_root`] as Arrow buffers: the venue changing
/// every [`VENUE_RUN`] rows, out of key order, one second between instants
/// from a fifteen-minute boundary, so a quarter-hour bucket holds 900 rows.
fn ticks_column() -> Serie {
    ticks_landed(cycling_venue)
}

/// The venue of row `index` in [`ticks_column`]: XNAS, XNYS, XPAR, XLON, then
/// XNAS again, each over [`VENUE_RUN`] rows.
fn cycling_venue(index: usize) -> &'static str {
    TICK_VENUES[(index / VENUE_RUN + 1) % TICK_VENUES.len()]
}

/// The same ticks with each venue over a quarter of the rows, in key order.
fn sorted_ticks_column() -> Serie {
    ticks_landed(|index| TICK_VENUES[index * TICK_VENUES.len() / ROWS])
}

/// [`ticks_batch`] landed under [`ticks_root`].
fn ticks_landed(venue_of: impl Fn(usize) -> &'static str) -> Serie {
    Serie::from_arrow_batch(
        Some(&ticks_root()),
        &ticks_batch(venue_of),
        ArrowCastOptions::new(),
    )
    .expect("a record column")
}

/// `ROWS` ticks under [`ticks_root`] as one batch, row `index` at venue
/// `venue_of(index)`.
fn ticks_batch(venue_of: impl Fn(usize) -> &'static str) -> RecordBatch {
    // 2024-01-01T00:00:00Z.
    const MIDNIGHT_NS: i64 = 1_704_067_200_000_000_000;
    let schema = ticks_root().into_arrow_schema().expect("a record schema");
    let instants = TimestampNanosecondArray::from(
        (0..ROWS)
            .map(|index| {
                MIDNIGHT_NS + i64::try_from(index).expect("a row count fits i64") * 1_000_000_000
            })
            .collect::<Vec<_>>(),
    )
    .with_data_type(schema.field(2).data_type().clone());
    let venues: Vec<&str> = (0..ROWS).map(venue_of).collect();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(venues)),
            price_array(),
            Arc::new(instants),
        ],
    )
    .expect("three equal columns")
}

/// [`ticks_root`]'s three children, then the order a tick fills - `order:
/// struct<venue, mic, ts>`, nullable - and, where `wide` asks, a serie of
/// two legs a row, a dictionary tag and forty-two int64 columns: four
/// children against forty-eight.
fn order_ticks_root(wide: bool) -> Field {
    let mut fields = ticks_root().fields().to_vec();
    let instant = fields[2].dtype().clone();
    fields.push(
        DataType::from(
            StructType::from_fields([
                DataType::utf8().required_field("venue"),
                DataType::Mic.required_field("mic"),
                instant.required_field("ts"),
            ])
            .expect("three named children"),
        )
        .nullable_field("order"),
    );
    if wide {
        fields.push(
            "serie<struct<px: float64, qty: int64>>"
                .parse::<DataType>()
                .expect("a serie of legs")
                .required_field("legs"),
        );
        fields.push(
            DataType::dictionary(DataType::Int32, DataType::utf8())
                .expect("a dictionary")
                .required_field("tag"),
        );
        fields.extend((0..42).map(|index| DataType::Int64.required_field(format!("c{index:02}"))));
    }
    Field::new(
        "tick",
        DataType::from(StructType::from_fields(fields).expect("named children")),
        false,
    )
}

/// [`ticks_column`]'s rows under [`order_ticks_root`]: the order's venue
/// and instant the tick's, its code the venue, never absent.
fn order_ticks_column(wide: bool) -> Serie {
    use arrow_array::types::Int32Type;
    use arrow_array::{DictionaryArray, Float64Array, Int32Array, ListArray};
    use arrow_buffer::OffsetBuffer;

    let root = order_ticks_root(wide);
    let schema = root.clone().into_arrow_schema().expect("a record schema");
    let mut columns: Vec<ArrayRef> = ticks_batch(cycling_venue).columns().to_vec();
    let ArrowDataType::Struct(order) = schema.field(3).data_type().clone() else {
        panic!("an order record projects to a struct")
    };
    columns.push(Arc::new(
        StructArray::try_new(
            order,
            vec![
                Arc::clone(&columns[0]),
                Arc::clone(&columns[0]),
                Arc::clone(&columns[2]),
            ],
            None,
        )
        .expect("an order record"),
    ));
    if wide {
        let ArrowDataType::List(item) = schema.field(4).data_type().clone() else {
            panic!("a serie of legs projects to a list")
        };
        let ArrowDataType::Struct(leg) = item.data_type().clone() else {
            panic!("a leg projects to a struct")
        };
        let legs = StructArray::try_new(
            leg,
            vec![
                Arc::new(Float64Array::from(
                    (0..2 * ROWS).map(|index| index as f64).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(
                    (0..2 * ROWS)
                        .map(|index| i64::try_from(index).expect("a leg count fits i64"))
                        .collect::<Vec<_>>(),
                )),
            ],
            None,
        )
        .expect("legs");
        columns.push(Arc::new(
            ListArray::try_new(
                item,
                OffsetBuffer::from_lengths(std::iter::repeat_n(2, ROWS)),
                Arc::new(legs),
                None,
            )
            .expect("two legs a row"),
        ));
        columns.push(Arc::new(
            DictionaryArray::<Int32Type>::try_new(
                Int32Array::from(
                    (0..ROWS)
                        .map(|index| i32::from(index % 2 == 1))
                        .collect::<Vec<_>>(),
                ),
                Arc::new(StringArray::from(vec!["bid", "ask"])),
            )
            .expect("tags"),
        ));
        columns.extend((0..42).map(|_| price_array()));
    }
    let batch = RecordBatch::try_new(schema, columns).expect("equal columns");
    Serie::from_arrow_batch(Some(&root), &batch, ArrowCastOptions::new()).expect("a record column")
}

/// `ROWS` orders of a registered venue code, a side and a count: the venue
/// changing every [`VENUE_RUN`] rows, the side every half run.
fn coded_ticks_column() -> Serie {
    use arrow_array::UInt8Array;

    let root = Field::new(
        "order",
        DataType::from(
            StructType::from_fields([
                DataType::Mic.required_field("venue"),
                DataType::Side.required_field("side"),
                DataType::Int64.required_field("count"),
            ])
            .expect("three named children"),
        ),
        false,
    );
    let ArrowDataType::Struct(fields) = root
        .clone()
        .into_arrow_field()
        .expect("a projection")
        .data_type()
        .clone()
    else {
        panic!("a record projects to a struct")
    };
    let records = StructArray::try_new(
        fields,
        vec![
            Arc::new(StringArray::from(
                (0..ROWS).map(cycling_venue).collect::<Vec<_>>(),
            )),
            Arc::new(UInt8Array::from(
                (0..ROWS)
                    .map(|index| 1 + u8::from(index / (VENUE_RUN / 2) % 2 == 1))
                    .collect::<Vec<_>>(),
            )),
            price_array(),
        ],
        None,
    )
    .expect("an order record");
    Serie::from_arrow_array(Some(&root), Arc::new(records), ArrowCastOptions::new())
        .expect("a record column")
}

/// One nullable union of an identifier and a symbol, in `mode`.
fn quote_field(mode: UnionMode) -> Field {
    Field::new(
        "quote",
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
}

/// One union row: the member's type id and its payload.
fn quote(index: usize) -> Scalar {
    if index.is_multiple_of(2) {
        Scalar::from_sequence([
            Scalar::from(0_i64),
            Scalar::from(i64::try_from(index).expect("a row count fits i64")),
        ])
    } else {
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")])
    }
}

/// `ROWS` alternating union rows in `mode`, laid out from proven rows.
fn quotes_union(mode: UnionMode) -> Serie {
    Serie::from_scalars(quote_field(mode), (0..ROWS).map(quote)).expect("a union column")
}

pub(crate) fn serie_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("serie");

    // In. Taking buffers under a field is one cast plan compiled from the
    // array's layout - the identity here, sharing the buffers - and a null
    // count on the validity words; taking values is the field's contract once
    // per row and one layout. The gap is what a column buys a reader that
    // already has Arrow.
    let array = price_array();
    group.bench_function("from_arrow_array", |bencher| {
        bencher.iter(|| {
            Serie::from_arrow_array(
                Some(&price_field()),
                ArrayRef::clone(black_box(&array)),
                ArrowCastOptions::new(),
            )
            .expect("an int64 column")
        });
    });
    let rows = price_rows();
    group.bench_function("from_scalars", |bencher| {
        bencher.iter(|| {
            Serie::from_scalars(price_field(), black_box(&rows).iter().cloned())
                .expect("proven int64 rows")
        });
    });

    // Random access: one read off the values buffer, against building the
    // value that row would otherwise cost.
    let serie = price_column();
    let leaf = serie.as_int64().expect("an int64 column");
    group.bench_function("value", |bencher| {
        bencher.iter(|| black_box(leaf).value(black_box(ROWS / 2)));
    });
    group.bench_function("scalar", |bencher| {
        bencher.iter(|| {
            black_box(&serie)
                .scalar(black_box(ROWS / 2))
                .expect("a stored row")
        });
    });

    // Growing a column that holds its buffers alone: one native value into
    // the values buffer, and one value through the field's contract on the
    // way to the same buffer. The column is rebuilt outside the timing so
    // every push is the amortized one and never the first edit's copy.
    group.bench_function("push_value", |bencher| {
        bencher.iter_batched(
            price_column,
            |mut growing| {
                growing
                    .get_int64_mut()
                    .expect("an int64 column")
                    .push_value(Some(1))
                    .expect("a present row");
                growing.len()
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("push", |bencher| {
        bencher.iter_batched(
            price_column,
            |mut growing| {
                growing.push(Scalar::from(1_i64)).expect("one int64 row");
                growing.len()
            },
            BatchSize::LargeInput,
        );
    });

    // Rewriting the middle slot: one present value over one present value
    // is one buffer write, against the amortized append `push` measured.
    group.bench_function("set_middle", |bencher| {
        bencher.iter_batched(
            price_column,
            |mut writing| {
                writing
                    .set(ROWS / 2, Scalar::from(1_i64))
                    .expect("one slot");
                writing.len()
            },
            BatchSize::LargeInput,
        );
    });

    // Out. A record root lends the buffers it holds as one table rather
    // than gathering rows.
    let records = Serie::from_arrow_array(
        Some(&quotes_root()),
        quotes_array(),
        ArrowCastOptions::new(),
    )
    .expect("a record column");
    group.bench_function("into_arrow_batch", |bencher| {
        bencher.iter(|| {
            black_box(&records)
                .into_arrow_batch()
                .expect("a record root is a table")
        });
    });

    // A nested write: one record whose child is a serie, pushed down through
    // the record's write into the serie's cut and the items under it.
    let next = i64::try_from(ROWS).expect("a row count fits i64");
    group.bench_function("struct_of_list_push", |bencher| {
        bencher.iter_batched(
            orders_column,
            |mut orders| {
                orders.push(order_row(next)).expect("one order row");
                SerieValue::len(orders.as_struct().expect("a record column"))
            },
            BatchSize::LargeInput,
        );
    });

    // An encoded push is a cut over the last run: the value it holds
    // lengthens it by one run-end write, and another value appends one run.
    group.bench_function("run_end_push", |bencher| {
        bencher.iter_batched(
            states_column,
            |mut states| {
                states
                    .push(Scalar::from("closed"))
                    .expect("the last run's value");
                states.push(Scalar::from("open")).expect("a run more");
                states.len()
            },
            BatchSize::LargeInput,
        );
    });

    // Ordering. A primitive column sorts its native slice, a string column
    // goes through the row format; `as_sorted` on a column held alone sorts
    // where it stands, so its distance from `into_sorted` is the take it
    // does not do; `partition_by` over the sorted keys cuts zero-copy slices.
    for (name, column) in [("int64", shuffled_column()), ("utf8", venues_column())] {
        group.bench_function(format!("sort_indices/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&column)
                    .sort_indices(SortOptions::default())
                    .expect("an order")
            });
        });
        group.bench_function(format!("into_sorted/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&column)
                    .into_sorted(SortOptions::default())
                    .expect("sorted")
            });
        });
        group.bench_function(format!("as_sorted/{name}"), |bencher| {
            bencher.iter_batched(
                || column.clone().into_reversed(),
                |mut held| {
                    held.as_sorted(SortOptions::default())
                        .expect("sorted in place");
                    held.len()
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("into_unique/{name}"), |bencher| {
            bencher.iter(|| black_box(&column).into_unique().expect("unique"));
        });
        let keys = venues_column()
            .into_sorted(SortOptions::default())
            .expect("sorted keys");
        group.bench_function(format!("partition_by/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&column)
                    .partition_by(black_box(&keys))
                    .expect("sixteen groups")
            });
        });
        // The reads that answer without building a serie: one comparator
        // pass, one hash set.
        group.bench_function(format!("is_sorted/{name}"), |bencher| {
            bencher.iter(|| black_box(&column).is_sorted(SortOptions::default()));
        });
        group.bench_function(format!("is_unique/{name}"), |bencher| {
            bencher.iter(|| black_box(&column).is_unique());
        });
        group.bench_function(format!("unique_count/{name}"), |bencher| {
            bencher.iter(|| black_box(&column).unique_count());
        });

        // A window reads through the serie and writes through it: its read
        // is the serie of its rows and the verb, its in-place sort the
        // native slice where it stands.
        let window_len = ROWS - 2;
        group.bench_function(format!("window_is_sorted/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&column)
                    .window(1, window_len)
                    .expect("a window")
                    .is_sorted(SortOptions::default())
            });
        });
        group.bench_function(format!("window_into_sorted/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&column)
                    .window(1, window_len)
                    .expect("a window")
                    .into_sorted(SortOptions::default())
                    .expect("sorted")
            });
        });
        group.bench_function(format!("window_as_sorted/{name}"), |bencher| {
            bencher.iter_batched(
                || column.clone().into_reversed(),
                |mut held| {
                    held.window_mut(1, window_len)
                        .expect("a window")
                        .as_sorted(SortOptions::default())
                        .expect("sorted in place")
                        .len()
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("window_as_reversed/{name}"), |bencher| {
            bencher.iter_batched(
                || column.clone().into_reversed(),
                |mut held| {
                    held.window_mut(1, window_len)
                        .expect("a window")
                        .as_reversed()
                        .expect("reversed in place")
                        .len()
                },
                BatchSize::LargeInput,
            );
        });

        // The same column held as eight chunks: what a chunk answers alone
        // stays per chunk, what needs every row together is the one join
        // and the serie's own verb.
        let chunked = eight_chunks(&column);
        let chunked_keys = eight_chunks(&keys);
        group.bench_function(format!("chunked_is_sorted/{name}"), |bencher| {
            bencher.iter(|| black_box(&chunked).is_sorted(SortOptions::default()));
        });
        group.bench_function(format!("chunked_sort_indices/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&chunked)
                    .sort_indices(SortOptions::default())
                    .expect("an order")
            });
        });
        group.bench_function(format!("chunked_into_sorted/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&chunked)
                    .into_sorted(SortOptions::default())
                    .expect("sorted")
            });
        });
        group.bench_function(format!("chunked_unique_count/{name}"), |bencher| {
            bencher.iter(|| black_box(&chunked).unique_count());
        });
        group.bench_function(format!("chunked_into_unique/{name}"), |bencher| {
            bencher.iter(|| black_box(&chunked).into_unique().expect("unique"));
        });
        group.bench_function(format!("chunked_into_reversed/{name}"), |bencher| {
            bencher.iter(|| black_box(&chunked).into_reversed());
        });
        let mask = Serie::new(
            (0..ROWS)
                .map(|index| Scalar::from(index % 3 != 0))
                .collect::<Vec<_>>(),
        );
        group.bench_function(format!("chunked_into_filtered/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&chunked)
                    .into_filtered(black_box(&mask))
                    .expect("filtered")
            });
        });
        group.bench_function(format!("chunked_partition_by/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&chunked)
                    .partition_by(black_box(&keys))
                    .expect("sixteen groups")
            });
        });
        group.bench_function(format!("chunked_partition_by_chunked/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&chunked)
                    .partition_by_chunked(black_box(&chunked_keys))
                    .expect("sixteen groups")
            });
        });
        group.bench_function(format!("chunked_as_sorted/{name}"), |bencher| {
            bencher.iter_batched(
                || chunked.clone(),
                |mut held| {
                    held.as_sorted(SortOptions::default())
                        .expect("sorted")
                        .len()
                },
                BatchSize::LargeInput,
            );
        });
    }

    // Windows by key: one bind, one key record, one comparator and one
    // bitmap a call, then one key per window walked. A column key is the
    // venue's own landed buffers, compared where they lie; a period key
    // evaluates its term over the instant column through the row tier, so
    // its distance from the column key is that tier's time. Windowing a
    // window keys the window's rows where they stand, its key cell sliced to
    // it; a chunked column keys each chunk and joins nothing.
    let ticks = ticks_column();
    let venue: Selector = "venue".parse().expect("a column key");
    let bucket: Selector = "minutes(ts, 15)".parse().expect("a period key");
    group.bench_function("window_by/column_key", |bencher| {
        bencher.iter(|| {
            black_box(&ticks)
                .window_by(&venue, false)
                .expect("windows")
                .iter()
                .map(black_box)
                .count()
        });
    });
    group.bench_function("window_by/epoch_key", |bencher| {
        bencher.iter(|| {
            black_box(&ticks)
                .window_by(&bucket, false)
                .expect("windows")
                .iter()
                .map(black_box)
                .count()
        });
    });
    group.bench_function("window_by/through_window", |bencher| {
        bencher.iter(|| {
            black_box(&ticks)
                .window(1, ROWS - 2)
                .expect("a window")
                .window_by(&venue, false)
                .expect("windows")
                .iter()
                .map(black_box)
                .count()
        });
    });
    let chunked_ticks = eight_chunks(&ticks);
    group.bench_function("chunked/window_by", |bencher| {
        bencher.iter(|| {
            black_box(&chunked_ticks)
                .window_by(&venue, false)
                .expect("windows")
        });
    });

    // Each key once, in key order. Keys already in order cut what the
    // unsorted call cuts, the verdict read in the comparator's one pass, so
    // `sorted_in_order` should sit on `column_key`; keys out of order - the
    // venue returning every four runs - regroup their runs and take the rows
    // once into key order, which is `sorted_gather`'s distance from it. A
    // chunked column regroups its runs as zero-copy pieces and takes nothing.
    let sorted_ticks = sorted_ticks_column();
    group.bench_function("window_by/sorted_in_order", |bencher| {
        bencher.iter(|| {
            black_box(&sorted_ticks)
                .window_by(&venue, true)
                .expect("windows")
                .iter()
                .map(black_box)
                .count()
        });
    });
    group.bench_function("window_by/sorted_gather", |bencher| {
        bencher.iter(|| {
            black_box(&ticks)
                .window_by(&venue, true)
                .expect("windows")
                .iter()
                .map(black_box)
                .count()
        });
    });
    group.bench_function("chunked/window_by_sorted", |bencher| {
        bencher.iter(|| {
            black_box(&chunked_ticks)
                .window_by(&venue, true)
                .expect("windows")
        });
    });

    // A record key with a registered code: the record rung compares each
    // cell on its own, the code over its values built once and the side over
    // its buffers, never a run per row.
    let coded = coded_ticks_column();
    let venue_side: Selector = "venue, side".parse().expect("a two-cell key");
    group.bench_function("window_by/record_code_key", |bencher| {
        bencher.iter(|| {
            black_box(&coded)
                .window_by(&venue_side, false)
                .expect("windows")
                .iter()
                .map(black_box)
                .count()
        });
    });

    // A key reads the columns it names and no other, so each key costs the
    // same over four children and over forty-eight: a column and a record
    // path key as the landed cells they reach, a period term over a batch of
    // the one column it reads.
    let order_venue: Selector = "order.venue".parse().expect("a path key");
    for (width, record) in [
        (4, order_ticks_column(false)),
        (48, order_ticks_column(true)),
    ] {
        for (name, key) in [
            ("venue", &venue),
            ("order_venue", &order_venue),
            ("period", &bucket),
        ] {
            group.bench_function(format!("window_by/wide_record/{name}/{width}"), |bencher| {
                bencher.iter(|| {
                    black_box(&record)
                        .window_by(key, false)
                        .expect("windows")
                        .iter()
                        .map(black_box)
                        .count()
                });
            });
        }
    }

    // A run is a window over one shared slice: a slice shares it, a row
    // read is the window's bounds and one index, and a window's verb reads
    // the run sliced to it rather than a copy of its rows.
    let run = Serie::new(price_rows());
    group.bench_function("run/slice", |bencher| {
        bencher.iter(|| black_box(&run).slice(1, ROWS - 2).expect("a slice"));
    });
    group.bench_function("run/row_at", |bencher| {
        bencher.iter(|| {
            black_box(black_box(&run).get(black_box(ROWS / 2)));
        });
    });
    group.bench_function("run/window_is_sorted", |bencher| {
        bencher.iter(|| {
            black_box(&run)
                .window(1, ROWS - 2)
                .expect("a window")
                .is_sorted(SortOptions::default())
        });
    });

    // A union push: a sparse one splices every member over the new row, a
    // dense one lands the payload on its own member.
    for mode in [UnionMode::Sparse, UnionMode::Dense] {
        let name = format!("union_push/{}", format!("{mode:?}").to_lowercase());
        group.bench_function(name, |bencher| {
            bencher.iter_batched(
                || quotes_union(mode),
                |mut quotes| {
                    quotes.push(quote(ROWS)).expect("one union row");
                    quotes.len()
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}
