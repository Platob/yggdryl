//! Reading a FIX message as an instrument classification.
//!
//! What a CFI code *is* - its six positions, which letters each accepts, how
//! two statements merge, when it is detailed - belongs to the value and lives
//! beside it in [`crate::Cfi`]. This module is the other half: which FIX
//! fields say something about a classification, and what each of them says.
//!
//! That split is the point. `Cfi::merged` is the same fold whether the two
//! codes came off a FIX wire, an ISIN registry or two columns of a table, so
//! a FIX-shaped copy of it would be a second answer to one question. What is
//! genuinely FIX's is the chain below: that `CFICode(461)` is the
//! classification of record, that a bridge states the detailed code beside a
//! coarse one under its own `DETAILEDCFICODE`, and that `PutOrCall(201)` is
//! the *group* of a listed option rather than an attribute.
//!
//! A market keeps only a detailed classification. `SecurityType(167)`,
//! `Product(460)` and `SecurityIDSource(22)` reach a category or a group and
//! no further, and a code that says nothing past its group - `ESXXXX` - is a
//! coarse fact the market answers none for, so those steps are gone.

use smol_str::SmolStr;

use crate::Cfi;

/// The tag FIX publishes the ISO 10962 classification under, since FIX 4.3.
///
/// This crate adds no column of its own for it: 461 is already where a
/// message states its classification, and a second column would be a second
/// owner of one fact.
pub(super) const CFICODE_TAG: i32 = 461;

/// The names a bridge states the detailed classification under, beside a
/// coarse `CFICode(461)`: the bare spelling and its `#`-marked twin.
pub(super) const DETAILED_NAMES: [&str; 2] = ["detailedcficode", "#detailedcficode"];

/// What a `PutOrCall(201)` says, as a listed-option **group**.
///
/// Category `O` divides into `OC` calls, `OP` puts and `OM` others, so
/// call/put is position 2 rather than an attribute - position 3 for `O` is
/// the exercise style (`A`merican, `B`ermudan, `E`uropean). This is the one
/// scalar tag that pins a group outright, and it pins it only for a listed
/// option: category `H` splits on something else entirely, so a non-listed
/// option takes nothing from 201.
fn option_group(put_or_call: i64) -> Option<char> {
    Some(match put_or_call {
        0 => 'P',
        1 => 'C',
        2 => 'M',
        _ => return None,
    })
}

