//! The coarse family every datatype identifier answers to.

use yggdryl::DataTypeKind;

#[test]
fn names_round_trip_case_insensitively() {
    for kind in DataTypeKind::ALL {
        assert_eq!(DataTypeKind::from_str(kind.as_str()).unwrap(), kind);
        assert_eq!(
            DataTypeKind::from_str(&kind.as_str().to_uppercase()).unwrap(),
            kind
        );
    }
}

#[test]
fn unknown_name_reports_the_input_and_vocabulary() {
    let error = DataTypeKind::from_str("int32").unwrap_err();
    let message = error.to_string();
    assert!(message.contains("\"int32\""), "{message}");
    assert!(message.contains("integer"), "{message}");
}

#[test]
fn nested_is_the_one_coarse_family_for_every_nested_shape() {
    assert!(DataTypeKind::Nested.is_nested());
    assert!(!DataTypeKind::Bytes.is_nested());
}

#[test]
fn categories_are_unique() {
    let mut names: Vec<_> = DataTypeKind::ALL.iter().map(|kind| kind.as_str()).collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(names.len(), total);
}
