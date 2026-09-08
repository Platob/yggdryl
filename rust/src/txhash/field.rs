//! The two properties that make a digest holder couple an instant.
//!
//! `digest:time` names the field whose instant leads the stored bytes, and
//! `digest:unit` states the clock resolution that instant is counted in,
//! microseconds when absent. Both live on the holder, beside its algorithm
//! and sources, so the field they read carries no metadata at all.

use smol_str::{SmolStr, format_smolstr};

use crate::types::protocol::{DigestField, DigestFieldMut};
use crate::{DataType, DigestAlgorithm, Error, Field, Result, TimeUnit};

use super::time::{DEFAULT_UNIT, validate_unit};
use super::value::{algorithm_of_width, dtype, fixed_width};

/// The two property names, spelled once for every reader of the vocabulary.
pub(crate) const TIME: &str = "time";
pub(crate) const UNIT: &str = "unit";
pub(crate) const DIGEST_TIME_KEY: &str = "digest:time";
pub(crate) const DIGEST_UNIT_KEY: &str = "digest:unit";

/// Parse a stored unit and return its canonical token.
pub(crate) fn canonicalize_digest_unit(value: &str) -> Result<String> {
    parse_digest_unit(value).map(|unit| unit.as_str().to_owned())
}

fn parse_digest_unit(value: &str) -> Result<TimeUnit> {
    let unit = TimeUnit::from_str(value).map_err(|error| Error::InvalidMetadataValue {
        key: SmolStr::new_static(DIGEST_UNIT_KEY),
        reason: SmolStr::new(error.to_string()),
    })?;
    validate_unit(unit).map_err(|error| Error::InvalidMetadataValue {
        key: SmolStr::new_static(DIGEST_UNIT_KEY),
        reason: SmolStr::new(error.to_string()),
    })?;
    Ok(unit)
}

/// Accept a stored time path: one non-empty field path, never the
/// select-everything spelling.
pub(crate) fn validate_digest_time(value: &str) -> Result<()> {
    if value.is_empty() || value == crate::metadata::ALL_SOURCES {
        return Err(Error::InvalidMetadataValue {
            key: SmolStr::new_static(DIGEST_TIME_KEY),
            reason: format_smolstr!("expected one non-empty field path, got {value:?}"),
        });
    }
    Ok(())
}

/// Return whether a coupled holder's storage carries this algorithm's width.
pub(crate) fn coupled_holder_accepts(field: &Field, algorithm: DigestAlgorithm) -> bool {
    matches!(field.dtype(), DataType::FixedSizeBinary(width) if *width == fixed_width(algorithm))
}

/// Return the datatype spelling a coupled holder needs for an algorithm.
pub(crate) fn expected_coupled_dtype(algorithm: DigestAlgorithm) -> String {
    dtype(algorithm).to_string()
}

/// Return the algorithm a coupled holder's width implies.
pub(crate) fn coupled_holder_algorithm(field: &Field) -> Option<DigestAlgorithm> {
    match field.dtype() {
        DataType::FixedSizeBinary(width) => algorithm_of_width(*width),
        _ => None,
    }
}

impl DigestField<'_> {
    /// Returns the path of the field whose instant this holder couples.
    ///
    /// A holder naming one stores an instant in front of its digest, and its
    /// storage is a `fixed_size_binary` of the coupled width. The path is
    /// relative to the holder's Struct, spelled the way `digest:sources` are.
    pub fn time(&self) -> Option<&str> {
        self.get(TIME)
    }

    /// Parses the clock resolution this holder counts its instant in.
    ///
    /// `None` is the default, [`DEFAULT_UNIT`]; a stored value is one of the
    /// four clock resolutions.
    ///
    /// # Errors
    ///
    /// Returns an error naming `digest:unit` when externally supplied
    /// metadata is not a clock resolution.
    pub fn unit(&self) -> Result<Option<TimeUnit>> {
        self.get(UNIT).map(parse_digest_unit).transpose()
    }

    /// Returns whether this holder couples an instant with its digest.
    pub fn is_coupled(&self) -> bool {
        self.is_holder() && self.time().is_some()
    }

    /// Parses the resolution a coupled holder stores its instant at,
    /// defaulted when none is declared.
    ///
    /// # Errors
    ///
    /// [`Self::unit`] carries the rule.
    pub fn coupled_unit(&self) -> Result<TimeUnit> {
        Ok(self.unit()?.unwrap_or(DEFAULT_UNIT))
    }
}

