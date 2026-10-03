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
use yggdryl::expression::IntoOrderings;
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, FieldPath, Scalar, Serie, SerieReader,
    SortOptions, StructType, UnionFields, UnionMode,
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

/// The options a refusal is pinned under: a present value is never nulled.
fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
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

/// `values` cut into chunks of `cut` rows each, under the price field;
/// `None` an absent row.
fn chunked(values: &[Option<i64>], cut: usize) -> ChunkedSerie {
    let field = Field::new("price", DataType::Int64, values.iter().any(Option::is_none));
    let arrays: Vec<ArrayRef> = values
        .chunks(cut)
        .map(|chunk| Arc::new(Int64Array::from(chunk.to_vec())) as ArrayRef)
        .collect();
    ChunkedSerie::from_arrow_arrays(Some(&field), arrays, ArrowCastOptions::new())
        .expect("int64 chunks")
}

#[test]
fn is_sorted_reads_each_chunk_and_every_chunk_edge() {
    let sorted = chunked(&[Some(1), Some(2), Some(2), Some(3), None], 2);
    assert!(sorted.is_sorted(SortOptions::default()));
    assert!(!sorted.is_sorted(SortOptions::descending()));
    // Each chunk sorted, the edge not: [1, 5] then [2, 3].
    let edge = chunked(&[Some(1), Some(5), Some(2), Some(3)], 2);
    assert!(!edge.is_sorted(SortOptions::default()));
    assert!(
        edge.slice(0, 2)
            .expect("a chunk")
            .is_sorted(SortOptions::default())
    );
    // An empty chunk between two is no edge.
    let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
    let one: ArrayRef = Arc::new(Int64Array::from(vec![1]));
    let two: ArrayRef = Arc::new(Int64Array::from(vec![2]));
    let gapped =
        ChunkedSerie::from_arrow_arrays(Some(&price()), [one, empty, two], ArrowCastOptions::new())
            .expect("three chunks");
    assert!(gapped.is_sorted(SortOptions::default()));
    assert!(
        ChunkedSerie::empty(price())
            .expect("no chunk")
            .is_sorted(SortOptions::default())
    );
}

#[test]
fn sorting_merges_the_chunks_and_uniqueness_filters_each_apart() {
    let prices = chunked(&[Some(3), None, Some(1), Some(3), Some(2)], 2);
    assert!(!prices.is_unique());
    assert_eq!(prices.unique_count(), 4);
    let order = prices
        .sort_indices(SortOptions::default())
        .expect("an order");
    assert_eq!(
        order.rows().to_vec(),
        vec![
            Scalar::from(2_u32),
            Scalar::from(4_u32),
            Scalar::from(0_u32),
            Scalar::from(3_u32),
            Scalar::from(1_u32)
        ]
    );
    let sorted = prices.into_sorted(SortOptions::default()).expect("sorted");
    assert_eq!((sorted.num_chunks(), sorted.len()), (1, 5));
    assert_eq!(sorted.field(), prices.field());
    assert!(sorted.is_sorted(SortOptions::default()));
    assert_eq!(
        sorted.rows(),
        price_rows(&[1, 2, 3, 3])
            .into_iter()
            .chain([Scalar::Null])
            .collect::<Vec<_>>()
    );
    // Each chunk keeps its own first occurrences: `[3, -]`, `[1]`, `[2]`.
    let unique = prices.into_unique().expect("unique");
    assert_eq!((unique.num_chunks(), unique.len()), (3, 4));
    assert!(unique.is_unique());
    assert_eq!(
        unique.rows(),
        vec![
            Scalar::from(3_i64),
            Scalar::Null,
            Scalar::from(1_i64),
            Scalar::from(2_i64)
        ]
    );
    // The serie is as it was.
    assert_eq!(prices.num_chunks(), 3);
    let taken = prices
        .into_taken(&Serie::new(vec![Scalar::from(4_u32), Scalar::from(0_u32)]))
        .expect("taken");
    assert_eq!(taken.rows(), price_rows(&[2, 3]));
    assert!(
        prices
            .into_taken(&Serie::new(vec![Scalar::from(5_u32)]))
            .is_err()
    );
}

#[test]
fn reversing_and_filtering_keep_the_chunks_apart() {
    let prices = chunked(&[Some(1), Some(2), Some(3), None, Some(5)], 2);
    let reversed = prices.into_reversed();
    assert_eq!(reversed.num_chunks(), 3);
    assert_eq!(
        reversed.rows(),
        vec![
            Scalar::from(5_i64),
            Scalar::Null,
            Scalar::from(3_i64),
            Scalar::from(2_i64),
            Scalar::from(1_i64)
        ]
    );
    assert_eq!(reversed.chunk(0).expect("a chunk").len(), 1);
    let mask = Serie::new(vec![
        Scalar::from(true),
        Scalar::from(false),
        Scalar::Null,
        Scalar::from(true),
        Scalar::from(true),
    ]);
    let kept = prices.into_filtered(&mask).expect("filtered");
    assert_eq!(kept.num_chunks(), 3);
    assert_eq!(
        kept.rows(),
        vec![Scalar::from(1_i64), Scalar::Null, Scalar::from(5_i64)]
    );
    let refused = prices
        .into_filtered(&Serie::new(vec![Scalar::from(true)]))
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("a mask of 1 rows cannot filter the 5 rows price holds"),
        "{refused}"
    );
}

#[test]
fn partition_by_merges_each_chunk_s_groups_in_first_occurrence_order() {
    let prices = chunked(&[Some(1), Some(2), Some(3), Some(4), Some(5)], 2);
    let keys = Serie::new(vec![
        Scalar::from("a"),
        Scalar::from("b"),
        Scalar::from("b"),
        Scalar::from("a"),
        Scalar::from("c"),
    ]);
    let groups = prices.partition_by(&keys).expect("groups");
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].0, Scalar::from("a"));
    assert_eq!(groups[0].1.rows(), price_rows(&[1, 4]));
    assert_eq!(groups[0].1.num_chunks(), 2);
    assert_eq!(groups[1].0, Scalar::from("b"));
    assert_eq!(groups[1].1.rows(), price_rows(&[2, 3]));
    assert_eq!(groups[2].0, Scalar::from("c"));
    assert_eq!(groups[2].1.field(), prices.field());
    let refused = prices
        .partition_by(&Serie::new(vec![Scalar::from("a")]))
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("1 keys cannot partition the 5 rows price holds"),
        "{refused}"
    );
}

