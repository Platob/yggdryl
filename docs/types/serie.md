# Serie

One column: the rows of one [`Field`](field.md), and the Arrow array, batch, and batch reader they cross as.

## Contract

| Aspect | Rule |
| --- | --- |
| What it is | The fourth side of the value model: `DataType` is the shape, `Field` the schema, `Scalar` one row, `Serie` the rows |
| Storage | One `Arc<Field>` and one `Arc<[Scalar]>`; a clone of a million-row column copies sixteen bytes |
| Proof | Every row through `Field::scalar` once, at construction or at an Arrow import; nothing re-checks a row after that |
| Datatype | `list(<the field>)`, read off the field rather than inferred, so an empty column still names its datatype |
| Registration | `Sequence::Serie`, so a column is `Scalar::Sequence(Sequence::Serie(..))` - a value wherever a sequence is one |
| Reads as | A sequence: `as_sequence`, `len`, `iter`, `get`, indexing and dotted paths all answer its rows |
| `kind()` | `serie`, so a refusal says which of the two it was handed |
| Not `ArrowScalar` | That holds Arrow *buffers* in four shapes and answers `None` to every native accessor; a serie holds the *rows* |
| Family | `SerieValue`, implemented by `Column` (the one leaf today) and by `Serie` |
| Wire | The crate's own serde writes `{"type": "serie", "value": {"field": .., "rows": [..]}}`; JSON, YAML and TOML write the rows alone, because a codec document carries no schema envelope |
| Bindings | Rust only |

## Use

A column is built from a field and the rows it types. The field is the one value contract, so the rows come out in the representation it declares whatever width they went in as.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Serie};

    let serie = Serie::from_rows(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(125_i32), Scalar::from(126_u8), Scalar::from(127_i64)],
    )?;

    // Three widths in, one width out.
    assert_eq!(serie.len(), 3);
    assert_eq!(serie.get(0), Some(&Scalar::from(125_i64)));

    // The datatype is read off the field, not agreed back out of the rows,
    // so an empty column names it too.
    let empty = Serie::from_rows(Field::new("price", DataType::Int64, false), [])?;
    assert_eq!(empty.dtype()?, serie.dtype()?);

    // A row the field refuses refuses the column, naming the field.
    assert!(
        Serie::from_rows(Field::new("price", DataType::Int64, false), [Scalar::Null]).is_err()
    );
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
    assert_eq!(serie.into_sequence(), run);
    ```

## Arrow: an array, a batch, a reader

Every crossing routes through the crate's single scalar-array boundary, so the field decides nullability, dictionary identity, and extension identity exactly as it does for a stored column. A column of a leaf field is an array; a column of a non-null Struct field is a table, and the stream of it is what a record write already speaks.

=== "Rust"

    ```rust
    use arrow_array::RecordBatchReader;
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};

    let field = Field::new("price", DataType::Int64, false);
    let serie = Serie::from_rows(field.clone(), [Scalar::from(125_i64), Scalar::from(126_i64)])?;

    // One column, both directions.
    let array = serie.into_arrow_array()?;
    assert_eq!(array.len(), 2);
    assert_eq!(Serie::from_arrow_array(field, array.as_ref())?, serie);

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
    assert_eq!(Serie::from_arrow_batch(&batch)?.as_slice(), rows.as_slice());

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
- `from_arrow_reader` drains: a stream is one-shot and a column is held. Keep rows a stream with [`IOMedia::read_arrow_reader`](../holder/index.md) instead.
- A batch read back names its root `row`, because Arrow names columns and never the record.
- A column and a schema-free run with the same rows are not equal, and hash to the same value; equality is what tells them apart, and the leaf breaks the ordering tie.
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
