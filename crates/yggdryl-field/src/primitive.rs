//! The `primitive_field!` macro — the single source of a primitive field, stamped out
//! once per data type (`I8Field`, …, `F64Field`, `BooleanField`) so every field
//! shares one implementation, mirroring the dtype layer's `primitive_type!` macro.
//!
//! A field never touches the value codec (it only holds a name + nullable flag and
//! names its data type), so the **same** macro covers every primitive — including
//! `Boolean`, unlike the dtype and buffer layers where the bit-packed member is
//! hand-written.

/// Generates one primitive field named `$field` whose values have data type `$dtype`
/// (native `$native`), with canonical name `$lit`.
macro_rules! primitive_field {
    ($field:ident, $dtype:ident, $native:ty, $lit:literal) => {
        #[doc = concat!("A named, nullable `", $lit, "` field, with optional headers.")]
        ///
        /// Holds a name, a nullable flag, and optional
        /// [`Headers`](yggdryl_http::Headers); its data type is
        #[doc = concat!("[`", stringify!($dtype), "`](yggdryl_dtype::", stringify!($dtype), ").")]
        /// It converts to and from an Arrow [`Field`](arrow_schema::Field) (headers are
        /// yggdryl-side only, so `to_arrow` omits them) and round-trips through bytes.
        ///
        #[doc = concat!("```")]
        #[doc = concat!("use yggdryl_dtype::DataType;")]
        #[doc = concat!("use yggdryl_http::{Headers, HeadersBased};")]
        #[doc = concat!("use yggdryl_field::{Field, TypedField, ", stringify!($field), "};")]
        #[doc = concat!("let field = ", stringify!($field), "::new(\"col\", true)")]
        #[doc = concat!("    .with_headers(Headers::from_pairs([(b\"k\".to_vec(), b\"v\".to_vec())]));")]
        #[doc = concat!("assert_eq!(field.name(), \"col\");")]
        #[doc = concat!("assert!(field.is_nullable());")]
        #[doc = concat!("assert_eq!(field.get_header(b\"k\"), Some(b\"v\".as_slice()));")]
        #[doc = concat!("assert_eq!(TypedField::data_type(&field).name(), \"", $lit, "\");")]
        #[doc = concat!("// Byte round-trip (headers included) and Arrow round-trip (headers dropped).")]
        #[doc = concat!("assert_eq!(", stringify!($field), "::deserialize_bytes(&field.serialize_bytes()).unwrap(), field);")]
        #[doc = concat!("assert_eq!(", stringify!($field), "::from_arrow(&field.to_arrow()).unwrap().name(), \"col\");")]
        #[doc = concat!("```")]
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $field {
            name: String,
            nullable: bool,
            headers: Option<yggdryl_http::Headers>,
        }

        impl $field {
            #[doc = concat!("Creates a `", $lit, "` field with the given `name` and nullability (no headers).")]
            pub fn new(name: impl Into<String>, nullable: bool) -> Self {
                Self {
                    name: name.into(),
                    nullable,
                    headers: None,
                }
            }

            /// Reconstructs the field from its serialised bytes: a 1-byte nullable flag,
            /// the length-prefixed UTF-8 name, then the headers bytes when present.
            ///
            /// # Errors
            /// [`FieldError`](crate::FieldError) if the payload is empty, truncated, or
            /// the name is not valid UTF-8.
            pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, $crate::FieldError> {
                let (&flag, rest) = bytes.split_first().ok_or($crate::FieldError::EmptyPayload)?;
                let (len_bytes, rest) = rest
                    .split_first_chunk::<4>()
                    .ok_or($crate::FieldError::Truncated { context: "field name" })?;
                let name_len = u32::from_le_bytes(*len_bytes) as usize;
                if rest.len() < name_len {
                    return Err($crate::FieldError::Truncated { context: "field name" });
                }
                let (name_bytes, rest) = rest.split_at(name_len);
                let name = core::str::from_utf8(name_bytes)
                    .map_err(|error| $crate::FieldError::InvalidUtf8 {
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

            #[doc = concat!("Builds the field from an Arrow [`Field`](arrow_schema::Field), validating its data type is `", stringify!($dtype), "` (Arrow metadata is not read).")]
            ///
            /// # Errors
            /// [`FieldError::Dtype`](crate::FieldError::Dtype) if the Arrow field's data
            /// type is a different variant.
            pub fn from_arrow(field: &arrow_schema::Field) -> Result<Self, $crate::FieldError> {
                yggdryl_dtype::$dtype::from_arrow(field.data_type())?;
                Ok(Self {
                    name: field.name().to_string(),
                    nullable: field.is_nullable(),
                    headers: None,
                })
            }
        }

        impl $crate::Field for $field {
            fn name(&self) -> &str {
                &self.name
            }

            fn is_nullable(&self) -> bool {
                self.nullable
            }

            fn arrow_data_type(&self) -> arrow_schema::DataType {
                <yggdryl_dtype::$dtype as yggdryl_dtype::DataType>::to_arrow(&yggdryl_dtype::$dtype::new())
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

        impl $crate::TypedField<yggdryl_dtype::$dtype, $native> for $field {
            fn data_type(&self) -> yggdryl_dtype::$dtype {
                yggdryl_dtype::$dtype::new()
            }
        }

        impl $crate::PrimitiveField for $field {}

        impl yggdryl_http::HeadersBased for $field {
            fn headers(&self) -> Option<&yggdryl_http::Headers> {
                self.headers.as_ref()
            }

            fn headers_mut(&mut self) -> &mut Option<yggdryl_http::Headers> {
                &mut self.headers
            }
        }
    };
}

pub(crate) use primitive_field;
