//! `rust/src/chunked_serie.rs`: many columns under one field, held apart -
//! the chunked array and the table, read across chunks as one column.

use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::types::Int8Type;
use arrow_array::{
    Array, ArrayRef, DictionaryArray, Int8Array, Int32Array, Int64Array, ListArray, RecordBatch,
    RecordBatchIterator, RecordBatchReader, StringArray, StringViewArray, StructArray,
};
use arrow_buffer::{Buffer, NullBuffer, OffsetBuffer};
use arrow_schema::{ArrowError, DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::arrow::{BatchReader, batch_reader};
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, FieldPath, Nullability, Scalar, Serie,
    SerieReader, StructType, UnionFields, UnionMode,
};

fn price() -> Field {
    Field::new("price", DataType::Int64, false)
}

fn prices() -> ChunkedSerie {
    let first: ArrayRef = Arc::new(Int64Array::from(vec![125, 126]));
    let second: ArrayRef = Arc::new(Int64Array::from(vec![127]));
    ChunkedSerie::from_arrow_arrays(Some(&price()), [first, second], ArrowCastOptions::new())
        .expect("two int64 chunks")
}

/// The options a refusal is pinned under: a present value is never nulled
/// and an absent one never repaired.
fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new()
        .with_safe(false)
        .with_nullability(Nullability::Strict)
}

/// The int64 column of `values` under `field`, straight off an Arrow array.
fn int64_column(field: &Field, values: Vec<Option<i64>>) -> Serie {
    let array: ArrayRef = Arc::new(Int64Array::from(values));
    Serie::from_arrow_array(Some(field), array, ArrowCastOptions::new()).expect("an int64 column")
}

/// The price column cut into one chunk per slice of `cuts`.
fn cut(cuts: &[&[i64]]) -> ChunkedSerie {
    ChunkedSerie::from_series(
        Some(&price()),
        cuts.iter()
            .map(|values| int64_column(&price(), values.iter().copied().map(Some).collect())),
        ArrowCastOptions::new(),
    )
    .expect("price chunks")
}

/// The price rows `values` as values.
fn price_rows(values: &[i64]) -> Vec<Scalar> {
    values.iter().copied().map(Scalar::from).collect()
}

/// The first buffer a column's Arrow array holds - the values of a leaf -
/// which is what sharing is proven on.
fn values_of(serie: &Serie) -> Buffer {
    serie
        .into_arrow_array()
        .expect("a column")
        .to_data()
        .buffers()[0]
        .clone()
}

/// Whether two columns lend the very same values buffer.
fn shares(left: &Serie, right: &Serie) -> bool {
    values_of(left).ptr_eq(&values_of(right))
}

/// A non-null record root over an identifier and a symbol.
fn quotes_root() -> Field {
    Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("symbol", DataType::utf8(), false),
            ])
            .expect("two named children"),
        ),
        false,
    )
}

/// The record column of `rows` under [`quotes_root`].
fn quotes(rows: &[(i64, &str)]) -> Serie {
    Serie::from_scalars(
        quotes_root(),
        rows.iter()
            .map(|&(id, symbol)| Scalar::from_sequence([Scalar::from(id), Scalar::from(symbol)])),
    )
    .expect("quote rows")
}

/// Three quotes as a table of two batches.
fn quote_table() -> ChunkedSerie {
    ChunkedSerie::from_series(
        None,
        [quotes(&[(1, "AAPL"), (2, "MSFT")]), quotes(&[(3, "TSLA")])],
        ArrowCastOptions::new(),
    )
    .expect("two batches under one root")
}

/// The same three quotes as rows.
fn quote_rows() -> Vec<Scalar> {
    [(1_i64, "AAPL"), (2, "MSFT"), (3, "TSLA")]
        .into_iter()
        .map(|(id, symbol)| Scalar::from_sequence([Scalar::from(id), Scalar::from(symbol)]))
        .collect()
}

/// Two quotes as one Arrow batch.
fn quote_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
    )
    .expect("two rows")
}

/// A required serie-of-int64 field, `legs`.
fn legs_field() -> Field {
    Field::new(
        "legs",
        DataType::serie(Field::new("item", DataType::Int64, false)),
        false,
    )
}

/// The legs `[1, 2]`, `[3]` in one chunk and `[4, 5]` in another.
fn legs() -> ChunkedSerie {
    let lists = |offsets: Vec<i32>, values: Vec<i64>| -> ArrayRef {
        Arc::new(ListArray::new(
            Arc::new(ArrowField::new("item", ArrowDataType::Int64, false)),
            OffsetBuffer::new(offsets.into()),
            Arc::new(Int64Array::from(values)),
            None,
        ))
    };
    ChunkedSerie::from_arrow_arrays(
        Some(&legs_field()),
        [
            lists(vec![0, 2, 3], vec![1, 2, 3]),
            lists(vec![0, 2], vec![4, 5]),
        ],
        ArrowCastOptions::new(),
    )
    .expect("two chunks of legs")
}

/// A nullable quotes root holding one absent row: no table states it.
fn absent_quotes() -> ChunkedSerie {
    let records = StructArray::new(
        vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
        ]
        .into(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
        Some(NullBuffer::from(vec![true, false])),
    );
    ChunkedSerie::from_arrow_arrays(
        Some(&quotes_root().with_nullable(true)),
        [Arc::new(records) as ArrayRef],
        ArrowCastOptions::new(),
    )
    .expect("a nullable record chunk")
}

/// The hash a value writes into the default hasher.
fn hashed(value: &impl Hash) -> u64 {
    let mut state = DefaultHasher::new();
    value.hash(&mut state);
    state.finish()
}

#[test]
fn a_run_and_no_chunk_with_no_field_are_refused_by_name() {
    let run = Serie::new(vec![Scalar::from(1_i64)]);
    let refusal = ChunkedSerie::from_serie(run.clone()).expect_err("a run names no field");
    assert!(
        refusal.to_string().contains("declares no field"),
        "{refusal}"
    );
    let refusal = ChunkedSerie::from_series(None, [run], ArrowCastOptions::new())
        .expect_err("a run names no field");
    assert!(
        refusal.to_string().contains("declares no field"),
        "{refusal}"
    );
    let refusal = ChunkedSerie::from_series(None, [], ArrowCastOptions::new())
        .expect_err("nothing names no field");
    assert!(refusal.to_string().contains("names no field"), "{refusal}");
    let refusal = ChunkedSerie::from_arrow_arrays(None, [], ArrowCastOptions::new())
        .expect_err("nothing names no field");
    assert!(refusal.to_string().contains("names no field"), "{refusal}");
}

#[test]
fn a_chunk_laid_out_unlike_the_first_is_refused_naming_it() {
    let first: ArrayRef = Arc::new(Int64Array::from(vec![1]));
    let second: ArrayRef = Arc::new(Int32Array::from(vec![2]));
    let refusal = ChunkedSerie::from_arrow_arrays(None, [first, second], ArrowCastOptions::new())
        .expect_err("two layouts are not one chunked array");
    assert!(refusal.to_string().contains("chunk 1"), "{refusal}");
}

