# Serie

One column: the rows of one [`Field`](field.md), and the Arrow array, batch, and batch reader they cross as.

## Contract

| Aspect | Rule |
| --- | --- |
| What it is | The fourth side of the value model: `DataType` is the shape, `Field` the schema, `Scalar` one row, `Serie` the rows |
| Storage | Arrow buffers, in runs, plus whatever has been pushed since the last freeze; the whole body is one shared pointer, so a clone of a million-row column copies eight bytes |
| Proof from values | `from_rows` and `push` send every row through `Field::scalar`, the one value contract |
| Proof from buffers | `from_arrow_*` prove the layout and the nullability in constant time; a value the field's own contract refuses is refused where it is read |
| Datatype | `list(<the field>)`, read off the field rather than inferred, so an empty column still names its datatype |
| Registration | `Sequence::Serie`, so a column is `Scalar::Sequence(Sequence::Serie(..))` - a value wherever a sequence is one |
| Reads as | A sequence: `as_sequence`, `len`, `iter`, `get`, indexing and dotted paths all answer its rows, decoding the buffers once and caching |
| `kind()` | `serie`, so a refusal says which of the two it was handed |
| Not `ArrowScalar` | That holds one payload in four shapes and answers `None` to every native accessor; a serie is one shape, a column, and reads as the sequence it is |
| Family | `SerieValue`, implemented by `Column` (the one leaf today) and by `Serie` |
| Wire | The crate's own serde writes `{"type": "serie", "value": {"field": .., "rows": [..]}}`; JSON, YAML and TOML write the rows alone, because a codec document carries no schema envelope |
| Bindings | Rust only |

## What each ask costs

| Ask | Cost |
| --- | --- |
| `len`, `is_null`, `is_empty`, `chunk_count` | constant, off the run offsets |
| `get`, `i64_at`, `f64_at`, `str_at`, `bytes_at`, `decimal_at`, `temporal_at` | one binary search over the runs, then one buffer read |
| `push`, `extend` | amortized constant, into the unfrozen tail |
| `append_arrow_array`, `append` | one run, no row copied |
| `slice` | one slice per run the window spans, buffers shared |
| `child`, `child_at`, `without_child` | one array per run, never one row |
| `with_child` | one gather per column, then one record array |
| `into_arrow_array` | the run itself when there is one, a concatenation when there are several |
| `into_arrow_reader`, `from_arrow_reader` | one batch per run, nothing gathered and no row decoded |
| `rows`, `as_sequence` | one decode of every row, cached |

## Use

