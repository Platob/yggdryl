//! One source a dictionary's definitions were read from: what a
//! `FIX:sources` id names.
//!
//! A field states the ids of the sources that contributed it
//! ([`FixField::sources`](crate::FixField::sources)), and the registry holds
//! one [`FixSource`] per id - the catalog where what is known of a source,
//! the file it was read from, is recorded once rather than on every field.
//! A store writes the catalog as `sources.json` beside its category folders.
//! The id grammar is this module's: [`source_id`] is the one reader every
//! door an id crosses - a field's setter, the catalog's constructor, a
//! `.cfb`'s dialect - reads through.

use smol_str::{SmolStr, format_smolstr};

use super::document::is_word;
use super::field::SOURCES_KEY;
use crate::{Error, PluginSide, Result, Scalar};

/// What a source id is, spelled once for every refusal.
const ID_SHAPE: &str = "a non-empty source id without a quote, a backslash or a control character";
/// The keys one catalog entry states, in the order a store writes them.
const ID: &str = "id";
const FILE: &str = "file";
const PLUGINSIDE: &str = "pluginside";

/// The id `text` spells, folded to ASCII lowercase.
///
/// An id is a word - non-empty, holding no byte the stored array would have
/// to escape - and it is lowercase, so registries built from the same
/// sources in any order hash alike and `AUTEX_FIX42` and `autex_fix42` name
/// one entry.
///
/// # Errors
///
/// Returns [`Error::InvalidMetadataValue`] naming the `FIX:sources` key
/// when `text` is no id, which is the refusal a field's setter raises.
pub(super) fn source_id(text: &str) -> Result<SmolStr> {
    if !is_word(text) {
        return Err(Error::InvalidMetadataValue {
            key: SmolStr::new_static(SOURCES_KEY),
            reason: format_smolstr!("expected {ID_SHAPE}, got {text:?}"),
        });
    }
    Ok(SmolStr::new(text.to_ascii_lowercase()))
}

/// One source of a dictionary's definitions: an entry of the registry's
/// catalog, named by the id a field's `FIX:sources` states.
///
/// Ordered and hashed by id first, as the catalog is.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FixSource {
    /// The id, folded: what a field states.
    pub(super) id: SmolStr,
    /// The file the source was read from, as it was named - `venue.cfb` -
    /// where one is known.
    pub(super) file: Option<SmolStr>,
    /// The role of the source's plugin - a CBlock's root `type` names
    /// it - `UKNW` where the source states none.
    pub(super) pluginside: PluginSide,
}

impl FixSource {
    /// An entry for `id`, stating no file and no plugin role.
    ///
    /// # Errors
    ///
    /// Returns the id grammar's refusal, naming the `FIX:sources` key, when
    /// `id` is empty or holds a quote, a backslash or a control character.
    pub fn new(id: &str) -> Result<Self> {
        Ok(Self {
            id: source_id(id)?,
            file: None,
            pluginside: PluginSide::Unknown,
        })
    }

    /// This entry stating the file the source was read from.
    #[must_use]
    pub fn with_file(mut self, file: impl Into<SmolStr>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// This entry stating the role of its plugin.
    #[must_use]
    pub const fn with_pluginside(mut self, pluginside: PluginSide) -> Self {
        self.pluginside = pluginside;
        self
    }

    /// The id, folded to ASCII lowercase.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The file the source was read from, where one is known.
    #[must_use]
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    /// The role of the source's plugin: `BUYS`, `SELL`, or `UKNW` where
    /// the source states none.
    #[must_use]
    pub const fn pluginside(&self) -> PluginSide {
        self.pluginside
    }

    /// The entry as a store writes it - `{"id": "venue", "file":
    /// "venue.cfb", "pluginside": "SELL"}` - the file left out where none is
    /// known, the plugin side always stated.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        Scalar::from_struct(
            std::iter::once((ID, Scalar::from(self.id.as_str())))
                .chain(self.file.as_deref().map(|file| (FILE, Scalar::from(file))))
                .chain(std::iter::once((
                    PLUGINSIDE,
                    Scalar::from(self.pluginside.as_str()),
                ))),
        )
        .expect("id, file and pluginside are distinct names")
    }

    /// The entry one stored value states.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the value is not an object
    /// stating a text `id`, at most a text `file` and at most a
    /// `pluginside` naming a member (absent reads `UKNW`), and the id
    /// grammar's refusal when the id is no id.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        let refused = |reason: SmolStr| Error::InvalidRecord {
            path: SmolStr::new_static("fix source"),
            reason,
        };
        let record = value.as_struct().ok_or_else(|| {
            refused(crate::text::expected_got(
                "a JSON source entry object",
                value.kind(),
            ))
        })?;
        if let Some(key) = record
            .keys()
            .find(|key| key.as_str() != ID && key.as_str() != FILE && key.as_str() != PLUGINSIDE)
        {
            return Err(refused(format_smolstr!(
                "expected the keys {ID:?}, {FILE:?} and {PLUGINSIDE:?}, got {key:?}"
            )));
        }
        let id = record.get(ID).and_then(Scalar::as_str).ok_or_else(|| {
            refused(SmolStr::new_static(
                "expected a source entry stating its id as text",
            ))
        })?;
        let file = match record.get(FILE) {
            None => None,
            Some(file) => Some(file.as_str().ok_or_else(|| {
                refused(format_smolstr!(
                    "expected the file of {id:?} as text, got {}",
                    file.kind()
                ))
            })?),
        };
        let pluginside = match record.get(PLUGINSIDE) {
            None => PluginSide::Unknown,
            Some(stated) => <PluginSide as crate::EnumValue>::from_scalar_value(stated)
                .ok_or_else(|| {
                    refused(format_smolstr!(
                        "expected the pluginside of {id:?} as a member - BUYS, SELL or UKNW - got {}",
                        // The value where the door reads its shape - a text, an
                        // integer code - and the kind where it never does.
                        stated.as_str().map_or_else(
                            || {
                                stated.as_i128().map_or_else(
                                    || stated.kind().to_string(),
                                    |code| code.to_string(),
                                )
                            },
                            |text| format!("{text:?}"),
                        )
                    ))
                })?,
        };
        let mut source = Self::new(id)?.with_pluginside(pluginside);
        if let Some(file) = file {
            source = source.with_file(file);
        }
        Ok(source)
    }
}
