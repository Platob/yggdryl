//! `rust/src/iceberg/schema.rs`: the schema object a caller's document
//! spells.
//!
//! These pin that an array in that document is read whichever way the caller
//! holds it - a run or a column - through `yggdryl::iceberg::schema_from_json`.

use yggdryl::iceberg::schema_from_json;
use yggdryl::{DataType, Field, Scalar, Serie};

#[test]
fn identifier_field_ids_read_the_same_from_a_column() -> yggdryl::Result<()> {
    let ids = Serie::from_scalars(
        Field::new("item", DataType::Int32, false),
        [Scalar::from(1_i32)],
    )?;
    let document = yggdryl::json::from_utf8(
        r#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"}
        ]}"#,
    )?
    .with_field("identifier-field-ids", Scalar::from(ids))?;
    let root = schema_from_json("row", &document)?;
    assert_eq!(root.as_iceberg().identifier_field_ids()?, [1]);
    Ok(())
}

#[test]
fn a_fields_column_reads_the_same_as_the_array() -> yggdryl::Result<()> {
    let fields = Serie::empty(Field::new(
        "item",
        DataType::from_str("map<utf8, utf8>")?,
        false,
    ))?;
    let document = yggdryl::json::from_utf8(r#"{"type":"struct","schema-id":0}"#)?
        .with_field("fields", Scalar::from(fields))?;
    assert_eq!(schema_from_json("row", &document)?.field_len(), 0);
    Ok(())
}

#[test]
fn an_unknown_declaration_is_written_only_where_it_holds() -> yggdryl::Result<()> {
    let row = |column: Field| -> yggdryl::Result<Field> {
        let mut root =
            DataType::from(yggdryl::StructType::from_fields([column])?).required_field("row");
        yggdryl::iceberg::assign_field_ids(&mut root, 1)?;
        Ok(root)
    };

    // A declared variant writes `unknown`, a null column too; a required
    // one is refused, because every value it holds is null.
    let mut later = DataType::Variant.nullable_field("later");
    later.as_iceberg_mut().set_unknown(true)?;
    for column in [later.clone(), DataType::Null.nullable_field("later")] {
        let written = yggdryl::iceberg::schema_into_json(&row(column)?)?;
        assert!(
            yggdryl::json::into_utf8(&written)?.contains(r#""type":"unknown""#),
            "{written:?}"
        );
    }
    let mut required = later.clone().with_nullable(false);
    required.as_iceberg_mut().set_unknown(true)?;
    let message = yggdryl::iceberg::schema_into_json(&row(required)?)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("later") && message.contains("optional"),
        "{message}"
    );

    // A declaration this crate did not write, or one over another datatype,
    // is refused rather than read one way or the other.
    let mut foreign = DataType::Variant.nullable_field("later");
    foreign.as_iceberg_mut().insert("type", "void")?;
    let mut stale = DataType::Int64.nullable_field("later");
    stale.as_iceberg_mut().insert("type", "unknown")?;
    for (column, said) in [(foreign, "void"), (stale, "int64")] {
        let message = yggdryl::iceberg::schema_into_json(&row(column)?)
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("ICEBERG:type") && message.contains(said) && message.contains("later"),
            "{message}"
        );
    }
    Ok(())
}
