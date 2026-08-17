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

    assert_eq!(DataType::from_str("varchar(255)").unwrap(), DataType::Utf8);
    assert_eq!(
        DataType::from_str("double precision").unwrap(),
        DataType::Float64
    );
    assert_eq!(DataType::from_str("bytea").unwrap(), DataType::Binary);
}
