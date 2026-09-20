use yggdryl::{DataType, Field, StructType, UnionMode};

#[test]
fn variant_builder_canonicalizes_to_a_dense_sequential_union() {
    let members = [
        Field::new("number", DataType::Int64, false),
        Field::from_parts("text", DataType::utf8(), true, [("source", "variant")]).unwrap(),
    ];
    let variant = DataType::dense_union(members.clone()).unwrap();
    let union = DataType::union(
        [(0, members[0].clone()), (1, members[1].clone())],
        UnionMode::Dense,
    )
    .unwrap();

    assert_eq!(variant, union);
    assert_eq!(variant.name(), "union");
    assert_eq!(
        variant.to_string(),
        r#"union(dense,0=field("number",int64,nullable=false,metadata={}),1=field("text",utf8,nullable=true,metadata={"source":"variant"}))"#
    );
    assert_eq!(
        variant.clone().into_json().unwrap(),
        union.into_json().unwrap()
    );
    assert_eq!(
        DataType::from_arrow_datatype(&variant.clone().into_arrow_datatype().unwrap()).unwrap(),
        variant
    );
}

#[test]
fn variant_builder_enforces_the_arrow_type_id_capacity() {
    let accepted = DataType::dense_union(
        (0..128).map(|index| Field::new(format!("member_{index}"), DataType::Int64, true)),
    )
    .unwrap();
    assert_eq!(accepted.field_len(), 128);
    assert_eq!(accepted.get_field(127).map(Field::name), Some("member_127"));

    let error = DataType::dense_union(
        (0..129).map(|index| Field::new(format!("member_{index}"), DataType::Int64, true)),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("variant cannot contain more than 128 members"),
        "{error}"
    );
}

#[test]
fn variant_builder_reuses_union_child_validation() {
    let duplicate = DataType::dense_union([
        Field::new("same", DataType::Int64, false),
        Field::new("same", DataType::utf8(), true),
    ])
    .unwrap_err();
    assert!(
        duplicate
            .to_string()
            .contains("duplicate field name \"same\"")
    );

    let empty = DataType::dense_union([]).unwrap();
    assert_eq!(empty, DataType::union([], UnionMode::Dense).unwrap());
}

#[test]
fn deeply_nested_variants_round_trip_without_a_second_logical_type() {
    let mut value = DataType::Int64;
    for depth in 0..24 {
        value = DataType::dense_union([Field::new(format!("level_{depth}"), value, true)]).unwrap();
    }

    value.validate().unwrap();
    assert_eq!(DataType::from_str(&value.to_string()).unwrap(), value);
    assert_eq!(
        DataType::from_json(&value.clone().into_json().unwrap()).unwrap(),
        value
    );
    assert_eq!(
        DataType::from_arrow_datatype(&value.clone().into_arrow_datatype().unwrap()).unwrap(),
        value
    );
}

#[test]
fn deeply_nested_sql_hive_and_spark_types_round_trip_canonically() {
    let values = [
        "array<struct<id:bigint,name:string,tags:array<string>>>",
        "map<string,array<decimal(38,18)>>",
        "struct<`quoted,name`:string, nested:map<string,struct<x:int,y:boolean>>>",
        "row(id integer not null, payload varbinary)",
    ];
    for value in values {
        let parsed = DataType::from_str(value).unwrap();
        let canonical = parsed.to_string();
        assert_eq!(
            DataType::from_str(&canonical).unwrap(),
            parsed,
            "{canonical}"
        );
    }
}

#[test]
fn wide_struct_validation_accepts_unique_names_and_reports_a_late_duplicate() {
    let fields = (0..1_024)
        .map(|index| Field::new(format!("column_{index:04}"), DataType::Int64, false))
        .collect::<Vec<_>>();
    let dtype = DataType::from(StructType::from_fields(fields.clone()).unwrap());
    assert_eq!(dtype.field_len(), 1_024);

    let mut duplicate = fields;
    duplicate.push(Field::new("column_0001", DataType::utf8(), true));
    let error = StructType::from_fields(duplicate)
        .map(DataType::from)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("duplicate field name \"column_0001\"")
    );
}

#[test]
fn the_nested_family_stands_for_a_sequence_a_mapping_and_a_record() {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use yggdryl::{DataTypeKind, FamilyValue, Map, Mapping, Nested, Scalar, Serie, Struct};

    let sequence = Serie::new(Arc::from([Scalar::from(1_i64), Scalar::from(2_i64)]));
    let mapping = Mapping::Map(Map::new(Arc::from([(
        Scalar::from("k"),
        Scalar::from(1_i64),
    )])));
    let record = Struct::new(Arc::new(BTreeMap::from([(
        "id".into(),
        Scalar::from(1_i64),
    )])));

    crate::scalar::assert_family_round_trip(
        vec![
            crate::family_leaf!(Nested::Sequence, sequence.clone()),
            crate::family_leaf!(Nested::Mapping, mapping.clone()),
            crate::family_leaf!(Nested::Struct, record.clone()),
        ],
        DataTypeKind::Nested,
        &Scalar::from(1_i64),
    );

    // The family answers the datatype the held leaf's children name: a list
    // of the items, a map of the keys and values, a struct of the fields.
    assert_eq!(
        Nested::from(sequence).dtype().unwrap(),
        DataType::list(DataType::Int64.required_field("item"))
    );
    assert_eq!(
        Nested::from(mapping).dtype().unwrap(),
        DataType::map_of(DataType::utf8(), DataType::Int64, false).unwrap()
    );
    assert_eq!(
        Nested::from(record).dtype().unwrap(),
        DataType::from(StructType::from_fields([DataType::Int64.required_field("id")]).unwrap())
    );
}
