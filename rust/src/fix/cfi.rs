//! Reading a FIX message as an instrument classification.
//!
//! What a CFI code *is* - its six positions, which letters each accepts, how
//! two statements merge - belongs to the value and lives beside it in
//! [`types::Cfi`](crate::types::Cfi). This module is the other half: which
//! FIX tags say something about a classification, and what each of them says.
//!
//! That split is the point. `Cfi::merged` is the same fold whether the two
//! codes came off a FIX wire, an ISIN registry or two columns of a table, so
//! a FIX-shaped copy of it would be a second answer to one question. What is
//! genuinely FIX's is the chain below: that `SecurityType(167)` is the only
//! scalar tag naming a group, that `PutOrCall(201)` is the *group* of a
//! listed option rather than an attribute, that `Product(460)` reaches a
//! category and no further.

use smol_str::SmolStr;

use crate::types::Cfi;

/// The tag FIX publishes the ISO 10962 classification under, since FIX 4.3.
///
/// This crate adds no column of its own for it: 461 is already where a
/// message states its classification, and a second column would be a second
/// owner of one fact.
pub(super) const CFICODE_TAG: i32 = 461;

/// What `Product(460)` says the instrument is, as a category.
///
/// The complete shipped code set, all thirteen. `Product` names an asset
/// class and nothing finer, so each answer is a category with no group -
/// which [`coarse`] turns into that category's Others.
///
/// `OTHER` maps to `M`, the standard's own Others category, rather than to
/// nothing: a message saying "some other kind of thing" has said something.
fn category_of_product(product: i64) -> Option<char> {
    Some(match product {
        1 | 3 | 6 | 8 | 9 | 10 | 11 => 'D', // agency, corporate, government,
        // loan, money market, mortgage, municipal - all debt
        4 | 7 => 'T',  // currency and index are referential instruments
        5 => 'E',      // equity
        13 => 'L',     // financing
        2 | 12 => 'M', // commodity and other
        _ => return None,
    })
}