#[test]
fn memory_size_sums_the_chunks_and_the_as_writes_replace_them_in_place() {
    let mut prices = chunked(&[Some(3), Some(1), Some(2), Some(2)], 2);
    assert_eq!(
        prices.memory_size(),
        prices
            .chunks()
            .iter()
            .map(Serie::memory_size)
            .sum::<usize>()
    );
    assert!(prices.memory_size() > 0);
    prices
        .as_sorted(SortOptions::descending())
        .expect("sorted")
        .as_unique()
        .expect("unique")
        .as_reversed()
        .expect("reversed");
    assert_eq!(prices.rows(), price_rows(&[1, 2, 3]));
    assert_eq!(prices.num_chunks(), 1);
    prices
        .as_taken(&Serie::new(vec![Scalar::from(2_u32), Scalar::from(0_u32)]))
        .expect("taken")
        .as_filtered(&Serie::new(vec![Scalar::from(true), Scalar::from(false)]))
        .expect("filtered");
    assert_eq!(prices.rows(), price_rows(&[3]));
    assert_eq!(prices.field(), &price());
    // A refused write leaves the serie as it was.
    assert!(
        prices
            .as_taken(&Serie::new(vec![Scalar::from(9_u32)]))
            .is_err()
    );
    assert_eq!(prices.rows(), price_rows(&[3]));
    let mut apart = chunked(&[Some(1), Some(2), Some(3)], 2);
    apart.as_reversed().expect("reversed");
    assert_eq!(
        (apart.num_chunks(), apart.rows()),
        (2, price_rows(&[3, 2, 1]))
    );
}

/// The four orderings `SortOptions` states.
const ORDERINGS: [SortOptions; 4] = [
    SortOptions::ascending(),
    SortOptions::ascending().with_nulls_first(true),
    SortOptions::descending(),
    SortOptions::descending().with_nulls_first(true),
];

/// `rows` under `field`, one chunk per row, so every pair of neighbours is
/// a chunk edge.
fn one_row_per_chunk(field: &Field, rows: &[Scalar]) -> ChunkedSerie {
    ChunkedSerie::from_series(
        Some(field),
        rows.iter().map(|row| {
            Serie::from_scalars(field.clone(), [row.clone()]).expect("a row the field accepts")
        }),
        ArrowCastOptions::new(),
    )
    .expect("chunks under one field")
}

#[test]
fn is_sorted_judges_a_chunk_edge_exactly_as_the_joined_column_judges_it() {
    let record = Field::new(
        "q",
        DataType::from(
            StructType::from_fields([Field::new("a", DataType::Int64, true)]).expect("a child"),
        ),
        false,
    );
    let version = Field::new("v", DataType::Version, false);
    let negative_nan = f64::from_bits(f64::NAN.to_bits() | (1 << 63));
    let float = Field::new("f", DataType::Float64, true);
    let cases = [
        (
            record.clone(),
            vec![
                Scalar::from_sequence([Scalar::from(1_i64)]),
                Scalar::from_sequence([Scalar::Null]),
            ],
        ),
        (
            record,
            vec![
                Scalar::from_sequence([Scalar::Null]),
                Scalar::from_sequence([Scalar::from(1_i64)]),
            ],
        ),
        (
            version.clone(),
            ["1.9.0", "1.10.0"]
                .map(|text| version.scalar(text).expect("a version"))
                .to_vec(),
        ),
        (
            version.clone(),
            ["1.10.0", "1.9.0"]
                .map(|text| version.scalar(text).expect("a version"))
                .to_vec(),
        ),
        (
            float.clone(),
            vec![Scalar::from(1.0_f64), Scalar::Null, Scalar::from(f64::NAN)],
        ),
        (
            float,
            vec![Scalar::from(f64::NAN), Scalar::from(1.0_f64), Scalar::Null],
        ),
    ];
    for (field, rows) in cases {
        let chunked = one_row_per_chunk(&field, &rows);
        let joined = chunked.into_serie().expect("one join");
        for options in ORDERINGS {
            assert_eq!(
                chunked.is_sorted(options),
                joined.is_sorted(options),
                "{chunked}{options}"
            );
            let sorted = chunked.into_sorted(options).expect("sorted");
            let resorted = one_row_per_chunk(&field, &sorted.rows());
            assert!(resorted.is_sorted(options), "{resorted}{options}");
        }
    }
    // A chunk holding a NaN payload Arrow orders first is still the one NaN
    // its rows hold, at the edge as inside the chunk.
    let arrays: [ArrayRef; 2] = [
        Arc::new(arrow_array::Float64Array::from(vec![1.0, 2.0])),
        Arc::new(arrow_array::Float64Array::from(vec![negative_nan])),
    ];
    let floats =
        ChunkedSerie::from_arrow_arrays(None, arrays, ArrowCastOptions::new()).expect("floats");
    assert!(floats.is_sorted(SortOptions::default()));
    assert!(
        floats
            .into_serie()
            .expect("one join")
            .is_sorted(SortOptions::default())
    );
}

#[test]
fn partition_by_chunked_pairs_chunks_cut_alike_and_joins_keys_cut_otherwise() {
    let prices = chunked(&[Some(1), Some(2), Some(3), Some(4), Some(5)], 2);
    let venue = Field::new("venue", DataType::utf8(), false);
    let venues = |cuts: &[&[&str]]| {
        ChunkedSerie::from_arrow_arrays(
            Some(&venue),
            cuts.iter()
                .map(|cut| Arc::new(StringArray::from(cut.to_vec())) as ArrayRef),
            ArrowCastOptions::new(),
        )
        .expect("venue chunks")
    };
    let expected = prices
        .partition_by(
            &venues(&[&["a", "b", "b", "a", "c"]])
                .into_serie()
                .expect("one key column"),
        )
        .expect("groups");
    // Cut where the rows are cut: chunk beside chunk, no join.
    let alike = venues(&[&["a", "b"], &["b", "a"], &["c"]]);
    // Cut elsewhere: the keys joined once and cut to each chunk of rows.
    let otherwise = venues(&[&["a"], &["b", "b", "a"], &["c"]]);
    for keys in [&alike, &otherwise] {
        let groups = prices.partition_by_chunked(keys).expect("groups");
        assert_eq!(groups.len(), 3);
        for ((key, rows), (expected_key, expected_rows)) in groups.iter().zip(&expected) {
            assert_eq!(key, expected_key);
            assert_eq!(rows.rows(), expected_rows.rows());
            assert_eq!(rows.num_chunks(), expected_rows.num_chunks());
        }
        assert_eq!(groups[0].1.rows(), price_rows(&[1, 4]));
        assert_eq!(groups[0].1.num_chunks(), 2);
    }
    let refused = prices
        .partition_by_chunked(&venues(&[&["a"]]))
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("1 keys cannot partition the 5 rows price holds"),
        "{refused}"
    );
}

/// The venue column cut `[XNAS, XNAS] [XNAS, XNYS] [] [XNYS]`.
fn venue_chunks() -> ChunkedSerie {
    let field = Field::new("venue", DataType::utf8(), false);
    let arrays: [ArrayRef; 4] = [
        Arc::new(StringArray::from(vec!["XNAS", "XNAS"])),
        Arc::new(StringArray::from(vec!["XNAS", "XNYS"])),
        Arc::new(StringArray::from(Vec::<&str>::new())),
        Arc::new(StringArray::from(vec!["XNYS"])),
    ];
    ChunkedSerie::from_arrow_arrays(Some(&field), arrays, ArrowCastOptions::new())
        .expect("four utf8 chunks")
}

/// The value bytes a utf8 column lends, which a slice of it keeps.
fn text_of(serie: &Serie) -> Buffer {
    serie
        .into_arrow_array()
        .expect("a column")
        .to_data()
        .buffers()[1]
        .clone()
}

/// Each window as its key and its rows, joined or chunked alike.
fn keyed_rows(windows: Vec<(Scalar, ChunkedSerie)>) -> Vec<(Scalar, Vec<Scalar>)> {
    windows
        .into_iter()
        .map(|(key, rows)| (key, rows.rows()))
        .collect()
}

