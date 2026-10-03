# Protocol

Field metadata the library reads: reserved keys, `SCHEME:name` properties behind live views, and the `FIELD:partition` marker.

## Contract

| Key | Datatype and rule |
| --- | --- |
| `PARQUET:field_id` | i32, canonicalized on write |
| `FIELD:enum` | the `StringEnum` document ([Codes](codes/index.md)); accepted on a fixed US-ASCII string of at most sixteen bytes or a registered code, refused by name elsewhere. A second key beside it rather than a copy of it is FIX's `FIX:codeset`, which holds no members at all: it names the [code set](../fix/registry.md#a-field-names-the-code-set-it-reads-by) the dictionary holds them under, and a field may carry both |
| `FIELD:init` | boolean, absent by default; `false` = declared but refused by constructors. Read at intake by the one [boolean reader](numeric/boolean.md#the-one-text-reader) - `no`, `False`, `0` - and stored `true` or `false`, so `is_init` compares the stored text and cannot fail |
| `FIELD:partition` | boolean, read and stored as `FIELD:init` is; `true` on partition columns, absent elsewhere |
| `location` | [`Url`](../uri/url-urn.md), a straight key |
| `alias`, `comment`, `display` | validated text; views fall back to straight `comment` and `display` |
| `SCHEME:name` | protocol property; the prefix is a known [`Scheme`](scalar.md) spelled upper case, as `ARROW:extension:name` and `PARQUET:field_id` spell theirs, and a key written in any case folds to it |
| `ICEBERG:table_name` | catalog coordinates are protocol properties, never straight keys |
| `DIGEST:role` | `holder`; anything else refused |
| `DIGEST:time`, `DIGEST:unit` | a holder's coupled instant: one [field path](paths.md) of field names - a column whose name holds a dot the quoted `"a.b"` - refused at `set_time` otherwise, and `s` / `ms` / `us` / `ns` canonicalized; the storage must be a coupled `fixed_size_binary` ([Hashing](../hashing.md#coupled-holders)) |
| `DIGEST:by`, `PARTITION:by`, `SORT:by`, `TRANSFORM:by` | one shape, a JSON array of expression texts, each read by the grammar its key names and stored as that grammar spells it: a term for `DIGEST:by` (`["*"]` and absence selecting every non-holder column) and for `TRANSFORM:by` (a function's arguments), a term with an optional alias for `PARTITION:by`, an `order by` key for `SORT:by`; an empty, repeated or unparseable entry is refused naming the key |
| `PYTHON:module`, `PYTHON:qualname` | the declaring Python class, as dotted names Python itself could have written; `<locals>` is the one non-identifier segment a qualified name may carry |
| `PYTHON:kind` | `field`, `dataclass`, `typed_dict`, `named_tuple`, `enum`, `newtype`, `type_alias`, or `class`; anything else refused |
| view | borrow of the one metadata map, cache-aware writes |
| Rust `set` | replaces only this protocol's keys; bindings expose `update`, not `set` |

## Use

The view remembers the scheme; the caller writes the bare name.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scheme};

    let mut field = Field::new("price", DataType::Int64, false);

    field.as_iceberg_mut().insert("doc", "closing price")?;
    field.as_iceberg_mut().update([("schema-id", "3"), ("field-id", "7")])?;
    field.as_postgres_mut().insert("type", "numeric")?;
    field.as_digest_mut().set_holder()?;
    field.as_identity_mut().update([("role", "primary"), ("nulls", "distinct")])?;
    field.as_sort_mut().set_by_texts(["price DESC"])?;

    assert_eq!(field.as_iceberg().get("doc"), Some("closing price"));
    assert_eq!(field.as_iceberg().key("doc"), "ICEBERG:doc");
    assert_eq!(field.as_iceberg().len(), 3);
    assert!(field.as_mysql().is_empty());
    assert!(field.as_digest().is_holder());
    assert_eq!(field.as_identity().get("role"), Some("primary"));
    assert_eq!(field.as_sort().by()?.map(|keys| keys[0].to_string()), Some("price desc".to_owned()));

    // It is a view of the one metadata map, not a copy of part of it.
    assert_eq!(field.get_metadata("ICEBERG:doc"), Some("closing price"));
    assert_eq!(field.get_metadata("DIGEST:role"), Some("holder"));
    assert_eq!(field.get_metadata("SORT:by"), Some(r#"["price desc"]"#));
    assert_eq!(field.metadata_len(), 8);

    // A protocol-scoped replacement leaves every other protocol alone.
    field.as_iceberg_mut().set([("doc", "close")])?;
    assert_eq!(field.as_iceberg().iter().collect::<Vec<_>>(), [("doc", "close")]);
    assert_eq!(field.as_postgres().get("type"), Some("numeric"));

    // The view is the field: it dereferences to one, and `as_field` hands back a
    // borrow that outlives the view rather than one that dies with it.
    assert_eq!(field.as_iceberg().dtype(), &DataType::Int64);
    let name = field.as_iceberg().as_field().name();
    assert_eq!(name, "price");

    // The protocol can also come from a value rather than from the code.
    assert_eq!(field.protocol(&Scheme::POSTGRES).get("type"), Some("numeric"));
    ```

=== "Python"

    ```python
    from yggdryl import Field

    field = Field("price", "int64", nullable=False)

    field.iceberg["doc"] = "closing price"
    field.iceberg.update({"schema-id": "3", "field-id": "7"})
    field.postgres["type"] = "numeric"
    field.digest["role"] = "holder"
    field.identity.update({"role": "primary", "nulls": "distinct"})
    field.sort.by = ["price DESC"]

    assert field.iceberg["doc"] == "closing price"
    assert field.iceberg.key("doc") == "ICEBERG:doc"
    assert len(field.iceberg) == 3
    assert not field.mysql
    assert field.digest["role"] == "holder"
    assert field.identity["role"] == "primary"
    # A `by` list is stored as the grammar spells it.
    assert field.sort.by == ["price desc"]
    assert field.metadata["SORT:by"] == '["price desc"]'

    # It is a view of the one metadata mapping, not a copy of part of it.
    assert field.metadata["ICEBERG:doc"] == "closing price"
    assert field.metadata["DIGEST:role"] == "holder"
    assert len(field.metadata) == 8
    assert dict(field.iceberg.items())["field-id"] == "7"

    del field.iceberg["field-id"]
    assert "field-id" not in field.iceberg
    assert field.protocol("postgres")["type"] == "numeric"
    ```

    !!! note "Rust-only"
        The per-protocol view types (`HttpField`, `IcebergField`, `FixField`, `DigestField`,
        `IdentityField`, and seventeen others) are Rust-only. Python reads the generic property
        mapping through `field.iceberg`, and the validated HTTP values stay attributes on the
        field. The `fix`, `digest`, `partition`, `sort`, `transform` and `python` views add typed
        vocabulary, each answered only by its own view: `id`, `tag`, `tags`, `aliases`,
        `branches`, `identifiers`, `description`, `nulls`, `directions` and the catalog
        references on `field.fix`; `is_holder`, `algorithm`, `time`, `unit`, `is_coupled` and
        `apply_arrow_batch` on `field.digest`;
        [`apply_arrow_batch`](../holder/index.md#derived-partition-columns) on `field.partition`
        and `field.transform`; `class_metadata` and its three parts on `field.python`. `by` is the
        typed list of the digest, partition and sort views - canonical texts, `None` when absent,
        assigned as a list and removed by `remove_by()` - and read only on the transform view,
        whose `term` is written (a `Term` or its text, `None` removes it) and `remove_term()`
        drops the derivation. Every other view refuses `by` and `term` naming its scheme.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = new Field('price', 'int64', false)

    field.iceberg.set('doc', 'closing price')
    field.iceberg.update({ 'schema-id': '3', 'field-id': '7' })
    field.postgres.set('type', 'numeric')
    field.digest.set('role', 'holder')
    field.identity.update({ role: 'primary', nulls: 'distinct' })
    field.sort.by = ['price DESC']

    assert.equal(field.iceberg.get('doc'), 'closing price')
    assert.equal(field.iceberg.key('doc'), 'ICEBERG:doc')
    assert.equal(field.iceberg.size, 3)
    assert.equal(field.mysql.size, 0)
    assert.equal(field.digest.get('role'), 'holder')
    assert.equal(field.identity.get('role'), 'primary')
    // A `by` list is stored as the grammar spells it.
    assert.deepEqual(field.sort.by, ['price desc'])
    assert.equal(field.get('SORT:by'), '["price desc"]')

    // It is a view of the one metadata map, not a copy of part of it.
    assert.equal(field.get('ICEBERG:doc'), 'closing price')
    assert.equal(field.get('DIGEST:role'), 'holder')
    assert.equal(field.size, 8)
    assert.deepEqual([...field.iceberg].sort(), [['doc', 'closing price'], ['field-id', '7'], ['schema-id', '3']])

    assert.equal(field.iceberg.delete('field-id'), true)
    assert.equal(field.iceberg.has('field-id'), false)
    assert.equal(field.protocol('postgres').get('type'), 'numeric')
    ```

    !!! note "Rust-only"
        The per-protocol view types (`HttpField`, `IcebergField`, `FixField`, `DigestField`,
        `IdentityField`, `PartitionField`, `SortField`, `PythonField`, and fifteen others) are
        Rust-only, and so is the Python-class vocabulary Python binds. JavaScript reads the
        generic property `Map` through `field.iceberg` and `field.python`; `field.fix` is the
        exception, answering `id`, `tag`, `tags`, `aliases`, `branches`, `identifiers`,
        `description`, `nulls`, `directions`, `addBranch` and `hasBranch`, and the validated HTTP
        values stay accessors on the field. The `partition`, `sort`, `digest` and `transform`
        views add `by`: a `string[]` of canonical texts, `null` when absent, assigned as an array
        and removed by `removeBy()` - read only on `transform`, whose `term` is written (a `Term`
        or its text, `null` removes it) and dropped by `removeTerm()`. Every other view refuses
        `by` and `term` by naming its scheme.

## Reserved keys

Typed accessors parse and canonicalize both ways.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MimeType, PythonKind, PythonMetadata, Scheme};

    let mut field = Field::new("payload", DataType::binary(), false);

    field.set_parquet_field_id(17);
    field.set_init(false);
    field.set_display("Raw payload")?;
    field.as_http_mut().set_content_type("application/json; charset=utf-8")?;
    field.set_property(&Scheme::POSTGRES, "type", "jsonb")?;
    field
        .as_python_mut()
        .set_class(&PythonMetadata::new("app.wire", "Payload", PythonKind::Dataclass)?)?;

    assert_eq!(field.parquet_field_id()?, Some(17));
    assert_eq!(field.get_metadata("PARQUET:field_id"), Some("17"));
    assert!(!field.is_init());
    assert_eq!(field.get_metadata("FIELD:init"), Some("false"));

    // A straight key belongs to no protocol, so every protocol falls back to it.
    assert_eq!(field.display(), Some("Raw payload"));
    assert_eq!(field.as_postgres().display(), Some("Raw payload"));

    // An http: property answers to either scheme and to a raw key lookup, and its
    // parsing accessors live on the field's http: view.
    assert_eq!(field.as_http().mime_type()?, MimeType::JSON);
    assert_eq!(
        field.get_property(&Scheme::HTTPS, "Content-Type"),
        field.as_http().content_type()
    );
    assert_eq!(
        field.get_metadata("HTTP:content-type"),
        field.as_http().content_type()
    );
    assert_eq!(
        field.property_iter(&Scheme::POSTGRES).collect::<Vec<_>>(),
        [("type", "jsonb")]
    );

    // The declaring class is one value, and the bare name is derived from the
    // qualified one rather than stored beside it.
    assert_eq!(field.as_python().class_name(), Some("Payload"));
    assert_eq!(field.get_metadata("PYTHON:kind"), Some("dataclass"));
    assert_eq!(field.as_python().import_path().as_deref(), Some("app.wire.Payload"));
    ```

=== "Python"

    ```python
    from yggdryl import Field, MimeType, PythonMetadata

    field = Field("payload", "binary", nullable=False)

    field.set_parquet_field_id(17)
    field.metadata["FIELD:init"] = "false"
    field.set_display("Raw payload")
    field.set_content_type("application/json; charset=utf-8")
    field.set_property("postgres", "type", "jsonb")
    field.python.class_metadata = PythonMetadata("app.wire", "Payload", "dataclass")

    assert field.parquet_field_id == 17
    assert field.metadata["PARQUET:field_id"] == "17"
    assert field.metadata["FIELD:init"] == "false"

    # A straight key belongs to no protocol, so every protocol falls back to it.
    assert field.display == "Raw payload"
    assert field.postgres.display == "Raw payload"

    assert field.mime_type == MimeType.JSON
    assert field.get_property("https", "Content-Type") == field.content_type
    assert field.metadata["HTTP:content-type"] == field.content_type
    assert list(field.property_iter("postgres")) == [("type", "jsonb")]

    # The declaring class is one value, and the bare name is derived from the
    # qualified one rather than stored beside it.
    assert field.python.class_name == "Payload"
    assert field.metadata["PYTHON:kind"] == "dataclass"
    assert field.python.import_path == "app.wire.Payload"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, MimeType } = require('yggdryl')

    const field = new Field('payload', 'binary', false)

    field.setParquetFieldId(17)
    field.set('FIELD:init', 'false')
    field.setDisplay('Raw payload')
    field.setContentType('application/json; charset=utf-8')
    field.setProperty('postgres', 'type', 'jsonb')
    field.python.update({ kind: 'dataclass', module: 'app.wire', qualname: 'Payload' })

    assert.equal(field.parquetFieldId, 17)
    assert.equal(field.get('PARQUET:field_id'), '17')
    assert.equal(field.get('FIELD:init'), 'false')

    // A straight key belongs to no protocol, so every protocol falls back to it.
    assert.equal(field.display, 'Raw payload')
    assert.equal(field.postgres.display, 'Raw payload')

    assert.ok(field.mimeType.equals(MimeType.JSON))
    assert.equal(field.getProperty('https', 'Content-Type'), field.contentType)
    assert.equal(field.get('HTTP:content-type'), field.contentType)
    assert.deepEqual(field.propertyIter('postgres'), [{ key: 'type', value: 'jsonb' }])

    // The declaring class a Python schema carries reads back by name here; the
    // typed vocabulary over it is Rust and Python only.
    assert.equal(field.python.get('qualname'), 'Payload')
    assert.equal(field.get('PYTHON:kind'), 'dataclass')
    ```

## Views

Typed vocabulary lives on the view, never on `Field`;
[`Field::apply_arrow_batch`](field.md#applying-a-schema) is the cast alone and fills no column a
view's protocol declares - the `transform` and `digest` views' own `apply_arrow_batch` do.

| View | Vocabulary |
| --- | --- |
| `HttpField`, `HttpFieldMut` | `content_type`, `content_length`, `mime_type`, `media_type`, `location` |
| [`IcebergField`, `IcebergFieldMut`](../media/iceberg.md) | `doc`, `schema_id`, `spec_id`, `transform` |
| [`FixField`, `FixFieldMut`](../fix/index.md) | `id` (derived from the tag and the name, never stored), `tag` and `tags` (positive only), `aliases`, `branches`, `identifiers` (a component's direct scalar members), `codeset` (the name of the vocabulary the dictionary holds its values under), `description` |
| [`DigestField`, `DigestFieldMut`](../hashing.md) | `is_holder`, `algorithm`, `by`, `apply_arrow_batch`, and their setters; `time`, `unit`, `is_coupled` and their setters |
| `IdentityField` | no typed vocabulary: arbitrary inert text under `IDENTITY:` |
| [`PartitionField`, `PartitionFieldMut`](#partition-columns) | `by`, `declares_partition`; `set_by`, `set_by_texts`, `remove_by`; [`Field::with_partition_by`](#partition-columns) is what marks the identity columns and materializes the derived ones, each a [transform](../expression/selectors.md#a-selector-declares-a-schema) column applied through `as_transform().apply_arrow_batch` |
| [`SortField`, `SortFieldMut`](#sort-order) | `by`, `declares_order`; `set_by`, `set_by_texts`, `remove_by`; a [plan](../expression/plans.md) moves the keys into its `order by` and an [Iceberg table](../media/iceberg.md#declared-partitioning-and-sort-order) into its default sort order |
| [`TransformField`, `TransformFieldMut`](../expression/selectors.md#a-selector-declares-a-schema) | `term`, `function`, `by`, `is_derived`, `apply_arrow_batch`; `set_term`, `set_function`, `remove_term` |
| `PythonField`, `PythonFieldMut` | `class`, `module`, `qualname`, `class_name`, `kind`, `import_path`, and their setters |

## Digest holders and their by

`DIGEST:role` has one value, `holder`: it says a field *stores* a row digest. What that digest
reads is named on the holder as `DIGEST:by`, a list of expression terms, so the fields it reads
carry no metadata at all. A bare column path feeds the column's own buffers; any other term -
`lower(symbol)`, `price * size` - is bound once and computed per batch into the value fed.
`["*"]` - which is also what naming nothing means - selects every field of the Struct except a
holder, in declaration order.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let id = Field::new("id", DataType::Int64, false);
    let price = Field::new("price", DataType::Float64, false);
    let mut stored = Field::new("row_digest", DataType::UInt64, false);
    stored.as_digest_mut().set_holder()?;

    let fallback = DataType::from(StructType::from_fields([id.clone(), price.clone(), stored.clone()])?)
        .required_field("row");
    assert_eq!(fallback.digest_field_names().collect::<Vec<_>>(), ["id", "price"]);

    // Narrowing the input is the holder's business; `id` stays an ordinary column,
    // and a term is stored as the grammar spells it.
    let mut narrowed = stored;
    narrowed.as_digest_mut().set_by(["id", "Lower(symbol)"])?;
    assert_eq!(
        narrowed.as_digest().by()?,
        Some(vec!["id".to_owned(), "lower(symbol)".to_owned()])
    );
    assert!(id.as_digest().is_empty());

    let explicit = DataType::from(StructType::from_fields([id, price, narrowed])?).required_field("row");
    assert_eq!(explicit.digest_field_names().collect::<Vec<_>>(), ["id", "price"]);
    assert_eq!(explicit.only_digest_fields()?.field_len(), 2);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    identifier = Field("id", "int64", nullable=False)
    price = Field("price", "float64", nullable=False)
    stored = Field("row_digest", "uint64", nullable=False)
    stored.digest["role"] = "holder"

    fallback = Field(
        "row", DataType.from_fields([identifier, price, stored]), nullable=False
    )
    assert fallback.digest_field_names == ["id", "price"]

    # Narrowing the input is the holder's business; `id` stays an ordinary column,
    # and a term is stored as the grammar spells it.
    stored.digest.by = ["id", "Lower(symbol)"]
    assert stored.digest.by == ["id", "lower(symbol)"]
    assert stored.digest["by"] == '["id","lower(symbol)"]'
    assert dict(identifier.digest) == {}

    explicit = Field(
        "row", DataType.from_fields([identifier, price, stored]), nullable=False
    )
    assert explicit.digest_field_names == ["id", "price"]
    assert len(explicit.only_digest_fields().dtype) == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const identifier = new Field('id', 'int64', false)
    const price = new Field('price', 'float64', false)
    const stored = new Field('row_digest', 'uint64', false)
    stored.digest.set('role', 'holder')

    const fallback = new Field(
      'row', DataType.fromFields([identifier, price, stored]), false,
    )
    assert.deepEqual(fallback.digestFieldNames(), ['id', 'price'])

    stored.digest.by = ['id', 'Lower(symbol)']
    assert.deepEqual(stored.digest.by, ['id', 'lower(symbol)'])
    assert.equal(stored.digest.get('by'), '["id","lower(symbol)"]')
    assert.deepEqual(identifier.digest.entries(), [])

    const explicit = new Field(
      'row', DataType.fromFields([identifier, price, stored]), false,
    )
    assert.deepEqual(explicit.digestFieldNames(), ['id', 'price'])
    assert.equal(explicit.onlyDigestFields().dtype.length, 2)
    ```

| key | rule |
| --- | --- |
| Namespace | `Scheme::DIGEST`; `digest_fields`, `digest_field_names`, `digest_field_len`, `only_digest_fields` |
| Selection | direct Struct children, in declaration order |
| `DigestField` | `is_holder`, `algorithm`, `by`, `apply_arrow_batch` |
| `DigestFieldMut` | `set_holder`, `set_algorithm`, `remove_algorithm`, `set_by`, `remove_by`, `remove_role` |
| `DIGEST:by` | holder-local ordered JSON array of expression texts; a bare path feeds the column, any other term is computed per batch; `["*"]` and absence both select every non-holder, `[]` selects nothing, `"*"` may not travel beside a term |
| `DIGEST:algorithm` | optional canonical [`DigestAlgorithm`](../hashing.md); its width must match the storage |
| Widths | XXH32: `int32`, `uint32`; XXH64 and XXH3-64: `int64`, `uint64`; XXH3-128: `fixed_size_binary(16)` |
| Signed storage | the same digest bits, never a checked numeric conversion |

Typed setters require `holder`, validate the width, and fail atomically; generic metadata writes
canonicalize the algorithm token and the `by` JSON. Arrow row hashing reads the same contract in
`row_digests`, and path resolution, nested-holder reuse, algorithm fallback, and batch filling
live with [Hashing](../hashing.md).

## Partition columns

A struct declares how its rows partition as `PARTITION:by`: the projections, in order - a bare
column an identity partition, a term a derived one, named by its alias or by
`{source}_{function}` in Iceberg's singular (`ts_year`, `ts_day`, `ts_minutes`, `name_truncate`).
`with_partition_by` stores the declaration and turns it into the layout: every identity column
is marked `FIELD:partition`, and every derived entry is added as a marked column carrying its
[transform](../expression/selectors.md#a-selector-declares-a-schema), typed by the term.
`with_partition_fields` is the same over bare columns. `partition_by` answers the declaration as canonical texts, else the marked columns; the `partition` view's `by` answers the declaration alone. A marked column the declaration does not
name is refused naming both; a declared column may be unmarked or absent, because a leaf stores
the rows minus the partition columns under the whole declaration, and an Iceberg table keeps a
derived value in its manifest ([Iceberg](../media/iceberg.md#declared-partitioning-and-sort-order)).

=== "Rust"

    ```rust
    use yggdryl::{DataType, StructType};

    let schema = DataType::from(StructType::from_fields([
        DataType::Int32.required_field("year"),
        DataType::utf8().required_field("venue"),
        DataType::Int64.required_field("price"),
    ])?)
    .required_field("row")
    .with_partition_fields(&["year", "venue"])?;

    assert!(schema.has_partition_fields());
    assert_eq!(schema.partition_field_names().collect::<Vec<_>>(), ["year", "venue"]);
    assert!(schema.get_field_by_path("year").expect("the column").is_partition());
    assert_eq!(schema.get_metadata("PARTITION:by"), Some(r#"["year","venue"]"#));

    // The two halves of the layout: what a path spells, and what a leaf stores.
    assert_eq!(schema.without_partition_fields()?.field_len(), 1);
    assert_eq!(schema.only_partition_fields()?.field_len(), 2);

    // The mark is reserved metadata, so it round-trips like any other.
    assert_eq!(
        schema.get_field_by_path("year").expect("the column").get_metadata("FIELD:partition"),
        Some("true")
    );

    // A derived entry is a marked column computed from the rows, named by
    // the convention unless aliased.
    let derived = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("venue"),
        DataType::date32().required_field("event"),
    ])?)
    .required_field("row")
    .with_partition_by(["venue".parse()?, "years(event)".parse()?, "truncate(venue, 2) as prefix".parse()?])?;
    assert_eq!(
        derived.partition_field_names().collect::<Vec<_>>(),
        ["venue", "event_year", "prefix"]
    );
    assert_eq!(
        derived.get_field_by_path("event_year").expect("the derived column").as_transform().term()?.map(|term| term.to_string()),
        Some("years(event)".to_owned())
    );
    assert_eq!(derived.partition_by()?.len(), 3);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    schema = Field(
        "row",
        DataType.from_fields([
            Field("year", "int32", nullable=False),
            Field("venue", "string", nullable=False),
            Field("price", "int64", nullable=False),
        ]),
        nullable=False,
    ).with_partition_fields(["year", "venue"])

    assert schema.has_partition_fields
    assert schema.partition_field_names == ["year", "venue"]
    assert schema.dtype["year"].is_partition
    assert not schema.dtype["price"].is_partition
    assert schema.metadata["PARTITION:by"] == '["year","venue"]'

    assert len(schema.without_partition_fields().dtype) == 1
    assert len(schema.only_partition_fields().dtype) == 2

    # A derived entry is a marked column computed from the rows, named by
    # the convention unless aliased.
    derived = Field(
        "row",
        DataType.from_fields([
            Field("venue", "string", nullable=False),
            Field("event", "date32", nullable=False),
        ]),
        nullable=False,
    ).with_partition_by(["venue", "years(event)", "truncate(venue, 2) as prefix"])
    assert derived.partition_field_names == ["venue", "event_year", "prefix"]
    assert str(derived.dtype["event_year"].transform.term) == "years(event)"
    assert derived.partition_by == ["venue", "years(event)", "truncate(venue, 2) as prefix"]
    assert derived.partition.by == derived.partition_by
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const schema = new Field(
      'row',
      DataType.fromFields([
        new Field('year', 'int32', false),
        new Field('venue', 'string', false),
        new Field('price', 'int64', false),
      ]),
      false,
    ).withPartitionFields(['year', 'venue'])

    assert.equal(schema.hasPartitionFields, true)
    assert.deepEqual(schema.partitionFieldNames(), ['year', 'venue'])
    assert.equal(schema.dtype.getFieldByPath('year').isPartition, true)
    assert.equal(schema.dtype.getFieldByPath('price').isPartition, false)
    assert.equal(schema.get('PARTITION:by'), '["year","venue"]')

    assert.equal(schema.withoutPartitionFields().dtype.length, 1)
    assert.equal(schema.onlyPartitionFields().dtype.length, 2)

    // A derived entry is a marked column computed from the rows, named by
    // the convention unless aliased.
    const derived = new Field(
      'row',
      DataType.fromFields([new Field('venue', 'string', false), new Field('event', 'date32', false)]),
      false,
    ).withPartitionBy(['venue', 'years(event)', 'truncate(venue, 2) as prefix'])
    assert.deepEqual(derived.partitionFieldNames(), ['venue', 'event_year', 'prefix'])
    assert.equal(derived.dtype.getFieldByPath('event_year').transform.term.toString(), 'years(event)')
    assert.deepEqual(derived.partitionBy(), ['venue', 'years(event)', 'truncate(venue, 2) as prefix'])
    assert.deepEqual(derived.partition.by, derived.partitionBy())
    ```

Folder writes and reads read the marks, and an Iceberg spec reads the declaration; a write only
casts, so a derived column is filled through `as_transform().apply_arrow_batch` before it, else
refused by path where required and written null where nullable:
[Partitions](../holder/index.md#partitions), [Iceberg](../media/iceberg.md#declared-partitioning-and-sort-order).

## Sort order

A struct declares the order its rows keep as `SORT:by`: the `order by` keys of the
[plan grammar](../expression/plans.md), most significant first, each a term then `asc` or `desc`
then `nulls first` or `nulls last`, stored as the grammar spells them. `Plan::from_field` moves
the keys into its `order by` section and writes them back from it, and an Iceberg table created
from the schema takes them as its default sort order.

On a [`Serie`](serie.md#a-declared-order) the declaration is a proven fact about the rows: the
sorts write it, the verbs that keep the order keep it, a write that breaks it clears it, and a
door landing foreign rows under a declaring root reads them once and refuses the first row out
of order by name. A table's `SORT:by` is how its writers lay each data file out - an Iceberg
write sorts each partition's rows as a whole by it, and writes a stream whose root proves it
as it arrived - so an Iceberg scan's root drops it while `IcebergTable::schema()` keeps reporting it.

=== "Rust"

    ```rust
    use yggdryl::expression::{Ordering, Plan, Term};
    use yggdryl::{DataType, SortOptions, StructType};

    let mut rows = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("venue"),
        DataType::Float64.nullable_field("price"),
    ])?)
    .required_field("trades");
    rows.as_sort_mut().set_by_texts(["venue", "price DESC NULLS FIRST"])?;

    assert_eq!(rows.get_metadata("SORT:by"), Some(r#"["venue","price desc nulls first"]"#));
    let keys = rows.as_sort().by()?.expect("an order");
    assert_eq!(keys[0], Ordering::asc(Term::column("venue")));
    assert_eq!(keys[1].options(), SortOptions::descending().with_nulls_first(true));

    // The plan owns the order: the key leaves the metadata for the section and
    // comes back from it, so a schema and its plan are one declaration.
    let plan = Plan::from_field(&rows);
    assert_eq!(plan.ordering(), keys.as_slice());
    assert_eq!(
        plan.to_string(),
        "create trades (venue utf8 not null, price float64 null) order by venue, price desc nulls first"
    );
    assert_eq!(plan.field()?, Some(rows));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    rows = Field(
        "trades",
        DataType.from_fields([
            Field("venue", "string", nullable=False),
            Field("price", "float64", nullable=True),
        ]),
        nullable=False,
    )
    rows.sort.by = ["venue", "price DESC NULLS FIRST"]

    assert rows.sort.by == ["venue", "price desc nulls first"]
    assert rows.metadata["SORT:by"] == '["venue","price desc nulls first"]'

    # Removing the declaration answers the stored text.
    assert rows.sort.remove_by() == '["venue","price desc nulls first"]'
    assert rows.sort.by is None
    ```

    !!! note "Rust-only"
        `Ordering` and `SortOptions` are Rust-only: the keys cross as texts, and the
        [serie verbs](serie.md#sorting-uniqueness-and-partitions) take `descending` and
        `nulls_first` keywords.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const rows = new Field(
      'trades',
      DataType.fromFields([
        new Field('venue', 'string', false),
        new Field('price', 'float64', true),
      ]),
      false,
    )
    rows.sort.by = ['venue', 'price DESC NULLS FIRST']

    assert.deepEqual(rows.sort.by, ['venue', 'price desc nulls first'])
    assert.equal(rows.get('SORT:by'), '["venue","price desc nulls first"]')

    // Removing the declaration answers the stored text.
    assert.equal(rows.sort.removeBy(), '["venue","price desc nulls first"]')
    assert.equal(rows.sort.by, null)
    ```

    !!! note "Rust-only"
        `Ordering` and `SortOptions` are Rust-only: the keys cross as texts, and the
        [serie verbs](serie.md#sorting-uniqueness-and-partitions) take a plain
        `{ descending, nullsFirst }` object.

## Edges

- `"+00017"` to `PARQUET:field_id` -> stored as `"17"`; `"2147483648"` -> refused.
- `HTTPS:Content-Type`, `HTTP:content-type`, `HTTP:content-type` -> one entry, matched case-insensitively.
- `https` -> no accessor; either scheme's view reports `http`.
- Rust `field.location()` -> straight `location`; `as_http().location()` -> `HTTP:location` (`http_location` / `httpLocation` in the bindings).
- `with_init` -> Rust only; `set_init` and `is_init` are in Python, and the bindings' mapping write validates identically.
- `display` -> named in all three (`set_display`, `display`, `remove_display`) on the field and every view; `try_with_display` is Rust only.
- Deleting a protocol's namespace -> leaves `Field`'s own reserved state untouched.
- Protocol write -> invalidates a populated Arrow projection, like a direct metadata write.
- Non-partition column -> no `FIELD:partition` key; schemas partitioned alike compare equal.
- `FIELD:partition` -> travels into Arrow, Parquet footers, and JSON round trips.
- `DIGEST:role` other than `holder` -> refused, and the failed write leaves the field unchanged.
- `DIGEST:by` on a field that is not a holder -> refused when the plan is built; a term names what it reads, it never marks a field.
- A `DIGEST:by` term that does not bind against the holder's Struct -> refused when the plan is built, naming the holder and the term.
- A root whose children are all holders -> no digest values, the empty ordered sequence.
- Changing or removing a holder role -> refused until `DIGEST:algorithm` and `DIGEST:by` are gone.
- `IDENTITY:` properties -> inert text; `FIELD:partition` stays the marker `partition_fields` reads.
- A `by` entry -> read by its key's grammar on every write, typed or generic, and stored as the grammar spells it: `Lower(symbol)` as `lower(symbol)`, `price DESC` as `price desc`; a column keeps its spelling and resolves case-insensitively where it binds.
- `PARTITION:by` entry with a datatype, nullability or `with (...)` -> a column declaration, not a partition entry: refused naming the key.
- A derived `PARTITION:by` entry with no alias that is not a function over a column -> nothing names it: refused.
- `FIELD:partition` on a column `PARTITION:by` does not name -> refused naming both; the declaration alone marks nothing, `with_partition_by` does.
- `without_fields` -> drops the `PARTITION:by` entries naming a removed column; `without_partition_fields` -> drops the declaration, which is the folder's.
- `SORT:by` -> the `order by` keys; `Plan::from_field` moves them out of the root's metadata into the section, and `field()` writes them back.
- `TRANSFORM:function` -> a grammar function by name or alias, or a [user function](../expression/functions.md) `namespace.name`, canonicalized on write; `TRANSFORM:by` the terms it reads in argument order; `term` reads `function(by...)`, and refuses a function with no `by` beside it.
- `TRANSFORM:expression` -> any other term; `set_term` writes the function and its `by` for a call over plain columns and the expression otherwise, and removes the spelling it did not write.
- `apply_arrow_batch` -> the one verb the `transform` and `digest` views answer; each walks the Structs it declares and leaves a written value alone, and `Field::apply_arrow_batch` is the cast alone and fills neither.

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- protocol::internal::tests
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test metadata --test root -- http protocol partition python
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(protocol_|http_|partition_|python_|without_partition|typed_location|typed_field_id)'
    cargo bench --manifest-path rust/Cargo.toml --features iceberg --bench types -- '^value/iceberg_'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_field.py -k "http or protocol or partition or python or typed_names"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="HTTP|protocol|partition|python|typed names" node/tests/field.test.js
    npm run --prefix node bench:types
    ```
