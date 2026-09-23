//! `rust/src/fix/document.rs`: a field's `FIX:` document properties read as
//! the arrays they are, whichever shape holds the array.

use yggdryl::{DataType, Field, Scalar, Serie};

/// `document` with its `metadata.FIX:names` stated as `names`.
fn with_names(document: &Scalar, names: &Scalar) -> Scalar {
    let restated =
        |object: &Scalar, key: &str, value: &dyn Fn(&Scalar) -> Scalar| {
            Scalar::from_struct(object.as_struct().expect("an object").iter().map(
                |(held, stated)| {
                    if held == key {
                        (held.as_str(), value(stated))
                    } else {
                        (held.as_str(), stated.clone())
                    }
                },
            ))
            .expect("an object")
        };
    restated(document, "metadata", &|metadata| {
        restated(metadata, "FIX:names", &|_| names.clone())
    })
}

#[test]
fn a_list_property_held_as_a_column_loads_as_its_array() -> yggdryl::Result<()> {
    let run = yggdryl::from_json_scalar(
        r#"{"name":"OrderQty","dtype":{"type":"float64"},"nullable":true,
            "metadata":{"FIX:tag":"38","FIX:names":["Alias"]}}"#,
    )?;
    let names = Scalar::from(Serie::from_scalars(
        Field::new("item", DataType::utf8(), false),
        [Scalar::from("Alias")],
    )?);
    assert_eq!(names.as_sequence(), None, "the fixture holds a column");
    let column = with_names(&run, &names);

    let expected = yggdryl::from_fix_document(run)?;
    let loaded = yggdryl::from_fix_document(column)?;
    assert_eq!(expected.get_metadata("FIX:names"), Some(r#"["Alias"]"#));
    assert_eq!(
        loaded.get_metadata("FIX:names"),
        expected.get_metadata("FIX:names")
    );
    assert_eq!(loaded, expected);
    Ok(())
}