impl super::FixMsg {
    /// The instrument's detailed classification, or none.
    ///
    /// The chain, each step merged into the last through
    /// [`Cfi::merged`](crate::Cfi::merged) so a later step can only
    /// *fill* what an earlier one left unknown:
    ///
    /// 1. a stated `CFICode(461)`, which is the instrument's classification
    ///    of record and outranks anything derived;
    /// 2. an unmapped `DETAILEDCFICODE` - the bare name or its `#`-marked
    ///    twin - which a bridge states beside a coarse 461 and which fills
    ///    the positions the stated code left unknown;
    /// 3. `PutOrCall(201)`, which is the *group* of a listed option - `OC`,
    ///    `OP`, `OM` - rather than an attribute, and so refines a stated
    ///    Others group `OM` and nothing else.
    ///
    /// The answer is kept only where it is [detailed](crate::Cfi::is_detailed):
    /// a code that says nothing past its category and group - `ESXXXX` - is
    /// a coarse fact the market answers none for, and a `SecurityType(167)`
    /// or `Product(460)` alone, which never reach further than that, answer
    /// none too.
    ///
    /// A step that names a different instrument than the one established -
    /// a different category or group - does not overwrite it and does not
    /// merge: [`Cfi::merged`](crate::Cfi::merged) answers `None` and
    /// the step is dropped, because the stated classification is the one of
    /// record.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::local::LocalFolder;
    /// # use yggdryl::{FixCodec, FixRegistry};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    /// let reader = FixCodec::new(Arc::clone(&registry));
    ///
    /// // A stated detailed code is the classification of record.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|461=ESVUFR|10=0|")?;
    /// assert_eq!(held.classification().as_deref(), Some("ESVUFR"));
    ///
    /// // A coarse code answers none: it says nothing past its group.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|461=ESXXXX|10=0|")?;
    /// assert_eq!(held.classification(), None);
    ///
    /// // The detailed code a bridge states beside it fills the gaps.
    /// let held = reader.parse_fix_line(
    ///     b"8=FIX.4.4|35=D|461=ESXXXX|DETAILEDCFICODE=ESVTFR|10=0|",
    /// )?;
    /// assert_eq!(held.classification().as_deref(), Some("ESVTFR"));
    ///
    /// // A stated Others group and a put is `OP`: call/put is the group here.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|461=OMAXXX|201=0|10=0|")?;
    /// assert_eq!(held.classification().as_deref(), Some("OPAXXX"));
    ///
    /// // A stated group is never overwritten by a disagreeing 201.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|461=OCAXXX|201=0|10=0|")?;
    /// assert_eq!(held.classification().as_deref(), Some("OCAXXX"));
    ///
    /// // SecurityType, Product and a symbol alone reach no detailed code.
    /// for line in [
    ///     &b"8=FIX.4.4|35=D|167=CS|10=0|"[..],
    ///     b"8=FIX.4.4|35=D|461=ESXXXX|167=CS|10=0|",
    ///     b"8=FIX.4.4|35=D|167=OPT|201=0|10=0|",
    ///     b"8=FIX.4.4|35=D|460=5|10=0|",
    ///     b"8=FIX.4.4|35=D|55=AAPL|10=0|",
    /// ] {
    ///     assert_eq!(reader.parse_fix_line(line)?.classification(), None);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn classification(&self) -> Option<SmolStr> {
        let clean = |held: Option<crate::Scalar>| {
            held.filter(|held| !held.is_null())
                .and_then(|held| held.as_str().map(str::trim).map(str::to_ascii_uppercase))
                .filter(|held| !held.is_empty())
        };
        let text = |tag: i32| clean(self.get_by_tag(tag));
        // A detailed code a bridge states is a field no dictionary maps, so
        // it is found among the row's own children by its spelling alone -
        // exactly, else by the one child the fold reaches - and never asks
        // the dictionary for a name it does not hold.
        let named = |name: &str| {
            let at = self.index_of_name(name)?;
            clean(self.as_value().as_sequence()?.get(at).cloned())
        };
        let number = |tag: i32| {
            self.get_by_tag(tag)
                .filter(|held| !held.is_null())
                .and_then(|held| held.as_i128())
                .and_then(|held| i64::try_from(held).ok())
        };
        // A step only ever fills what is still unknown: `merged` keeps a
        // stated attribute over a derived one because the stated code leads,
        // and answers `X` where two steps disagree inside one instrument.
        let mut held: Option<SmolStr> = None;
        let mut fold = |candidate: Option<String>| {
            let Some(candidate) = candidate.filter(|held| Cfi::is_classified(held)) else {
                return;
            };
            held = match &held {
                None => Some(SmolStr::new(candidate)),
                // A step describing a different instrument is dropped rather
                // than merged: the classification of record stands.
                Some(current) => {
                    Some(Cfi::merged(current, &candidate).unwrap_or_else(|| current.clone()))
                }
            };
        };
        // `PutOrCall` refines a listed option stated as its Others group,
        // before the code is read as one: `OM` and "a put" means `OP` rather
        // than "an option of unspecified kind", and `OM` licenses no
        // attribute of its own, so the stated attributes only read under
        // the group 201 names. A group actually stated is never overwritten.
        let listed_option_group = number(201).and_then(option_group);
        let refined = |code: String| match listed_option_group {
            Some(group) if group != 'M' && code.starts_with("OM") => {
                let mut spelled: Vec<char> = code.chars().collect();
                spelled[1] = group;
                spelled.into_iter().collect()
            }
            _ => code,
        };
        fold(text(CFICODE_TAG).map(refined));
        fold(
            DETAILED_NAMES
                .iter()
                .find_map(|name| named(name))
                .map(refined),
        );
        held.filter(|held| Cfi::is_detailed(held))
    }
}
