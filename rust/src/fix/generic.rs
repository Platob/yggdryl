//! The message a capture is read as when nothing said which message to read.
//!
//! A parse lands every line in the [fixed row](super::fix_schema) and an
//! [enrichment](super::FixCodec::enrich_messages) fills what each message
//! implies. A consumer then wants those rows under a shape it names - its
//! own message type, a blotter's handful of columns, a venue's dialect - and
//! [`FixCodec::format_messages`](super::FixCodec::format_messages) is the
//! verb that answers them under one. This is the target it uses when a
//! caller names none.
//!
//! # Why it is a registered message and not a second schema
//!
//! A format takes a message field of the registry and fills it. A desk with
//! a message type of its own names that type; a desk with none names this,
//! and the two go through one door. Making the default a registered message
//! rather than a function that builds columns is what keeps the format stage
//! from having a special case, and it is what lets a caller reach it by code
//! like any other message.
//!
//! `pluginconfig` is the precedent (decision 19): a component carrying
//! `fix:msgtype`, under a code opening with `U`, which FIX reserves for
//! user-defined messages so nothing a dictionary publishes collides with it.
//!
//! # What is in it
//!
//! The fixed row's own columns, exactly: the standard header and trailer, the
//! fields a financial consumer reads, the three repeating groups worth
//! persisting whole, the crate's own facts, and the arrival record that
//! closes them. A default target with fewer columns than the row a capture
//! lands in would lose what the row already carried, so it has none fewer.
//!
//! What a *narrower* target loses is the caller's own choice, and a format
//! into a wider one reads the arrival record for what the narrower row
//! dropped - which is the other half of why the record closes every row.
//!
//! The columns depend on the dictionary: `price` is typed as the dictionary
//! types tag 44, and a dictionary that never heard of tag 44 has no such
//! column. So the message is built against one registry rather than declared
//! once, and [`FixRegistry::with_generic_message`] is where a registry takes
//! its own.

use crate::{Field, Result};

use super::FixRegistry;

/// The wire code and canonical name of the crate's own generic message.
///
/// `U` opens every message type FIX reserves for a counterparty's own, so a
/// code invented here collides with nothing a dictionary publishes and needs
/// no dictionary edited to be read - the same reasoning `pluginconfig`
/// followed for `UCFG` (decision 19).
pub const GENERICMESSAGE_CODE_NAME: (&str, &str) = ("UGEN", "genericmessage");

/// The crate's own generic message, built against one dictionary.
///
/// The [fixed row](super::fix_schema)'s own columns, carrying the
/// `fix:msgtype` that makes them a message - so a row under this field is a
/// FIX row and not a projection that has stopped being one: it settles,
/// identifies and replays exactly as the row it was formatted from. `name` is
/// the root's name, which a caller spells for the table it is writing; the
/// registered definition is named [`GENERICMESSAGE_CODE_NAME`]`.1`.
///
/// Built without reading a single message, so two captures that share a
/// dictionary share this field exactly.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// # use yggdryl::holder::local::Folder;
/// # use yggdryl::{FixRegistry, GENERICMESSAGE_CODE_NAME, fix_generic_message, fix_schema};
/// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
/// # let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
/// let row = fix_schema(&registry, "fix")?;
/// let generic = fix_generic_message(&registry, "fix")?;
///
/// // The columns a capture lands in, and the same columns as a message: a
/// // target with fewer would lose what the row already carried.
/// let names = |held: &yggdryl::Field| -> Vec<String> {
///     held.fields().iter().map(|column| column.name().to_owned()).collect()
/// };
/// assert_eq!(names(&row), names(&generic));
/// assert!(generic.index_of("symbol").is_some());
///
/// // What makes it a message is the code on the root, which is how a format
/// // names it.
/// assert_eq!(row.as_fix().msgtype(), None);
/// assert_eq!(generic.as_fix().msgtype(), Some(GENERICMESSAGE_CODE_NAME.0));
///
/// // And it closes on the arrival record, so a formatted row is lossless.
/// assert_eq!(generic.fields().last().map(yggdryl::Field::name), Some("fixentries"));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the schema grammar's refusal when the columns do not make a
/// struct, or when this crate's own fields do not build.
pub fn fix_generic_message(
    registry: &FixRegistry,
    name: impl Into<smol_str::SmolStr>,
) -> Result<Field> {
    let mut field = super::schema::rooted(registry, super::fix_generic_tags(), name)?;
    field.as_fix_mut().set_msgtype(GENERICMESSAGE_CODE_NAME.0)?;
    Ok(field)
}

impl FixRegistry {
    /// Registers the crate's own [`GenericMessage`](fix_generic_message).
    ///
    /// The definition a [format](super::FixCodec::format_messages) reaches by
    /// name when a caller names no message of its own. It is registered on
    /// demand rather than by [`FixRegistry::new`], because unlike
    /// `pluginconfig` its columns are the dictionary's: a registry with no
    /// fields in it would register a message of nothing, and that message
    /// would still be there once a dictionary was loaded over it.
    ///
    /// Idempotent under the guard `with_plugin_fields` uses: a registry that
    /// already answers the code keeps what it has.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::{FixRegistry, GENERICMESSAGE_CODE_NAME};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// let registry = FixRegistry::from_handle(&Folder::new(root)?)?.with_generic_message()?;
    /// let held = registry.msgtype(GENERICMESSAGE_CODE_NAME.0)?;
    /// assert_eq!(held.name(), GENERICMESSAGE_CODE_NAME.1);
    /// assert!(held.as_field().index_of("symbol").is_some());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the message does not build,
    /// or the catalog's when the definition does not register.
    pub fn with_generic_message(mut self) -> Result<Self> {
        if self.get_msgtype(GENERICMESSAGE_CODE_NAME.0).is_none() {
            let message = fix_generic_message(&self, GENERICMESSAGE_CODE_NAME.1)?;
            self.create_definition(crate::FixCategory::Components, message)?;
        }
        Ok(self)
    }
}
