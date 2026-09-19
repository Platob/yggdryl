//! The spellings every desk uses for the same field.
//!
//! A venue writes `OfferPx`, a blotter writes `AskPrice`, a risk system
//! writes `AskPx`, and FIX publishes exactly one of them. The registry
//! already resolves a name through one fold and then through aliases
//! ([`FixFieldMut::set_names`](super::FixFieldMut::set_names)), so the
//! other spellings are alias rows rather than a second resolver - and this is
//! where the rows come from.
//!
//! # They are generated, not listed
//!
//! Four substitutions carry all of them, and each is a word this industry
//! uses two ways for one thing:
//!
//! | Canonical | Also written |
//! | --- | --- |
//! | `bid` | `demand` |
//! | `offer` | `ask` |
//! | `px` | `price` |
//! | `size` | `qty` |
//!
//! Every combination applies to every field name, so `offerpx` lends
//! `askpx`, `offerprice` and `askprice`, and `bidsize` lends `bidqty`,
//! `demandsize` and `demandqty`, without any of those twelve spellings being
//! written down. A field whose name contains none of the four lends nothing,
//! which is almost every field in the dictionary.
//!
//! The substitution is one-directional by construction - canonical to
//! alternate - because the alternate is what arrives and the canonical is
//! what the registry answers. `askpx` reaching `offerpx` is the whole
//! contract; `offerpx` was never going to reach `askpx`, because nothing
//! stores a field under `askpx`.
//!
//! # When both spellings arrive
//!
//! They resolve to one field, and the crate already has exactly one answer
//! for two voices on one field: conflicting voices fill nothing. A message
//! carrying `OfferPx=10` and `AskPx=11` states one field twice and
//! disagrees, so the build leaves it unfilled and the arrival record keeps
//! both pairs - the same rule a bridge's two spellings of one tag already
//! met. Agreeing voices are not a conflict and fill once.
//!
//! A spelling a dictionary already defines as a field of its own is never
//! taken from it: [`FixRegistry`] refuses to lend an alias another field
//! answers for, and notes it rather than failing, so a venue that really has
//! an `AskPx` tag keeps it and only the fields nobody claimed lend theirs.

use smol_str::SmolStr;

use super::FixRegistry;
use crate::{FixCategory, Result};

/// The word pairs, canonical first.
///
/// Ordered longest-canonical first so a name carrying two of them substitutes
/// each once rather than re-entering a replacement.
const SPELLINGS: [(&str, &str); 4] = [
    ("offer", "ask"),
    ("size", "qty"),
    ("bid", "demand"),
    ("px", "price"),
];

/// Every alternate spelling of one canonical name, the name itself excluded.
///
/// The power set of the applicable substitutions: a name carrying two of them
/// lends the three names that change one or both.
fn spellings_of(name: &str) -> Vec<SmolStr> {
    let applicable: Vec<&(&str, &str)> = SPELLINGS
        .iter()
        .filter(|(canonical, _)| name.contains(canonical))
        .collect();
    if applicable.is_empty() {
        return Vec::new();
    }
    // A bit per applicable substitution; zero is the name itself, which is
    // not an alias of itself.
    (1..(1_u32 << applicable.len()))
        .map(|mask| {
            let mut held = name.to_owned();
            for (at, (canonical, alternate)) in applicable.iter().enumerate() {
                if mask & (1 << at) != 0 {
                    held = held.replace(canonical, alternate);
                }
            }
            SmolStr::new(held)
        })
        .collect()
}

impl FixRegistry {
    /// Lends every field the other spellings of its own name.
    ///
    /// Registered on demand rather than by [`FixRegistry::new`], because the
    /// spellings are the *dictionary's* names: a registry with no fields in
    /// it would lend nothing and still count as done once a dictionary was
    /// loaded over it.
    ///
    /// Idempotent: a field already carrying a spelling keeps the one it has,
    /// and a spelling another field answers for stays with that field.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use yggdryl::local::Folder;
    /// # use yggdryl::FixRegistry;
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// let registry = FixRegistry::from_handle(&Folder::new(root)?)?.with_default_aliases()?;
    ///
    /// // One field, reached by every spelling a desk writes.
    /// for spelled in ["offerpx", "askpx", "offerprice", "askprice"] {
    ///     assert_eq!(registry.field_by_name(spelled)?.name(), "offerpx", "{spelled}");
    /// }
    /// for spelled in ["bidsize", "bidqty", "demandsize", "demandqty"] {
    ///     assert_eq!(registry.field_by_name(spelled)?.name(), "bidsize", "{spelled}");
    /// }
    /// // A name carrying none of the four lends nothing and is still itself.
    /// assert_eq!(registry.field_by_name("symbol")?.name(), "symbol");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the catalog's refusal when a re-registered definition does not
    /// land.
    pub fn with_default_aliases(mut self) -> Result<Self> {
        let mut lending: Vec<(SmolStr, Vec<SmolStr>)> = Vec::new();
        for field in self.definitions(FixCategory::Fields) {
            let spellings = spellings_of(field.name());
            if !spellings.is_empty() {
                lending.push((SmolStr::new(field.name()), spellings));
            }
        }
        for (name, spellings) in lending {
            let Some(held) = self.get_field_by_name(&name) else {
                continue;
            };
            let mut field = held.clone();
            let mut aliases: Vec<SmolStr> = field.as_fix().names().map(SmolStr::new).collect();
            for spelled in spellings {
                // A spelling another field claims canonically is that
                // field's; the registry says so too, and saying it here keeps
                // the definition itself honest about what it lends.
                if self.get_field_by_name(&spelled).is_some() {
                    continue;
                }
                if !aliases.iter().any(|held| held == &spelled) {
                    aliases.push(spelled);
                }
            }
            field
                .as_fix_mut()
                .set_names(aliases.iter().map(SmolStr::as_str))?;
            self.insert_definition(FixCategory::Fields, field)?;
        }
        Ok(self)
    }
}
