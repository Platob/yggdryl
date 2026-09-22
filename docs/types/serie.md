# Serie

Many values: a schema-free run, or the Arrow buffers of one [`Field`](field.md). `Serie` is what the sequence family holds, so there is one type for "many values" and not two.

## Contract

| Aspect | Rule |
| --- | --- |
| What it is | The fourth side of the value model: [`DataType`](datatype.md) is the shape, [`Field`](field.md) the schema, [`Scalar`](scalar.md) one value, `Serie` many of them |
| Two leaves | `Serie::Run` is a schema-free ordered run - what a row canonicalizes to - and holds its values in one shared slice; every other leaf is a column and holds Arrow buffers. What separates them is the field: a run declares none |
| Storage | A column stores a values buffer, offsets where the layout has them, and a validity bitmap. No `Scalar` is stored anywhere in a column; a row is built when one is asked for and kept nowhere |
| Recursion | A record's children, a sequence's items, a mapping's entries, a dictionary's keys and values, a run-end column's ends and values, a union's members are each a `Serie`, so the nesting is one type all the way down |
| Size | 24 bytes: the run's slice inline, because a row canonicalizes to one; every column leaf behind one shared pointer, so a `Scalar` carrying a column is two words and a clone is a pointer bump |
| Shape | A family enum over one leaf per Arrow layout, spelled as `DataType`'s families are: `Int32Serie` lends `&[i32]`, `Utf8StringSerie` lends the offsets and the characters, `StructSerie` holds one child `Serie` per child field |
| Invariant 1 | A column holds only rows its field accepts: `from_scalars`, `splice` and every typed writer prove values through the field's contract, and the Arrow door proves them at import. A stored row never refuses to be read; `scalar(i)`'s one refusal is an index past the end |
| Invariant 2 | A nested column's children are aligned: every record child has exactly `len` rows, a list's offsets are monotone from `0` to `items.len()`, a fixed-size list's items hold `len * width` rows, a mapping's entries are a record column of the entries field, a union's members hold the rows its type ids reach. No public path hands a child out mutably, so nothing can break it and `into_arrow_array` cannot fail |
| Invariant 3 | Every offsets buffer is rebased: the first offset is `0` and the last is `items.len()`, so a column that grows knows where its items end |
| Writes | One mutation, `splice`; every other write is spelled over it. A column proves once, checks once, then writes without failing, so a refusal leaves it exactly as it was. A write is in place when the column holds its buffers alone and copies them once when it does not |
| Identity | The rows, and nothing else: a run and a column of equal rows are one value and hash alike, and so are an int32 column and an int64 column of equal numbers, exactly as their `Scalar`s are. Not the leaf, not the field |
| Datatype | For a column, `list(<the field named item>)` - read off the field, so an empty column still names it. For a run, agreed back out of its rows |
| Registration | `Scalar::Sequence(Serie)`. It adds no `DataTypeId`, no `DataType` variant and no `Field` variant, and `kind()` answers `sequence` for either leaf |
| Family | `SerieValue`, implemented by every column leaf and every family enum. `Serie` itself does not implement it, because a run has no field to answer with; the root answers the same verbs inherently, with `field()` an `Option` |
| Wire | The crate's own serde writes a run as its list and a column as `{"field": .., "rows": [..]}` under the `serie` tag, and reads back only that column wire; JSON, YAML and TOML write the rows alone, because a codec document carries no schema envelope |
| Not `ArrowScalar` | That is the shape wrapper a value takes crossing the Arrow boundary, holding its payload opaquely; a serie is the column itself, the buffers a caller reads and writes as a collection. The two share one layout proof and nothing else |
| Bindings | Rust only. A column crosses a binding as an Arrow array or record batch, and `ArrowScalar` is what carries one there |

## The leaves

A leaf is one Arrow layout under one field, and its accessors are that layout's own components, lent where they lie. A typed writer exists only where the native domain is the datatype's whole domain, and it validates what remains - nullability - refusing `None` under a required field by name; everywhere else `push`, `set` and `splice` with a `Scalar` are the one writer, because the field's contract is narrower than the storage.