#[test]
fn rows_are_found_by_their_chunk_and_the_identity_is_the_rows() {
    let prices = prices();
    assert_eq!(
        (prices.len(), prices.num_chunks(), prices.null_count()),
        (3, 2, 0)
    );
    assert_eq!(prices.scalar(0).unwrap(), Scalar::from(125_i64));
    assert_eq!(prices.scalar(2).unwrap(), Scalar::from(127_i64));
    assert!(prices.scalar(3).is_err());
    assert_eq!(prices.get(3), None);
    assert_eq!(prices.rows().len(), 3);
    let joined = prices.into_serie().unwrap();
    assert_eq!(joined.len(), 3);
    assert!(prices == joined);
    assert!(joined == prices);
    assert_eq!(prices, ChunkedSerie::from_serie(joined).unwrap());
    // Rendered as the column of its rows is.
    assert_eq!(prices.to_string(), prices.into_serie().unwrap().to_string());
    assert!(prices.to_string().starts_with("price["), "{prices}");
}

#[test]
fn a_window_keeps_the_chunks_it_reaches() {
    let prices = prices();
    let window = prices.slice(1, 2).unwrap();
    assert_eq!((window.len(), window.num_chunks()), (2, 2));
    assert_eq!(
        window.rows(),
        vec![Scalar::from(126_i64), Scalar::from(127_i64)]
    );
    let one = prices.slice(2, 1).unwrap();
    assert_eq!((one.len(), one.num_chunks()), (1, 1));
    assert_eq!(prices.slice(0, 0).unwrap().num_chunks(), 0);
    assert!(prices.slice(2, 2).is_err());
}

#[test]
fn a_table_crosses_as_its_batches_and_back() {
    let root = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("symbol", DataType::utf8(), false),
            ])
            .unwrap(),
        ),
        false,
    );
    let batch = |id: i64, symbol: &str| {
        Serie::from_scalars(
            root.clone(),
            [Scalar::from_sequence([
                Scalar::from(id),
                Scalar::from(symbol),
            ])],
        )
        .unwrap()
    };
    let table = ChunkedSerie::from_series(
        None,
        [batch(1, "AAPL"), batch(2, "MSFT")],
        ArrowCastOptions::new(),
    )
    .unwrap();
    assert_eq!((table.len(), table.num_chunks()), (2, 2));
    assert_eq!(table.field(), &root);

    // A column of the table is the child of every batch.
    let ids = table.child("id").expect("a child");
    assert_eq!(ids.num_chunks(), 2);
    assert_eq!(ids.rows(), vec![Scalar::from(1_i64), Scalar::from(2_i64)]);
    assert_eq!(table.children().len(), 2);
    assert!(table.child("venue").is_none());

    // One batch per chunk, and the stream reads back as the same chunks.
    let reader = table.into_arrow_reader().unwrap();
    assert_eq!(reader.schema().fields().len(), 2);
    let back =
        ChunkedSerie::from_arrow_reader(Some(&root), reader, ArrowCastOptions::new()).unwrap();
    assert_eq!(back.num_chunks(), 2);
    assert_eq!(back, table);

    // A held chunked column is the stream of its chunks.
    let series = SerieReader::from_chunked(table.clone()).unwrap();
    assert_eq!(series.field(), &root);
    let chunks = series.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(chunks, table.chunks().to_vec());
    let handed = SerieReader::from_chunked(table.clone())
        .unwrap()
        .into_arrow_reader();
    assert_eq!(handed.count(), 2);

    // A leaf chunked column crosses as the one column of a row root.
    let prices = prices();
    let reader = prices.into_arrow_reader().unwrap();
    assert_eq!(reader.schema().field(0).name(), "price");
    assert_eq!(
        reader.map(|batch| batch.unwrap().num_rows()).sum::<usize>(),
        3
    );
}

#[test]
fn a_cast_is_one_plan_over_every_chunk_and_a_chunk_of_another_field_is_cast_in() {
    let prices = prices();
    let wide = Field::new("price", DataType::Float64, true);
    let cast = prices.cast(&wide, ArrowCastOptions::new()).unwrap();
    assert_eq!((cast.num_chunks(), cast.field()), (2, &wide));
    assert_eq!(cast.scalar(2).unwrap(), Scalar::from(127.0_f64));
    assert!(prices.cast(&price(), ArrowCastOptions::new()).unwrap() == prices);

    let narrow: ArrayRef = Arc::new(Int32Array::from(vec![128]));
    let narrow = Serie::from_arrow_array(None, narrow, ArrowCastOptions::new()).unwrap();
    let mut grown = prices.clone();
    grown
        .push_chunk(narrow.clone(), ArrowCastOptions::new())
        .unwrap();
    assert_eq!((grown.len(), grown.num_chunks()), (4, 3));
    assert_eq!(grown.scalar(3).unwrap(), Scalar::from(128_i64));
    assert_eq!(
        grown.chunk(2).map(|chunk| chunk.field()),
        Some(Some(&price()))
    );

    let mixed = ChunkedSerie::from_series(
        Some(&price()),
        [narrow, prices.into_serie().unwrap()],
        ArrowCastOptions::new(),
    )
    .unwrap();
    assert_eq!(mixed.len(), 4);
    assert_eq!(mixed.into_arrow_arrays().len(), 2);
    assert!(
        mixed
            .into_arrow_arrays()
            .iter()
            .all(|array| array.data_type() == &arrow_schema::DataType::Int64)
    );
}

#[test]
fn an_empty_chunked_serie_names_its_field_and_its_children() {
    let root = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("id", DataType::Int64, false)]).unwrap(),
        ),
        false,
    );
    let empty = ChunkedSerie::empty(root.clone()).unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.num_chunks(), 0);
    assert_eq!(
        empty.child("id").unwrap().field(),
        &Field::new("id", DataType::Int64, false)
    );
    assert_eq!(
        empty.into_serie().unwrap(),
        Serie::empty(root.clone()).unwrap()
    );
    assert_eq!(
        empty.into_arrow_reader().unwrap().schema().fields().len(),
        1
    );
    assert_eq!(SerieReader::from_chunked(empty).unwrap().count(), 0);
}

