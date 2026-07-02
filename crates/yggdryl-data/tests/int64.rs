//! Integration tests for the first concrete case — `Int64` / `Int64Scalar` — and the
//! trait stack it exercises (raw, typed, category).

use yggdryl_data::{
    DataError, DataType, Field, Int64, Int64Scalar, Primitive, RawDataType, RawField, RawScalar,
    Scalar,
};

#[test]
fn int64_describes_itself() {
    assert_eq!(Int64.name(), "int64");
    assert_eq!(Int64.arrow_format(), "l");
    assert_eq!(Int64.byte_width(), Some(8));
    assert_eq!(Int64.bit_width(), Some(64)); // default: eight times the byte width
}

#[test]
fn int64_codec_round_trips() {
    for value in [0i64, 1, -1, i64::MIN, i64::MAX, 42] {
        let bytes = Int64.native_to_bytes(&value);
        assert_eq!(bytes.len(), 8);
        assert_eq!(Int64.native_from_bytes(&bytes).unwrap(), value);
    }
    // Little-endian layout.
    assert_eq!(Int64.native_to_bytes(&1), vec![1, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn int64_decode_rejects_the_wrong_length() {
    let error = Int64.native_from_bytes(&[1, 2, 3]).unwrap_err();
    assert!(matches!(
        error,
        DataError::InvalidByteLength {
            expected: 8,
            got: 3
        }
    ));
}

#[test]
fn int64_scalar_holds_a_value_or_null() {
    let answer = Int64Scalar::new(42);
    assert!(!answer.is_null());
    assert_eq!(answer.value(), Some(&42));
    assert_eq!(answer.data_type().name(), "int64");

    let missing = Int64Scalar::null();
    assert!(missing.is_null());
    assert_eq!(missing.value(), None);
    assert_eq!(Int64Scalar::default(), missing); // default is null
}

// A field of int64, exercising the raw and typed field traits together.
#[derive(Debug)]
struct Column {
    name: String,
    data_type: Int64,
    nullable: bool,
}

impl RawField<Int64> for Column {
    fn name(&self) -> &str {
        &self.name
    }
    fn data_type(&self) -> &Int64 {
        &self.data_type
    }
    fn is_nullable(&self) -> bool {
        self.nullable
    }
}

impl Field<i64> for Column {
    type Type = Int64;
}

#[test]
fn typed_field_pairs_a_name_with_int64() {
    let id = Column {
        name: "id".to_string(),
        data_type: Int64,
        nullable: false,
    };
    assert_eq!(id.name(), "id");
    assert_eq!(id.data_type().name(), "int64");
    assert!(!id.is_nullable());
}

// Generic code over the typed traits composes across raw/typed/category.
fn first_byte<D: DataType<i64>>(data_type: &D, value: i64) -> u8 {
    data_type.native_to_bytes(&value)[0]
}

fn is_null_scalar<S: Scalar<i64>>(scalar: &S) -> bool {
    scalar.is_null()
}

fn primitive_bit_width<P: Primitive>(primitive: &P) -> usize {
    // Bit width is the invariant shared by every primitive (a boolean is one bit and
    // has no byte width).
    primitive
        .bit_width()
        .expect("a primitive has a fixed bit width")
}

#[test]
fn generic_bounds_compose() {
    assert_eq!(first_byte(&Int64, 5), 5);
    assert!(is_null_scalar(&Int64Scalar::null()));
    assert!(!is_null_scalar(&Int64Scalar::new(1)));
    assert_eq!(primitive_bit_width(&Int64), 64);
}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn the_type_system_is_send_and_sync() {
    // Types, fields and scalars are shared across threads and handed over FFI.
    assert_send_sync::<Int64>();
    assert_send_sync::<Int64Scalar>();
    assert_send_sync::<Column>();
}

#[test]
fn raw_data_type_is_object_safe() {
    // A heterogeneous schema holds `Box<dyn RawDataType>` (and stays Send + Sync).
    let types: Vec<Box<dyn RawDataType>> = vec![Box::new(Int64)];
    assert_send_sync::<Box<dyn RawDataType>>();
    assert_eq!(types[0].name(), "int64");
    assert_eq!(types[0].arrow_format(), "l");
    assert_eq!(types[0].byte_width(), Some(8));
    assert_eq!(types[0].bit_width(), Some(64));
}

// A minimal string type and scalar, proving an *unsized* value reaches the typed
// `Scalar<str>` layer — the borrowed `Option<&str>` the `?Sized` value enables.
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