/// What the joined column's windows answer, as keys and rows.
fn joined_windows(chunked: &ChunkedSerie, by: &str, sorted: bool) -> Vec<(Scalar, Vec<Scalar>)> {
    let joined = chunked.into_serie().expect("one join");
    let windows = joined.window_by(by, sorted).expect("windows");
    windows
        .iter()
        .map(|(key, window)| (key, window.rows().into_owned()))
        .collect()
}

/// Each window as its length and the pieces it keeps.
fn shapes(windows: &[(Scalar, ChunkedSerie)]) -> Vec<(usize, usize)> {
    windows
        .iter()
        .map(|(_, rows)| (rows.len(), rows.num_chunks()))
        .collect()
}

/// The venue column cut `[XNYS, XNAS] [XNAS, XNYS]`: XNAS runs across the
/// edge, and each XNYS stands alone.
fn mixed_venue_chunks() -> ChunkedSerie {
    let field = Field::new("venue", DataType::utf8(), false);
    let arrays: [ArrayRef; 2] = [
        Arc::new(StringArray::from(vec!["XNYS", "XNAS"])),
        Arc::new(StringArray::from(vec!["XNAS", "XNYS"])),
    ];
    ChunkedSerie::from_arrow_arrays(Some(&field), arrays, ArrowCastOptions::new())
        .expect("two utf8 chunks")
}

/// The quotes `AAPL, -` and `-, AAPL`, `-` a row no table states, in two
/// chunks under the nullable root: the absent run crosses the edge.
fn absent_quote_chunks() -> ChunkedSerie {
    let records = |symbols: Vec<&str>, present: Vec<bool>| -> ArrayRef {
        Arc::new(StructArray::new(
            vec![
                ArrowField::new("id", ArrowDataType::Int64, false),
                ArrowField::new("symbol", ArrowDataType::Utf8, false),
            ]
            .into(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
                Arc::new(StringArray::from(symbols)),
            ],
            Some(NullBuffer::from(present)),
        ))
    };
    ChunkedSerie::from_arrow_arrays(
        Some(&quotes_root().with_nullable(true)),
        [
            records(vec!["AAPL", "MSFT"], vec![true, false]),
            records(vec!["TSLA", "AAPL"], vec![false, true]),
        ],
        ArrowCastOptions::new(),
    )
    .expect("two nullable record chunks")
}

#[test]
fn window_by_merges_a_run_across_a_chunk_edge() {
    let venues = venue_chunks();
    assert_eq!(venues.num_chunks(), 4);
    let windows = venues.window_by("venue", false).expect("windows");
    assert_eq!(windows.len(), 2);
    let (xnas, xnys) = (&windows[0], &windows[1]);
    // The XNAS run crosses the first edge: one window over both chunks.
    assert_eq!(xnas.0, Scalar::from_sequence([Scalar::from("XNAS")]));
    assert_eq!((xnas.1.len(), xnas.1.num_chunks()), (3, 2));
    assert_eq!(xnas.1.rows(), vec![Scalar::from("XNAS"); 3]);
    // The XNYS run crosses the empty chunk, which the slice keeps.
    assert_eq!(xnys.0, Scalar::from_sequence([Scalar::from("XNYS")]));
    assert_eq!((xnys.1.len(), xnys.1.num_chunks()), (2, 3));
    assert_eq!(xnys.1.rows(), vec![Scalar::from("XNYS"); 2]);
    // Every window is a slice of the chunks: their bytes, shared.
    assert!(text_of(&xnas.1.chunks()[0]).ptr_eq(&text_of(&venues.chunks()[0])));
    assert!(text_of(&xnas.1.chunks()[1]).ptr_eq(&text_of(&venues.chunks()[1])));
    assert!(text_of(&xnys.1.chunks()[2]).ptr_eq(&text_of(&venues.chunks()[3])));
    assert_eq!(keyed_rows(windows), joined_windows(&venues, "venue", false));

    // Keys already in order answer the same windows sorted, the empty chunk
    // the slice crosses kept.
    let sorted = venues.window_by("venue", true).expect("windows");
    assert_eq!(shapes(&sorted), vec![(3, 2), (2, 3)]);
    assert_eq!(keyed_rows(sorted), joined_windows(&venues, "venue", true));

    // One key over every chunk is one window keeping them all.
    let every = cut(&[&[1, 1], &[1], &[1, 1]]);
    let windows = every.window_by("price", false).expect("windows");
    assert_eq!(windows.len(), 1);
    assert_eq!((windows[0].1.len(), windows[0].1.num_chunks()), (5, 3));
    assert!(shares(&windows[0].1.chunks()[1], &every.chunks()[1]));
}

#[test]
fn window_by_refuses_before_any_chunk_is_read() {
    let field = Field::new("venue", DataType::utf8(), false);
    for sorted in [false, true] {
        for venues in [
            ChunkedSerie::empty(field.clone()).expect("no chunk"),
            venue_chunks(),
            mixed_venue_chunks(),
        ] {
            let refused = venues.window_by("*", sorted).unwrap_err().to_string();
            assert!(
                refused.contains(
                    "venue: expected at least one column to window by, got an empty match key"
                ),
                "{refused}"
            );
            let refused = venues.window_by("tier", sorted).unwrap_err().to_string();
            assert!(refused.contains("tier"), "{refused}");
            let refused = venues
                .window_by("unnest(venue)", sorted)
                .unwrap_err()
                .to_string();
            assert!(refused.contains("in a key"), "{refused}");
        }
        // No chunk, no window.
        let empty = ChunkedSerie::empty(field.clone()).expect("no chunk");
        assert!(
            empty
                .window_by("venue", sorted)
                .expect("windows")
                .is_empty()
        );
    }
}

#[test]
fn window_by_states_no_record_so_a_key_cell_named_as_one_is_taken() {
    // A window is its key and its rows, its place its place in the `Vec`:
    // no record names `windownum` or `rownum`, so a key cell may - where the
    // joined column's held windows, which state one, refuse it.
    let venues = venue_chunks();
    for sorted in [false, true] {
        for key in ["venue as rownum", "venue as WindowNum"] {
            let windows = venues.window_by(key, sorted).expect("windows");
            assert_eq!(windows.len(), 2, "{key}");
            let joined = venues.into_serie().expect("joined");
            let refused = joined
                .window_by(key, sorted)
                .err()
                .map(|error| error.to_string());
            assert!(
                refused
                    .as_deref()
                    .is_some_and(|refused| refused.contains("alias")),
                "{key}: {refused:?}"
            );
        }
    }
}