| Family | Leaves | What the leaf lends | Typed writes |
| --- | --- | --- | --- |
| `IntegerSerie` | `Int8Serie` .. `UInt64Serie` | `values() -> &[T]`, `value(i) -> Option<T>`, `nulls()`, `array()` | `push_value`, `set_value`, `splice_values`, `extend_values` over `Option<T>` |
| `FloatingSerie` | `Float16Serie`, `Float32Serie`, `Float64Serie` | the same | the same |
| `DecimalSerie` | `Decimal32Serie` .. `Decimal256Serie` | the coefficients, at the field's scale | none: precision is narrower than the width |
| `TemporalSerie` | `Date32Serie` .. `IntervalMonthDayNanoSerie` | the counts, at the field's unit | `Date32`, every `DateTime`, `Duration` and `Interval` leaf writes natively; `Date64`, `Time32`, `Time64` do not, because whole days and a time of day are narrower than the storage |
| `BooleanSerie` | - | `values() -> &BooleanBuffer`, `value(i)`, `nulls()`, `array()` | `push_value`, `set_value`, `splice_values` over `Option<bool>` |
| `NullSerie` | - | `array()`, built on demand: a null column is a length | `push_value()`, `splice_values(range, count)` |
| `StringSerie` | `Utf8StringSerie`, `LargeUtf8StringSerie`, `Utf8ViewStringSerie`, `BinaryStringSerie`, `LargeBinaryStringSerie`, `BinaryViewStringSerie`, `FixedStringSerie` | `offsets()` and `payload()`, or `views()` and `payloads()` for a viewed leaf, `width()` and `payload()` for a fixed one; `value(i)`, `nulls()`, `array()` | none: codes, charsets and sizes are narrower than the bytes |
| `BytesSerie` | `BinarySerie`, `LargeBinarySerie`, `BinaryViewSerie`, `FixedBytesSerie` | the same | none: a UUID and well-known binary are narrower than the bytes |
| `StructSerie` | - | `children()`, `child(name)`, `child_at(i)`, `nulls()` | `set_child(child)`, `set_cell(path, i, value)`, `without_child(name)` |
| `SequenceSerie` | `ListSerie`, `LargeListSerie`, `ListViewSerie`, `LargeListViewSerie`, `FixedSizeListSerie` | `items()`, `offsets()` at the leaf's own width - and `sizes()` for a view, `width()` for a fixed size - `range(i)`, `row(i)` (the item column sliced to that row, zero copy), `nulls()` | none: a row is a sequence, and the items are not mutably reachable |
| `MappingSerie` | - | `entries()` - a record column of the entries field - `keys()`, `values()`, `offsets()`, `range(i)`, `row(i)`, `nulls()`, `keys_sorted()` | none |
| `EnumSerie` | `DictionarySerie` | `keys()` - an integer column whose validity is the column's - `values()`, `array()` built on demand | none: a write rebuilds |
| `RunEndEncodedSerie` | - | `run_ends()`, `values()`, `nulls()` | none: a write rebuilds |
| `UnionSerie` | - | `type_ids()`, `offsets()` (`None` for sparse), `children()`, `child_of(type_id)`, `mode()` | none: a write rebuilds |
| `VariantSerie` | - | `metadata(i)`, `value(i)` - the Parquet [Variant](variant.md) pair, neither run read - `metadata_array()`, `value_array()`, `nulls()` | none: the pair is validated by `Variant::new`, and `push(Scalar::Variant(..))` is the writer |

Arrow spells one binary layout for text and for bytes, so `ByteSerie<T, K>`, `ByteViewSerie<T, K>` and `FixedSerie<K>` are each one implementation under two markers, `Chars` and `Octets`: `Utf8StringSerie` and `BinarySerie` are two names over identical buffers, and the field - a `utf8`, a `currency`, a windows-1252 leaf, a `uuid` - is what says what a row means. The width and the shape are the leaf, so nothing branches on either per row, and narrowing to another leaf answers `None`.

A mapping's rows are `Scalar::Mapping`, and its entries column is a `StructSerie` of the entries field: `Field::scalar` on a map field answers a mapping whose keys and values are already canonical under the entries record's two fields, so a write turns each pair into a two-cell run for the entries record's own write, and `scalar(i)` pairs them back.

A null row in a nested column is a cleared validity bit and what Arrow needs underneath it: every record child receives one placeholder slot, a list, list-view or map row cuts zero items, a fixed-size-list row holds `width` placeholder items, a union row the placeholder member. A column built by pushes is therefore buffer-for-buffer the column `from_scalars` builds from the same rows.

## Reads and writes

Every verb answers on both leaves; only its cost differs.