A column is built from a field and either the rows it types or the buffers that already hold them. Values go through the field's value contract; buffers are proven by their layout and their nullability and cross without a row being read.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{DataType, Field, Scalar, Serie};

    let field = Field::new("price", DataType::Int64, false);

    // Values in: three widths, one width out.
    let serie = Serie::from_rows(
        field.clone(),
        [Scalar::from(125_i32), Scalar::from(126_u8), Scalar::from(127_i64)],
    )?;
    assert_eq!(serie.len(), 3);
    assert_eq!(serie.get(0)?, Scalar::from(125_i64));

    // Buffers in: nothing copied, nothing decoded.
    let array: ArrayRef = Arc::new(Int64Array::from(vec![125, 126, 127]));
    let held = Serie::from_arrow_array(field.clone(), array)?;
    assert_eq!(held.chunk_count(), 1);
    assert_eq!(held.i64_at(2), Some(127));

    // Either way it is the same column.
    assert_eq!(Scalar::from(held), Scalar::from(serie));

    // The datatype is read off the field, not agreed back out of the rows,
    // so an empty column names it too.
    assert_eq!(Serie::empty(field.clone()).dtype()?, DataType::list(field.clone()));

    // A row the field refuses refuses the column, naming the field.
    assert!(Serie::from_rows(field, [Scalar::Null]).is_err());
    ```

=== "Python"

    ```python
    # Rust only. A column crosses a binding as an Arrow array or record batch;
    # `ArrowScalar` is what carries one there.
    ```

=== "JavaScript"

    ```javascript
    // Rust only. A column crosses a binding as an Arrow array or record batch;
    // `ArrowScalar` is what carries one there.
    ```

## Random access, and growing

Reading row `i` is a binary search over the runs and one buffer read; the typed readers borrow rather than build a value, and each spans every width its family spans. Growing never rebuilds what the column already holds: a pushed value lands in an unfrozen tail, and a whole run appends as one chunk.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{DataType, Field, Scalar, Serie};

    let field = Field::new("price", DataType::Int64, false);
    let array: ArrayRef = Arc::new(Int64Array::from(vec![125, 126]));
    let mut serie = Serie::from_arrow_array(field, array)?;

    // Straight off the buffer: no value built, and the column stays undecoded.
    assert_eq!(serie.i64_at(1), Some(126));
    assert_eq!(serie.i128_at(0), Some(125));
    assert!(!serie.is_null(0));
    assert_eq!(serie.as_slice(), None);

    // A pushed row lands in the tail, so the run is untouched.
    serie.push(Scalar::from(127_i64))?;
    assert_eq!(serie.len(), 3);
    assert_eq!(serie.chunk_count(), 1);
    assert_eq!(serie.i64_at(2), Some(127));

    // A whole run appends as one chunk, and nothing is copied.
    serie.append_arrow_array(Arc::new(Int64Array::from(vec![200, 201])))?;
    assert_eq!(serie.len(), 5);
    assert_eq!(serie.chunk_count(), 3);

    // A window shares the buffers it spans.
    assert_eq!(serie.slice(1, 3).len(), 3);

    // And the rows read back in order across every run, decoded once.
    assert_eq!(serie.rows()?.len(), 5);
    assert!(serie.as_slice().is_some());
    ```

## Children, added and dropped

A record column is made of child columns, and Arrow already holds each one separately. Reaching one, dropping one or putting one back is an array per run and never a row.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};

    let root = Field::new(
        "row",
        DataType::Struct(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), false),
        ])?),
        false,
    );
    let records = Serie::from_rows(
        root,
        [Scalar::from_struct([
            ("id", Scalar::from(1_i64)),
            ("symbol", Scalar::from("AAPL")),
        ])?],
    )?;

    // One child is a column of its own field, over the same buffers.
    let symbols = records.child("symbol")?;
    assert_eq!(symbols.field().name(), "symbol");
    assert_eq!(symbols.str_at(0), Some("AAPL"));

    // Dropping one takes its field with it; the other is shared, not rebuilt.
    let without = records.without_child("symbol")?;
    assert_eq!(without.field().field_len(), 1);
    assert!(without.child("symbol").is_err());

    // And putting it back reaches the same shape.
    assert_eq!(without.with_child(&symbols)?.field().field_len(), 2);
    ```

## A column is a value

`Serie` registers in the sequence family, so a column needs no second reader anywhere: every accessor that answers a sequence answers a column, and `kind()` still says which one it was handed.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Serie};

    let serie = Serie::from_rows(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(125_i64), Scalar::from(126_i64)],
    )?;
    let value = Scalar::from(serie.clone());

    assert_eq!(value.kind(), "serie");
    assert_eq!(value.len(), 2);
    assert_eq!(value[0], Scalar::from(125_i64));
    assert_eq!(value.iter().count(), 2);
    assert!(value.is_container());

    // A column and the run it holds order by their rows; the leaf only
    // breaks the tie, so a column never sorts away from its own rows.
    let run = Scalar::from_sequence([Scalar::from(125_i64), Scalar::from(126_i64)]);
    assert_ne!(value, run);
    assert!(run < value);

    // Dropping the field is spelled, never implied.
    assert_eq!(serie.into_sequence()?, run);
    ```

## Arrow: an array, a batch, a reader

Every crossing routes through the crate's single scalar-array boundary, so the field decides nullability, dictionary identity, and extension identity exactly as it does for a stored column. A column of a leaf field is an array; a column of a non-null Struct field is a table, and the stream of it is what a record write already speaks. A column of several runs streams as several batches and gathers nothing, in either direction.