#[test]
fn an_empty_chunked_serie_refuses_what_an_empty_column_refuses() {
    // A precision no decimal holds, built by hand past the validating
    // constructor: no column of it can exist, so no chunked one can either,
    // and the refusal is the column's own.
    let unbuildable = Field::new(
        "price",
        DataType::Decimal128 {
            precision: 0,
            scale: 0,
        },
        false,
    );
    let column = Serie::empty(unbuildable.clone()).expect_err("no column holds it");
    let refusal = ChunkedSerie::empty(unbuildable).expect_err("no chunked column holds it");
    assert_eq!(refusal.to_string(), column.to_string());
    assert!(refusal.to_string().contains("precision"), "{refusal}");

    let empty = ChunkedSerie::empty(price()).expect("an int64 field");
    assert_eq!(empty.field(), &price());
    assert_eq!(empty.field_ref().as_ref(), &price());
    assert_eq!(empty.dtype(), DataType::serie(price().with_name("item")));
    assert_eq!(
        (empty.len(), empty.num_chunks(), empty.null_count()),
        (0, 0, 0)
    );
    assert!(empty.is_empty());
    assert!(empty.chunks().is_empty());
    assert!(empty.chunk(0).is_none());
    assert!(empty.rows().is_empty());
    assert_eq!(empty.iter().next(), None);
    assert_eq!(empty.get(0), None);
    let refusal = empty.scalar(0).expect_err("no row 0");
    assert!(
        refusal
            .to_string()
            .contains("row 0 is past the end of 0 rows"),
        "{refusal}"
    );
    assert_eq!(
        empty,
        ChunkedSerie::from_series(Some(&price()), [], ArrowCastOptions::new()).unwrap()
    );
}

#[test]
fn one_held_column_is_one_shared_chunk_and_a_run_is_refused() {
    let refusal = ChunkedSerie::from_serie(Serie::new(vec![Scalar::from(1_i64)]))
        .expect_err("a run names no field");
    assert!(
        refusal.to_string().contains("declares no field"),
        "{refusal}"
    );

    let column = int64_column(&price(), vec![Some(125), Some(126)]);
    let chunked = ChunkedSerie::from_serie(column.clone()).expect("a column");
    assert_eq!(chunked.field(), &price());
    assert_eq!((chunked.len(), chunked.num_chunks()), (2, 1));
    assert_eq!(chunked.chunk(0), Some(&column));
    assert!(shares(&chunked.chunks()[0], &column));
    assert!(Arc::ptr_eq(
        chunked.field_ref(),
        column.field_ref().expect("a column")
    ));
}

#[test]
fn chunks_with_no_field_take_the_first_ones_widened_to_any_nullable_one() {
    let refusal = ChunkedSerie::from_series(None, [], ArrowCastOptions::new())
        .expect_err("nothing names no field");
    assert!(
        refusal
            .to_string()
            .contains("a chunked serie of no chunk names no field"),
        "{refusal}"
    );

    // Every chunk under the first one's field is held as it stands.
    let first = int64_column(&price(), vec![Some(1), Some(2)]);
    let second = int64_column(&price(), vec![Some(3)]);
    let held = ChunkedSerie::from_series(
        None,
        [first.clone(), second.clone()],
        ArrowCastOptions::new(),
    )
    .expect("one field");
    assert_eq!(held.field(), &price());
    assert!(shares(&held.chunks()[0], &first));
    assert!(shares(&held.chunks()[1], &second));

    // One nullable chunk makes the field nullable, and the required chunk
    // is the same buffers relabelled under it.
    let nullable = price().with_nullable(true);
    let absent = int64_column(&nullable, vec![Some(3), None]);
    let widened = ChunkedSerie::from_series(
        None,
        [first.clone(), absent.clone()],
        ArrowCastOptions::new(),
    )
    .expect("nullability widens");
    assert_eq!(widened.field(), &nullable);
    assert!(
        widened
            .chunks()
            .iter()
            .all(|chunk| chunk.field() == Some(&nullable))
    );
    assert!(shares(&widened.chunks()[0], &first));
    assert!(shares(&widened.chunks()[1], &absent));
    assert_eq!(
        widened.rows(),
        vec![
            Scalar::from(1_i64),
            Scalar::from(2_i64),
            Scalar::from(3_i64),
            Scalar::Null
        ]
    );

    // The name is the first chunk's: an equal layout under another name is
    // relabelled into it, and another datatype is refused naming the chunk,
    // because nothing picks which of two datatypes the rows are.
    let cost = int64_column(&Field::new("cost", DataType::Int64, false), vec![Some(4)]);
    let narrow = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int32, false)),
        Arc::new(Int32Array::from(vec![5])),
        ArrowCastOptions::new(),
    )
    .expect("an int32 column");
    let landed =
        ChunkedSerie::from_series(None, [first.clone(), cost.clone()], ArrowCastOptions::new())
            .expect("every chunk lands under price");
    assert_eq!(landed.field(), &price());
    assert!(
        landed
            .chunks()
            .iter()
            .all(|chunk| chunk.field() == Some(&price()))
    );
    assert!(shares(&landed.chunks()[1], &cost));
    assert_eq!(landed.rows(), price_rows(&[1, 2, 4]));
    let refusal = ChunkedSerie::from_series(
        None,
        [first.clone(), narrow.clone()],
        ArrowCastOptions::new(),
    )
    .expect_err("two datatypes are not one column in pieces");
    let shown = refusal.to_string();
    assert!(shown.contains("chunk 1 of \"price\" is int32"), "{shown}");
    assert!(shown.contains("first chunk int64"), "{shown}");

    // A declared field is what casts: the same chunks land under it.
    let cast = ChunkedSerie::from_series(
        Some(&price()),
        [first, cost, narrow.clone()],
        ArrowCastOptions::new(),
    )
    .expect("every chunk lands under price");
    assert!(!shares(&cast.chunks()[2], &narrow));
    assert_eq!(cast.chunks()[2].as_int64().expect("int64").values(), &[5]);
    assert_eq!(cast.rows(), price_rows(&[1, 2, 4, 5]));
}

#[test]
fn a_chunk_the_field_cannot_hold_is_refused() {
    let text = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::utf8(), false)),
        Arc::new(StringArray::from(vec!["AAPL"])),
        ArrowCastOptions::new(),
    )
    .expect("a utf8 column");
    let refusal = ChunkedSerie::from_series(
        Some(&price()),
        [int64_column(&price(), vec![Some(1)]), text],
        strict(),
    )
    .expect_err("AAPL is no int64");
    let shown = refusal.to_string();
    assert!(shown.contains("$.price"), "names the column: {shown}");
    assert!(shown.contains("AAPL"), "names the value: {shown}");

    // A run in any position is refused before a chunk is cast.
    let refusal = ChunkedSerie::from_series(
        Some(&price()),
        [
            int64_column(&price(), vec![Some(1)]),
            Serie::new(vec![Scalar::from(2_i64)]),
        ],
        ArrowCastOptions::new(),
    )
    .expect_err("a run names no field");
    assert!(
        refusal.to_string().contains("declares no field"),
        "{refusal}"
    );

    // A record is no price whatever its rows.
    let refusal = ChunkedSerie::from_series(
        Some(&price()),
        [quotes(&[(1, "AAPL")])],
        ArrowCastOptions::new(),
    )
    .expect_err("a record is no int64");
    assert!(refusal.to_string().contains("Int64"), "{refusal}");
}

