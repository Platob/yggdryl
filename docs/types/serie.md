# Serie

Many values: a schema-free run, or the Arrow buffers of one [`Field`](field.md). `Serie` is what the sequence family holds, so there is one type for "many values" and not two.

## Contract

| Aspect | Rule |
| --- | --- |
| What it is | The fourth side of the value model: `DataType` is the shape, `Field` the schema, `Scalar` one value, `Serie` many of them |
| Two leaves | `Serie::List` is a schema-free ordered run and holds its values; every other leaf is a column and holds Arrow buffers. What separates them is the field: a run declares none |
| Storage | A column stores a values buffer, offsets where the layout has them, and a validity bitmap. No `Scalar` is stored anywhere in a column |
| Recursion | A record's child, a sequence's items and a mapping's entries are each a `Serie`, so the nesting is the same type all the way down. A mapping's items are the key-value records |
| Size | 24 bytes — the column leaves are shared behind one pointer each, the run is held inline because a row canonicalizes to one |
| Shape | A family enum over one leaf per Arrow layout: `Int32Serie` lends `&[i32]`, `Utf8StringSerie` lends the offsets and the characters, `StructSerie` holds one child `Serie` per child field |
| Proof from values | `from_scalars` and `push` send every row through `Field::scalar`, the one value contract, then lay the rows out once |
| Proof from buffers | `from_arrow_*` prove the layout and the nullability of every level in constant time; a value the field's own contract refuses is refused where it is read |
| Datatype | For a column, `list(<the field>)` — read rather than inferred, so an empty column still names it. For a run, agreed back out of its rows |
| Registration | `Scalar::Sequence(Serie)`. It adds no `DataTypeId`, no `DataType` variant and no `Field` variant |
| Reads as | A sequence, because it is one: `len`, `iter` and the rest answer for both leaves. `as_sequence` borrows, and a column has no value to lend, so it answers `None` there and `Serie::rows` is the door that reads either |
| `kind()` | `serie` for a column, `sequence` for a run, so a refusal says which of the two it was handed |
| Not `ArrowScalar` | That holds one payload in four shapes and answers `None` to every native accessor; a serie is one shape, a column, and reads as the sequence it is |
| Family | `SerieValue`, implemented by every column leaf and every family enum. `Serie` itself does not implement it, because a run has no field to answer with |
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
| `StructSerie` | - | `children()` (each a `Serie`), `child(name)`, `child_at`, `nulls()` |
| `SequenceSerie`, `LargeSequenceSerie` | - | `items()` (a `Serie`), `offsets()` typed at the leaf's own width, `range(row)`, `nulls()` |
| `MappingSerie` | - | the same leaf read as entries: `items()` are the key-value records, plus `offsets()`, `range(row)`, `nulls()` |
| `VariantSerie` | - | `bytes(row)`, `payload()`, `offsets()`. One width only: `DataType::Variant` projects to Arrow `Binary` and nothing else |

Arrow spells a sequence at two offset widths, and lays a mapping out as a list of non-null key-value entry records — so `SequenceSerie`, `LargeSequenceSerie` and `MappingSerie` are three names for one implementation, `GenericSequenceSerie<O, K>`, generic over the offset width and over a `SequenceKind` marker. All three hold the same four things: a field, offsets, one `Serie` of what the offsets cut, and a validity bitmap. The marker decides three things and nothing else — how a row reads (`from_sequence` or paired into `from_mapping`), which Arrow array it lays out (`ListArray`, `LargeListArray`, `MapArray`), and which leaf of the root it is.

The bound is on the pair, so a shape Arrow has no type for is unrepresentable rather than dead: there is no `SequenceKind<i64> for Entries`, because Arrow has no large map.

`ByteSerie<T, K>` splits the same way — a string leaf and a byte leaf over identical binary buffers, told apart by `TextBytes` and `RawBytes` — but its markers carry less. A byte marker only picks the leaf; what a row *means* comes from the field. A sequence marker decides the meaning too, because the field cannot supply it here: a mapping's rows are stored as records of a key and a value, and `Field::scalar` on a mapping field takes a mapping rather than a sequence of those records. That pairing has to happen somewhere the field is not, and this is the one place the leaf departs from "the leaf names the layout, the field names the meaning".