#[test]
fn window_by_sorted_regroups_runs_across_chunks_with_no_row_copied() {
    let venues = mixed_venue_chunks();
    let xnas = Scalar::from_sequence([Scalar::from("XNAS")]);
    let xnys = Scalar::from_sequence([Scalar::from("XNYS")]);
    // Unsorted, the XNAS run across the edge is one window between two.
    let windows = venues.window_by("venue", false).expect("windows");
    assert_eq!(shapes(&windows), vec![(1, 1), (2, 2), (1, 1)]);

    // Sorted, each key once in key order: XNAS the one edge-merged run over
    // both chunks, XNYS its two runs as two pieces, in arrival order.
    let windows = venues.window_by("venue", true).expect("windows");
    assert_eq!(
        windows
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>(),
        vec![xnas, xnys]
    );
    assert_eq!(shapes(&windows), vec![(2, 2), (2, 2)]);
    // No row copied: every piece lends the bytes of the chunk it came from.
    let (xnas_rows, xnys_rows) = (&windows[0].1, &windows[1].1);
    assert!(text_of(&xnas_rows.chunks()[0]).ptr_eq(&text_of(&venues.chunks()[0])));
    assert!(text_of(&xnas_rows.chunks()[1]).ptr_eq(&text_of(&venues.chunks()[1])));
    assert!(text_of(&xnys_rows.chunks()[0]).ptr_eq(&text_of(&venues.chunks()[0])));
    assert!(text_of(&xnys_rows.chunks()[1]).ptr_eq(&text_of(&venues.chunks()[1])));
    assert_eq!(xnys_rows.field(), venues.field());
    assert_eq!(keyed_rows(windows), joined_windows(&venues, "venue", true));

    // A descent at an edge alone regroups as well.
    let edge = cut(&[&[2, 2], &[1], &[2]]);
    let windows = edge.window_by("price", true).expect("windows");
    assert_eq!(
        keyed_rows(windows.clone()),
        vec![
            (
                Scalar::from_sequence([Scalar::from(1_i64)]),
                price_rows(&[1])
            ),
            (
                Scalar::from_sequence([Scalar::from(2_i64)]),
                price_rows(&[2, 2, 2])
            ),
        ]
    );
    assert_eq!(shapes(&windows), vec![(1, 1), (3, 2)]);
    assert!(shares(&windows[1].1.chunks()[0], &edge.chunks()[0]));
    assert!(shares(&windows[1].1.chunks()[1], &edge.chunks()[2]));
    assert_eq!(keyed_rows(windows), joined_windows(&edge, "price", true));

    // Absent keys go last, the absent run across the edge one piece set.
    let quotes = absent_quote_chunks();
    for sorted in [false, true] {
        assert_eq!(
            keyed_rows(quotes.window_by("symbol", sorted).expect("windows")),
            joined_windows(&quotes, "symbol", sorted)
        );
    }
    let windows = quotes.window_by("symbol", true).expect("windows");
    assert_eq!(
        windows
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>(),
        vec![Scalar::from_sequence([Scalar::from("AAPL")]), Scalar::Null]
    );
    assert_eq!(shapes(&windows), vec![(2, 2), (2, 2)]);
}

#[test]
fn window_by_compares_chunk_edges_in_place() {
    // A NaN on each side of an edge is one key, whatever its payload; -0.0
    // and 0.0 across the next are two, as the joined column has them.
    let payload_nan = f64::from_bits(f64::NAN.to_bits() | 1);
    let field = Field::new("px", DataType::Float64, false);
    let arrays: [ArrayRef; 3] = [
        Arc::new(arrow_array::Float64Array::from(vec![1.0, f64::NAN])),
        Arc::new(arrow_array::Float64Array::from(vec![payload_nan, -0.0])),
        Arc::new(arrow_array::Float64Array::from(vec![0.0])),
    ];
    let prices = ChunkedSerie::from_arrow_arrays(Some(&field), arrays, ArrowCastOptions::new())
        .expect("float chunks");
    let windows = prices.window_by("px", false).expect("windows");
    assert_eq!(shapes(&windows), vec![(1, 1), (2, 2), (1, 1), (1, 1)]);
    assert_eq!(keyed_rows(windows), joined_windows(&prices, "px", false));

    // Sorted, NaN orders after every number, -0.0 before 0.0, and the NaN
    // run keeps its two pieces and the key its first row states.
    let windows = prices.window_by("px", true).expect("windows");
    assert_eq!(shapes(&windows), vec![(1, 1), (1, 1), (1, 1), (2, 2)]);
    let nan = windows[3]
        .0
        .as_serie()
        .and_then(|key| key.get(0))
        .expect("a one-cell key");
    assert_eq!(
        nan.as_f64().map(f64::to_bits),
        Some(f64::NAN.to_bits()),
        "the key is the NaN its first row holds"
    );
    assert_eq!(keyed_rows(windows), joined_windows(&prices, "px", true));
}

#[test]
fn sorting_by_keys_merges_and_answers_what_the_joined_column_answers() {
    let table = ChunkedSerie::from_series(
        None,
        [
            quotes(&[(3, "MSFT"), (1, "AAPL"), (2, "MSFT")]),
            quotes(&[(1, "MSFT"), (2, "AAPL")]),
        ],
        ArrowCastOptions::new(),
    )
    .expect("two batches under one root");
    let joined = table.into_serie().expect("joined");
    for by in ["symbol, id desc", "id, symbol desc", "id * -1, symbol"] {
        let order = table.sort_indices_by(by).expect("an order");
        assert_eq!(
            order.rows(),
            joined.sort_indices_by(by).expect("an order").rows(),
            "{by}"
        );
        let sorted = table.into_sort_by(by).expect("sorted");
        assert_eq!((sorted.num_chunks(), sorted.len()), (1, 5), "{by}");
        assert_eq!(
            without_order(sorted.field()),
            without_order(table.field()),
            "{by}"
        );
        assert_eq!(
            sorted.rows(),
            joined.into_sort_by(by).expect("sorted").rows().into_owned(),
            "{by}"
        );
        let mut held = table.clone();
        held.as_sort_by(by)
            .expect("sorted")
            .as_reversed()
            .expect("reversed");
        assert_eq!(held.num_chunks(), 1, "{by}");
        assert_eq!(held.rows(), sorted.into_reversed().rows(), "{by}");
    }
    assert_eq!(
        table
            .sort_indices_by("symbol, id desc")
            .expect("an order")
            .rows()
            .to_vec(),
        [4_u32, 1, 0, 2, 3].map(Scalar::from).to_vec()
    );
    // The serie is as it was, and a refusal leaves it so.
    assert_eq!(table.num_chunks(), 2);
    let mut held = table.clone();
    for refused in ["symbol,", "tier", "id, id desc"] {
        assert!(held.as_sort_by(refused).is_err(), "{refused}");
        assert_eq!(
            (held.num_chunks(), held.rows()),
            (2, table.rows()),
            "{refused}"
        );
    }
}

/// The trades root: an identifier every row holds once, then a nullable
/// integer price with repeats, a venue, a nullable decimal quantity, a
/// nullable float rate - NaN, both zeroes - a version and a nested serie of
/// legs, so a key on any of them carries the rest of the row along.
fn trades_root() -> Field {
    Field::new(
        "trade",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("px", DataType::Int64, true),
                Field::new("venue", DataType::utf8(), false),
                Field::new("qty", DataType::decimal128(10, 2).expect("a decimal"), true),
                Field::new("rate", DataType::Float64, true),
                Field::new("release", DataType::Version, false),
                legs_field(),
            ])
            .expect("seven named children"),
        ),
        false,
    )
}