#[test]
fn chunks_under_a_declared_field_are_held_or_cast_into_it() {
    let nullable = price().with_nullable(true);
    let own = int64_column(&nullable, vec![Some(1), None]);
    let required = int64_column(&price(), vec![Some(2)]);
    let chunked = ChunkedSerie::from_series(
        Some(&nullable),
        [own.clone(), required.clone()],
        ArrowCastOptions::new(),
    )
    .expect("both fit the declared field");
    assert_eq!(chunked.field(), &nullable);
    assert_eq!(chunked.num_chunks(), 2);
    assert!(shares(&chunked.chunks()[0], &own));
    assert!(shares(&chunked.chunks()[1], &required));
    assert_eq!(chunked.chunks()[1].field(), Some(&nullable));
    assert_eq!(chunked.null_count(), 1);

    // A declared float field casts every int64 chunk into it.
    let wide = Field::new("price", DataType::Float64, false);
    let cast = ChunkedSerie::from_series(
        Some(&wide),
        [required, int64_column(&price(), vec![Some(3)])],
        ArrowCastOptions::new(),
    )
    .expect("int64 widens to float64");
    assert_eq!(cast.field(), &wide);
    assert_eq!(
        cast.rows(),
        vec![Scalar::from(2.0_f64), Scalar::from(3.0_f64)]
    );

    // No chunk under a declared field is the empty chunked serie of it.
    let empty =
        ChunkedSerie::from_series(Some(&wide), [], ArrowCastOptions::new()).expect("a field alone");
    assert_eq!((empty.field(), empty.num_chunks()), (&wide, 0));
}

#[test]
fn arrow_arrays_land_by_one_plan_per_run_of_one_layout() {
    let mixed = || -> [ArrayRef; 2] {
        [
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(Int32Array::from(vec![2])),
        ]
    };
    // A declared field casts every array into it, whatever its layout.
    let cast = ChunkedSerie::from_arrow_arrays(Some(&price()), mixed(), ArrowCastOptions::new())
        .expect("int64 and int32 both land as int64");
    assert!(
        cast.chunks()
            .iter()
            .all(|chunk| chunk.as_int64().is_some() && chunk.field() == Some(&price()))
    );
    assert_eq!(cast.rows(), price_rows(&[1, 2]));

    // With no field the arrays are one datatype in pieces, and another
    // datatype is refused naming the chunk rather than cast into the first.
    let refusal = ChunkedSerie::from_arrow_arrays(None, mixed(), ArrowCastOptions::new())
        .expect_err("two datatypes are not one column in pieces");
    let shown = refusal.to_string();
    assert!(
        shown.contains("chunk 1 of \"item\" is int32, and the first chunk int64"),
        "{shown}"
    );

    // With no field the arrays are the `item` column of their own layout,
    // nullable exactly where one of them holds an absent row.
    let present = ChunkedSerie::from_arrow_arrays(
        None,
        [
            Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(Int64Array::from(vec![3])),
        ],
        ArrowCastOptions::new(),
    )
    .expect("two int64 arrays");
    assert_eq!(present.field(), &Field::new("item", DataType::Int64, false));
    let absent = ChunkedSerie::from_arrow_arrays(
        None,
        [
            Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(Int32Array::from(vec![None])),
        ],
        ArrowCastOptions::new(),
    )
    .expect("two int32 arrays");
    assert_eq!(absent.field(), &Field::new("item", DataType::Int32, true));
    assert_eq!(
        absent.rows(),
        vec![Scalar::from(1_i32), Scalar::from(2_i32), Scalar::Null]
    );

    // The field's own layout shares every buffer.
    let array = Int64Array::from(vec![125, 126]);
    let values = array.values().as_ptr();
    let shared =
        ChunkedSerie::from_arrow_arrays(Some(&price()), [Arc::new(array) as ArrayRef], strict())
            .expect("an exact layout");
    assert_eq!(
        shared.chunks()[0]
            .as_int64()
            .expect("int64")
            .values()
            .as_ptr(),
        values
    );

    // A foreign layout is cast, every array by the one plan.
    let cast = ChunkedSerie::from_arrow_arrays(
        Some(&price()),
        [
            Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(Int32Array::from(vec![3])),
        ],
        strict(),
    )
    .expect("int32 widens to int64");
    assert_eq!((cast.field(), cast.num_chunks()), (&price(), 2));
    assert!(
        cast.chunks()
            .iter()
            .all(|chunk| chunk.as_int64().is_some() && chunk.field() == Some(&price()))
    );
    assert_eq!(cast.rows(), price_rows(&[1, 2, 3]));

    // No array under a declared field is its empty chunked serie.
    let empty = ChunkedSerie::from_arrow_arrays(Some(&price()), [], ArrowCastOptions::new())
        .expect("a field alone");
    assert_eq!((empty.field(), empty.num_chunks()), (&price(), 0));
}

#[test]
fn an_array_value_the_field_refuses_is_refused_or_nulled_as_the_options_say() {
    let text = || -> ArrayRef { Arc::new(StringArray::from(vec!["125", "AAPL"])) };
    let nullable = price().with_nullable(true);
    let refusal = ChunkedSerie::from_arrow_arrays(Some(&nullable), [text()], strict())
        .expect_err("AAPL is no int64");
    assert!(refusal.to_string().contains("AAPL"), "{refusal}");

    let nulled =
        ChunkedSerie::from_arrow_arrays(Some(&nullable), [text()], ArrowCastOptions::new())
            .expect("a present value no reading takes is nulled");
    assert_eq!(nulled.rows(), vec![Scalar::from(125_i64), Scalar::Null]);
    assert_eq!(nulled.null_count(), 1);
}

#[test]
fn a_stream_is_one_chunk_per_batch_and_none_is_joined() {
    // A batch that fails fails the whole drain.
    let batch = quote_batch();
    let broken: BatchReader = Box::new(RecordBatchIterator::new(
        vec![
            Ok(batch.clone()),
            Err(ArrowError::ComputeError("the second batch broke".into())),
        ],
        batch.schema(),
    ));
    let refusal = ChunkedSerie::from_arrow_reader(None, broken, ArrowCastOptions::new())
        .expect_err("a failing batch");
    assert!(
        refusal.to_string().contains("the second batch broke"),
        "{refusal}"
    );

    let table = ChunkedSerie::from_arrow_reader(
        None,
        batch_reader(batch.schema(), vec![batch.clone(); 3]),
        ArrowCastOptions::new(),
    )
    .expect("three batches");
    assert_eq!(table.field(), &quotes_root());
    assert_eq!((table.len(), table.num_chunks()), (6, 3));
    assert!(table.chunks().iter().all(|chunk| chunk.len() == 2));
    let two = &quote_rows()[..2];
    assert_eq!(table.rows(), [two, two, two].concat());

    // A declared root casts every batch into it by one plan.
    let wide = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Float64, false),
                Field::new("symbol", DataType::utf8(), false),
            ])
            .expect("two named children"),
        ),
        false,
    );
    let cast = ChunkedSerie::from_arrow_reader(
        Some(&wide),
        batch_reader(batch.schema(), vec![batch.clone(); 2]),
        ArrowCastOptions::new(),
    )
    .expect("int64 widens to float64");
    assert_eq!((cast.field(), cast.num_chunks()), (&wide, 2));
    assert_eq!(
        cast.scalar(3).expect("row 3"),
        Scalar::from_sequence([Scalar::from(2.0_f64), Scalar::from("MSFT")])
    );

    // The reader door is the same chunks.
    let series = SerieReader::from_arrow_reader(
        None,
        batch_reader(batch.schema(), vec![batch.clone(); 3]),
        ArrowCastOptions::new(),
    )
    .expect("the reader names its root");
    let drained = ChunkedSerie::from_serie_reader(series).expect("three batches");
    assert_eq!(drained.num_chunks(), 3);
    assert_eq!(drained.chunks(), table.chunks());

    // An empty stream is the empty chunked serie of its root.
    let empty = ChunkedSerie::from_arrow_reader(
        None,
        batch_reader(batch.schema(), []),
        ArrowCastOptions::new(),
    )
    .expect("no batch");
    assert!(empty.is_empty());
    assert_eq!((empty.field(), empty.num_chunks()), (&quotes_root(), 0));
}