| Verb | Contract |
| --- | --- |
| `field()`, `field_ref()`, `require_field()` | `None` for a run; `require_field` refuses a run by name |
| `len()`, `is_empty()`, `null_count()` | constant; a run walks its values for `null_count`, since it keeps no bitmap |
| `is_null(i)`, `scalar(i)` | one row; refused past the end naming the serie and both counts |
| `get(i)` | `None` past the end; borrowed for a run, built for a column - the one lookup that allocates on a column, by contract |
| `rows()` | `Cow<[Scalar]>`: a run lends, a column builds every row and keeps none |
| `iter()` | `Cow<Scalar>` per row: lent for a run, built one at a time for a column |
| `dtype()` | read off a column's field; agreed out of a run's rows |
| `as_run()`, `as_slice()`, `is_column()`, `into_run()` | the run's values, `None` for a column; `into_run` builds a column's rows once, the one direction that drops the field, spelled rather than implied |
| `slice(offset, length)` | zero copy for a column, offsets rebased; a run copies its window |
| `child(name)`, `child_at(i)`, `children()` | a record column's children, a union's members; empty elsewhere |
| `items()` | a sequence column's items, a mapping's entries, an encoding's values; `None` elsewhere |
| `get_child_by_path(path)` | exactly `DataType::get_field_by_path`'s segments: a record child by name, a sequence transparent to its item, a mapping through its entries field; an index, key, range or predicate segment reaches no column |
| `as_<leaf>()` / `get_<leaf>_mut()` | one pair per family and per named leaf, `as_int64` to `as_dictionary`; the borrow allocates nothing, the mutable one copies the leaf struct once when the column is shared |
| `splice(range, rows)` | the one mutation: `range` replaced by `rows`, refused when reversed or past the end |
| `set(i, v)`, `push(v)`, `insert(i, v)`, `remove(i)`, `pop()` | spelled over `splice`; `remove` and `pop` read the row first |
| `truncate(len)`, `clear()`, `extend(rows)`, `resize(len, v)` | `clear` keeps the field; `resize` proves `v` once and writes the clones |
| `extend_from_serie(other)` | two columns whose datatypes agree and whose nullability fits append buffer to buffer with no row read; anything else reads `other`'s rows |
| `set_child(child)`, `set_cell(path, i, v)` | a record column only: replace or add a child of `len` rows; write one cell `path` deep in place, every level row-aligned |

Construction is `new(values)` for a run; `empty(field)`, `with_capacity(field, rows)` and `from_scalars(field, rows)` for a column; `from_arrow_array`, `from_arrow_batch` and `from_arrow_reader` for buffers already holding it. `Serie` is `Default` (the empty run), `FromIterator<Scalar>` (a run in one allocation), `From<Run>`, and `From<Serie> for Scalar`.

## What each ask costs

| Ask | Cost |
| --- | --- |
| `len`, `is_null`, `is_empty`, `null_count` | constant for a column, off its buffers; `null_count` walks a run |
| `values`, `offsets`, `payload`, `views`, `nulls`, `array` | constant, and nothing is copied: these are the buffers themselves |
| `value(i)` on a leaf | a bounds check and a buffer read; no value is built |
| `scalar(i)`, `get(i)` | one value built, through the crate's one schema-directed decode |
| `rows`, `into_run`, and walking a column as a value | one row built per row, every time; nothing is cached, so hold the answer rather than asking twice |
| `push`, `extend`, `push_value`, `extend_values` on a primitive, boolean, byte or fixed leaf | in place, amortized, once the leaf owns its buffer; the first edit on a foreign buffer - a kernel's, an IPC reader's, a shared or sliced one - copies it once |
| `set`, `set_value` on a primitive or boolean leaf | one buffer write, one bit write; a row that gains or loses its absence rebuilds the validity bitmap |
| `set` on a byte leaf | a rebuild from `i` on, every time: a run is as long as it is, so every later offset moves |
| any write on a viewed leaf | one rewrite of the views: no builder hands views back |
| general `splice` on a buffer leaf | one pass through the leaf's own builder - prefix, replacement, suffix |
| any write on a nested column | the offsets re-cut from `range.start`, the items spliced in place, the validity spliced, the children written one slot each |
| any write on a dictionary column | the vocabulary read once into a map, the values it does not hold yet appended to the values column, the keys spliced in place - so a push of a value already held moves one key |
| any write on a run-end or union column | a rebuild: the replacement laid out, joined, and taken back through the door already proven |
| `push`, `set`, `splice` on a run | a copy of every value it holds: a run is one shared slice, so building one `push` at a time is quadratic, and `Scalar::from_sequence` or `Serie::new` builds it in one allocation |
| a write on a shared column | the leaf struct copied once (pointer bumps: its buffers are shared) and the first buffer edit copying the rows once; every later edit in place |
| `child`, `child_at`, `children`, `items` | constant: a child is already a column |
| `set_child`, `without_child` | one field edit and one `Vec` of pointers, never a row |
| `slice` | zero copy on a column; a copy of the window on a run |
| `into_arrow_array` | the array itself, shared |
| `into_arrow_batch`, `into_arrow_reader` | one batch of the children's own arrays, no row decoded |
| `from_arrow_array` | the projection compared, the validity words counted, and - only where the datatype is narrower than its layout - each row read once |
| `from_arrow_reader` | the whole stream held: a column is one contiguous set of buffers, so the bound is the stream itself |

## Use