/// Trade `id`: every field a short cycle of `id`, so values repeat across
/// chunks and the identifier tells equal keys apart.
fn trade(id: i64) -> Scalar {
    let at = |cycle: &[i64]| cycle[usize::try_from(id).expect("an id") % cycle.len()];
    let px = [3, -1, 1, 3, 2, -1, 1][usize::try_from(id).expect("an id") % 7];
    let rate =
        [0.5, f64::NAN, -0.0, 0.0, f64::NEG_INFINITY, 0.5][usize::try_from(id).expect("an id") % 6];
    Scalar::from_sequence([
        Scalar::from(id),
        if px < 0 {
            Scalar::Null
        } else {
            Scalar::from(px)
        },
        Scalar::from(["XNYS", "XNAS", "XPAR", "XNAS"][usize::try_from(id).expect("an id") % 4]),
        match at(&[125, 0, 125, 990, -1]) {
            -1 => Scalar::Null,
            unscaled => Scalar::decimal128(i128::from(unscaled), 2),
        },
        if id % 5 == 4 {
            Scalar::Null
        } else {
            Scalar::from(rate)
        },
        DataType::Version
            .scalar(["1.10.0", "1.9.0", "2.0.0"][usize::try_from(id).expect("an id") % 3])
            .expect("a version"),
        Scalar::from_sequence((0..at(&[2, 0, 1])).map(|leg| Scalar::from(id * 10 + leg))),
    ])
}

/// Trades `0..` cut into chunks of `cuts` rows each, an empty chunk and a
/// chunk of one row among them.
fn trade_chunks(cuts: &[usize]) -> ChunkedSerie {
    let mut id = 0_i64;
    ChunkedSerie::from_series(
        Some(&trades_root()),
        cuts.iter().map(|&rows| {
            Serie::from_scalars(
                trades_root(),
                (0..rows).map(|_| {
                    id += 1;
                    trade(id - 1)
                }),
            )
            .expect("trade rows")
        }),
        ArrowCastOptions::new(),
    )
    .expect("trade chunks")
}

/// Assert the merge answers what the one join answers: `into_sorted` under
/// every ordering and `into_unique` over the rows, and `into_sort_by` each
/// key list of `by`, field and rows alike, and every output chunk at most
/// one batch.
fn assert_merges_as_joined(chunked: &ChunkedSerie, by: &[&str]) {
    let joined = chunked.into_serie().expect("one join");
    let batch = yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE;
    let shaped = |merged: &ChunkedSerie, what: &str| {
        assert_eq!(
            without_order(merged.field()),
            without_order(chunked.field()),
            "{what}"
        );
        assert_eq!(
            merged.len(),
            merged.chunks().iter().map(Serie::len).sum::<usize>(),
            "{what}"
        );
        assert!(
            merged
                .chunks()
                .iter()
                .all(|chunk| chunk.len() <= batch && !chunk.is_empty()),
            "{what}: every chunk holds a row and at most a batch"
        );
    };
    for options in ORDERINGS {
        let merged = chunked.into_sorted(options).expect("merged");
        shaped(&merged, &format!("{options:?}"));
        assert_eq!(
            merged.rows(),
            joined
                .into_sorted(options)
                .expect("sorted")
                .rows()
                .into_owned(),
            "into_sorted {options:?} of {}",
            chunked.field().name()
        );
    }
    let unique = chunked.into_unique().expect("unique");
    shaped(&unique, "into_unique");
    assert_eq!(
        unique.rows(),
        joined.into_unique().expect("unique").rows().into_owned(),
        "into_unique of {}",
        chunked.field().name()
    );
    for by in by {
        let merged = chunked.into_sort_by(*by).expect("merged");
        shaped(&merged, by);
        assert_eq!(
            merged.rows(),
            joined
                .into_sort_by(*by)
                .expect("sorted")
                .rows()
                .into_owned(),
            "into_sort_by {by}"
        );
    }
}

#[test]
fn a_merged_sort_answers_what_the_one_join_answers_on_every_rung() {
    // Repeats across chunks, absent values, an empty chunk and a chunk of
    // one row; the identifier keeps every row distinct, so a tie broken
    // out of chunk order or row order shows in the rows.
    let trades = trade_chunks(&[5, 0, 7, 1, 9, 3]);
    assert_eq!((trades.len(), trades.num_chunks()), (25, 6));
    assert_merges_as_joined(
        &trades,
        &[
            "px desc, venue",
            "venue, px nulls first",
            "px desc nulls first, qty",
            "qty desc, id",
            "rate, id",
            "release desc, venue",
            "legs desc",
            "px * -1, venue desc",
        ],
    );
    // Each column alone: an integer, a text, a decimal and a nested serie
    // on the row format; a float and a version on the values' order.
    for name in ["px", "venue", "qty", "legs", "rate", "release"] {
        let column = trades.child(name).expect("a child");
        assert_merges_as_joined(&column, &[&format!("{name} desc"), name]);
    }
}

#[test]
fn a_merged_sort_keeps_equal_keys_in_chunk_order_then_row_order() {
    let trades = trade_chunks(&[4, 4, 4]);
    let sorted = trades.into_sort_by("venue").expect("merged");
    let ids: Vec<Scalar> = sorted.child("id").expect("ids").rows();
    // XNAS rows 1, 3, 5, 7, 9, 11 come first and in arrival order, then
    // XNYS 0, 4, 8, then XPAR 2, 6, 10.
    assert_eq!(
        ids,
        [1_i64, 3, 5, 7, 9, 11, 0, 4, 8, 2, 6, 10]
            .map(Scalar::from)
            .to_vec()
    );
}

#[test]
fn a_merged_sort_cuts_its_output_into_batches_and_one_chunk_into_slices() {
    let batch = yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE;
    let rows = batch * 5 / 2;
    let values: Vec<Option<i64>> = (0..rows)
        .map(|index| {
            let index = i64::try_from(index).expect("a row");
            (index % 11 != 3).then_some((index * 7_919) % 100_003)
        })
        .collect();
    let field = Field::new("price", DataType::Int64, true);
    let cuts = [70_000, 1, 50_000, rows - 70_001 - 50_000];
    let mut start = 0;
    let arrays: Vec<ArrayRef> = cuts
        .iter()
        .map(|&len| {
            let array = Arc::new(Int64Array::from(values[start..start + len].to_vec())) as ArrayRef;
            start += len;
            array
        })
        .collect();
    let prices = ChunkedSerie::from_arrow_arrays(Some(&field), arrays, ArrowCastOptions::new())
        .expect("four unequal chunks");
    let joined = prices.into_serie().expect("one join");
    for options in [
        SortOptions::ascending(),
        SortOptions::descending().with_nulls_first(true),
    ] {
        let sorted = prices.into_sorted(options).expect("merged");
        assert_eq!(
            sorted.chunks().iter().map(Serie::len).collect::<Vec<_>>(),
            vec![batch, batch, batch / 2],
            "{options:?}"
        );
        assert!(sorted.is_sorted(options));
        assert!(
            sorted == joined.into_sorted(options).expect("sorted"),
            "{options:?}"
        );
    }
    let by = prices.into_sort_by("price desc").expect("merged");
    assert!(by.is_sorted(SortOptions::descending()));
    assert_eq!(by.num_chunks(), 3);
    // One chunk merges nothing: its sorted self, cut into slices.
    let whole = ChunkedSerie::from_serie(joined.clone()).expect("one chunk");
    let sorted = whole.into_sorted(SortOptions::default()).expect("sliced");
    assert_eq!(
        sorted.chunks().iter().map(Serie::len).collect::<Vec<_>>(),
        vec![batch, batch, batch / 2]
    );
    assert!(sorted == joined.into_sorted(SortOptions::default()).expect("sorted"));
    // Uniqueness keeps the chunks apart and their first occurrences.
    let unique = prices.into_unique().expect("unique");
    assert!(unique == joined.into_unique().expect("unique"));
    assert!(unique.num_chunks() <= 4);
}

