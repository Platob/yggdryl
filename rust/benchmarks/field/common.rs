use yggdryl::Field;

pub(crate) fn nested_field() -> Field {
    Field::from_str(
        r#"field("record",struct<id:bigint,events:array<struct<name:string,value:decimal(18,4)>>>,nullable=false,metadata={"source":"benchmark","version":"1"})"#,
    )
    .expect("the static benchmark field must parse")
}