#[test]
fn rows_are_found_across_chunk_boundaries_and_past_empty_chunks() {
    let nullable = price().with_nullable(true);
    let empty = Serie::empty(nullable.clone()).expect("an empty column");
    let chunked = ChunkedSerie::from_series(
        None,
        [
            int64_column(&nullable, vec![Some(1), None]),
            empty.clone(),
            int64_column(&nullable, vec![Some(3)]),
        ],
        ArrowCastOptions::new(),
    )
    .expect("three chunks");
    assert_eq!(
        (
            chunked.len(),
            chunked.num_chunks(),
            chunked.null_count(),
            chunked.is_empty()
        ),
        (3, 3, 1, false)
    );
    let expected = vec![Scalar::from(1_i64), Scalar::Null, Scalar::from(3_i64)];
    for (index, row) in expected.iter().enumerate() {
        assert_eq!(&chunked.scalar(index).expect("in range"), row);
        assert_eq!(chunked.get(index).as_ref(), Some(row));
        assert_eq!(
            chunked.is_null(index).expect("in range"),
            row == &Scalar::Null
        );
    }
    assert_eq!(chunked.rows(), expected);
    assert_eq!(chunked.iter().collect::<Vec<_>>(), expected);
    assert_eq!((&chunked).into_iter().collect::<Vec<_>>(), expected);

    // The walk counts what is left, whatever the chunks hold.
    let mut rows = chunked.iter();
    assert_eq!(rows.size_hint(), (3, Some(3)));
    rows.next();
    assert_eq!(rows.size_hint(), (2, Some(2)));
    assert_eq!(rows.by_ref().count(), 2);
    assert_eq!(rows.next(), None);

    // Empty chunks at either end hide no row either.
    let padded = ChunkedSerie::from_series(
        None,
        [
            empty.clone(),
            int64_column(&nullable, vec![Some(7)]),
            empty.clone(),
        ],
        ArrowCastOptions::new(),
    )
    .expect("three chunks");
    assert_eq!((padded.len(), padded.num_chunks()), (1, 3));
    assert_eq!(padded.scalar(0).expect("row 0"), Scalar::from(7_i64));
    assert_eq!(padded.get(1), None);
}

#[test]
fn a_row_past_the_end_is_refused_naming_the_serie_and_both_counts() {
    let prices = prices();
    for refusal in [
        prices.scalar(3).expect_err("no row 3"),
        prices.is_null(3).expect_err("no row 3"),
    ] {
        let shown = refusal.to_string();
        assert!(shown.contains("price"), "{shown}");
        assert!(shown.contains("row 3 is past the end of 3 rows"), "{shown}");
    }
    assert_eq!(prices.get(3), None);
    assert_eq!(prices.get(usize::MAX), None);
}

#[test]
fn a_window_slices_the_chunks_at_its_edges_and_keeps_the_rest_whole() {
    let prices = cut(&[&[1, 2, 3], &[4, 5], &[6]]);
    for (offset, length) in [(5, 2), (7, 0), (usize::MAX, 2)] {
        let refusal = prices
            .slice(offset, length)
            .expect_err("the window reaches past the end");
        assert!(refusal.to_string().contains("price"), "{refusal}");
    }
    let refusal = prices.slice(5, 2).expect_err("past the end");
    assert!(refusal.to_string().contains("6 rows"), "{refusal}");

    // The whole is every chunk, shared.
    let whole = prices.slice(0, 6).expect("the whole");
    assert_eq!(whole.num_chunks(), 3);
    for (window, chunk) in whole.chunks().iter().zip(prices.chunks()) {
        assert!(shares(window, chunk));
    }
    assert_eq!(whole, prices);

    // Inside one chunk: that chunk, sliced over the same buffer.
    let inside = prices.slice(1, 1).expect("one row");
    assert_eq!((inside.len(), inside.num_chunks()), (1, 1));
    assert_eq!(inside.rows(), price_rows(&[2]));
    assert_eq!(
        inside.chunks()[0]
            .as_int64()
            .expect("int64")
            .values()
            .as_ptr(),
        prices.chunks()[0].as_int64().expect("int64").values()[1..].as_ptr()
    );

    // Across two chunks: the two, each cut at its edge.
    let across = prices.slice(2, 2).expect("two rows");
    assert_eq!((across.len(), across.num_chunks()), (2, 2));
    assert_eq!(across.rows(), price_rows(&[3, 4]));

    // From a chunk boundary: the chunks from there, whole.
    let boundary = prices.slice(3, 3).expect("the last three rows");
    assert_eq!((boundary.len(), boundary.num_chunks()), (3, 2));
    assert!(shares(&boundary.chunks()[0], &prices.chunks()[1]));
    assert!(shares(&boundary.chunks()[1], &prices.chunks()[2]));
    assert_eq!(boundary.rows(), price_rows(&[4, 5, 6]));

    // A zero-length window anywhere, the end included, holds no chunk.
    for offset in [0, 3, 6] {
        let none = prices.slice(offset, 0).expect("an empty window");
        assert_eq!((none.len(), none.num_chunks()), (0, 0));
        assert_eq!(none.field(), &price());
    }
}

