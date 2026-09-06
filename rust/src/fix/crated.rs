//! The two fields this crate invents, on a branch of its own.
//!
//! A message digest and a direction are facts about a captured line that no
//! dictionary publishes, and both belong in a column. They are ordinary
//! fields rather than properties beside a message, so they lift, column,
//! serialize and resolve like every other field with no special case
//! anywhere.
//!
//! # Why a branch
//!
//! Their tags are in FIX's user-defined range - high in it, at 30001 and
//! 30002, rather than down in the 5000s where venues actually crowd. That is
//! not enough on its own: a venue is free to define its own 30001. They are
//! therefore carried on this crate's own branch, so the packed
//! [identifier](super::FixId) differs even where the tag does not - same tag,
//! different branch, different identity.
//!
//! Two fields, one mechanism. A third would use it too.

use std::sync::LazyLock;

use crate::{DataType, Field, Result};

use super::FixBranch;

/// The branch this crate's own fields are defined on.
pub const CRATE_BRANCH: &str = "yggdryl";

/// The tag carrying a message's value digest.
pub const MSGHASH_TAG: i32 = 30001;

/// The tag carrying which way a captured line moved.
pub const DIRECTION_TAG: i32 = 30002;

/// The digest's width in bytes, which is the algorithm's.
const DIGEST_WIDTH: i32 = 16;

/// This crate's branch, built once.
fn branch() -> Result<FixBranch> {
    FixBranch::from_str(CRATE_BRANCH)
}

/// The two fields, built once and shared.
static FIELDS: LazyLock<Option<[Field; 2]>> = LazyLock::new(|| build().ok());

/// Builds both fields on the crate's branch.
fn build() -> Result<[Field; 2]> {
    let branch = branch()?;

    // `FixedSizeBinary`, big-endian, because a digest is not a string and
    // must not become one. Big-endian is the one layout where byte order and
    // numeric order agree on every machine: a little-endian digest sorts
    // differently than it compares, and someone eventually sorts it.
    let mut msghash = DataType::fixed_size_binary(DIGEST_WIDTH)?.nullable_field("msghash");
    msghash.as_fix_mut().set_id(&branch, MSGHASH_TAG)?;

    // The packed four-byte coded datatype, the same discipline every coded
    // column in the crate keeps: the value, not a name for it.
    let mut direction = DataType::Direction.nullable_field("direction");
    direction.as_fix_mut().set_id(&branch, DIRECTION_TAG)?;

    Ok([msghash, direction])
}

/// The fields this crate defines, in tag order.
///
/// Registering them is a caller's choice rather than a load-time side effect:
/// a dictionary read from a store is what that store held, and a reader that
/// silently gained two fields would write them back out again.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// let held = yggdryl::fix_crate_fields()?;
/// assert_eq!(held[0].name(), "msghash");
/// assert_eq!(held[1].name(), "direction");
/// // Same tag as a venue's own 30001 would be, and a different identity.
/// let mine = held[0].as_fix().id()?.expect("an identity");
/// assert_ne!(mine, yggdryl::FixId::standard(30001));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the schema grammar's refusal when the crate's own branch or
/// either datatype does not build, which is a defect in this module rather
/// than anything a caller did.
pub fn fix_crate_fields() -> Result<&'static [Field; 2]> {
    FIELDS.as_ref().ok_or_else(|| crate::Error::InvalidRecord {
        path: CRATE_BRANCH.into(),
        reason: crate::text::expected_got("the crate's own fields", "a build failure"),
    })
}

impl super::FixRegistry {
    /// Adds this crate's own fields, so they resolve by tag and by name.
    ///
    /// A dictionary that has them can type a `msghash` or `direction` column
    /// from the registry like any other. One that does not is unchanged -
    /// nothing in reading a message needs them, because both are facts about
    /// the capture rather than about the wire.
    ///
    /// # Errors
    ///
    /// Returns the registry's own refusal when either field collides with
    /// something already held, which cannot happen on a dictionary that does
    /// not already declare this crate's branch.
    pub fn with_crate_fields(mut self) -> Result<Self> {
        for field in fix_crate_fields()? {
            self.insert(field.clone())?;
        }
        Ok(self)
    }
}
