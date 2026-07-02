//! `Field` construction, functional updates and rendering.

use yggdryl_schema::{Field, Int32Type, TypedField, TypedFieldRef, Utf8Type};

#[test]
fn copy_overrides_only_the_given_parts() {
    let metadata = [("k".to_string(), "v".to_string())].into_iter().collect();
    let field = TypedField::from_parts("id", Int32Type, false, metadata);

    let renamed = field.copy(Some("key".to_string()), None, None, None);
    assert_eq!(renamed.name(), "key");
    assert_eq!(renamed.data_type(), &Int32Type);
    assert!(!renamed.nullable());
    assert_eq!(renamed.metadata(), field.metadata());

    let unchanged = field.copy(None, None, None, None);
    assert_eq!(unchanged, field);
}

#[test]
fn with_and_without_delegate_to_copy() {
    let field = TypedField::from_parts("id", Int32Type, false, Default::default());

    assert_eq!(field.with_name("key").name(), "key");
    assert_eq!(field.with_data_type(Int32Type), field);
    assert!(field.with_nullable(true).nullable());

    let metadata = [("k".to_string(), "v".to_string())].into_iter().collect();
    let tagged = field.with_metadata(metadata);
    assert_eq!(tagged.metadata().get("k").unwrap(), "v");
    assert!(tagged.without_metadata().metadata().is_empty());

    // Updates never mutate the original.
    assert_eq!(field.name(), "id");
    assert!(field.metadata().is_empty());
}

#[test]
fn display_renders_name_type_and_nullability() {
    assert_eq!(
        TypedField::from_parts("id", Int32Type, false, Default::default()).to_string(),
        "id: int32"
    );
    assert_eq!(
        TypedField::from_parts("name", Utf8Type, true, Default::default()).to_string(),
        "name: utf8?"
    );
}

#[test]
fn field_ref_is_the_shared_handle() {
    let field: TypedFieldRef<Int32Type> =
        TypedField::from_parts("id", Int32Type, false, Default::default()).into();
    let shared = field.clone();
    assert_eq!(*shared, *field);
}

#[test]
fn per_type_fields_have_their_own_implementations() {
    use yggdryl_schema::{AnyField, Int64Field, Int64Type, TypedField};

    let field = Int64Field::from_parts("id", Int64Type, false, Default::default());
    assert_eq!(field.name(), "id");
    assert_eq!(field.to_string(), "id: int64");
    assert_eq!(Int64Field::from_arrow(&field.to_arrow()), Ok(field.clone()));
    assert_eq!(Int64Field::from_bytes(&field.to_bytes()), Ok(field.clone()));

    // Family members convert to and from the generic engine.
    let engine: TypedField<Int64Type> = field.clone().into();
    assert_eq!(Int64Field::from(engine), field);

    // The erased family member holds any data type.
    let any = AnyField::from_parts("x", Int64Type.into(), true, Default::default());
    assert_eq!(AnyField::from_arrow(&any.to_arrow()), Ok(any));
}