#[test]
fn a_records_children_are_the_same_child_of_every_chunk() {
    let table = quote_table();
    assert!(table.child("venue").is_none());
    assert!(table.child_at(2).is_none());
    assert!(table.items().is_none());
    assert!(table.get_child_by_path(&FieldPath::root()).is_none());
    assert!(
        table
            .get_child_by_path(&FieldPath::from_str("id.value").unwrap())
            .is_none()
    );

    let ids = table.child("id").expect("a child");
    assert_eq!(ids.field(), &Field::new("id", DataType::Int64, false));
    assert_eq!(ids.num_chunks(), 2);
    assert_eq!(ids.rows(), price_rows(&[1, 2, 3]));
    for (column, chunk) in ids.chunks().iter().zip(table.chunks()) {
        assert!(shares(column, chunk.child("id").expect("an id child")));
    }
    let symbols = table.child_at(1).expect("the second child");
    assert_eq!(symbols.field().name(), "symbol");
    assert_eq!(
        symbols.rows(),
        vec![
            Scalar::from("AAPL"),
            Scalar::from("MSFT"),
            Scalar::from("TSLA")
        ]
    );
    assert_eq!(
        table
            .get_child_by_path(&FieldPath::from_str("symbol").unwrap())
            .expect("a path to a child"),
        symbols
    );
    let children = table.children();
    assert_eq!(
        children
            .iter()
            .map(|child| child.field().name())
            .collect::<Vec<_>>(),
        ["id", "symbol"]
    );

    // A leaf has no child and no items.
    let prices = prices();
    assert!(prices.child("price").is_none());
    assert!(prices.children().is_empty());
    assert!(prices.items().is_none());
}

#[test]
fn a_serie_columns_items_are_the_items_of_every_chunk() {
    let legs = legs();
    assert_eq!((legs.len(), legs.num_chunks()), (3, 2));
    assert!(legs.child("item").is_none());
    assert!(legs.children().is_empty());

    let items = legs.items().expect("a serie column has items");
    assert_eq!(items.field(), &Field::new("item", DataType::Int64, false));
    assert_eq!(items.num_chunks(), 2);
    assert_eq!(items.rows(), price_rows(&[1, 2, 3, 4, 5]));
    for (column, chunk) in items.chunks().iter().zip(legs.chunks()) {
        assert!(shares(column, chunk.items().expect("items")));
    }
    assert_eq!(
        legs.scalar(2).expect("the third legs"),
        Scalar::from_sequence([Scalar::from(4_i64), Scalar::from(5_i64)])
    );
}

#[test]
fn an_empty_chunked_serie_answers_its_children_from_its_field() {
    let table = ChunkedSerie::empty(quotes_root()).expect("a record field");
    let ids = table.child("id").expect("the id field");
    assert_eq!(
        (ids.field(), ids.num_chunks()),
        (&Field::new("id", DataType::Int64, false), 0)
    );
    assert_eq!(
        table.child_at(1).expect("the symbol field").field(),
        &Field::new("symbol", DataType::utf8(), false)
    );
    assert_eq!(table.children().len(), 2);
    assert!(table.child("venue").is_none());
    assert!(table.items().is_none());
    assert_eq!(
        table
            .get_child_by_path(&FieldPath::from_str("symbol").unwrap())
            .expect("a path to the symbol field")
            .field()
            .name(),
        "symbol"
    );

    let legs = ChunkedSerie::empty(legs_field()).expect("a serie field");
    let items = legs.items().expect("the item field");
    assert_eq!(
        (items.field(), items.num_chunks()),
        (&Field::new("item", DataType::Int64, false), 0)
    );
    assert!(legs.children().is_empty());
    assert!(ChunkedSerie::empty(price()).unwrap().children().is_empty());
}

#[test]
fn a_pushed_chunk_is_held_or_cast_and_a_refused_one_changes_nothing() {
    let mut prices = prices();
    let refusal = prices
        .push_chunk(
            Serie::new(vec![Scalar::from(1_i64)]),
            ArrowCastOptions::new(),
        )
        .expect_err("a run names no field");
    assert!(
        refusal.to_string().contains("declares no field"),
        "{refusal}"
    );
    let text = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::utf8(), false)),
        Arc::new(StringArray::from(vec!["AAPL"])),
        ArrowCastOptions::new(),
    )
    .expect("a utf8 column");
    let refusal = prices
        .push_chunk(text, strict())
        .expect_err("AAPL is no int64");
    assert!(refusal.to_string().contains("$.price"), "{refusal}");
    assert_eq!((prices.len(), prices.num_chunks()), (3, 2));
    assert!(prices.scalar(3).is_err());

    // A column under the field is appended as it stands.
    let own = int64_column(&price(), vec![Some(128)]);
    prices
        .push_chunk(own.clone(), ArrowCastOptions::new())
        .expect("a column under the field");
    assert_eq!((prices.len(), prices.num_chunks()), (4, 3));
    assert!(shares(&prices.chunks()[2], &own));
    assert_eq!(prices.scalar(3).expect("row 3"), Scalar::from(128_i64));

    // Any other column is cast into it.
    let wide = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Float64, false)),
        Arc::new(arrow_array::Float64Array::from(vec![129.0])),
        ArrowCastOptions::new(),
    )
    .expect("a float64 column");
    prices
        .push_chunk(wide, ArrowCastOptions::new())
        .expect("float64 narrows to int64");
    assert_eq!((prices.len(), prices.num_chunks()), (5, 4));
    assert_eq!(prices.chunks()[3].field(), Some(&price()));
    assert_eq!(prices.scalar(4).expect("row 4"), Scalar::from(129_i64));
}

#[test]
fn joining_is_one_column_of_the_rows_and_one_chunk_joins_to_itself() {
    let empty = ChunkedSerie::empty(price()).expect("a field");
    let joined = empty.into_serie().expect("no chunk");
    assert_eq!(joined, Serie::empty(price()).unwrap());
    assert_eq!(joined.field(), Some(&price()));

    let column = int64_column(&price(), vec![Some(1), Some(2)]);
    let one = ChunkedSerie::from_serie(column.clone()).expect("one chunk");
    assert!(shares(&one.into_serie().expect("one chunk"), &column));

    let many = cut(&[&[1, 2], &[], &[3]]);
    let joined = many.into_serie().expect("three chunks");
    assert_eq!(joined.field(), Some(&price()));
    assert_eq!(joined.len(), 3);
    assert_eq!(joined.rows().into_owned(), many.rows());
    assert!(joined.as_int64().is_some());

    let table = quote_table();
    let joined = table.into_serie().expect("two batches");
    assert_eq!(joined.field(), Some(&quotes_root()));
    assert_eq!(joined.rows().into_owned(), quote_rows());
}

#[test]
fn a_cast_onto_its_own_field_is_a_clone_and_a_refused_one_touches_no_chunk() {
    let prices = prices();
    let same = prices
        .cast(&price(), ArrowCastOptions::new())
        .expect("an identity");
    for (cast, chunk) in same.chunks().iter().zip(prices.chunks()) {
        assert!(shares(cast, chunk));
    }

    let wide = Field::new("price", DataType::Float64, true);
    let cast = prices
        .cast(&wide, ArrowCastOptions::new())
        .expect("int64 widens to float64");
    assert_eq!((cast.field(), cast.num_chunks()), (&wide, 2));
    assert!(
        cast.chunks()
            .iter()
            .all(|chunk| chunk.as_float64().is_some() && chunk.field() == Some(&wide))
    );
    assert_eq!(
        cast.rows(),
        vec![
            Scalar::from(125.0_f64),
            Scalar::from(126.0_f64),
            Scalar::from(127.0_f64)
        ]
    );

    // The two fields alone decide a refused cast, so it is refused whether
    // or not a chunk is there to touch.
    let refusal = prices
        .cast(&quotes_root(), ArrowCastOptions::new())
        .expect_err("an int64 is no record");
    let empty = ChunkedSerie::empty(price())
        .expect("a field")
        .cast(&quotes_root(), ArrowCastOptions::new())
        .expect_err("an int64 is no record, chunk or none");
    assert_eq!(refusal.to_string(), empty.to_string());
    let empty = ChunkedSerie::empty(price())
        .expect("a field")
        .cast(&wide, ArrowCastOptions::new())
        .expect("no chunk casts");
    assert_eq!((empty.field(), empty.num_chunks()), (&wide, 0));
}

