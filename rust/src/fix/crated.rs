//! The fields this crate invents, on a branch of its own.
//!
//! A capture states things about a message that no dictionary publishes: what
//! its bytes hash to, which way its line moved, which version it was read at,
//! and the two derived facts a store is organised by - one instrument symbol
//! that is the same across venues, and one timestamp a partition is cut on.
//! Each belongs in a column, so each is an ordinary field: they lift, column,
//! serialize and resolve like every other field with no special case anywhere.
//!
//! # Why a branch
//!
//! Their tags are in FIX's user-defined range - high in it, from 30001 up,
//! rather than down in the 5000s where venues actually crowd. That is not
//! enough on its own: a venue is free to define its own 30001. They are
//! therefore carried on this crate's own branch, so the packed
//! [identifier](super::FixId) differs even where the tag does not - same tag,
//! different branch, different identity.
//!
//! # And why one of them is not here
//!
//! `MsgDirection` is not invented: FIX publishes it at tag 385, and a field
//! the specification already has is never given a second tag. What this crate
//! adds there is the *type* - the packed four bytes rather than a string -
//! which the dictionary carries like any other coded field.
//!
//! One mechanism, five fields. A sixth would use it too.

use std::sync::LazyLock;

use crate::{DataType, Field, Result, TimeUnit, Timezone};

use super::FixBranch;

/// The branch this crate's own fields are defined on.
pub const CRATE_BRANCH: &str = "yggdryl";

/// The tag carrying a message's value digest.
pub const MSGHASH_TAG: i32 = 30001;

/// The tag carrying the FIX version a message was read at.
pub const VERSION_TAG: i32 = 30002;

/// The tag carrying one instrument symbol that is the same across venues.
pub const SYMBOLTICKER_TAG: i32 = 30003;

/// The tag carrying the timestamp a capture is ordered by.
pub const TIMESTAMP_TAG: i32 = 30004;

/// The tag carrying the partition that timestamp falls in.
pub const UNIXPARTITION_TAG: i32 = 30005;

/// FIX's own tag for which way a message moved.
///
/// Published, not invented: the specification has spelled this `MsgDirection`
/// since 4.4, and a field it already declares is never given a second tag.
pub const MSGDIRECTION_TAG: i32 = 385;

/// The digest's width in bytes, which is the algorithm's.
const DIGEST_WIDTH: i32 = 16;

/// How wide a partition is by default, in seconds.
///
/// An hour. A day is too coarse to prune a capture with - a session's whole
/// traffic lands in one partition - and a minute makes a day of capture
/// fourteen hundred of them, which is more files than rows in the quiet ones.
pub const DEFAULT_PARTITION_SECONDS: i64 = 3_600;

/// This crate's branch, built once.
fn branch() -> Result<FixBranch> {
    FixBranch::from_str(CRATE_BRANCH)
}

/// The fields, built once and shared.
static FIELDS: LazyLock<Option<Vec<Field>>> = LazyLock::new(|| build().ok());

/// One field on the crate's branch.
fn crated(name: &str, tag: i32, dtype: DataType) -> Result<Field> {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_id(&branch()?, tag)?;
    Ok(field)
}

/// Builds every field this crate defines.
fn build() -> Result<Vec<Field>> {
    Ok(vec![
        // `FixedSizeBinary`, big-endian, because a digest is not a string and
        // must not become one. Big-endian is the one layout where byte order
        // and numeric order agree on every machine: a little-endian digest
        // sorts differently than it compares, and someone eventually sorts it.
        crated(
            "msghash",
            MSGHASH_TAG,
            DataType::fixed_size_binary(DIGEST_WIDTH)?,
        )?,
        // The version the message was *read* at, which is not always the one
        // its `BeginString` claims: a venue that mislabels its session still
        // produces rows, and the column says which dictionary answered them.
        crated("version", VERSION_TAG, DataType::Utf8)?,
        // One symbol for one instrument, whatever the venue called it.
        crated("symbolticker", SYMBOLTICKER_TAG, DataType::Utf8)?,
        // The timestamp a capture is ordered by, in UTC because a capture
        // spans venues and a local time cannot be compared across them.
        crated(
            "timestamp",
            TIMESTAMP_TAG,
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::UTC,
            },
        )?,
        // The partition that timestamp falls in, as whole seconds since the
        // epoch. An integer rather than a rendered date: a partition value is
        // compared and ranged over, and a string would sort lexically.
        crated("unixpartition", UNIXPARTITION_TAG, DataType::Int64)?,
    ])
}

