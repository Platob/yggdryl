//! Integration tests for the trait layer itself, independent of any concrete type —
//! most notably that an *unsized* value (`str`) reaches the typed [`Scalar`] layer.

use yggdryl_data::{RawDataType, RawScalar, Scalar};

// A minimal string type and scalar, proving an unsized value reaches the typed
// `Scalar<str>` layer — the borrowed `Option<&str>` the `?Sized` value enables (an
// integer scalar, being `Sized`, cannot exercise this path).
#[derive(Debug)]
struct Utf8;

impl RawDataType for Utf8 {
    fn name(&self) -> &str {
        "utf8"
    }
    fn arrow_format(&self) -> String {
        "u".to_string()
    }
    fn byte_width(&self) -> Option<usize> {
        None // variable-width
    }
}

#[derive(Debug)]
struct Utf8Scalar {
    data_type: Utf8,
    value: Option<String>,
}

impl RawScalar<Utf8> for Utf8Scalar {
    type Value = str;
    fn data_type(&self) -> &Utf8 {
        &self.data_type
    }
    fn is_null(&self) -> bool {
        self.value.is_none()
    }
    fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

impl Scalar<str> for Utf8Scalar {
    type Type = Utf8;
}

#[test]
fn a_string_scalar_exposes_borrowed_str() {
    fn value_of<S: Scalar<str>>(scalar: &S) -> Option<&str> {
        scalar.value()
    }
    let hello = Utf8Scalar {
        data_type: Utf8,
        value: Some("hi".to_string()),
    };
    assert_eq!(value_of(&hello), Some("hi")); // Option<&str>, not Option<&String>
    assert!(value_of(&Utf8Scalar {
        data_type: Utf8,
        value: None
    })
    .is_none());
}

#[test]
fn a_variable_width_type_has_no_fixed_width() {
    assert_eq!(Utf8.byte_width(), None);
    assert_eq!(Utf8.bit_width(), None); // default: eight times a `None` width is `None`
}
