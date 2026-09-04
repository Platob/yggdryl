//! Metadata-aware equality and formatted schema differences.

use crate::field::{Differences, dtypes_equal, show_diff};

use super::DataType;

impl DataType {
    /// Compares datatypes, optionally including metadata on every nested field.
    ///
    /// With `with_metadata = true`, this is exactly [`PartialEq`]. With
    /// `with_metadata = false`, field metadata is ignored recursively while
    /// names, nullability, datatype parameters, and dictionary state remain
    /// significant.
    pub fn equals(&self, other: &Self, with_metadata: bool) -> bool {
        dtypes_equal(self, other, with_metadata)
    }

    /// Lazily yields stable, UTF-8 lines describing every difference.
    ///
    /// `return_equal` decides what an equal comparison yields: `false`
    /// yields nothing, and `true` yields one equal line so a caller
    /// rendering a full report never shows an empty section.
    pub fn show_diffs<'schema>(
        &'schema self,
        other: &'schema Self,
        with_metadata: bool,
        return_equal: bool,
    ) -> Differences<'schema> {
        Differences::from_dtypes(self, other, with_metadata, return_equal)
    }

    /// Returns all formatted differences joined with newlines.
    ///
    /// Equal values produce `✓ equal`.
    pub fn show_diff(&self, other: &Self, with_metadata: bool, return_equal: bool) -> String {
        show_diff(self.show_diffs(other, with_metadata, return_equal))
    }
}