The width and the shape are the leaf, so nothing branches on either per row, and narrowing to another answers `None`.

A layout that is both a string leaf and a byte leaf - `Binary` under a windows-1252 column, `Binary` under a byte column - is told apart by the field rather than by the buffers, which is why `BinaryStringSerie` and `BinarySerie` are two names for one Arrow array.

## What each ask costs

| Ask | Cost |
| --- | --- |
| `len`, `is_null`, `is_empty` | constant, for either leaf |
| `null_count` | constant for a column, off its validity bitmap; a walk for a run, which keeps none |
| `values`, `offsets`, `payload`, `views`, `nulls`, `array` | constant, and nothing is copied - these are the buffers themselves |
| `value(i)` on a leaf | a bounds check and a buffer read; no value is built |
| `scalar(i)` | one value built, through the crate's one schema-directed decode |
| `push_value`, `set_value` | one buffer write when nothing else holds the buffers, one copy of the rows when something does |
| the same on a variable-length, viewed, fixed-width or boolean leaf | one rewrite: an offset moves every later run, a view word names which buffer holds it, a fixed payload has no builder to hand back, and a bitmap has none either. That is what those layouts cost a write; their reads stay one load |
| `push`, `set` | on a column, the field's value contract then the write above. On a run, a copy of every value it holds — a run is one shared slice, so building one `push` at a time is quadratic and `Scalar::from_sequence` is what builds it in one go |
| `child`, `child_at`, `children` | constant: a child is already a column |
| `with_child`, `without_child` | one field edit and one `Vec` of pointers, never a row |
| `into_arrow_array` | the array itself, shared |
| `into_arrow_batch`, `into_arrow_reader` | one batch of the children's own arrays, no row decoded |
| `scalars`, `Serie::rows`, and walking a column as a value | one row built per row, every time — nothing is cached, so hold the answer rather than asking twice |

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
    assert_eq!(symbols.field().expect("a column").name(), "symbol");
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

`Serie` registers in the sequence family, so a column needs no second reader anywhere: every accessor that answers a sequence answers a column, and `kind()` still says which one it was handed. A column holds no rows and keeps none: each row is built as it is reached and nothing is cached, so reading a column leaves it exactly as it was. The one thing it cannot do is lend a slice it does not have — `as_sequence` borrows, so it answers `None` for a column, and a walk yields `Cow`, borrowed for a run and owned for a column.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Serie, SerieValue};

    let serie = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(125_i64), Scalar::from(126_i64)],
    )?;
    let value = Scalar::from(serie.clone());

    assert_eq!(value.kind(), "serie");
    assert_eq!(value.len(), 2);
    assert!(value.is_container());

    // A walk answers every row, building each as it is reached.
    assert_eq!(
        value.iter().map(|row| row.into_owned()).collect::<Vec<_>>(),
        vec![Scalar::from(125_i64), Scalar::from(126_i64)]
    );

    // What it cannot do is lend a row it does not store: `get` and `[i]`
    // borrow, so they answer for a run and not for a column. One row of a
    // column is `Serie::scalar`, which builds it.
    assert_eq!(value.get(0), None);
    assert_eq!(serie.scalar(0)?, Scalar::from(125_i64));

    // A column and the run it holds order by their rows; the leaf only
    // breaks the tie, so a column never sorts away from its own rows.
    let run = Scalar::from_sequence([Scalar::from(125_i64), Scalar::from(126_i64)]);
    assert_ne!(value, run);
    assert!(run < value);

    // A run lends its values; a column has none to lend, and reading it
    // keeps nothing.
    let Scalar::Sequence(sequence) = &value else { panic!("a column is a sequence value") };
    assert_eq!(sequence.as_slice(), None);
    assert_eq!(sequence.rows()?.len(), 2);
    assert_eq!(sequence.as_slice(), None, "the read kept nothing");
    assert_eq!(run.as_sequence().expect("a run lends").len(), 2);

    // The buffers are untouched by being read, and the sequence value *is*
    // the serie - there is no second type between them.
    assert!(sequence.is_column());
    assert!(sequence.as_run().is_none());
    assert_eq!(sequence.as_int64().unwrap().values(), &[125, 126]);

    // Dropping the field is spelled, never implied.
    assert_eq!(serie.into_sequence()?, run);
    ```

## The nesting points back here

A record's child, a sequence's items and a mapping's entries are each a `Serie` — the same type, all the way down. A column of records is therefore columns of columns, and reaching a leaf three levels deep is three borrows and no copy.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};

    let root = Field::new(
        "row",
        DataType::Struct(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new(
                "tags",
                DataType::list(Field::new("item", DataType::utf8(), false)),
                false,
            ),
        ])?),
        false,
    );
    let column = Serie::from_scalars(
        root,
        [Scalar::from_struct([
            ("id", Scalar::from(1_i64)),
            ("tags", Scalar::from_sequence([Scalar::from("a"), Scalar::from("b")])),
        ])?],
    )?;

    // Every step down answers a `Serie`, and every one carries its own field.
    let tags = column.as_struct().expect("a record column").child("tags").expect("a named child");
    assert_eq!(tags.field().expect("a column").name(), "tags");

    let items = tags.as_sequence().expect("a sequence column").items();
    assert_eq!(items.field().expect("a column").name(), "item");
    assert_eq!(items.as_utf8().expect("a utf8 column").value(1), Some("b"));

    // A schema-free run is the same type, and the one leaf with no field.
    let run = Serie::new(vec![Scalar::from(1_i64)]);
    assert_eq!(run.field(), None);
    assert!(run.require_field().is_err());
    assert!(!run.is_column());
    ```

