//! One way to name a time zone, everywhere in the project - and the datatype
//! a column of them declares.
//!
//! Registered names, aliases, fixed offsets, and the explicit zone-free marker
//! all resolve through this one value.
//!
//! [`Timezone::NAIVE`] gives every native temporal value and datatype a
//! non-optional zone while Arrow projects that marker as an absent timezone. A
//! zone is a process-lifetime interned handle; [`Timezone::offset_at`] applies
//! the registry rules bundled by this build.
//!
//! A zone is also a value in its own right. [`crate::DataType::Timezone`] is
//! the column that holds one: canonical text in Arrow's `Utf8`, kept a zone
//! across a round trip by its extension name, and read back through the same
//! `from_str` every other spelling crosses. It is neither a bounded ASCII code
//! nor a static vocabulary - an IANA name is as long as the registry says and
//! a fixed offset is generated, not enumerated - so it is its own datatype,
//! the way a URL and a version are.
//!
//! ```
//! use yggdryl::{DataType, Scalar, Timezone};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Aliases and case both canonicalize, so two spellings compare equal.
//! assert_eq!(Timezone::from_str("Asia/Calcutta")?, Timezone::from_str("Asia/Kolkata")?);
//! assert_eq!(Timezone::from_str("Z")?, Timezone::UTC);
//!
//! // A registered zone knows its own rules.
//! let new_york = Timezone::from_str("America/New_York")?;
//! assert_eq!(new_york.offset_at(1_700_000_000), Some(-5 * 3600));  // November: EST
//! assert_eq!(new_york.offset_at(1_688_000_000), Some(-4 * 3600));  // June: EDT
//! assert_eq!(new_york.abbreviation_at(1_688_000_000), Some("EDT"));
//!
//! // A fixed offset needs no registry at all.
//! assert_eq!(Timezone::from_str("+05:30")?.offset_at(0), Some(5 * 3600 + 1800));
//!
//! // And a column of zones declares the datatype, which canonicalizes on the
//! // way in exactly as the value does.
//! assert_eq!(DataType::from_str("timezone")?, DataType::Timezone);
//! assert_eq!(
//!     DataType::Timezone.scalar("Asia/Calcutta")?,
//!     Scalar::Timezone(Timezone::from_str("Asia/Kolkata")?),
//! );
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod fields;
mod registry;
mod value;

pub use fields::{TimezoneField, TimezoneType};
pub use value::Timezone;
pub(crate) use value::{civil_from_days, days_from_civil};

/// The Arrow extension name preserving [`crate::DataType::Timezone`] over its
/// Utf8 storage.
pub(crate) const TIMEZONE_EXTENSION_NAME: &str = "yggdryl.timezone";

#[cfg(test)]
mod tests;