#[test]
fn every_chunk_crosses_out_as_its_own_shared_array() {
    let prices = prices();
    let arrays = prices.into_arrow_arrays();
    assert_eq!(arrays.len(), 2);
    for (array, chunk) in arrays.iter().zip(prices.chunks()) {
        assert_eq!(array.data_type(), &ArrowDataType::Int64);
        assert_eq!(array.len(), chunk.len());
        assert!(array.to_data().buffers()[0].ptr_eq(&values_of(chunk)));
    }
    assert!(
        ChunkedSerie::empty(price())
            .unwrap()
            .into_arrow_arrays()
            .is_empty()
    );
}

#[test]
fn a_table_is_one_batch_per_chunk_under_one_schema() {
    let refusal = absent_quotes()
        .into_arrow_reader()
        .err()
        .expect("a table states no absent row");
    assert!(refusal.to_string().contains("1 absent rows"), "{refusal}");

    let table = quote_table();
    let reader = table.into_arrow_reader().expect("every row present");
    let schema = reader.schema();
    assert_eq!(
        schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        ["id", "symbol"]
    );
    let batches = reader
        .collect::<Result<Vec<_>, _>>()
        .expect("every batch reads");
    assert_eq!(
        batches
            .iter()
            .map(RecordBatch::num_rows)
            .collect::<Vec<_>>(),
        [2, 1]
    );
    assert!(batches.iter().all(|batch| batch.schema() == schema));

    // A leaf's chunks are each the one column of a row root.
    let reader = prices().into_arrow_reader().expect("a leaf");
    let schema = reader.schema();
    assert_eq!(schema.fields().len(), 1);
    assert_eq!(schema.field(0).name(), "price");
    assert_eq!(
        reader
            .map(|batch| batch.expect("a batch").num_rows())
            .collect::<Vec<_>>(),
        [2, 1]
    );

    // No chunk is a reader of no batch that still states its schema.
    for (field, columns) in [(quotes_root(), 2), (price(), 1)] {
        let reader = ChunkedSerie::empty(field)
            .expect("a field")
            .into_arrow_reader()
            .expect("no chunk");
        assert_eq!(reader.schema().fields().len(), columns);
        assert_eq!(reader.count(), 0);
    }
}

#[test]
fn the_identity_is_the_rows_however_they_are_cut() {
    let two = cut(&[&[1, 2], &[3]]);
    let other = cut(&[&[1], &[], &[2, 3]]);
    assert_eq!(two, other);
    assert_eq!(two.cmp(&other), Ordering::Equal);
    assert_eq!(hashed(&two), hashed(&other));

    // And the column of the same rows is the same value, both ways round.
    let joined = int64_column(&price(), vec![Some(1), Some(2), Some(3)]);
    assert!(two == joined);
    assert!(joined == two);
    assert_eq!(hashed(&two), hashed(&joined));
    assert!(two != int64_column(&price(), vec![Some(1), Some(2)]));

    // Ordered row by row, then by length.
    assert!(two < cut(&[&[1, 2], &[4]]));
    assert!(two > cut(&[&[1, 2]]));
    assert!(two < cut(&[&[1, 2, 3, 0]]));
    assert_eq!(
        two.partial_cmp(&cut(&[&[0, 9, 9]])),
        Some(Ordering::Greater)
    );
    assert_ne!(two, cut(&[&[1, 2], &[4]]));
}

#[test]
fn it_renders_as_the_column_of_its_rows_and_debugs_its_shape() {
    let prices = prices();
    let joined = prices.into_serie().expect("two chunks");
    assert_eq!(prices.to_string(), joined.to_string());
    let empty = ChunkedSerie::empty(price()).expect("a field");
    assert_eq!(
        empty.to_string(),
        Serie::empty(price()).unwrap().to_string()
    );
    assert_eq!(empty.to_string(), "price[]");

    let shown = format!("{:?}", cut(&[&[1], &[], &[2, 3]]));
    assert!(shown.starts_with("ChunkedSerie {"), "{shown}");
    for part in ["field:", "price", "chunks: 3", "len: 3", "nulls: 0"] {
        assert!(shown.contains(part), "{part} in {shown}");
    }
}

#[test]
fn a_clone_shares_every_chunk() {
    let prices = prices();
    let clone = prices.clone();
    assert_eq!(clone, prices);
    assert!(Arc::ptr_eq(clone.field_ref(), prices.field_ref()));
    for (left, right) in clone.chunks().iter().zip(prices.chunks()) {
        assert!(shares(left, right));
    }
}

#[test]
fn a_chunked_serie_orders_against_a_column_by_the_rows_without_a_join() {
    let prices = prices();
    let joined = prices.into_serie().unwrap();
    let mut higher = joined.clone();
    higher.set(2, Scalar::from(128_i64)).unwrap();
    assert_eq!(prices.partial_cmp(&joined), Some(std::cmp::Ordering::Equal));
    assert_eq!(joined.partial_cmp(&prices), Some(std::cmp::Ordering::Equal));
    assert!(prices < higher);
    assert!(higher > prices);
    assert!(prices <= joined);
    assert!(prices >= joined);
    let shorter = prices.slice(0, 2).unwrap();
    assert!(shorter < prices);
    assert!(joined > shorter);
}

