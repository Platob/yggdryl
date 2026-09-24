# Unit

The unit a quantity is stated in: FIX `UnitOfMeasure(996)`, held as the text it is - `Shares`, `Bbl`, `MWh` - under its own identity, the way [`timeinforce`](timeinforce.md) holds a wire value rather than a name for it.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `unit`, `UnitType`/`UnitField`, the `Unit` value and `Scalar::Unit` |
| Validates | US-ASCII, no NUL, at most thirty-two bytes - and nothing else: no registry closes the space a venue's own unit may take |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A thirty-third byte or a byte past `0x7F`, naming the width: `at most 32 bytes` |

`Unit::none()` is the empty unit, the value stated as none: it is the code's default, and a [merge](index.md#the-code-family-value) takes the other unit over it.

## Rust

The bindings follow in a later step; the Rust surface is the whole of it for now.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::CodeValue as _;
    use yggdryl::{DataType, DataTypeKind, Field, Scalar, Unit, UnitField};

    // The datatype: one spelling, the code family, a bound rather than a layout.
    assert_eq!(DataType::from_str("unit")?, DataType::Unit);
    assert_eq!(DataType::unit().to_string(), "unit");
    assert_eq!(DataType::Unit.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Unit.code_name(), Some("unit"));
    assert_eq!(DataType::Unit.code_width(), Some(32));
    assert_eq!(DataType::Unit.fixed_byte_width(), None);

    // The field: the typed marker over the variant.
    let field = UnitField::unit("unit", true);
    assert_eq!(field.to_field(), Field::new("unit", DataType::Unit, true));

    // The value: the text under the unit identity, and its wire.
    let shares = DataType::Unit.scalar("Shares")?;
    assert_eq!(shares, Scalar::Unit(Unit::new("Shares")?));
    assert_eq!(shares.as_str(), Some("Shares"));
    assert_eq!(shares.kind(), "unit");
    assert_eq!(serde_json::to_string(&shares).unwrap(), r#"{"type":"unit","value":"Shares"}"#);

    // None, and the merge that takes the other unit over it.
    assert!(Unit::none().is_none());
    assert_eq!(DataType::Unit.default_value()?, Scalar::Unit(Unit::none()));
    assert_eq!(Unit::none().merge_with(&Unit::new("MWh")?), Unit::new("MWh")?);

    // Arrow: `Utf8` under `yggdryl.unit`, and the identity comes back.
    let arrow = Field::new("unit", DataType::Unit, false).into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.unit");

    // The width is the whole rule, and it names itself in the refusal.
    let refused = DataType::Unit.scalar("X".repeat(33).as_str()).unwrap_err().to_string();
    assert!(refused.contains("32 bytes"), "{refused}");
    ```

## Edges

- `at most 32 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`.
- Nothing gates the value: this code declares no vocabulary, so a unit no standard names is held as it stands.
- The default value is the empty unit, `Unit::none()`, answered as a `unit` scalar ([Cast](../cast.md#empty-text)).
- A `unit` and a [`timeinforce`](timeinforce.md) of the same bytes are two values: the identity leads, then the text.
- [`merge_with`](index.md#the-code-family-value) keeps this unit unless it is none, in which case the other stands.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- unit::coded code::datatypes
    ```
