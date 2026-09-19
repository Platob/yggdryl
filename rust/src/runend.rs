//! Run-end encoding: a run-ends column over a values column.

use std::cmp::Ordering;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::invalid;
use crate::structure::cmp_fields;
use crate::{DataType, Field, Result};
use smol_str::format_smolstr;

/// Shared run-end encoding child fields.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct RunEndEncodedType {
    pub(crate) run_ends: Field,
    pub(crate) values: Field,
}

impl RunEndEncodedType {
    /// Returns the non-null signed integer run-end field.
    pub const fn run_ends(&self) -> &Field {
        &self.run_ends
    }

    /// Returns the encoded values field.
    pub const fn values(&self) -> &Field {
        &self.values
    }
}

impl Ord for RunEndEncodedType {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_fields(&self.run_ends, &other.run_ends)
            .then_with(|| cmp_fields(&self.values, &other.values))
    }
}

impl PartialOrd for RunEndEncodedType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'de> Deserialize<'de> for RunEndEncodedType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            run_ends: Field,
            values: Field,
        }
        let repr = Repr::deserialize(deserializer)?;
        validate_run_ends(&repr.run_ends).map_err(serde::de::Error::custom)?;
        Ok(Self {
            run_ends: repr.run_ends,
            values: repr.values,
        })
    }
}

impl DataType {
    /// Creates a run-end encoded type after validating its run-end field.
    pub fn run_end_encoded(run_ends: Field, values: Field) -> Result<Self> {
        validate_run_ends(&run_ends)?;
        Ok(Self::RunEndEncoded(Arc::new(RunEndEncodedType {
            run_ends,
            values,
        })))
    }
}

pub(crate) fn validate_run_ends(run_ends: &Field) -> Result<()> {
    // Two independent rules; report the one that actually fired so a caller
    // fixes the right half.
    if run_ends.is_nullable() {
        return Err(invalid(
            "RunEndEncoded",
            format_smolstr!(
                "expected a non-null run_ends field, got nullable field {:?}",
                run_ends.name()
            ),
        ));
    }
    if !run_ends.dtype().is_run_ends_type() {
        return Err(invalid(
            "RunEndEncoded",
            format_smolstr!(
                "expected a run_ends datatype of int16, int32, or int64, got {}",
                run_ends.dtype()
            ),
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------------
// Arrow projection: a run-end column beside the values it repeats.
// ------------------------------------------------------------------------

mod arrow {
    use std::sync::Arc;

    use arrow_schema::DataType as ArrowDataType;

    use super::{RunEndEncodedType, validate_run_ends};
    use crate::value::ArrowFfiParts;
    use crate::{DataType, Field, Result};

    impl RunEndEncodedType {
        /// The Arrow storage this encoding lays out.
        ///
        /// # Errors
        ///
        /// Returns an error when the run ends are not a non-null signed integer
        /// column, or either child has no Arrow projection.
        pub(crate) fn arrow_storage(&self) -> Result<ArrowDataType> {
            validate_run_ends(&self.run_ends)?;
            Ok(ArrowDataType::RunEndEncoded(
                self.run_ends.clone().into_arrow_field_ref()?,
                self.values.clone().into_arrow_field_ref()?,
            ))
        }

        /// The same projection, consuming a uniquely held encoding.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn into_arrow_storage(encoded: Arc<Self>) -> Result<ArrowDataType> {
            match Arc::try_unwrap(encoded) {
                Ok(encoded) => {
                    validate_run_ends(&encoded.run_ends)?;
                    Ok(ArrowDataType::RunEndEncoded(
                        encoded.run_ends.into_arrow_field_ref()?,
                        encoded.values.into_arrow_field_ref()?,
                    ))
                }
                Err(encoded) => encoded.arrow_storage(),
            }
        }

        /// The C Data Interface node this encoding writes.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn arrow_ffi_parts(&self) -> Result<ArrowFfiParts> {
            validate_run_ends(&self.run_ends)?;
            Ok(ArrowFfiParts::nested(
                "+r",
                vec![
                    self.run_ends.clone().into_arrow_field_ffi()?,
                    self.values.clone().into_arrow_field_ffi()?,
                ],
            ))
        }

        /// The run-end datatype one Arrow run-end storage imports as.
        ///
        /// # Errors
        ///
        /// Returns an error when either child cannot be imported or the run
        /// ends are not a non-null signed integer column.
        pub(crate) fn from_arrow_storage_at_depth(
            run_ends: &arrow_schema::FieldRef,
            values: &arrow_schema::FieldRef,
            depth: usize,
        ) -> Result<DataType> {
            DataType::run_end_encoded(
                Field::from_arrow_field_ref_at_depth(Arc::clone(run_ends), depth)?,
                Field::from_arrow_field_ref_at_depth(Arc::clone(values), depth)?,
            )
        }

        /// The same import, consuming Arrow's shared children.
        ///
        /// # Errors
        ///
        /// [`Self::from_arrow_storage_at_depth`] carries the rule.
        pub(crate) fn from_arrow_storage_owned_at_depth(
            run_ends: arrow_schema::FieldRef,
            values: arrow_schema::FieldRef,
            depth: usize,
        ) -> Result<DataType> {
            DataType::run_end_encoded(
                Field::from_arrow_field_ref_at_depth(run_ends, depth)?,
                Field::from_arrow_field_ref_at_depth(values, depth)?,
            )
        }
    }
}