## Arrow: an array, a batch, a reader

Every crossing shares buffers. A column of a leaf field is an array; a column of a non-null Struct field is a table, and the stream of it is what a record write already speaks. Coming the other way, the field decides which leaf the buffers land in and proves every level of the layout and its nullability before taking them.

=== "Rust"

    ```rust
    use arrow_array::RecordBatchReader;
    use yggdryl::{DataType, Field, Scalar, Serie, SerieValue, StructType};

    let field = Field::new("price", DataType::Int64, false);
    let serie = Serie::from_scalars(field.clone(), [Scalar::from(125_i64), Scalar::from(126_i64)])?;

    // One column, both directions, nothing copied. A run names no Arrow
    // layout, so the buffers answer for a column and `None` for a run.
    let array = serie.into_arrow_array().expect("a column has buffers");
    assert_eq!(array.len(), 2);
    assert_eq!(Serie::from_arrow_array(field, array)?, serie);
    assert_eq!(Serie::new(vec![Scalar::from(1_i64)]).into_arrow_array(), None);

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
- `Serie::as_slice` and `Scalar::as_sequence` borrow, so they answer only for the schema-free run: a column holds Arrow buffers and no `Scalar`, so there is nothing to borrow. `Serie::rows` reads either — borrowing the run's values, building the column's — and `Scalar::iter` walks either, yielding `Cow`.
- `Serie::field` answers `None` for a run, because a run declares none. `Serie::require_field` is the same read as a refusal, and it is what a record's child, a sequence's items and a mapping's entries go through — a run cannot stand where Arrow names a layout.
- Nothing caches a decoded row. Reading a column's rows twice reads them twice; hold the answer rather than asking again.
- The value contract reads a column the way it reads a run: `Field::scalar` on a `list(...)` field accepts one, leaves it alone when there is nothing to rewrite, and rewrites its rows into the run the declaring field asked for when there is. From there the declaring field is the authority, which is why a rewritten column comes back a run.
- `Scalar::get` and `scalar[i]` borrow a stored row, so they answer for the schema-free run and not for a column, which stores none — `value.get(0)` on a column is `None` and `value[0]` panics, the way they do on any value that is not a run. `Serie::scalar(i)` builds that one row, and `Scalar::iter` walks them all.
- A walk has nowhere to report a refusal, so a row a column's field refuses reads as `Scalar::Null` there; `Serie::scalar` and `Serie::scalars` return the refusal instead.
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
