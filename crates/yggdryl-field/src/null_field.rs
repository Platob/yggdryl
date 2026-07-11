//! [`NullField`] — a named `null` field.

use crate::{Field, FieldError, TypedField};

/// A named, nullable `null` field, with optional headers.
///
/// Holds a name, a nullable flag, and optional [`Headers`](yggdryl_http::Headers); its data
/// type is [`NullType`](yggdryl_dtype::NullType). Like [`NullType`](yggdryl_dtype::NullType)
/// it is **sui generis** — it joins no category trait ([`PrimitiveField`](crate::PrimitiveField)
/// / logical / nested) — but otherwise mirrors a primitive field: it converts to and from an
/// Arrow [`Field`](arrow_schema::Field) (headers are yggdryl-side only, so `to_arrow` omits
/// them) and round-trips through bytes.
///
/// ```
/// use yggdryl_dtype::DataType;
/// use yggdryl_http::{Headers, HeadersBased};
/// use yggdryl_field::{Field, NullField, TypedField};
///
/// let field = NullField::new("maybe", true)
///     .with_headers(Headers::from_pairs([(b"k".to_vec(), b"v".to_vec())]));
/// assert_eq!(field.name(), "maybe");
/// assert!(field.is_nullable());
/// assert_eq!(field.get_header(b"k"), Some(b"v".as_slice()));
/// assert_eq!(TypedField::data_type(&field).name(), "null");
/// // Byte round-trip (headers included) and Arrow round-trip (headers dropped).
/// assert_eq!(NullField::deserialize_bytes(&field.serialize_bytes()).unwrap(), field);
/// assert_eq!(NullField::from_arrow(&field.to_arrow()).unwrap().name(), "maybe");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NullField {
    name: String,
    nullable: bool,
    headers: Option<yggdryl_http::Headers>,
}

impl NullField {
    /// Creates a `null` field with the given `name` and nullability (no headers).
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            nullable,
            headers: None,
        }
    }

    /// Reconstructs the field from its serialised bytes: a 1-byte nullable flag, the
    /// length-prefixed UTF-8 name, then the headers bytes when present.
    ///
    /// # Errors
    /// [`FieldError`] if the payload is empty, truncated, or the name is not valid UTF-8.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, FieldError> {
        let (&flag, rest) = bytes.split_first().ok_or(FieldError::EmptyPayload)?;
        let (len_bytes, rest) = rest.split_first_chunk::<4>().ok_or(FieldError::Truncated {
            context: "field name",
        })?;
        let name_len = u32::from_le_bytes(*len_bytes) as usize;
        if rest.len() < name_len {
            return Err(FieldError::Truncated {
                context: "field name",
            });
        }
        let (name_bytes, rest) = rest.split_at(name_len);
        let name = core::str::from_utf8(name_bytes)
            .map_err(|error| FieldError::InvalidUtf8 {
                valid_up_to: error.valid_up_to(),
            })?
            .to_string();
        let headers = if rest.is_empty() {
            None
        } else {
            Some(yggdryl_http::Headers::deserialize_bytes(rest)?)
        };
        Ok(Self {
            name,
            nullable: flag != 0,
            headers,
        })
    }

    /// Builds the field from an Arrow [`Field`](arrow_schema::Field), validating its data
    /// type is `Null` (Arrow metadata is not read).
    ///
    /// # Errors
    /// [`FieldError::Dtype`] if the Arrow field's data type is a different variant.
    pub fn from_arrow(field: &arrow_schema::Field) -> Result<Self, FieldError> {
        yggdryl_dtype::NullType::from_arrow(field.data_type())?;
        Ok(Self {
            name: field.name().to_string(),
            nullable: field.is_nullable(),
            headers: None,
        })
    }
}

impl Field for NullField {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn arrow_data_type(&self) -> arrow_schema::DataType {
        <yggdryl_dtype::NullType as yggdryl_dtype::DataType>::to_arrow(
            &yggdryl_dtype::NullType::new(),
        )
    }

    fn serialize_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + self.name.len());
        out.push(u8::from(self.nullable));
        out.extend_from_slice(&(self.name.len() as u32).to_le_bytes());
        out.extend_from_slice(self.name.as_bytes());
        if let Some(headers) = &self.headers {
            out.extend_from_slice(&headers.serialize_bytes());
        }
        out
    }
}

impl TypedField<yggdryl_dtype::NullType, ()> for NullField {
    fn data_type(&self) -> yggdryl_dtype::NullType {
        yggdryl_dtype::NullType::new()
    }
}

impl yggdryl_http::HeadersBased for NullField {
    fn headers(&self) -> Option<&yggdryl_http::Headers> {
        self.headers.as_ref()
    }

    fn headers_mut(&mut self) -> &mut Option<yggdryl_http::Headers> {
        &mut self.headers
    }
}