=== "Rust"

    ```rust
    use arrow_array::RecordBatchReader;
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};

    let field = Field::new("price", DataType::Int64, false);
    let serie = Serie::from_rows(field.clone(), [Scalar::from(125_i64), Scalar::from(126_i64)])?;

    // One column, both directions.
    let array = serie.into_arrow_array()?;
    assert_eq!(array.len(), 2);
    assert_eq!(Serie::from_arrow_array(field, array)?, serie);

    // The rows of a record root: a table, and the stream of it.
    let root = Field::new(
        "row",
        DataType::Struct(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), false),
        ])?),
        false,
    );
    let rows = Serie::from_rows(
        root,
        [Scalar::from_struct([
            ("id", Scalar::from(1_i64)),
            ("symbol", Scalar::from("AAPL")),
        ])?],
    )?;

    let batch = rows.into_arrow_batch()?;
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(Serie::from_arrow_batch(&batch)?.rows()?, rows.rows()?);

    // A reader states its schema before it is pulled; one held column is one
    // batch, and a stream of any number of them drains back into one column.
    let reader = rows.into_arrow_reader()?;
    assert_eq!(reader.schema().fields().len(), 2);
    assert_eq!(Serie::from_arrow_reader(rows.into_arrow_reader()?)?.len(), 1);

    // A column of a leaf field is not a table, and says so by name.
    assert!(serie.into_arrow_batch().is_err());
    ```

## What a column keeps

A serie keeps its field across exactly what the value contract leaves alone. Rows a declaring field does not have to rewrite come back the column they were, nested in a record row or not; rows it does rewrite come back the run they now hold, because from there the declaring field is the authority on the column and carrying a second one would be two owners for one fact.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Serie};

    let item = Field::new("price", DataType::Int64, false);
    let value = Scalar::from(Serie::from_rows(item.clone(), [Scalar::from(125_i64)])?);

    // Nothing to rewrite: still a column.
    let exact = Field::new("prices", DataType::list(item), false);
    assert_eq!(exact.scalar(value.clone())?.kind(), "serie");

    // A narrower width rewrites the rows, and the declaring field takes over.
    let narrower = Field::new(
        "prices",
        DataType::list(Field::new("price", DataType::Int32, false)),
        false,
    );
    assert_eq!(narrower.scalar(value)?.kind(), "sequence");
    ```

## Edges

- A row the field refuses refuses the whole column, naming the field; a nullable field is what admits `Scalar::Null`.
- An Arrow array whose physical datatype is not the field's is refused rather than reconciled - reshape it with [`cast`](cast.md) first.
- An Arrow array carrying nulls under a required field is refused: the import proves the rows against the field, it does not take the array's word.
- `into_arrow_batch`, `into_arrow_reader` and `from_arrow_batch` need a bounded, non-null Struct root; anything else is refused by name.
- `from_arrow_reader` drains: a stream is one-shot and a column is held. Each batch becomes one run, so nothing is decoded and nothing is gathered; keep rows a stream with [`IOMedia::read_arrow_reader`](../holder/index.md) instead.
- A batch read back names its root `row`, because Arrow names columns and never the record.
- A column and a schema-free run with the same rows are not equal; equality is what tells them apart, and the leaf breaks the ordering tie.
- How a column is cut into runs is not part of its value: two columns over the same rows under the same field are equal whichever way they were assembled, and a column hashes by its field and its length so a hash never decodes a buffer.
- `as_slice` lends a slice only where the rows are already values; `rows` is the door that decodes and reports. Borrowing a column's rows through `Scalar::as_sequence` decodes them once and caches the reading.
- Indexing a column whose buffers hold a value its field refuses panics, as indexing any value the accessor cannot answer does; `get` returns that refusal instead.
- JSON, YAML and TOML write a column as its rows. The field is restored on the read side by a `Field`, never by an envelope in the document.
- The variant encoding writes a column as the list it is; the field is schema and the encoding carries values.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- serie
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test arrow -- serie
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^serie/'
    ```

=== "Python"

    ```bash
    # Rust only: no Python surface to exercise.
    ```

=== "JavaScript"

    ```bash
    # Rust only: no JavaScript surface to exercise.
    ```
