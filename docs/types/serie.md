# Serie

One column: the Arrow buffers that hold the rows of one [`Field`](field.md), and the array, batch and batch reader they cross as.

## Contract

| Aspect | Rule |
| --- | --- |
| What it is | The fourth side of the value model: `DataType` is the shape, `Field` the schema, `Scalar` one row, `Serie` the rows |
| Storage | Arrow buffers and nothing else - a values buffer, offsets where the layout has them, a validity bitmap. No `Scalar` is stored anywhere in a column |
| Shape | A family enum over one leaf per Arrow layout: `Int32Serie` lends `&[i32]`, `Utf8StringSerie` lends the offsets and the characters, `StructSerie` holds one child `Serie` per child field |
| Proof from values | `from_scalars` and `push` send every row through `Field::scalar`, the one value contract, then lay the rows out once |
| Proof from buffers | `from_arrow_*` prove the layout and the nullability of every level in constant time; a value the field's own contract refuses is refused where it is read |
| Datatype | `list(<the field>)`, read off the field rather than inferred, so an empty column still names its datatype |
| Registration | `Sequence::Serie`, so a column is `Scalar::Sequence(Sequence::Serie(..))` - a value wherever a sequence is one |
| Reads as | A sequence: `as_sequence`, `len`, `iter`, `get`, indexing and dotted paths all answer its rows, decoded on the first ask and shared from then on |
| `kind()` | `serie`, so a refusal says which of the two it was handed |
| Not `ArrowScalar` | That holds one payload in four shapes and answers `None` to every native accessor; a serie is one shape, a column, and reads as the sequence it is |
| Family | `SerieValue`, implemented by every leaf, by every family enum, and by `Serie` itself |
| Wire | The crate's own serde writes `{"type": "serie", "value": {"field": .., "rows": [..]}}`; JSON, YAML and TOML write the rows alone, because a codec document carries no schema envelope |
| Bindings | Rust only |

## The leaves

A leaf is one Arrow layout under one field, and its accessors are that layout's own components.

| Family | Leaves | What the leaf lends |
| --- | --- | --- |
| `IntegerSerie` | `Int8Serie` .. `UInt64Serie` | `values() -> &[i32]` and its kind, `nulls()`, `array()` |
| `FloatingSerie` | `Float16Serie`, `Float32Serie`, `Float64Serie` | the same |
| `DecimalSerie` | `Decimal32Serie` .. `Decimal256Serie` | the coefficients, at the field's scale |
| `TemporalSerie` | `Date32Serie` .. `IntervalMonthDayNanoSerie` | the counts, at the field's unit |
| `StringSerie` | `Utf8StringSerie`, `LargeUtf8StringSerie`, `Utf8ViewStringSerie`, `BinaryStringSerie`, `LargeBinaryStringSerie`, `BinaryViewStringSerie`, `FixedStringSerie` | `offsets()`, `payload()` - or `views()` and `payloads()` for a viewed leaf, `width()` for a fixed one |
| `BytesSerie` | `BinarySerie`, `LargeBinarySerie`, `BinaryViewSerie`, `FixedBytesSerie` | the same |
| `BooleanSerie` | - | `values() -> &BooleanBuffer` |
| `NullSerie` | - | a length, and no buffer at all |
| `StructSerie` | - | `children()`, `child(name)`, `child_at`, `nulls()` |
| `ListSerie` | - | `items()`, `range(row)`, `nulls()` |
| `MapSerie` | - | `entries()`, `range(row)`, `nulls()` |
| `VariantSerie` | - | `bytes(row)`, `payload()`, `offsets()` |

A layout that is both a string leaf and a byte leaf - `Binary` under a windows-1252 column, `Binary` under a byte column - is told apart by the field rather than by the buffers, which is why `BinaryStringSerie` and `BinarySerie` are two names for one Arrow array.

## What each ask costs

| Ask | Cost |
| --- | --- |
| `len`, `null_count`, `is_null`, `is_empty` | constant, off the array |
| `values`, `offsets`, `payload`, `views`, `nulls`, `array` | constant, and nothing is copied - these are the buffers themselves |
| `value(i)` on a leaf | a bounds check and a buffer read; no value is built |
| `scalar(i)` | one value built, through the crate's one schema-directed decode |
| `push_value`, `set_value` | one buffer write when nothing else holds the buffers, one copy of the rows when something does |
| the same on a variable-length, viewed, fixed-width or boolean leaf | one rewrite: an offset moves every later run, a view word names which buffer holds it, a fixed payload has no builder to hand back, and a bitmap has none either. That is what those layouts cost a write; their reads stay one load |
| `push`, `set` | the field's value contract, then the write above |
| `child`, `child_at`, `children` | constant: a child is already a column |
| `with_child`, `without_child` | one field edit and one `Vec` of pointers, never a row |
| `into_arrow_array` | the array itself, shared |
| `into_arrow_batch`, `into_arrow_reader` | one batch of the children's own arrays, no row decoded |
| `scalars`, and reading a column as a sequence | one decode of every row, once |