A column is built from a field and either the rows it types or the buffers that already hold them. Values go through the field's value contract once and are laid out once; buffers are proven by their layout and their nullability and cross without a copy.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{DataType, Field, Scalar, Serie};

    let field = Field::new("price", DataType::Int64, false);

    // Values in: three widths, one width out, each row through `Field::scalar`.
    let serie = Serie::from_scalars(
        field.clone(),
        [Scalar::from(125_i32), Scalar::from(126_u8), Scalar::from(127_i64)],
    )?;
    assert_eq!(serie.len(), 3);
    assert_eq!(serie.scalar(0)?, Scalar::from(125_i64));

    // Buffers in: nothing copied, and nothing read, because an int64 layout
    // is the whole of what an int64 field asks.
    let array: ArrayRef = Arc::new(Int64Array::from(vec![125, 126, 127]));
    let held = Serie::from_arrow_array(field.clone(), array)?;
    assert_eq!(held.as_int64().expect("an int64 column").values(), &[125, 126, 127]);

    // Either way it is the same column.
    assert_eq!(held, serie);

    // The datatype is read off the field, not agreed back out of the rows,
    // so an empty column names it too.
    assert_eq!(
        Serie::empty(field.clone())?.dtype()?,
        DataType::list(field.clone().with_name("item"))
    );

    // A row the field refuses refuses the column, naming the field, and
    // nothing is built.
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