impl DigestFieldMut<'_> {
    /// Names the field whose instant this holder stores in front of its digest.
    ///
    /// ```
    /// use yggdryl::{DataType, DigestAlgorithm, TimeUnit};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut key = DataType::FixedSizeBinary(16).required_field("key");
    /// key.as_digest_mut().set_holder()?;
    /// key.as_digest_mut().set_time("event")?;
    /// key.as_digest_mut().set_unit(TimeUnit::Second)?;
    ///
    /// assert!(key.as_digest().is_coupled());
    /// assert_eq!(key.as_digest().time(), Some("event"));
    /// assert_eq!(key.as_digest().coupled_unit()?, TimeUnit::Second);
    /// // Sixteen coupled bytes hold a 64-bit digest, never the 128-bit one.
    /// assert!(key.as_digest_mut().set_algorithm(DigestAlgorithm::Xxh128).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a holder, when `path` is
    /// empty or the select-everything spelling, or when the storage is not
    /// the `fixed_size_binary` a coupled digest of its declared or implied
    /// algorithm needs, leaving the field unchanged.
    pub fn set_time(&mut self, path: &str) -> Result<()> {
        if !self.as_protocol().is_holder() {
            return Err(self.rejected(TIME, "requires digest:role=holder".into()));
        }
        validate_digest_time(path)?;
        let declared = self.as_protocol().algorithm()?;
        let implied = coupled_holder_algorithm(self.as_field());
        let accepted = match declared {
            Some(algorithm) => coupled_holder_accepts(self.as_field(), algorithm),
            None => implied.is_some(),
        };
        if !accepted {
            return Err(self.rejected(
                TIME,
                format_smolstr!(
                    "a coupled holder stores {}, got {}",
                    declared.map_or_else(
                        || "fixed_size_binary[12], [16], or [24]".to_owned(),
                        expected_coupled_dtype
                    ),
                    self.as_field().dtype()
                ),
            ));
        }
        self.insert(TIME, path).map(|_| ())
    }

    /// Removes the coupled instant, which makes this a plain holder again.
    ///
    /// # Errors
    ///
    /// Returns an error when `digest:unit` or `digest:algorithm` is still
    /// present, leaving the field unchanged: a unit without an instant
    /// states nothing, and an algorithm declared against the coupled width
    /// may not fit the plain one.
    pub fn remove_time(&mut self) -> Result<Option<String>> {
        if self.contains_key(UNIT) || self.contains_key("algorithm") {
            return Err(self.rejected(
                TIME,
                "cannot remove the coupled instant while digest:unit or digest:algorithm is present"
                    .into(),
            ));
        }
        Ok(self.remove(TIME))
    }

    /// Records the clock resolution this holder counts its instant in.
    ///
    /// # Errors
    ///
    /// Returns an error when this holder couples no instant or `unit` is not
    /// a clock resolution, leaving the field unchanged.
    pub fn set_unit(&mut self, unit: TimeUnit) -> Result<()> {
        if !self.as_protocol().is_coupled() {
            return Err(self.rejected(UNIT, "requires digest:time".into()));
        }
        validate_unit(unit)
            .map_err(|error| self.rejected(UNIT, SmolStr::new(error.to_string())))?;
        self.insert(UNIT, unit.as_str()).map(|_| ())
    }

    /// Removes the explicit resolution, which is the microsecond default again.
    pub fn remove_unit(&mut self) -> Option<String> {
        self.remove(UNIT)
    }
}