## Use

A column is built from a field and either the rows it types or the buffers that already hold them. Values go through the field's value contract; buffers are proven by their layout and their nullability and cross without a row being read.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{DataType, Field, Scalar, Serie, SerieValue};

    let field = Field::new("price", DataType::Int64, false);

    // Values in: three widths, one width out.
    let serie = Serie::from_scalars(
        field.clone(),
        [Scalar::from(125_i32), Scalar::from(126_u8), Scalar::from(127_i64)],
    )?;
    assert_eq!(serie.len(), 3);
    assert_eq!(serie.scalar(0)?, Scalar::from(125_i64));

    // Buffers in: nothing copied, nothing decoded.
    let array: ArrayRef = Arc::new(Int64Array::from(vec![125, 126, 127]));
    let held = Serie::from_arrow_array(field.clone(), array)?;
    assert_eq!(held.as_int64().unwrap().values(), &[125, 126, 127]);

    // Either way it is the same column.
    assert_eq!(held, serie);

    // The datatype is read off the field, not agreed back out of the rows,
    // so an empty column names it too.
    assert_eq!(Serie::empty(field.clone())?.dtype()?, DataType::list(field.clone()));

    // A row the field refuses refuses the column, naming the field.
    assert!(Serie::from_scalars(field, [Scalar::Null]).is_err());
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

## The buffers, read and written