The typed accessors are the buffers themselves: reading row `i` off one is a bounds check and a load. A typed writer stores straight into that buffer and validates the one thing the field still has to say about a native value, whether it may be absent; the value doors sit above it and add the rest of the field's contract.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array, StringArray};
    use yggdryl::{DataType, Field, Scalar, Serie};

    let field = Field::new("price", DataType::Int64, true);
    let array: ArrayRef = Arc::new(Int64Array::from(vec![125, 126]));
    let mut serie = Serie::from_arrow_array(field, array)?;

    // Straight off the values buffer: no value built.
    let prices = serie.as_int64().expect("an int64 column");
    assert_eq!(prices.values(), &[125, 126]);
    assert_eq!(prices.value(1), Some(126));
    assert!(prices.nulls().is_none());

    // A typed write lands in that same buffer; the field admits an absent
    // row, so `None` is one.
    let prices = serie.get_int64_mut().expect("an int64 column");
    prices.push_value(Some(127))?;
    prices.push_value(None)?;
    assert_eq!(prices.values().len(), 4);
    assert_eq!(prices.value(3), None);
    assert_eq!(serie.null_count(), 1);

    // The value door writes the same buffer through the field, which
    // narrows the width; one slot is rewritten where it lies.
    serie.set(3, Scalar::from(128_i16))?;
    assert_eq!(serie.as_int64().expect("an int64 column").values(), &[125, 126, 127, 128]);
    assert_eq!(serie.null_count(), 0);

    // A required field refuses an absent native value by name, and the
    // column is left as it was.
    let mut required =
        Serie::from_scalars(Field::new("size", DataType::Int64, false), [Scalar::from(1_i64)])?;
    let refusal = required
        .get_int64_mut()
        .expect("an int64 column")
        .push_value(None)
        .unwrap_err();
    assert!(refusal.to_string().contains("size"), "{refusal}");
    assert_eq!(required.len(), 1);

    // Text lends its two components rather than its rows.
    let symbols: ArrayRef = Arc::new(StringArray::from(vec!["AAPL", "MSFT"]));
    let text = Serie::from_arrow_array(Field::new("symbol", DataType::utf8(), false), symbols)?;
    let leaf = text.as_utf8().expect("a utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 4, 8]);
    assert_eq!(leaf.payload().as_slice(), b"AAPLMSFT");
    assert_eq!(leaf.value(1), Some("MSFT"));
    ```

=== "Python"

    ```python
    # Rust only.
    ```

=== "JavaScript"

    ```javascript
    // Rust only.
    ```

## Children

A record column is made of child columns, and Arrow already holds each one separately. Reaching one is a borrow, replacing or dropping one moves pointers and never a row, and writing one cell descends the record levels by name to the leaf that proves the value. No child is handed out mutably: that is what keeps every child at exactly `len` rows, and what lets the Arrow array assemble without a refusal clause.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{DataType, Field, FieldPath, Scalar, Serie, StructType};

    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), false),
        ])?),
        false,
    );
    let mut records = Serie::from_scalars(
        root,
        [
            Scalar::from_struct([("id", Scalar::from(1_i64)), ("symbol", Scalar::from("AAPL"))])?,
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("MSFT")]),
        ],
    )?;

    // One child is a column of its own field, over its own buffers.
    let symbols = records.child("symbol").expect("a named child");
    assert_eq!(symbols.field().map(Field::name), Some("symbol"));
    assert_eq!(symbols.as_utf8().expect("a utf8 column").value(1), Some("MSFT"));
    assert_eq!(records.children().len(), 2);

    // One cell of one row, proven by the leaf's field and written in place.
    records.set_cell(&FieldPath::from_str("id")?, 1, Scalar::from(20_i8))?;
    let ids = records.child("id").expect("a named child");
    assert_eq!(ids.as_int64().expect("an int64 column").values(), &[1, 20]);

    // Adding a child extends the field; replacing one moves a pointer.
    let volumes: ArrayRef = Arc::new(Int64Array::from(vec![10_i64, 20]));
    let volume = Serie::from_arrow_array(Field::new("volume", DataType::Int64, false), volumes)?;
    records.set_child(volume)?;
    assert_eq!(records.field().expect("a column").field_len(), 3);

    // A run, or a column of another length, is refused by name and nothing moves.
    assert!(records.set_child(Serie::new(vec![Scalar::from(1_i64)])).is_err());
    assert_eq!(records.children().len(), 3);

    // Dropping a child takes its field with it; every other child is shared.
    let without = records.as_struct().expect("a record column").without_child("symbol")?;
    assert!(without.child("symbol").is_none());
    assert_eq!(without.children().len(), 2);
    ```

=== "Python"

    ```python
    # Rust only.
    ```

=== "JavaScript"

    ```javascript
    // Rust only.
    ```

## The nesting points back here

A record's children, a sequence's items and a mapping's entries are each a `Serie` - the same type, all the way down - and every step down carries its own field. A column of records is therefore columns of columns, and reaching a leaf three levels deep is three borrows and no copy. A list row is the item column sliced to that row, and a path reaches a column exactly as it reaches a field.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, FieldPath, Scalar, Serie, StructType};

    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("tags", DataType::list(Field::new("item", DataType::utf8(), false)), true),
        ])?),
        false,
    );
    let mut column = Serie::from_scalars(
        root,
        [
            Scalar::from_struct([
                ("id", Scalar::from(1_i64)),
                ("tags", Scalar::from_sequence([Scalar::from("a"), Scalar::from("b")])),
            ])?,
            Scalar::from_struct([("id", Scalar::from(2_i64)), ("tags", Scalar::Null)])?,
        ],
    )?;

    // Every step down answers a `Serie`, and every one carries its own field.
    let tags = column.child("tags").expect("a named child");
    let lists = tags.as_list().expect("a list column");
    assert_eq!(lists.offsets().as_ref(), &[0, 2, 2]);
    assert_eq!(lists.range(0), Some(0..2));
    assert_eq!(lists.range(1), None, "an absent row cuts nothing");
    let items = lists.items();
    assert_eq!(items.field().map(Field::name), Some("item"));
    assert_eq!(items.as_utf8().expect("a utf8 column").value(1), Some("b"));

    // One row is the item column sliced to it, zero copy.
    assert_eq!(lists.row(0).expect("a present row").rows().len(), 2);

    // A path reaches the same column the schema walk reaches: a list is
    // transparent to its item.
    let reached = column.get_child_by_path(&FieldPath::from_str("tags.item")?);
    assert_eq!(reached.and_then(Serie::field).map(Field::name), Some("item"));

    // A row set with another item count re-cuts the offsets; every later
    // row reads unchanged.
    column.set_cell(&FieldPath::from_str("tags")?, 0, Scalar::from_sequence([Scalar::from("c")]))?;
    let lists = column.child("tags").expect("a named child").as_list().expect("a list column");
    assert_eq!(lists.offsets().as_ref(), &[0, 1, 1]);
    assert_eq!(lists.items().len(), 1);
    assert!(column.child("tags").expect("a named child").is_null(1)?);

    // A schema-free run is the same type, and the one leaf with no field.
    let run = Serie::new(vec![Scalar::from(1_i64)]);
    assert_eq!(run.field(), None);
    assert!(run.require_field().is_err());
    assert!(!run.is_column());
    ```

=== "Python"

    ```python
    # Rust only.
    ```

=== "JavaScript"

    ```javascript
    // Rust only.
    ```

## A column is a value

`Serie` registers in the sequence family, so a column needs no second reader anywhere: every accessor that answers a sequence answers a column. Identity is the rows alone - a run and a column of equal rows are one value, hash alike, and order alike, whichever field types them - so a `Vec<Int32Serie>` and the `Vec<Serie>` holding the same columns sort the same way. A column holds no rows and keeps none: each row is built as it is reached, so reading a column leaves it exactly as it was. The one thing it cannot do is lend a slice it does not have - `as_sequence` and `as_slice` borrow, so they answer `None` for a column, and a walk yields `Cow`, borrowed for a run and owned for a column.

=== "Rust"

    ```rust
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use yggdryl::{DataType, Field, Scalar, Serie};

    fn hashed(value: &impl Hash) -> u64 {
        let mut state = DefaultHasher::new();
        value.hash(&mut state);
        state.finish()
    }

    let column = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(125_i64), Scalar::from(126_i64)],
    )?;
    let run = Serie::new(vec![Scalar::from(125_i64), Scalar::from(126_i64)]);
    let narrow = Serie::from_scalars(
        Field::new("size", DataType::Int32, true),
        [Scalar::from(125_i32), Scalar::from(126_i32)],
    )?;

    // Identity is the rows: not the leaf, not the field, not the width.
    assert_eq!(column, run);
    assert_eq!(column, narrow);
    assert_eq!(hashed(&column), hashed(&run));
    assert!(Serie::new(vec![Scalar::from(125_i64)]) < column);

    // As a value it is the sequence it is, for either leaf.
    let value = Scalar::from(column.clone());
    assert_eq!(value.kind(), "sequence");
    assert_eq!(value.len(), 2);
    assert_eq!(value, Scalar::from(run.clone()));

    // A walk answers every row, built as each is reached; `get` builds one.
    assert_eq!(
        value.iter().map(|row| row.into_owned()).collect::<Vec<_>>(),
        vec![Scalar::from(125_i64), Scalar::from(126_i64)]
    );
    assert_eq!(value.get(1).as_deref(), Some(&Scalar::from(126_i64)));

    // What a column cannot do is lend a row it does not store: `as_sequence`
    // borrows, `sequence_rows` reads either leaf, `as_serie` reaches the column.
    assert_eq!(value.as_sequence(), None);
    assert_eq!(Scalar::from(run.clone()).as_sequence().map(<[Scalar]>::len), Some(2));
    assert_eq!(value.sequence_rows().map(|rows| rows.len()), Some(2));
    assert!(value.as_serie().is_some_and(Serie::is_column));

    // Reading keeps nothing, and the buffers are untouched by it.
    assert_eq!(column.as_slice(), None);
    assert_eq!(column.rows().len(), 2);
    assert_eq!(column.as_int64().expect("an int64 column").values(), &[125, 126]);

    // Dropping the field is spelled, never implied.
    assert_eq!(column.into_run(), run.into_run());
    ```

=== "Python"

    ```python
    # Rust only.
    ```

=== "JavaScript"

    ```javascript
    // Rust only.
    ```

## A mapping is a cut over its entries

Arrow lays a mapping out as a list of non-null key-value records, and so does the column: a field, offsets, one record column of the entries field, and a validity bitmap. What a row means is the field's: `Field::scalar` on a map field takes a mapping, the write stores each pair as a two-cell record, and `scalar(i)` pairs the records back into the mapping that went in.

=== "Rust"

    ```rust
    use arrow_array::{Array, MapArray};
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};

    let entries = Field::new(
        "entries",
        DataType::from(StructType::from_fields([
            Field::new("key", DataType::utf8(), false),
            Field::new("value", DataType::Int64, false),
        ])?),
        false,
    );
    let field = Field::new("weights", DataType::map(entries, false)?, false);
    let column = Serie::from_scalars(
        field.clone(),
        [Scalar::from_mapping([
            (Scalar::from("AAPL"), Scalar::from(1_i64)),
            (Scalar::from("MSFT"), Scalar::from(2_i64)),
        ])?],
    )?;

    // One row in, the same row out.
    assert_eq!(column.scalar(0)?.kind(), "mapping");

    // Underneath, the entries are a record column of the entries field, and
    // the keys and the values are its two children.
    let maps = column.as_mapping().expect("a mapping column");
    assert_eq!(maps.range(0), Some(0..2));
    assert!(!maps.keys_sorted(), "read off the field");
    assert_eq!(maps.keys().as_utf8().expect("a utf8 column").value(1), Some("MSFT"));
    assert_eq!(maps.values().as_int64().expect("an int64 column").values(), &[1, 2]);
    assert_eq!(column.items().and_then(Serie::field).map(Field::name), Some("entries"));

    // Arrow takes it as a map, and the buffers come back without a row read.
    let array = column.into_arrow_array().expect("a column has buffers");
    assert!(array.as_any().downcast_ref::<MapArray>().is_some());
    assert_eq!(Serie::from_arrow_array(field, array)?, column);
    ```

=== "Python"

    ```python
    # Rust only.
    ```

=== "JavaScript"

    ```javascript
    // Rust only.
    ```

## Arrow: the door, and what it proves

Every crossing shares buffers. `from_arrow_array(field, array)` takes an array as the column of `field` and proves three things at the door, so that invariant 1 holds for a column that was never built from values:

- the layout: the array's datatype is the field's own Arrow projection, exactly - an array laid out otherwise is refused by name rather than reconciled, and [`cast`](cast.md) is what reshapes it;
- the absence: a required field refuses absent rows, counted on the validity words, and a record's children are judged only where the record itself is present, because a null record row leaves its children's slots unspecified; a dictionary, run-end or union column is judged on its logical nulls;
- the values: where the datatype's layout is its whole contract - null, boolean, every integer and float width, `date32`, every datetime, duration and interval, the plain `utf8` leaves, the plain byte leaves, `uuid`, and every nesting of those - no row is read. Everywhere else - a code, an ASCII or windows-1252 or sized string, a decimal, `date64`, a time, a URL, a version, a variant - each row of that leaf is read once and dropped, and the first one the field's own contract refuses is named with its column and its row.

A sliced list's offsets are rebased onto exactly the items they reach, so the column knows where its items end when it grows. A column the crate itself laid out - `from_scalars`, whose rows went through `Field::scalar` - takes the door already proven and no row is read a second time.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array, StringArray};
    use yggdryl::{DataType, Field, Serie};

    // Absence is judged on the validity words, under the field's nullability.
    let absent: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
    let refusal = Serie::from_arrow_array(Field::new("size", DataType::Int64, false), absent.clone())
        .unwrap_err();
    assert!(refusal.to_string().contains("size"), "{refusal}");
    assert_eq!(
        Serie::from_arrow_array(Field::new("size", DataType::Int64, true), absent)?.null_count(),
        1
    );

    // A layout that is not the field's projection is refused, never reconciled.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AB", "ABCD"]));
    assert!(Serie::from_arrow_array(Field::new("size", DataType::Int64, false), text.clone()).is_err());

    // The plain UTF-8 layout is its own contract, so nothing is read; a
    // sized string is narrower than its storage, so each row is read once
    // and the first the field refuses is named.
    assert!(Serie::from_arrow_array(Field::new("code", DataType::utf8(), false), text.clone()).is_ok());
    let refusal =
        Serie::from_arrow_array(Field::new("code", DataType::sized_utf8(2)?, false), text).unwrap_err();
    assert!(refusal.to_string().contains("code"), "{refusal}");
    assert!(refusal.to_string().contains("row 1"), "{refusal}");
    ```

