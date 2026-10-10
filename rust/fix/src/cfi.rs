//! Reading a FIX message as an instrument classification.
//!
//! What a CFI code *is* - its six positions, which letters each accepts, how
//! one statement refines another, when it is detailed - belongs to the value
//! and lives beside it in [`yggdryl::Cfi`]. This module is the other half:
//! which FIX fields say something about a classification, and what each of
//! them says.
//!
//! That split is the point. [`Cfi::refined`] is the same fold whether the
//! two codes came off a FIX wire, the instruments or two columns of a
//! table, so a FIX-shaped copy of it would be a second answer to one
//! question. What is genuinely FIX's is the chain below: that
//! `CFICode(461)` is the classification of record - a bridge's
//! `DETAILEDCFICODE` being one of its names, folded into it where the
//! message is built ([`merged_statement`]) - and that `PutOrCall(201)` is
//! the *group* of a listed option rather than an attribute.
//!
//! A market keeps only a detailed classification. `SecurityType(167)`,
//! `Product(460)` and `SecurityIDSource(22)` reach a category or a group and
//! no further, and a code that says nothing past its group - `ESXXXX` - is a
//! coarse fact the market answers none for, so those steps are gone.

use smol_str::SmolStr;

use yggdryl::{Cfi, Scalar};

/// The tag FIX publishes the ISO 10962 classification under, since FIX 4.3.
///
/// This crate adds no column of its own for it: 461 is already where a
/// message states its classification, and a second column would be a second
/// owner of one fact. A bridge's `DETAILEDCFICODE` is one of its names.
pub(super) const CFICODE_TAG: i32 = 461;

/// The one `CFICode(461)` two statements of it in one message make: `lead`,
/// which is the canonical name or the tag over a name, the earlier name over
/// a later one and the first arrival over a repeat, with each `X` it states
/// filled from `other` where the two describe one instrument
/// ([`Cfi::refined`]), each read trimmed and upper-cased. A lead stating
/// nothing (`XXXXXX`) yields to a classified `other` whole. `None` where the
/// two contradict each other (another group, or two different letters at
/// one position) or either is no code, and the two stay two statements.
pub(super) fn merged_statement(lead: &Scalar, other: &Scalar) -> Option<Scalar> {
    let text = |held: &Scalar| held.as_str().map(|held| held.trim().to_ascii_uppercase());
    let (lead_text, other_text) = (text(lead)?, text(other)?);
    let stated =
        |code: &str| Cfi::is_classified(code) || code == yggdryl::implementer::CFI_UNCLASSIFIED;
    if !stated(&lead_text) || !stated(&other_text) {
        return None;
    }
    let refined = Cfi::refined(&lead_text, &other_text)?;
    Some(if refined == lead_text {
        lead.clone()
    } else {
        Scalar::from(refined.as_str())
    })
}

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
    /// The stated `CFICode(461)` - the classification of record, a
    /// bridge's `DETAILEDCFICODE` already folded into it where the message
    /// was built - refined by `PutOrCall(201)`, which is the *group* of a
    /// listed option - `OC`, `OP`, `OM` - rather than an attribute, and so
    /// refines a stated Others group `OM` and nothing else.
    ///
    /// The answer is kept only where it is [detailed](yggdryl::Cfi::is_detailed):
    /// a code that says nothing past its category and group - `ESXXXX` - is
    /// a coarse fact the market answers none for, and a `SecurityType(167)`
    /// or `Product(460)` alone, which never reach further than that, answer
    /// none too.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// #     yggdryl_fix::install().unwrap();
    /// # use std::sync::Arc;
    /// # use yggdryl::local::LocalFolder;
    /// # use yggdryl_fix::{FixCodec, FixRegistry};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/fix");
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
        let held = |tag: i32| self.get_by_tag(tag).filter(|held| !held.is_null());
        let code = held(CFICODE_TAG)
            .and_then(|held| held.as_str().map(str::trim).map(str::to_ascii_uppercase))?;
        // `PutOrCall` refines a listed option stated as its Others group:
        // `OM` and "a put" means `OP` rather than "an option of unspecified
        // kind", and `OM` licenses no attribute of its own, so the stated
        // attributes only read under the group 201 names. A group actually
        // stated is never overwritten.
        let listed_option_group = held(201)
            .and_then(|held| held.as_i128())
            .and_then(|held| i64::try_from(held).ok())
            .and_then(option_group);
        let code = match listed_option_group {
            Some(group) if group != 'M' && code.starts_with("OM") => {
                let mut spelled: Vec<char> = code.chars().collect();
                spelled[1] = group;
                spelled.into_iter().collect()
            }
            _ => code,
        };
        Some(SmolStr::new(code)).filter(|code| Cfi::is_detailed(code))
    }
}
