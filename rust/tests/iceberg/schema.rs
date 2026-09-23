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