=== "Python"

    ```python
    # Rust only.
    ```

=== "JavaScript"

    ```javascript
    // Rust only.
    ```

## Arrow: an array, a batch, a reader

A column of a leaf field is an array; a column of a non-null Struct field is a table, and the stream of it is what a record write already speaks. Coming the other way, the field decides which leaf the buffers land in. A run names no Arrow layout, so it answers `None` for the array and is refused by name where one is required.

`arrow::ArrowScalar` is where `Serie` ends: the shape wrapper a value takes when it crosses the Arrow boundary - one scalar, array, batch or stream under one field, with the cast door and the shape queries, holding its payload opaquely and answering `None` to every native accessor. `Serie` is the column itself. The two share one layout proof and no bridge method: `Serie::from_arrow_array(field, held.into_array()?)` reaches the column from a held array, and `ArrowScalar::from_array(field, serie.require_arrow_array()?)` crosses back.

=== "Rust"

    ```rust
    use arrow_array::{Array, RecordBatchReader};
    use yggdryl::{ArrowScalar, DataType, Field, Scalar, Serie, StructType};

    let field = Field::new("price", DataType::Int64, false);
    let serie = Serie::from_scalars(field.clone(), [Scalar::from(125_i64), Scalar::from(126_i64)])?;

    // One column, both directions, nothing copied.
    let array = serie.into_arrow_array().expect("a column has buffers");
    assert_eq!(array.len(), 2);
    assert_eq!(Serie::from_arrow_array(field.clone(), array)?, serie);
    assert_eq!(Serie::new(vec![Scalar::from(1_i64)]).into_arrow_array(), None);
    assert!(Serie::new(vec![Scalar::from(1_i64)]).require_arrow_array().is_err());

    // The shape wrapper is reached through the two doors, and no third exists.
    let crossing = ArrowScalar::from_array(field.clone(), serie.require_arrow_array()?)?;
    assert!(crossing.is_array());
    assert_eq!(Serie::from_arrow_array(field, crossing.into_array()?)?, serie);

    // The rows of a record root: a table, and the stream of it.
    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), false),
        ])?),
        false,
    );
    let rows = Serie::from_scalars(
        root,
        [Scalar::from_struct([("id", Scalar::from(1_i64)), ("symbol", Scalar::from("AAPL"))])?],
    )?;
    let batch = rows.into_arrow_batch()?;
    assert_eq!((batch.num_rows(), batch.num_columns()), (1, 2));
    assert_eq!(Serie::from_arrow_batch(&batch)?, rows);

    // A reader states its schema before it is pulled; one held column is
    // one batch, and a stream of any number of them drains into one column.
    let reader = rows.into_arrow_reader()?;
    assert_eq!(reader.schema().fields().len(), 2);
    assert_eq!(Serie::from_arrow_reader(rows.into_arrow_reader()?)?, rows);

    // A column of a leaf field is not a table, and says so by name.
    assert!(serie.into_arrow_batch().is_err());
    ```

