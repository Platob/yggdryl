# Media types

`mimetype` and `mediatype`: the two datatypes that carry a column of routing vocabulary - what a record's bytes are, and what they were declared under.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::MimeType` and `DataType::MediaType`, over the `MimeType` and `MediaType` values the [media layer](../media/index.md) routes on |
| Validates | A MIME type refuses text that is not a `type/subtype` name; a media type's intake is total by construction, so unrecognized text answers the default base |
| Lazy | Nothing - text parses once at the door and is held as the value |
| Cached | The Arrow projection of a [`Field`](field.md); a `MediaType` rides behind one shared pointer, because a base, a charset and a coding list are wider than the scalar |
| Refuses | A name that is not `type/subtype` in a MIME column; a null in a required column; a merge with any other datatype, including each other |
| Kinds | `DataTypeKind::Text`, ids `0x67` and `0x68` - in the text family's range, after the [version](version.md), because `as_u8` is a wire contract laid out by family |
| Bindings | Both cross as their canonical text; the `MimeType` and `MediaType` classes Python and JavaScript expose are the [media](../media/index.md) routing values, not a scalar wrapper |

A column of media types declares what a reader would otherwise have to guess:
a stored `text/csv;charset=utf-8` is the base, the [charset](../charset/index.md)
and the ordered content codings that a handle was read and written under, in
one canonical rendering. The vocabulary itself - what each name means, how a
scheme is picked, how a name is inferred from a path, from headers or from
magic bytes - is on [Media](../media/index.md); this page is only what makes
those values a datatype, a field and a scalar.

## DataType