#[test]
fn a_merged_sort_over_one_or_no_chunk_and_empty_chunks_answers_the_join() {
    let none = ChunkedSerie::empty(price()).expect("no chunk");
    assert_eq!(
        none.into_sorted(SortOptions::default())
            .expect("merged")
            .num_chunks(),
        0
    );
    assert_eq!(none.into_unique().expect("unique").num_chunks(), 0);
    assert_eq!(
        none.into_sort_by("price desc")
            .expect("merged")
            .num_chunks(),
        0
    );
    let empties = ChunkedSerie::from_series(
        Some(&price()),
        [
            int64_column(&price(), vec![]),
            int64_column(&price(), vec![]),
        ],
        ArrowCastOptions::new(),
    )
    .expect("two empty chunks");
    assert_merges_as_joined(&empties, &["price"]);
    assert_eq!(
        empties
            .into_sorted(SortOptions::default())
            .expect("merged")
            .num_chunks(),
        0
    );
    // One row per chunk: every pair of neighbours an edge.
    let ones = cut(&[&[3], &[1], &[3], &[2], &[1]]);
    assert_merges_as_joined(&ones, &["price desc", "price"]);
    let unique = ones.into_unique().expect("unique");
    assert_eq!(
        (unique.num_chunks(), unique.rows()),
        (3, price_rows(&[3, 1, 2]))
    );
    // One chunk holding rows, beside empty ones, is that chunk sorted.
    let one = cut(&[&[], &[3, 1, 2, 1], &[]]);
    assert_merges_as_joined(&one, &["price desc"]);
    assert_eq!(
        one.into_sorted(SortOptions::default())
            .expect("merged")
            .num_chunks(),
        1
    );
    // Duplicates and absent values across the chunks.
    assert_merges_as_joined(
        &chunked(
            &[
                Some(3),
                None,
                Some(1),
                Some(3),
                Some(2),
                None,
                Some(1),
                Some(3),
            ],
            3,
        ),
        &["price nulls first", "price desc"],
    );
}

#[test]
fn a_merged_sort_by_keys_places_an_absent_record_as_absent_in_every_key() {
    let quotes = absent_quote_chunks();
    assert_merges_as_joined(
        &quotes,
        &["symbol", "id desc", "symbol desc nulls first, id"],
    );
}

#[test]
fn a_merged_sort_refuses_its_keys_before_any_chunk_with_no_chunk_as_with_many() {
    for table in [
        quote_table(),
        ChunkedSerie::empty(quotes_root()).expect("no chunk"),
    ] {
        for refused in ["symbol,", "tier", "id, id desc", "unnest(symbol)"] {
            assert!(table.into_sort_by(refused).is_err(), "{refused}");
        }
        let none: Vec<yggdryl::expression::Ordering> = Vec::new();
        let refused = table.into_sort_by(none).unwrap_err().to_string();
        assert!(
            refused.contains("expected at least one `order by` key to sort row by, got none"),
            "{refused}"
        );
    }
}

#[test]
fn a_merged_sort_reads_spilled_chunks_where_they_lie_and_leaves_them_spilled() {
    let mut trades = trade_chunks(&[6, 5, 7]);
    let before = trades.rows();
    trades
        .spill(&yggdryl::SpillOptions::new().with_byte_size(0))
        .expect("spilled");
    assert!(trades.chunks().iter().all(Serie::is_spilled));
    assert_eq!(trades.rows(), before);
    assert_merges_as_joined(&trades, &["px desc, id", "release, id"]);
    // The inputs stay where they lie.
    assert!(trades.chunks().iter().all(Serie::is_spilled));
    assert!(trades.is_spilled());
}

#[test]
fn a_merged_sort_refuses_the_gather_arrow_would_panic_on_and_uniqueness_needs_none() {
    // Two view-text vocabularies of a hundred values each: the int8 key
    // reaches neither gathered whole, so the sort's gather is refused by
    // name, as the join is. Uniqueness filters each chunk by its own mask
    // and gathers nothing, so it answers.
    let venues = |from: usize| -> ArrayRef {
        let names: Vec<String> = (from..from + 100).map(|at| format!("v{at}")).collect();
        let keys: Vec<i8> = (0..100).rev().collect();
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
    for refused in [
        chunked.into_sorted(SortOptions::default()),
        chunked.into_sort_by("item desc"),
    ] {
        let shown = refused
            .expect_err("no int8 key reaches 200 values")
            .to_string();
        assert!(
            shown.contains("gathers 200 dictionary values at $,"),
            "{shown}"
        );
    }
    let unique = chunked.into_unique().expect("no gather");
    assert_eq!((unique.num_chunks(), unique.len()), (2, 200));
    assert_eq!(unique.rows(), chunked.rows());
    // A chunk alone sorts with no gather at all.
    let one = chunked.slice(0, 100).expect("the first chunk");
    let sorted = one.into_sorted(SortOptions::default()).expect("sorted");
    assert!(sorted.is_sorted(SortOptions::default()));
}

/// `field` without the order its root declares: what a sort leaves equal
/// to the field it sorted, the `SORT:by` it wrote aside.
fn without_order(field: &Field) -> Field {
    field.clone().with_metadata_removed("SORT:by")
}

// ---------------------------------------------------------------------------
// The declared order across chunks: a field declaring `SORT:by` proves its
// rows at every door a chunk enters - each chunk's rows and every chunk
// edge - and a sort declares on the field and on every chunk.
// ---------------------------------------------------------------------------

/// The quotes root declaring its rows keep `by`.
fn declaring_root(by: &[&str]) -> Field {
    let mut root = quotes_root();
    root.as_sort_mut().set_by_texts(by).expect("the keys");
    root
}

/// A refusal's path and reason.
fn order_refusal(error: yggdryl::arrow::Error) -> (String, String) {
    match error {
        yggdryl::arrow::Error::Core(yggdryl::Error::InvalidRecord { path, reason }) => {
            (path.to_string(), reason.to_string())
        }
        other => panic!("expected an invalid record, got {other}"),
    }
}

/// The refusal of chunk `index` opening out of `row`'s declared `id`.
fn edge_refused(index: usize) -> (String, String) {
    (
        "row".to_owned(),
        format!("chunk {index} of row opens out of the order its field declares, `id`"),
    )
}

/// The refusal of a chunk's row `index` out of `row`'s declared `id`.
fn row_refused(index: usize) -> (String, String) {
    (
        "row".to_owned(),
        format!("row {index} of row is out of the order its root declares, `id`"),
    )
}

/// The record arrays of `rows` under the quotes root's layout.
fn quote_array(rows: &[(i64, &str)]) -> ArrayRef {
    Arc::new(StructArray::new(
        vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
        ]
        .into(),
        vec![
            Arc::new(Int64Array::from(
                rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            )),
        ],
        None,
    ))
}