=== "Python"

    ```python
    # Rust only. `ArrowScalar` is what carries a column across a binding.
    ```

=== "JavaScript"

    ```javascript
    // Rust only. `ArrowScalar` is what carries a column across a binding.
    ```

## Encodings cross and rebuild

A dictionary, run-end or union column holds its encoding as columns - the keys and the values, the run ends and the values, the type ids and one member each - and reads a row through it. Its absence is logical and counted once. None has a typed writer. A dictionary write interns: the rows are looked up in the vocabulary, the values it does not hold yet are appended to it, and the keys are spliced in place, so the vocabulary never holds a value twice on the column's account. A run-end or union write lays the replacement out, joins it, and takes the result back through the door already proven.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::types::Int8Type;
    use arrow_array::{ArrayRef, DictionaryArray, Int8Array, StringArray};
    use yggdryl::{DataType, Field, Scalar, Serie};

    let field = Field::new("symbol", DataType::dictionary(DataType::Int8, DataType::utf8())?, true);
    let array: ArrayRef = Arc::new(DictionaryArray::<Int8Type>::try_new(
        Int8Array::from(vec![Some(0), Some(1), None, Some(0)]),
        Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
    )?);
    let mut column = Serie::from_arrow_array(field, array)?;

    // The keys and the values are each a column; absence is the keys'.
    let encoded = column.as_dictionary().expect("a dictionary column");
    assert_eq!(encoded.keys().len(), 4);
    assert_eq!(encoded.values().len(), 2);
    assert_eq!(column.null_count(), 1);
    assert!(column.is_null(2)?);
    assert_eq!(column.scalar(1)?, Scalar::from("MSFT"));

    // A write interns: a value already held moves one key and the vocabulary
    // stays two long.
    column.push(Scalar::from("AAPL"))?;
    assert_eq!(column.len(), 5);
    assert_eq!(column.scalar(4)?, Scalar::from("AAPL"));
    assert_eq!(column.as_dictionary().expect("a dictionary column").values().len(), 2);
    assert!(column.into_arrow_array().is_some_and(|array| array.as_any().is::<DictionaryArray<Int8Type>>()));
    ```

=== "Python"

    ```python
    # Rust only.
    ```

=== "JavaScript"

    ```javascript
    // Rust only.
    ```

## Edges

- A row the field refuses refuses the whole write, naming the field, and the column is left as it was; a nullable field is what admits `Scalar::Null`.
- `splice` proves every row before it checks, and checks before it writes: a record-of-(int64, utf8) splice whose utf8 child would overflow 32-bit offsets is refused, and every child's buffers are the ones they were.
- A typed writer exists only on a leaf whose native domain is the datatype's whole domain, and it still validates nullability: `push_value(None)` under a required field is refused by name. A decimal, `date64`, `time32` or `time64` column has none, because its rule is narrower than its storage; a byte, string, sequence, mapping or variant column has none for the same reason.
- No child is handed out mutably. `set_child` replaces a whole child of exactly `len` rows and `set_cell` writes one slot through the leaf's field, so a record's children stay aligned and `into_arrow_array` never refuses.
- `set_cell` on a row whose record is absent at any level is refused: set the whole row.
- An Arrow array whose physical datatype is not the field's is refused rather than reconciled; reshape it with [`cast`](cast.md) first. An array carrying absent rows under a required field is refused at every level, a record's children judged only where the record itself is present.
- The door reads a row only where the datatype is narrower than its layout, and then reads each row of that leaf exactly once; a column crossing back is never read.
- A list, list-view or map array that was sliced crosses with its offsets rebased onto the items it reaches; a list-view column's write compacts, so the written column's offsets are contiguous.
- A dictionary write interns into its vocabulary and moves keys in place, so a vocabulary that outgrows its key width is refused by name before anything moves; a run-end or union write is a rebuild, not an edit, so write those columns in bulk.
- `from_arrow_reader` drains: a column is one contiguous set of buffers, so the bound is the stream itself. Keep rows a stream with [`IOMedia::read_arrow_reader`](../holder/index.md) when they should stay one.
- `into_arrow_batch`, `into_arrow_reader` and `from_arrow_batch` need a non-null Struct root; anything else is refused by name. A batch read back names its root `row`, because Arrow names columns and never the record.
- A run is one shared slice: every write copies it, so building one `push` at a time is quadratic. `Scalar::from_sequence` and `Serie::new` build one from values in hand, in one allocation.
- A column and a run of equal rows are equal and hash alike, and so are two columns of equal rows under different fields or widths: identity is the rows and nothing else, exactly as a `Scalar`'s is.
- `Serie::as_slice` and `Scalar::as_sequence` borrow, so they answer only for the run; `Serie::rows`, `Scalar::sequence_rows`, `get` and `iter` read either, building a column's rows and keeping none. Reading a column's rows twice reads them twice.
- `Field::scalar` on a `list(...)` field accepts a column and leaves it untouched when its field's datatype is the item's and its nullability fits; otherwise it walks the rows and answers a run. A row is always a run: row canonicalization reads a column through `sequence_rows` and never stores one.
- `Serie: Deserialize` accepts only the column wire; a list under the `serie` tag is refused naming the tag. A run is never spelled through `Serie`'s own serde: it is the `sequence` tag of `Scalar`.
- A clone shares the buffers. Writing one of two clones copies the rows once and the two go their own way, which is what makes a column a value rather than a handle; every later write to the owner is in place.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- serie sequence
    cargo test --features "internals parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test serie
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test allocations -- sequence column leaf
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