#[test]
fn a_window_over_many_chunks_finds_its_edges_by_search() {
    // Eight one-row chunks: a window reaching several keeps each whole,
    // slices only the two at its edges, and one past the end is refused.
    let chunks = (0..8_i64)
        .map(|value| Serie::from_scalars(price(), [Scalar::from(value)]).unwrap())
        .collect::<Vec<_>>();
    let eight = ChunkedSerie::from_series(Some(&price()), chunks, ArrowCastOptions::new()).unwrap();
    let window = eight.slice(2, 5).unwrap();
    assert_eq!((window.len(), window.num_chunks()), (5, 5));
    assert_eq!(
        window.rows(),
        (2..7_i64).map(Scalar::from).collect::<Vec<_>>()
    );
    // Every chunk the window keeps whole is the chunk itself: the very same
    // values buffer, not a copy of equal rows.
    for (kept, own) in window.chunks().iter().zip(&eight.chunks()[2..7]) {
        assert!(shares(kept, own));
    }
    assert_eq!(eight.slice(7, 1).unwrap().rows(), vec![Scalar::from(7_i64)]);
    assert_eq!(eight.slice(8, 0).unwrap().num_chunks(), 0);
    assert!(eight.slice(8, 1).is_err());

    // Two-row chunks: a window from the middle of one to the middle of
    // another slices both edges and keeps the one between.
    let chunks = (0..4_i64)
        .map(|value| {
            Serie::from_scalars(
                price(),
                [Scalar::from(value * 2), Scalar::from(value * 2 + 1)],
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let four = ChunkedSerie::from_series(Some(&price()), chunks, ArrowCastOptions::new()).unwrap();
    let window = four.slice(1, 4).unwrap();
    assert_eq!(
        window.chunks().iter().map(Serie::len).collect::<Vec<_>>(),
        vec![1, 2, 1]
    );
    assert_eq!(
        window.rows(),
        (1..5_i64).map(Scalar::from).collect::<Vec<_>>()
    );
}

#[test]
fn chunks_of_one_datatype_name_their_items_as_they_like() {
    // Arrow says a list's item name is no part of its datatype: a reader
    // names it `item` or `element`, and both are one datatype in pieces.
    let list = |item: &str, values: Vec<i64>| -> ArrayRef {
        let rows = i32::try_from(values.len()).expect("a short list");
        Arc::new(ListArray::new(
            Arc::new(ArrowField::new(item, ArrowDataType::Int64, false)),
            OffsetBuffer::new(vec![0, rows].into()),
            Arc::new(Int64Array::from(values)),
            None,
        ))
    };
    let chunked = ChunkedSerie::from_arrow_arrays(
        None,
        [list("item", vec![1, 2]), list("element", vec![3])],
        ArrowCastOptions::new(),
    )
    .expect("one datatype, its items named twice");
    let field = chunked.field().clone();
    assert!(
        chunked
            .chunks()
            .iter()
            .all(|chunk| chunk.field() == Some(&field))
    );
    assert_eq!(chunked.len(), 2);

    // Columns in hand read the same way: the second is relabelled.
    let columns: Vec<Serie> = [list("item", vec![1]), list("element", vec![2])]
        .into_iter()
        .map(|array| {
            Serie::from_arrow_array(None, array, ArrowCastOptions::new()).expect("a list column")
        })
        .collect();
    let relabelled = ChunkedSerie::from_series(None, columns, ArrowCastOptions::new())
        .expect("one datatype, its items named twice");
    assert_eq!(relabelled.chunks()[1].field(), Some(relabelled.field()));

    // A nested nullability is part of the datatype, so it is refused.
    let record = |nullable: bool| -> ArrayRef {
        Arc::new(StructArray::new(
            vec![Arc::new(ArrowField::new(
                "bid",
                ArrowDataType::Int64,
                nullable,
            ))]
            .into(),
            vec![Arc::new(Int64Array::from(vec![1])) as ArrayRef],
            None,
        ))
    };
    let refusal = ChunkedSerie::from_arrow_arrays(
        None,
        [record(false), record(true)],
        ArrowCastOptions::new(),
    )
    .expect_err("a required child and a nullable one are two datatypes");
    assert!(
        refusal.to_string().contains("chunk 1 of \"item\""),
        "{refusal}"
    );
}

#[test]
fn an_absent_row_behind_a_key_makes_the_field_nullable() {
    // The key is valid and the value it reaches is null: the row is absent,
    // so the column of no field is nullable and keeps it absent.
    let keyed = || -> ArrayRef {
        Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from(vec![0, 1]),
                Arc::new(StringArray::from(vec![Some("XNYS"), None])),
            )
            .expect("keys into the values"),
        )
    };
    let chunked = ChunkedSerie::from_arrow_arrays(None, [keyed()], ArrowCastOptions::new())
        .expect("one dictionary array");
    assert!(chunked.field().is_nullable());
    assert_eq!(chunked.null_count(), 1);
    assert!(chunked.is_null(1).expect("row 1"));

    let column = Serie::from_arrow_array(None, keyed(), ArrowCastOptions::new())
        .expect("the column door reads absence the same way");
    assert_eq!(column.field(), Some(chunked.field()));
}

#[test]
fn a_join_arrow_would_panic_on_is_refused_naming_the_dictionary() {
    // Arrow merges plain text vocabularies but gathers view text whole, and
    // two hundred values are past the largest int8 key.
    let venues = |from: usize| -> ArrayRef {
        let names: Vec<String> = (from..from + 100).map(|at| format!("v{at}")).collect();
        let keys: Vec<i8> = (0..100).collect();
        Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from(keys),
                Arc::new(StringViewArray::from_iter_values(names)),
            )
            .expect("keys into the values"),
        )
    };
    let chunked =
        ChunkedSerie::from_arrow_arrays(None, [venues(0), venues(100)], ArrowCastOptions::new())
            .expect("two dictionary arrays");
    let refusal = chunked
        .into_serie()
        .expect_err("no int8 key reaches 200 values");
    let shown = refusal.to_string();
    assert!(
        shown.contains("gathers 200 dictionary values at $,"),
        "{shown}"
    );
    assert!(shown.contains("int8 key (127)"), "{shown}");

    // Beneath a record the concatenation reaches the dictionary the same
    // way, and the refusal names its path.
    let record = |from: usize| -> ArrayRef {
        let venue = venues(from);
        Arc::new(StructArray::new(
            vec![Arc::new(ArrowField::new(
                "venue",
                venue.data_type().clone(),
                false,
            ))]
            .into(),
            vec![venue],
            None,
        ))
    };
    let nested =
        ChunkedSerie::from_arrow_arrays(None, [record(0), record(100)], ArrowCastOptions::new())
            .expect("two record arrays");
    let shown = nested
        .into_serie()
        .expect_err("the same vocabulary")
        .to_string();
    assert!(shown.contains("at $.venue,"), "{shown}");

    // One vocabulary shared by every chunk is never gathered twice.
    let first = venues(0);
    let shared = ChunkedSerie::from_arrow_arrays(
        None,
        [Arc::clone(&first), first.slice(0, 50)],
        ArrowCastOptions::new(),
    )
    .expect("two windows of one dictionary");
    assert_eq!(shared.into_serie().expect("one vocabulary").len(), 150);
}

#[test]
fn a_union_of_no_member_is_refused_rather_than_laid_out() {
    let memberless = Field::new(
        "leg",
        DataType::Union(
            UnionFields::from_fields([]).expect("no member"),
            UnionMode::Sparse,
        ),
        false,
    );
    let refusal = ChunkedSerie::empty(memberless.clone()).expect_err("no column of it exists");
    assert!(
        refusal
            .to_string()
            .contains("a union of no member lays out no column"),
        "{refusal}"
    );
    // The declared doors prove their field the same way when nothing
    // else does.
    assert!(ChunkedSerie::from_series(Some(&memberless), [], ArrowCastOptions::new()).is_err());
    assert!(
        ChunkedSerie::from_arrow_arrays(Some(&memberless), [], ArrowCastOptions::new()).is_err()
    );
}