/// The same rows with an `int32` id, which the plan casts.
fn narrow_quote_array(rows: &[(i64, &str)]) -> ArrayRef {
    Arc::new(StructArray::new(
        vec![
            ArrowField::new("id", ArrowDataType::Int32, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
        ]
        .into(),
        vec![
            Arc::new(Int32Array::from(
                rows.iter()
                    .map(|row| i32::try_from(row.0).expect("a small id"))
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            )),
        ],
        None,
    ))
}

#[test]
fn chunks_held_under_a_declaring_field_are_proven_at_every_chunk_edge() {
    let root = declaring_root(&["id"]);
    // Chunks of the plain root, cast in: each proven, then every edge.
    let refused = ChunkedSerie::from_series(
        Some(&root),
        [quotes(&[(1, "A"), (2, "B")]), quotes(&[(0, "C")])],
        ArrowCastOptions::new(),
    )
    .unwrap_err();
    assert_eq!(order_refusal(refused), edge_refused(1));
    let refused = ChunkedSerie::from_series(
        Some(&root),
        [quotes(&[(1, "A")]), quotes(&[(3, "B"), (2, "C")])],
        ArrowCastOptions::new(),
    )
    .unwrap_err();
    assert_eq!(order_refusal(refused), row_refused(1));
    // Chunks in order across every edge, an empty one among them, are held
    // and declare the order on the field and on every chunk.
    let held = ChunkedSerie::from_series(
        Some(&root),
        [
            quotes(&[(1, "A"), (2, "B")]),
            quotes(&[]),
            quotes(&[(2, "C"), (5, "D")]),
        ],
        ArrowCastOptions::new(),
    )
    .expect("in order");
    assert_eq!(held.field().get_metadata("SORT:by"), Some(r#"["id"]"#));
    assert!(
        held.chunks()
            .iter()
            .all(|chunk| chunk.field() == Some(&root))
    );
    let keys = held
        .declared_order()
        .expect("well formed")
        .expect("declared");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].to_string(), "id");
    // With no field, the first chunk's declaration is the field's, and the
    // chunks under it are proven at their edges.
    let declared = |rows: &[(i64, &str)]| quotes(rows).into_sort_by("id").expect("sorted");
    let refused = ChunkedSerie::from_series(
        None,
        [declared(&[(1, "A"), (4, "B")]), declared(&[(3, "C")])],
        ArrowCastOptions::new(),
    )
    .unwrap_err();
    assert_eq!(order_refusal(refused), edge_refused(1));
    // A field declaring nothing proves nothing, and a non-record column
    // declares nothing at all.
    assert!(
        ChunkedSerie::from_series(
            Some(&quotes_root()),
            [quotes(&[(1, "A")]), quotes(&[(0, "C")])],
            ArrowCastOptions::new(),
        )
        .expect("no order declared")
        .declared_order()
        .expect("none")
        .is_none()
    );
    assert!(prices().declared_order().expect("none").is_none());
}

#[test]
fn arrow_arrays_under_a_declaring_field_are_proven_at_every_chunk_edge() {
    let root = declaring_root(&["id"]);
    for (what, arrays) in [
        (
            "exact",
            [quote_array(&[(1, "A"), (2, "B")]), quote_array(&[(0, "C")])],
        ),
        (
            "cast",
            [
                narrow_quote_array(&[(1, "A"), (2, "B")]),
                narrow_quote_array(&[(0, "C")]),
            ],
        ),
    ] {
        let refused = ChunkedSerie::from_arrow_arrays(Some(&root), arrays, ArrowCastOptions::new())
            .unwrap_err();
        assert_eq!(order_refusal(refused), edge_refused(1), "{what}");
    }
    let held = ChunkedSerie::from_arrow_arrays(
        Some(&root),
        [
            quote_array(&[(1, "A")]),
            narrow_quote_array(&[(1, "B"), (7, "C")]),
        ],
        ArrowCastOptions::new(),
    )
    .expect("in order");
    assert_eq!(held.len(), 3);
    assert!(held.declared_order().expect("well formed").is_some());
}

#[test]
fn an_arrow_array_under_a_declaring_field_is_proven_row_by_row() {
    // One array whose own rows are out of the order the field declares is
    // refused by its row, on the exact landing as on the plan's - as
    // `Serie::from_arrow_array` refuses it.
    let root = declaring_root(&["id"]);
    for (what, array) in [
        ("exact", quote_array(&[(3, "A"), (2, "B")])),
        ("cast", narrow_quote_array(&[(3, "A"), (2, "B")])),
    ] {
        let refused = ChunkedSerie::from_arrow_arrays(
            Some(&root),
            [quote_array(&[(1, "Z")]), array],
            ArrowCastOptions::new(),
        )
        .map(|held| held.rows());
        assert_eq!(
            refused.map_err(order_refusal),
            Err(row_refused(1)),
            "{what}"
        );
    }
}

#[test]
fn a_cast_onto_a_declaring_field_proves_each_chunk_and_every_edge() {
    let root = declaring_root(&["id"]);
    let plain = ChunkedSerie::from_series(
        None,
        [quotes(&[(1, "A"), (2, "B")]), quotes(&[(0, "C")])],
        ArrowCastOptions::new(),
    )
    .expect("two chunks");
    assert_eq!(
        order_refusal(plain.cast(&root, ArrowCastOptions::new()).unwrap_err()),
        edge_refused(1)
    );
    let disordered = ChunkedSerie::from_series(
        None,
        [quotes(&[(1, "A")]), quotes(&[(3, "B"), (2, "C")])],
        ArrowCastOptions::new(),
    )
    .expect("two chunks");
    assert_eq!(
        order_refusal(disordered.cast(&root, ArrowCastOptions::new()).unwrap_err()),
        row_refused(1)
    );
    let ordered = ChunkedSerie::from_series(
        None,
        [quotes(&[(1, "A")]), quotes(&[(2, "B"), (3, "C")])],
        ArrowCastOptions::new(),
    )
    .expect("two chunks");
    let cast = ordered
        .cast(&root, ArrowCastOptions::new())
        .expect("in order");
    assert_eq!(cast.field(), &root);
    assert!(
        cast.chunks()
            .iter()
            .all(|chunk| chunk.field() == Some(&root))
    );
    // A cast onto a field declaring nothing lands declaring nothing.
    let back = cast
        .cast(&quotes_root(), ArrowCastOptions::new())
        .expect("plain");
    assert!(back.declared_order().expect("none").is_none());
    assert!(
        back.chunks()
            .iter()
            .all(|chunk| chunk.field() == Some(&quotes_root()))
    );
}

#[test]
fn a_pushed_chunk_is_proven_against_the_last_chunk_and_a_refused_one_changes_nothing() {
    let root = declaring_root(&["id"]);
    let mut held = ChunkedSerie::from_series(
        Some(&root),
        [quotes(&[(1, "A"), (2, "B")])],
        ArrowCastOptions::new(),
    )
    .expect("one chunk");
    let before = held.rows();
    // Out of order at the edge, cast in or already under the field.
    for chunk in [
        quotes(&[(0, "C")]),
        quotes(&[(0, "C")])
            .cast(&root, ArrowCastOptions::new())
            .expect("in order alone"),
    ] {
        let refused = held.push_chunk(chunk, ArrowCastOptions::new()).unwrap_err();
        assert_eq!(order_refusal(refused), edge_refused(1));
        assert_eq!((held.num_chunks(), held.rows()), (1, before.clone()));
    }
    // Out of order within itself.
    let refused = held
        .push_chunk(quotes(&[(4, "C"), (3, "D")]), ArrowCastOptions::new())
        .unwrap_err();
    assert_eq!(order_refusal(refused), row_refused(1));
    assert_eq!(held.num_chunks(), 1);
    // In order: held, an equal key at the edge included.
    held.push_chunk(quotes(&[(2, "C"), (3, "D")]), ArrowCastOptions::new())
        .expect("in order");
    held.push_chunk(quotes(&[]), ArrowCastOptions::new())
        .expect("an empty chunk");
    held.push_chunk(quotes(&[(4, "E")]), ArrowCastOptions::new())
        .expect("in order past an empty chunk");
    assert_eq!(held.num_chunks(), 4);
    assert!(
        held.chunks()
            .iter()
            .all(|chunk| chunk.field() == Some(&root))
    );
    // The edge past an empty chunk is the last row held.
    let refused = held
        .push_chunk(quotes(&[(3, "F")]), ArrowCastOptions::new())
        .unwrap_err();
    assert_eq!(order_refusal(refused), edge_refused(4));
}

#[test]
fn a_merged_sort_declares_on_the_field_and_on_every_chunk() {
    let table = ChunkedSerie::from_series(
        None,
        [quotes(&[(3, "C"), (1, "A")]), quotes(&[(2, "B"), (0, "D")])],
        ArrowCastOptions::new(),
    )
    .expect("two chunks");
    for (sorted, text) in [
        (
            table.into_sorted(SortOptions::default()).expect("sorted"),
            r#"["id","symbol"]"#,
        ),
        (
            table
                .into_sorted(SortOptions::descending())
                .expect("sorted"),
            r#"["id desc","symbol desc"]"#,
        ),
        (
            table.into_sort_by("symbol desc").expect("sorted"),
            r#"["symbol desc"]"#,
        ),
    ] {
        assert_eq!(sorted.field().get_metadata("SORT:by"), Some(text));
        assert!(
            sorted
                .chunks()
                .iter()
                .all(|chunk| chunk.field() == Some(sorted.field())),
            "{text}"
        );
        // What the merge declared, its rows land under.
        assert!(
            ChunkedSerie::from_series(
                Some(sorted.field()),
                sorted.chunks().iter().cloned(),
                ArrowCastOptions::new()
            )
            .is_ok(),
            "{text}"
        );
    }
    let keys = table
        .into_sort_by("symbol desc, id")
        .expect("sorted")
        .declared_order()
        .expect("well formed")
        .expect("declared");
    assert_eq!(
        keys.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["symbol desc", "id"]
    );
    // A column that is not a record declares nothing.
    let sorted = prices()
        .into_sorted(SortOptions::default())
        .expect("sorted");
    assert!(sorted.declared_order().expect("none").is_none());
    assert_eq!(sorted.field(), &price());
}

#[test]
fn a_merged_sort_over_chunks_already_declaring_it_answers_the_same_rows() {
    let root = declaring_root(&["id", "symbol desc"]);
    let held = ChunkedSerie::from_series(
        Some(&root),
        [
            quotes(&[(1, "B"), (1, "A"), (2, "C")]),
            quotes(&[(2, "A"), (4, "D")]),
        ],
        ArrowCastOptions::new(),
    )
    .expect("in order");
    for by in ["id, symbol desc", "id"] {
        let sorted = held.into_sort_by(by).expect("sorted");
        assert_eq!(sorted.rows(), held.rows(), "{by}");
        // What it declares begins with what was asked.
        let asked = by.into_orderings().expect("keys");
        let declared = sorted
            .declared_order()
            .expect("well formed")
            .expect("declared");
        assert!(declared.starts_with(&asked), "{by}: {declared:?}");
    }
    let whole = ChunkedSerie::from_series(
        Some(&declaring_root(&["id", "symbol"])),
        [quotes(&[(1, "A"), (1, "B")]), quotes(&[(2, "A")])],
        ArrowCastOptions::new(),
    )
    .expect("in order");
    assert_eq!(
        whole
            .into_sorted(SortOptions::default())
            .expect("sorted")
            .rows(),
        whole.rows()
    );
    assert!(whole.is_sorted(SortOptions::default()));
}

#[test]
fn a_chunked_reversal_flips_and_a_disordered_take_clears_the_fields_declaration() {
    // The field is what `declared_order` reads and what a pushed chunk is
    // proven against, so it states exactly what its chunks state.
    let root = declaring_root(&["id"]);
    let held = ChunkedSerie::from_series(
        Some(&root),
        [quotes(&[(1, "A"), (2, "B")]), quotes(&[(3, "C")])],
        ArrowCastOptions::new(),
    )
    .expect("in order");
    let flipped = r#"["id desc nulls first"]"#;
    let mut in_place = held.clone();
    in_place.as_reversed().expect("reversed");
    for reversed in [held.into_reversed(), in_place] {
        assert!(reversed.chunks().iter().all(|chunk| {
            chunk
                .field()
                .and_then(|field| field.get_metadata("SORT:by"))
                == Some(flipped)
        }));
        assert_eq!(reversed.field().get_metadata("SORT:by"), Some(flipped));
    }
    let picks = Serie::new(vec![Scalar::from(2_u32), Scalar::from(0_u32)]);
    let mut in_place = held.clone();
    in_place.as_taken(&picks).expect("taken");
    for taken in [held.into_taken(&picks).expect("taken"), in_place] {
        assert!(taken.chunks()[0].declared_order().expect("none").is_none());
        assert!(taken.declared_order().expect("none").is_none());
    }
    // Picked in increasing position, the order stays declared.
    let kept = held
        .into_taken(&Serie::new(vec![Scalar::from(0_u32), Scalar::from(2_u32)]))
        .expect("taken");
    assert_eq!(kept.field(), &root);
}

#[test]
fn as_spilled_and_into_spilled_move_the_chunks_as_spill_does() {
    use yggdryl::{ChunkedSerie, DataType, Scalar, Serie, SpillOptions};

    let field = DataType::Int64.required_field("price");
    let chunk = || Serie::from_scalars(field.clone(), (0..1_024_i64).map(Scalar::from)).unwrap();
    let everything = SpillOptions::new().with_byte_size(0);
    let mut chunked =
        ChunkedSerie::from_series(Some(&field), [chunk(), chunk()], Default::default()).unwrap();
    assert!(chunked.as_spilled(&everything).unwrap().is_spilled());
    assert_eq!(chunked.resident_size(), 0);

    let original =
        ChunkedSerie::from_series(Some(&field), [chunk(), chunk()], Default::default()).unwrap();
    let copy = original.into_spilled(&everything).unwrap();
    assert!(copy.is_spilled() && !original.is_spilled());
    assert_eq!(copy.rows(), original.rows());
    assert_eq!(copy.num_chunks(), 2, "the chunks stay apart");
}