The typed accessors are the buffers themselves: reading row `i` off one is a bounds check and a load, and writing one is a store. The value doors sit above them and add exactly one thing, the field's own contract.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array, StringArray};
    use yggdryl::{DataType, Field, Scalar, Serie, SerieValue};

    let field = Field::new("price", DataType::Int64, false);
    let array: ArrayRef = Arc::new(Int64Array::from(vec![125, 126]));
    let mut serie = Serie::from_arrow_array(field, array)?;

    // Straight off the values buffer: no value built.
    let prices = serie.as_int64().expect("an int64 column");
    assert_eq!(prices.values(), &[125, 126]);
    assert_eq!(prices.value(1), Some(126));
    assert!(prices.nulls().is_none());

    // A native append writes that same buffer.
    serie.as_int64_mut().expect("an int64 column").push_value(Some(127));
    assert_eq!(serie.as_int64().unwrap().values(), &[125, 126, 127]);

    // And the value door writes it through the field, which narrows the width.
    serie.push(Scalar::from(128_i16))?;
    assert_eq!(serie.as_int64().unwrap().values(), &[125, 126, 127, 128]);

    // One slot, rewritten where it lies.
    serie.set(0, Scalar::from(999_i64))?;
    assert_eq!(serie.as_int64().unwrap().value(0), Some(999));

    // Text lends its two components rather than its rows.
    let symbols: ArrayRef = Arc::new(StringArray::from(vec!["AAPL", "MSFT"]));
    let text = Serie::from_arrow_array(Field::new("symbol", DataType::utf8(), false), symbols)?;
    let leaf = text.as_utf8().expect("a utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 4, 8]);
    assert_eq!(leaf.payload().as_slice(), b"AAPLMSFT");
    assert_eq!(leaf.value(1), Some("MSFT"));
    ```

## Children, added and dropped

A record column is made of child columns, and Arrow already holds each one separately. Reaching one, dropping one or putting one back moves pointers and never a row.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{DataType, Field, Scalar, Serie, SerieValue, StructType};

    let root = Field::new(
        "row",
        DataType::Struct(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), false),
        ])?),
        false,
    );
    let records = Serie::from_scalars(
        root,
        [Scalar::from_struct([
            ("id", Scalar::from(1_i64)),
            ("symbol", Scalar::from("AAPL")),
        ])?],
    )?;
    let structure = records.as_struct().expect("a record column");

    // One child is a column of its own field, over its own buffers.
    let symbols = structure.child("symbol").expect("a named child");
    assert_eq!(symbols.field().name(), "symbol");
    assert_eq!(symbols.as_utf8().unwrap().value(0), Some("AAPL"));

    // Dropping one takes its field with it; the other is shared, not rebuilt.
    let without = structure.without_child("symbol")?;
    assert_eq!(without.field().field_len(), 1);
    assert!(without.child("symbol").is_none());

    // And putting one back is the same move in reverse.
    let volumes: ArrayRef = Arc::new(Int64Array::from(vec![10_i64]));
    let volume = Serie::from_arrow_array(Field::new("volume", DataType::Int64, false), volumes)?;
    assert_eq!(without.with_child(&volume)?.field().field_len(), 2);
    ```

## A column is a value

`Serie` registers in the sequence family, so a column needs no second reader anywhere: every accessor that answers a sequence answers a column, and `kind()` still says which one it was handed. A column holds no rows, so the first ask for one decodes them; the reading is kept beside the column rather than in it, and the buffers are untouched by being read.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Sequence, Serie, SerieValue};

    let serie = Serie::from_scalars(
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

    // The reading is lazy, and it is kept beside the column, never in it.
    let Scalar::Sequence(sequence) = &value else { panic!("a column is a sequence value") };
    let rows = sequence.as_serie_rows().expect("a column read as rows");
    assert_eq!(rows.rows()?.len(), 2);
    assert_eq!(rows.column().as_int64().unwrap().values(), &[125, 126]);

    // Dropping the field is spelled, never implied.
    assert_eq!(serie.into_sequence()?, run);
    ```

## Arrow: an array, a batch, a reader

Every crossing shares buffers. A column of a leaf field is an array; a column of a non-null Struct field is a table, and the stream of it is what a record write already speaks. Coming the other way, the field decides which leaf the buffers land in and proves every level of the layout and its nullability before taking them.

=== "Rust"

    ```rust
    use arrow_array::RecordBatchReader;
    use yggdryl::{DataType, Field, Scalar, Serie, SerieValue, StructType};

    let field = Field::new("price", DataType::Int64, false);
    let serie = Serie::from_scalars(field.clone(), [Scalar::from(125_i64), Scalar::from(126_i64)])?;

    // One column, both directions, nothing copied.
    let array = serie.into_arrow_array();
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
    let rows = Serie::from_scalars(
        root,
        [Scalar::from_struct([
            ("id", Scalar::from(1_i64)),
            ("symbol", Scalar::from("AAPL")),
        ])?],
    )?;

    let batch = rows.into_arrow_batch()?;
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(Serie::from_arrow_batch(&batch)?.scalars()?, rows.scalars()?);

    // A reader states its schema before it is pulled; one held column is one
    // batch, and a stream of any number of them drains back into one column.
    let reader = rows.into_arrow_reader()?;
    assert_eq!(reader.schema().fields().len(), 2);
    assert_eq!(Serie::from_arrow_reader(rows.into_arrow_reader()?)?.len(), 1);

    // A column of a leaf field is not a table, and says so by name.
    assert!(serie.into_arrow_batch().is_err());
    ```

## Variant: one encoded run per row

A variant row is one run of the crate's own [variant encoding](variant.md), so a variant column is a byte column: the offsets cut one encoded value per row, and `bytes` lends that run where it lies. Reading a row decodes it, writing one encodes it, and a caller forwarding a row rather than reading it moves the bytes untouched.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Serie, SerieValue};

    let column = Serie::from_scalars(
        Field::new("payload", DataType::Variant, false),
        [Scalar::from(1_i64), Scalar::from("AAPL")],
    )?;
    let leaf = column.as_variant().expect("a variant column");

    // The runs are bytes, and they are lent rather than decoded.
    assert!(leaf.bytes(0).is_some());
    assert!(!leaf.payload().is_empty());

    // A row becomes a value only when one is asked for, and it comes back
    // as what it went in as.
    assert_eq!(column.scalar(0)?, Scalar::from(1_i64));
    assert_eq!(column.scalar(1)?, Scalar::from("AAPL"));
    ```

## Edges

- A row the field refuses refuses the whole column, naming the field; a nullable field is what admits `Scalar::Null`.
- An Arrow array whose physical datatype is not the field's is refused rather than reconciled - reshape it with [`cast`](cast.md) first.
- An Arrow array carrying absent rows under a required field is refused, at every level: a record's children are judged only where the record itself is present, because a null record row leaves its children's slots unspecified.
- A leaf built from buffers is proven by its layout, which is what buffers can be asked in constant time. A value those buffers hold that the field's own contract would refuse - text no `Currency` registers, bytes that are not well-known binary - is refused by `scalar`, where it is read, and never silently.
- Dictionary, run-end and union layouts have no column here yet and are refused by name rather than read as something else; every other layout the crate's datatypes project to has one.
- `into_arrow_batch`, `into_arrow_reader` and `from_arrow_batch` need a bounded, non-null Struct root; anything else is refused by name.
- `from_arrow_reader` drains: a stream is one-shot and a column is held. The batches are concatenated once, and each column below keeps its own buffers; keep rows a stream with [`IOMedia::read_arrow_reader`](../holder/index.md) instead.
- A batch read back names its root `row`, because Arrow names columns and never the record.
- A column and a schema-free run with the same rows are not equal; equality is what tells them apart, and the leaf breaks the ordering tie.
- Two columns are equal when their field and their rows are, whichever buffers hold them - a `Utf8View` column and a `Utf8` column of one field are not, because the field names the layout. A column hashes by its field and its length, so a hash never decodes a buffer.
- `Sequence::as_slice` lends a slice only once the rows have been decoded; `Sequence::rows` is the door that decodes, and `Scalar::as_sequence` goes through it.
- Indexing a column whose buffers hold a value its field refuses panics, as indexing any value the accessor cannot answer does; `scalar` returns that refusal instead.
- JSON, YAML and TOML write a column as its rows. The field is restored on the read side by a `Field`, never by an envelope in the document.
- A buffer write is in place only when nothing else holds the buffers. A clone shares them, so writing to one of two clones copies the rows once and the two go their own way - which is what makes a column a value rather than a handle.

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