Two parameterless variants, so the enum is the constructor. `mime` is the
second spelling of the first and `content_type` of the second, and both render
under their canonical name.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    assert_eq!(DataType::from_str("MIMETYPE")?, DataType::MimeType);
    assert_eq!(DataType::from_str("mime")?, DataType::MimeType);
    assert_eq!(DataType::MimeType.to_string(), "mimetype");
    assert_eq!(DataType::MimeType.id(), DataTypeId::MimeType);
    assert_eq!(DataTypeId::MimeType.as_u8(), 0x67);

    assert_eq!(DataType::from_str("content_type")?, DataType::MediaType);
    assert_eq!(DataType::MediaType.to_string(), "mediatype");
    assert_eq!(DataTypeId::MediaType.as_u8(), 0x68);

    // Both are text, and both serialize under their own tag.
    assert_eq!(DataType::MimeType.kind(), DataTypeKind::Text);
    assert_eq!(DataType::MediaType.kind(), DataTypeKind::Text);
    assert_eq!(DataType::MimeType.into_json()?, r#"{"type":"mimetype"}"#);
    assert_eq!(DataType::from_json(r#"{"type":"mediatype"}"#)?, DataType::MediaType);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    assert DataType("MIMETYPE") == DataType("mimetype") == DataType("mime")
    assert str(DataType("mimetype")) == "mimetype"
    assert DataType("mimetype").id == "mimetype"

    assert DataType("content_type") == DataType("mediatype")
    assert str(DataType("mediatype")) == "mediatype"

    # Both are text, and both serialize under their own tag.
    assert DataType("mimetype").kind == "text"
    assert DataType("mediatype").kind == "text"
    assert DataType("mimetype").into_dict() == {"type": "mimetype"}
    assert DataType("mediatype").into_dict() == {"type": "mediatype"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.ok(DataType.from('MIMETYPE').equals(DataType.from('mimetype')))
    assert.ok(DataType.from('mime').equals(DataType.from('mimetype')))
    assert.equal(DataType.from('mimetype').id, 'mimetype')

    assert.ok(DataType.from('content_type').equals(DataType.from('mediatype')))
    assert.equal(DataType.from('mediatype').toString(), 'mediatype')

    // Both are text, and both serialize under their own tag.
    assert.equal(DataType.from('mimetype').kind, 'text')
    assert.equal(DataType.from('mediatype').kind, 'text')
    assert.deepEqual(DataType.from('mimetype').toJSON(), { type: 'mimetype' })
    ```

## Field

`MimeTypeField` and `MediaTypeField` are the typed markers, and there is
nothing to pass: `unit(name, nullable)` is the whole constructor. The bindings
declare the same two columns with `yggdryl.mimetype` / `yggdryl.mediatype` and
`fields.mimetype` / `fields.mediatype`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MediaTypeField, MimeType, MimeTypeField, Scalar};

    let held = MimeTypeField::unit("held", false);
    assert_eq!(held.name(), "held");
    assert_eq!(held.dtype(), &DataType::MimeType);
    assert!(!held.is_nullable());
    assert_eq!(MediaTypeField::unit("held", true).dtype(), &DataType::MediaType);

    // The field is where a caller's text becomes a stored value.
    let field = Field::new("held", DataType::MimeType, false);
    assert_eq!(field.scalar("APPLICATION/JSON")?, Scalar::MimeType(MimeType::JSON));
    assert!(field.scalar("not a type").is_err());
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    from yggdryl import Field

    held = yggdryl.mimetype("held", nullable=False)
    assert isinstance(held, Field)
    assert str(held.dtype) == "mimetype"
    assert held.nullable is False
    assert str(yggdryl.mediatype("held").dtype) == "mediatype"

    # The field is where a caller's text becomes a stored value.
    assert held.scalar("APPLICATION/JSON").as_py() == "application/json"
    with pytest.raises(ValueError, match="mimetype"):
        held.scalar("not a type")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Scalar, fields } = require('yggdryl')

    const held = fields.mimetype('held', { nullable: false })
    assert.ok(held instanceof Field)
    assert.equal(held.dtype.toString(), 'mimetype')
    assert.equal(held.nullable, false)
    assert.equal(fields.mediatype('held').dtype.toString(), 'mediatype')

    // The field is where a caller's text becomes a stored value.
    assert.equal(Scalar.from('APPLICATION/JSON', { field: held }).asJs(), 'application/json')
    assert.throws(() => Scalar.from('not a type', { field: held }), /mimetype/)
    ```

## Scalar

`Scalar::MimeType(MimeType)` holds one canonical `type/subtype` name;
`Scalar::MediaType(Arc<MediaType>)` holds that name with the charset and the
ordered content codings it was declared under. Case and parameters
canonicalize on the way in, so two writings are one value and one hash.

=== "Rust"

    ```rust
    use yggdryl::{Charset, DataType, MediaType, MimeType, Scalar};

    let mime = MimeType::from_str("APPLICATION/JSON")?;
    assert_eq!(mime, MimeType::JSON);
    assert_eq!(mime.as_str(), "application/json");
    assert_eq!(DataType::MimeType.scalar("APPLICATION/JSON")?, Scalar::MimeType(mime));

    // A media type carries the charset and the codings in one rendering.
    let media = MediaType::from_str("text/csv; charset=utf-8")?;
    assert_eq!(media.base(), &MimeType::CSV);
    assert_eq!(media.charset(), Some(Charset::Utf8));
    assert_eq!(DataType::MediaType.scalar("TEXT/CSV; CHARSET=UTF-8")?, Scalar::from(media));

    // A MIME type refuses what is not a name; a media type infers instead.
    assert!(DataType::MimeType.scalar("not a type").is_err());
    assert_eq!(
        DataType::MediaType.scalar("README")?,
        Scalar::from(MediaType::new(MimeType::OCTET_STREAM)),
    );
    // And the two datatypes never collapse into one another.
    assert_ne!(DataType::MimeType.scalar("text/csv")?, DataType::MediaType.scalar("text/csv")?);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, MediaType, MimeType

    mime = MimeType("APPLICATION/JSON")
    assert mime == MimeType.JSON
    assert str(mime) == "application/json"
    assert DataType("mimetype").scalar("APPLICATION/JSON").as_py() == "application/json"

    # A media type carries the charset and the codings in one rendering.
    media = MediaType("text/csv; charset=utf-8")
    assert str(media.base) == "text/csv"
    assert media.charset == "utf-8"
    assert DataType("mediatype").scalar("TEXT/CSV; CHARSET=UTF-8").as_py() == "text/csv;charset=utf-8"

    # A MIME type refuses what is not a name; a media type infers instead.
    assert DataType("mediatype").scalar("README").as_py() == "application/octet-stream"
    assert (
        DataType("mediatype").scalar("part.tgz").as_py()
        == "application/x-tar;encodings=application/gzip"
    )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, MediaType, MimeType } = require('yggdryl')

    assert.equal(String(MimeType.from('APPLICATION/JSON')), 'application/json')
    assert.equal(DataType.from('mimetype').scalar('APPLICATION/JSON').asJs(), 'application/json')

    // A media type carries the charset and the codings in one rendering.
    const media = MediaType.from('text/csv; charset=utf-8')
    assert.equal(String(media.base), 'text/csv')
    assert.equal(media.charset, 'utf-8')
    assert.equal(
      DataType.from('mediatype').scalar('TEXT/CSV; CHARSET=UTF-8').asJs(),
      'text/csv;charset=utf-8',
    )

    // A MIME type refuses what is not a name; a media type infers instead.
    assert.throws(() => DataType.from('mimetype').scalar('not a type'), /mimetype/)
    assert.equal(DataType.from('mediatype').scalar('README').asJs(), 'application/octet-stream')
    assert.equal(
      DataType.from('mediatype').scalar('part.tgz').asJs(),
      'application/x-tar;encodings=application/gzip',
    )
    ```

## Arrow storage

`Utf8` holding the canonical text - a base, and for a media type its charset
and codings in the same rendering - under the extension names
`yggdryl.mimetype` and `yggdryl.mediatype`, which is what keeps the column its
own datatype across a round trip. A cast into either column canonicalizes every
cell on the way in.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    for (dtype, extension) in [
        (DataType::MimeType, "yggdryl.mimetype"),
        (DataType::MediaType, "yggdryl.mediatype"),
    ] {
        let field = Field::new("held", dtype.clone(), true);
        let arrow = field.clone().into_arrow_field()?;
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(arrow.metadata()["ARROW:extension:name"], extension);
        assert_eq!(Field::from_arrow_field(&arrow)?, field);
    }
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import Field

    for field, extension in (
        (yggdryl.mimetype("held"), b"yggdryl.mimetype"),
        (yggdryl.mediatype("held"), b"yggdryl.mediatype"),
    ):
        arrow = field.into_arrow()
        assert arrow.type == pa.string()
        assert arrow.metadata[b"ARROW:extension:name"] == extension
        assert Field.from_arrow(arrow) == field

    # A cast into the column canonicalizes every cell on the way in.
    assert yggdryl.mimetype("held").cast_arrow_array(
        pa.array(["APPLICATION/JSON"])
    ).to_pylist() == ["application/json"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    // A cast through a struct root answers the Arrow field a column is written as.
    const projected = (field, value) =>
      fields
        .struct('row', [field], { nullable: false })
        .castArrow(
          new arrow.Table({ [field.name]: arrow.vectorFromArray([value], new arrow.Utf8()) }),
          { safe: false },
        )

    const held = projected(fields.mimetype('held', { nullable: false }), 'APPLICATION/JSON')
    assert.equal(String(held.schema.fields[0].type), 'Utf8')
    assert.equal(held.schema.fields[0].metadata.get('ARROW:extension:name'), 'yggdryl.mimetype')
    // A cast into the column canonicalizes every cell on the way in.
    assert.deepEqual(Array.from(held.getChild('held')), ['application/json'])

    const declared = projected(fields.mediatype('held', { nullable: false }), 'TEXT/CSV; CHARSET=UTF-8')
    assert.equal(
      declared.schema.fields[0].metadata.get('ARROW:extension:name'),
      'yggdryl.mediatype',
    )
    assert.deepEqual(Array.from(declared.getChild('held')), ['text/csv;charset=utf-8'])
    ```

## A media type says more than a MIME type

A MIME type is one name. A media type is that name plus the charset and the
ordered content codings a handle was read under, which is why `part.tgz`
answers `application/x-tar;encodings=application/gzip` rather than one opaque
name, and why the two never merge into each other: a MIME type has no charset
and no coding list to keep. That asymmetry is also why a MIME column refuses
text that is not a `type/subtype` name while a media column has an answer for
every text - it is the crate's filename and content-negotiation reader, so its
intake is total by construction.

| rule | behaviour |
| --- | --- |
| Kind | `text`; the aliases are `MimeTypeField` and `MediaTypeField`, `yggdryl.mimetype` / `fields.mimetype` and `yggdryl.mediatype` / `fields.mediatype` |
| Value | `crate::MimeType` inline; `crate::MediaType` behind one shared pointer, because a base, a charset and a coding list are wider than the scalar |
| Storage | `Utf8` holding the canonical text, extension names `yggdryl.mimetype` and `yggdryl.mediatype` |
| Intake | a MIME type refuses a name that is not `type/subtype`; a media type infers, so every text has an answer |
| Default | `application/octet-stream`, which is what both values answer `Default` with |
| Merging | only with itself, and never with each other: a media type models a charset and codings a MIME type does not |
| Bindings | both cross as their canonical text; the wrapper classes are the media routing values |

## Edges

- `APPLICATION/JSON` -> `application/json`: case folds to the one canonical spelling, and an unregistered but well-formed name is kept as written, lower-cased.
- `not a type` in a MIME column -> refused naming the datatype; the same text in a media column -> the default base, because a media type's intake is total.
- The empty text -> no value: like every non-text column, it reads as null, and a required column refuses that null.
- `text/csv` as a MIME type and `text/csv` as a media type are two values and never compare equal; neither merges with the other, or with `utf8`.
- A bare type and the same type under a charset are two values: `text/csv` is not `text/csv; charset=utf-8`.
- Hashing reads the canonical text, so `TEXT/CSV` and `text/csv` are one hash.
- The canonical default of both is `application/octet-stream`.
- A stored column under `yggdryl.mimetype` or `yggdryl.mediatype` over a storage that is not `Utf8` -> a foreign field wearing our name, imported as its storage.
- A [charset](../charset/index.md) a media type declares is the charset vocabulary, not a [string](text/string.md) leaf: a column of media types stores names, never the bytes they describe.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test media_type
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test media -- mod_
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py
    python/.venv/bin/python -m pytest python/tests/media/test_init.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fields.test.js
    node --test node/tests/media/index.test.js
    ```