/// What `SecurityType(167)` says the instrument is, as a category and a group.
///
/// `SecurityType` is a 194-code set and most of it names debt of one kind or
/// another, which `Product` already answers coarsely; what this table adds is
/// the codes that pin a *group*, because a group is the position `Product`
/// can never reach. A code not here is not a gap in the chain - it falls
/// through to `Product` and lands on the category's Others - so this grows by
/// rows when a desk needs a finer answer than that.
fn classification_of_security_type(security_type: &str) -> Option<(char, Option<char>)> {
    Some(match security_type {
        "CS" => ('E', Some('S')),                          // common stock
        "PS" => ('E', Some('P')),                          // preferred stock
        "CVPS" => ('E', Some('F')),                        // convertible preferred stock
        "DR" | "ADR" => ('E', Some('D')),                  // depositary receipts
        "LTD" => ('E', Some('L')),                         // limited partnership
        "SMP" | "SCP" => ('E', Some('Y')),                 // structured participation
        "MF" => ('C', Some('I')),                          // mutual fund
        "ETF" => ('C', Some('E')),                         // exchange traded fund
        "REIT" => ('C', Some('B')),                        // real estate investment trust
        "CORP" | "CB" | "CPP" | "CMB" => ('D', Some('B')), // corporate bonds
        "TBOND" | "TNOTE" | "GO" | "REV" | "USTB" => ('D', Some('B')),
        "CONVBOND" | "CVB" => ('D', Some('C')), // convertible bonds
        "MTN" => ('D', Some('T')),              // medium-term notes
        "CD" | "CP" | "BA" | "TB" | "STN" => ('D', Some('Y')), // money market
        "ABS" => ('D', Some('A')),              // asset-backed
        "MBS" | "MPT" | "CMO" | "TBA" => ('D', Some('G')), // mortgage-backed
        "MUNI" => ('D', Some('N')),             // municipal
        "OPT" => ('O', None),                   // listed option
        "FUT" => ('F', None),                   // future
        "FWD" => ('J', None),                   // forward
        "SWAP" | "IRS" | "CDS" => ('S', None),  // swaps
        "WAR" => ('R', Some('W')),              // warrants
        "RIGHT" => ('R', Some('S')),            // subscription rights
        "FX" | "FXSPOT" => ('I', None),         // spot
        "REPO" | "REVREPO" | "BUYSELL" | "SECLOAN" | "SECPLEDGE" => ('L', None),
        _ => return None,
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

/// The category an `SecurityIDSource(22)` licenses, where it licenses one.
///
/// An identifier scheme is evidence about the instrument: a message whose
/// `SecurityID` is an ISO 4217 currency code is describing a currency, and
/// one whose identifier is a settlement-entity code is not describing an
/// equity. Most schemes - ISIN, CUSIP, SEDOL, RIC, Bloomberg - are issued
/// across every category and license nothing, which is the honest answer for
/// them.
fn category_of_id_source(source: &str) -> Option<char> {
    Some(match source {
        // ISO 4217 currency code: the instrument is a currency.
        "6" => 'T',
        // Clearing house / clearing organization contract.
        "H" => 'F',
        _ => return None,
    })
}

impl super::FixMsg {
    /// The instrument's classification, filled to the maximum the message
    /// licenses.
    ///
    /// The chain, each step merged into the last through [`fix_cfi_merged`]
    /// so a later step can only *fill* what an earlier one left unknown:
    ///
    /// 1. a stated `CFICode(461)`, which is the instrument's classification
    ///    of record and outranks anything derived;
    /// 2. `SecurityType(167)`, which is the only scalar tag that pins a
    ///    group;
    /// 3. `Product(460)`, which pins a category and leaves the group to that
    ///    category's Others;
    /// 4. `SecurityIDSource(22)`, where the identifier's scheme licenses a
    ///    category at all;
    /// 5. `PutOrCall(201)`, which is the *group* of a listed option - `OC`,
    ///    `OP`, `OM` - rather than an attribute, and so refines a category
    ///    `O` this chain reached only as far as its Others group.
    ///
    /// A step that names a different instrument than the one established -
    /// a different category or group - does not overwrite it and does not
    /// merge: [`fix_cfi_merged`] answers `None` and the step is dropped,
    /// because a message stating `CFICode=ESXXXX` and `SecurityType=FUT` has
    /// disagreed with itself and the stated classification is the one of
    /// record.
    ///
    /// `None` where the message licenses no category at all. That is the
    /// honest answer and not `XXXXXX`: `X` is not a category.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::{FixCodec, FixRegistry};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    /// let reader = FixCodec::new(Arc::clone(&registry));
    ///
    /// // A stated code is the classification of record.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|461=ESVUFR|10=0|")?;
    /// assert_eq!(held.classification().as_deref(), Some("ESVUFR"));
    ///
    /// // SecurityType alone pins a category and a group.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|167=CS|10=0|")?;
    /// assert_eq!(held.classification().as_deref(), Some("ESXXXX"));
    ///
    /// // A stated code that left attributes unknown takes what the rest says.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|461=ESXXXX|167=CS|10=0|")?;
    /// assert_eq!(held.classification().as_deref(), Some("ESXXXX"));
    ///
    /// // An option and a put is `OP`, because call/put is the group here.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|167=OPT|201=0|10=0|")?;
    /// assert_eq!(held.classification().as_deref(), Some("OPXXXX"));
    ///
    /// // A stated group is never overwritten by a disagreeing 201.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|461=OCAXXX|201=0|10=0|")?;
    /// assert_eq!(held.classification().as_deref(), Some("OCAXXX"));
    ///
    /// // Product alone reaches the category, and its Others group.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|460=5|10=0|")?;
    /// assert_eq!(held.classification().as_deref(), Some("EMXXXX"));
    ///
    /// // Nothing classifying is no code, never XXXXXX.
    /// let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|55=AAPL|10=0|")?;
    /// assert_eq!(held.classification(), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn classification(&self) -> Option<SmolStr> {
        let text = |tag: i32| {
            self.get_by_tag(tag)
                .filter(|held| !held.is_null())
                .and_then(crate::Scalar::as_str)
                .map(str::trim)
                .filter(|held| !held.is_empty())
                .map(str::to_ascii_uppercase)
        };
        let number = |tag: i32| {
            self.get_by_tag(tag)
                .filter(|held| !held.is_null())
                .and_then(crate::Scalar::as_i128)
                .and_then(|held| i64::try_from(held).ok())
        };
        // A step only ever fills what is still unknown: `merged` keeps a
        // stated attribute over a derived one because the stated code leads,
        // and answers `X` where two steps disagree inside one instrument.
        let mut held: Option<SmolStr> = None;
        let mut fold = |candidate: Option<SmolStr>| {
            let Some(candidate) = candidate else { return };
            held = match &held {
                None => Some(candidate),
                // A step describing a different instrument is dropped rather
                // than merged: the classification of record stands.
                Some(current) => {
                    Some(Cfi::merged(current, &candidate).unwrap_or_else(|| current.clone()))
                }
            };
        };
        // Read once, because it is the group of a listed option rather than
        // an attribute filled after the fact.
        let listed_option_group = number(201).and_then(option_group);
        fold(
            text(461)
                .map(SmolStr::new)
                .filter(|held| Cfi::is_classified(held)),
        );
        fold(
            text(167)
                .as_deref()
                .and_then(classification_of_security_type)
                .and_then(|(category, group)| {
                    let group = group.or(match category {
                        'O' => listed_option_group,
                        _ => None,
                    });
                    Cfi::coarse(category, group)
                }),
        );
        fold(
            number(460)
                .and_then(category_of_product)
                .and_then(|category| Cfi::coarse(category, None)),
        );
        fold(
            text(22)
                .as_deref()
                .and_then(category_of_id_source)
                .and_then(|category| Cfi::coarse(category, None)),
        );
        // `PutOrCall` also refines a listed option this chain only reached
        // coarsely: `OM` is the Others group `coarse` falls back to, so a
        // message that said "an option" and "a put" means `OP` rather than
        // "an option of unspecified kind". A group another step actually
        // stated is never overwritten - only the fallback is.
        if let (Some(current), Some(group)) = (held.clone(), listed_option_group) {
            let mut spelled: Vec<char> = current.chars().collect();
            if spelled.first() == Some(&'O') && spelled.get(1) == Some(&'M') && group != 'M' {
                spelled[1] = group;
                let candidate: String = spelled.into_iter().collect();
                if Cfi::is_classified(&candidate) {
                    held = Some(SmolStr::new(candidate));
                }
            }
        }
        held
    }
}
