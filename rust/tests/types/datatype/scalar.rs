use yggdryl::DataType;

#[test]
fn scalar_aliases_and_balanced_outer_wrappers_normalize() {
    for value in [
        "bigint",
        "BIGINT",
        "(bigint)",
        "[ bigint ]",
        "{bigint}",
        "'bigint'",
        "\"bigint\"",
    ] {
        assert_eq!(
            DataType::from_str(value).unwrap(),
            DataType::Int64,
            "{value}"
        );
    }

    assert_eq!(DataType::from_str("varchar").unwrap(), DataType::utf8());
    // A declared length is the maximum the column holds, which Arrow has
    // nowhere to say and this crate carries in its own metadata.
    assert_eq!(
        DataType::from_str("varchar(255)").unwrap().to_string(),
        "utf8(255)"
    );
    assert_eq!(
        DataType::from_str("double precision").unwrap(),
        DataType::Float64
    );
    assert_eq!(DataType::from_str("bytea").unwrap(), DataType::binary());
}
