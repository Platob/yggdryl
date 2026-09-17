//! [`MimeType`] as a scalar value.
//!
//! The value type itself lives beside [`crate::MimeType`]; this is only what
//! makes it a scalar.

use crate::types::scalar::{Value, text_scalar_value};
use crate::{DataType, MimeType, Result, Scalar};

text_scalar_value!(MimeType, MimeType, DataTypeId::MimeType, DataType::MimeType);