/// The fields this crate defines, in tag order.
///
/// Registering them is a caller's choice rather than a load-time side effect:
/// a dictionary read from a store is what that store held, and a reader that
/// silently gained five fields would write them back out again.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// let held = yggdryl::fix_crate_fields()?;
/// assert_eq!(held[0].name(), "msghash");
/// // Same tag as a venue's own 30001 would be, and a different identity.
/// let mine = held[0].as_fix().id()?.expect("an identity");
/// assert_ne!(mine, yggdryl::FixId::standard(30001));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the schema grammar's refusal when the crate's own branch or one of
/// the datatypes does not build, which is a defect in this module rather than
/// anything a caller did.
pub fn fix_crate_fields() -> Result<&'static [Field]> {
    FIELDS
        .as_deref()
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: CRATE_BRANCH.into(),
            reason: crate::text::expected_got("the crate's own fields", "a build failure"),
        })
}

impl super::FixRegistry {
    /// Adds this crate's own fields, so they resolve by tag and by name.
    ///
    /// A dictionary that has them can type a `msghash` or `timestamp` column
    /// from the registry like any other. One that does not is unchanged -
    /// nothing in reading a message needs them, because all of them are facts
    /// about the capture rather than about the wire.
    ///
    /// # Errors
    ///
    /// Returns the registry's own refusal when a field collides with
    /// something already held, which cannot happen on a dictionary that does
    /// not already declare this crate's branch.
    pub fn with_crate_fields(mut self) -> Result<Self> {
        for field in fix_crate_fields()? {
            self.insert(field.clone())?;
        }
        Ok(self)
    }
}

/// FIX's own tag for a message's type.
pub const MSGTYPE_TAG: i32 = 35;

impl super::FixRegistry {
    /// Registers one message type, answering the value it takes.
    ///
    /// A dictionary is never complete. Venues invent message types, bridges
    /// write composite keys like `P Report Ack`, and a reader that refused
    /// what it had not been told about would drop exactly the traffic someone
    /// is trying to understand. So a type the code set does not have is added
    /// to it rather than rejected, and the value it takes is
    /// [`MsgType::coerce`](crate::types::MsgType::coerce)'s - itself where it
    /// fits, a stable synthesized value where it does not.
    ///
    /// Nothing is hard-coded: the vocabulary is the dictionary's own code set
    /// on tag 35, and this adds to it exactly as a generator would.
    ///
    /// Idempotent. Registering a type the dictionary already spells answers
    /// its existing value and changes nothing, so a reader may call it per
    /// row without growing the code set per row.
    ///
    /// # Errors
    ///
    /// Returns the registry's own refusal when tag 35 is absent, when its
    /// code set will not read, or when the synthesized value collides with a
    /// different spelling - which is a real conflict and not something to
    /// resolve by picking one.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::FixRegistry;
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// let mut registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    ///
    /// // One the specification publishes is already there.
    /// assert_eq!(registry.register_msgtype("NewOrderSingle")?.as_str(), "D");
    ///
    /// // One a bridge invents is added, and stays put.
    /// let held = registry.register_msgtype("P Report Ack")?;
    /// assert!(held.is_synthetic());
    /// assert_eq!(registry.register_msgtype("P Report Ack")?, held);
    /// # Ok(())
    /// # }
    /// ```
    pub fn register_msgtype(&mut self, spelling: &str) -> Result<crate::types::MsgType> {
        let field = self.field_by_tag(MSGTYPE_TAG)?;
        let view = field.as_fix();
        // Already spelled, by name or by value: answer what it already is.
        if let Some(held) = view.code_value(spelling) {
            return crate::types::MsgType::new(held);
        }
        let value = crate::types::MsgType::coerce(spelling);
        if let Some(taken) = view.code_name(value.as_str()) {
            return Err(crate::Error::Conflict {
                expected: "a free message type value",
                actual: "one another spelling holds",
                path: crate::text::expected_got(
                    format_args!("{spelling:?} at {:?}", value.as_str()),
                    format_args!("{taken:?}"),
                ),
            });
        }
        let mut codes: Vec<super::FixCode> = view
            .codes()
            .filter_map(std::result::Result::ok)
            .map(super::FixCode::from)
            .collect();
        codes.push(super::FixCode::new(spelling, value.as_str()));
        let mut field = field.clone();
        field.as_fix_mut().set_codes(&codes)?;
        self.update(field)?;
        Ok(value)
    }
}
