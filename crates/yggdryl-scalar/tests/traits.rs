//! Integration tests for the trait layer itself, independent of any concrete type —
//! most notably that an *unsized* value (`str`) reaches the typed [`TypedScalar`]
//! layer.

use yggdryl_scalar::arrow_array::Array; // len / is_null / null_count on the arrow side
use yggdryl_scalar::yggdryl_dtype::{DataError, DataType};
use yggdryl_scalar::{arrow_array, arrow_schema, yggdryl_core, Scalar, TypedScalar};

// A minimal string type and scalar, proving an unsized value reaches the typed
// `TypedScalar<Utf8, str>` layer — the borrowed `Option<&str>` the `?Sized` value
// enables (an integer scalar, being `Sized`, cannot exercise this path).
#[derive(Debug)]
struct Utf8;

impl DataType for Utf8 {
    fn name(&self) -> &str {
        "utf8"
    }
    fn arrow_format(&self) -> String {
        "u".to_string()
    }
    fn byte_width(&self) -> Option<usize> {
        None // variable-width
    }
    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::Utf8
    }
    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        match data_type {
            arrow_schema::DataType::Utf8 => Ok(Utf8),
            other => Err(DataError::IncompatibleArrowType {
                expected: "Utf8".to_string(),
                got: other.to_string(),
            }),
        }
    }
}

#[derive(Debug)]
struct Utf8Scalar {
    data_type: Utf8,
    value: Option<String>,
}

impl Scalar<Utf8> for Utf8Scalar {
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
    fn to_arrow(&self) -> arrow_array::ArrayRef {
        std::sync::Arc::new(match &self.value {
            Some(value) => arrow_array::StringArray::from_iter_values([value]),
            None => arrow_array::StringArray::new_null(1),
        })
    }
    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        if array.len() != 1 {
            return Err(DataError::InvalidScalarLength { got: array.len() });
        }
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .ok_or_else(|| DataError::IncompatibleArrowType {
                expected: "Utf8".to_string(),
                got: array.data_type().to_string(),
            })?;
        Ok(Utf8Scalar {
            data_type: Utf8,
            value: (!array.is_null(0)).then(|| array.value(0).to_string()),
        })
    }
    fn as_str(
        &self,
        charset: Option<&dyn yggdryl_core::Charset>,
    ) -> Result<std::borrow::Cow<'_, str>, DataError> {
        // The native type: borrowed directly, never copied; a string is already
        // decoded, so the charset never applies.
        let _ = charset;
        self.value
            .as_deref()
            .map(std::borrow::Cow::Borrowed)
            .ok_or(DataError::NullValue)
    }
}

impl TypedScalar<Utf8, str> for Utf8Scalar {}

#[test]
fn a_string_scalar_exposes_borrowed_str() {
    fn value_of<S: TypedScalar<Utf8, str>>(scalar: &S) -> Option<&str> {
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
fn string_access_borrows_without_copying() {
    let hello = Utf8Scalar {
        data_type: Utf8,
        value: Some("hi".to_string()),
    };
    // `as_str` hands back a borrow of the held String — same address, no copy.
    let borrowed = hello.as_str(None).unwrap();
    assert_eq!(borrowed, "hi");
    assert_eq!(borrowed.as_ptr(), hello.value.as_ref().unwrap().as_ptr());
    // A string is not a number (the trait defaults): an actionable error.
    assert!(matches!(
        hello.as_i64(),
        Err(DataError::UnsupportedConversion { data_type, target: "i64" }) if data_type == "utf8"
    ));
}

#[test]
fn a_variable_width_scalar_round_trips_through_arrow() {
    let hello = Utf8Scalar {
        data_type: Utf8,
        value: Some("hi".to_string()),
    };
    let arrow = hello.to_arrow();
    assert_eq!((arrow.len(), arrow.null_count()), (1, 0));
    assert_eq!(
        Utf8Scalar::from_arrow(arrow.as_ref()).unwrap().value(),
        Some("hi")
    );

    let missing = Utf8Scalar {
        data_type: Utf8,
        value: None,
    };
    assert!(Utf8Scalar::from_arrow(missing.to_arrow().as_ref())
        .unwrap()
        .is_null());
}
