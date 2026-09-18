//! Run-end encoding: a run-ends column over a values column.

use std::cmp::Ordering;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::types::structure::cmp_fields;
use smol_str::format_smolstr;
use crate::types::invalid;
use crate::types::typed::define_field_types;
use crate::{
    DataType, Field, Result,
};

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

define_field_types!(
    RunEndEncodedTypeMarker,
    RunEndEncoded,
    crate::DataType::RunEndEncoded(_)
);
