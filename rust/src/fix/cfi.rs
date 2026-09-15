//! The instrument's classification of record: ISO 10962, six characters.
//!
//! `CFICode(461)` is a plain string in every FIX version that publishes it -
//! FIX 4.3 introduced it and defines no validation of its own, deferring
//! entirely to ISO 10962 - so the meaning is this crate's to own. This module
//! owns it: what a code is, what it means position by position, and how two
//! statements about one instrument become one.
//!
//! # Six positions, and only four of them take `X`
//!
//! Position 1 is the **category** and position 2 the **group** within it.
//! Positions 3 to 6 are four **attributes** whose meaning depends on the
//! `(category, group)` pair - position 5 is "payment status" for `ES`,
//! "income" for `EP`, "assets" for `CI` and not applicable at all for `SE` -
//! so nothing here reads an attribute by position alone.
//!
//! `X` means "not applicable or unknown" and is valid **only in positions 3
//! to 6**. There is no valid category `X` and no valid group `X`. This is the
//! one thing a fallback chain has to respect: "a position nothing licenses
//! stays `X`" is true of the attributes and false of the category and the
//! group, so a message that licenses no category produces no code at all
//! rather than `XXXXXX`.
//!
//! Where a category is known and its group is not, the answer is that
//! category's **Others** group rather than an `X`, because that is how the
//! standard itself spells "this kind of thing, kind unspecified": `EM` is
//! Equities/Others, `DM` Debt/Others, `CM` CIVs/Others.
//!
//! # Merging two statements
//!
//! Two codes for one instrument merge position by position, and only when
//! they agree on what the instrument *is*: same category, same group. A
//! non-`X` attribute fills an `X`, so `ESXXXX` merged with `ESVUFR` is
//! `ESVUFR`. Two different non-`X` attributes are a conflict, and this crate
//! has exactly one answer for two voices that disagree - ambiguity answers
//! nothing - so that position answers `X` rather than picking a winner. Two
//! different categories or groups are not a merge at all: they are two
//! statements about two different instruments, and [`merged`] answers `None`.
//!
//! That rule is not this module's invention. `lift.rs` states it for facets
//! ("Two candidates therefore answer `None`, never the first"), `field.rs`
//! for code spellings ("Ambiguity answers nothing: two codes a caller's
//! spelling reaches are two answers, and picking one is a guess"), and
//! `build.rs` for arrivals ("Conflicting voices fill nothing"). A fourth
//! policy here would be a fourth thing to reason about.
//!
//! # Provenance
//!
//! The tables below are the ISO 10962:2021 code list published by SIX
//! Financial Information, the standard's appointed Maintenance Agency, as
//! `cfi-20210507-current`. The 2021 edition moved the code list out of the
//! standard document and publishes it as a free external list, so this is the
//! list itself rather than a reading of it.

use smol_str::SmolStr;

/// One ISO 10962 category and the groups it contains.
pub(super) struct Category {
    pub(super) letter: char,
    pub(super) name: &'static str,
    pub(super) groups: &'static [Group],
}

/// One group within a category, and what its four attribute positions accept.
pub(super) struct Group {
    pub(super) letter: char,
    pub(super) name: &'static str,
    /// Positions 3, 4, 5 and 6, each the letters that position accepts
    /// besides `X`. An empty string means only `X` reads there.
    pub(super) attributes: [&'static str; 4],
}

/// How many characters a CFI code has, in every edition of the standard.
pub const CFI_LENGTH: usize = 6;

/// The character every attribute position accepts, meaning "not applicable or
/// unknown". Never valid as a category or a group.
pub const CFI_UNKNOWN: char = 'X';

/// Every category, its groups, and each group's four attribute sets.
///
/// Generated from the ISO 10962:2021 code list published by SIX Financial
/// Information, the standard's Maintenance Agency (`cfi-20210507-current`).
/// An attribute set is the letters that position accepts *besides* `X`; an
/// empty one means the position is not applicable to that group and only `X`
/// reads there.
pub(super) const CATEGORIES: [Category; 14] = [
    Category {
        letter: 'E',
        name: "Equities",
        groups: &[
            Group {
                letter: 'C',
                name: "Common/ordinary convertible shares",
                attributes: ["ENRV", "TU", "FOP", "BMNR"],
            },
            Group {
                letter: 'D',
                name: "Depositary receipts on equities",
                attributes: ["CFLMPS", "BDNR", "ACDFNPQU", "BMNR"],
            },
            Group {
                letter: 'F',
                name: "Preferred/preference convertible shares",
                attributes: ["ENRV", "ACEGNRT", "ACFNPQU", "BMNR"],
            },
            Group {
                letter: 'L',
                name: "Limited partnership units",
                attributes: ["ENRV", "TU", "FOP", "BMNR"],
            },
            Group {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", "BMNR"],
            },
            Group {
                letter: 'P',
                name: "Preferred/preference shares",
                attributes: ["ENRV", "ACEGNRT", "ACFNPQU", "BMNR"],
            },
            Group {
                letter: 'S',
                name: "Common/ordinary shares",
                attributes: ["ENRV", "TU", "FOP", "BMNR"],
            },
            Group {
                letter: 'Y',
                name: "Structured instruments",
                attributes: ["ABCDEM", "DMY", "EFMV", "BCDGIMNST"],
            },
        ],
    },
    Category {
        letter: 'C',
        name: "CIVs",
        groups: &[
            Group {
                letter: 'B',
                name: "Real estate investment trust",
                attributes: ["CMO", "GIJ", "", "QSUY"],
            },
            Group {
                letter: 'E',
                name: "Exchange traded funds",
                attributes: ["CMO", "GIJ", "BCDEFKLMRV", "SU"],
            },
            Group {
                letter: 'F',
                name: "Funds of funds",
                attributes: ["CMO", "GIJ", "BEHIMP", "QSUY"],
            },
            Group {
                letter: 'H',
                name: "Hedge funds",
                attributes: ["ADELMNRS", "", "", ""],
            },
            Group {
                letter: 'I',
                name: "Standard",
                attributes: ["CMO", "GIJ", "BCDEFKLMRV", "QSUY"],
            },
            Group {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", "QSUY"],
            },
            Group {
                letter: 'P',
                name: "Private equity funds",
                attributes: ["CMO", "GIJ", "BCDEFKLMRV", "QSUY"],
            },
            Group {
                letter: 'S',
                name: "Pension funds",
                attributes: ["CMO", "BGLM", "BMR", "SU"],
            },
        ],
    },
    Category {
        letter: 'D',
        name: "Debt instruments",
        groups: &[
            Group {
                letter: 'A',
                name: "Asset-backed securities",
                attributes: ["FVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            Group {
                letter: 'B',
                name: "Bonds",
                attributes: ["CFKVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            Group {
                letter: 'C',
                name: "Convertible bonds",
                attributes: ["FKVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            Group {
                letter: 'D',
                name: "Depositary receipts on debt instruments",
                attributes: ["ABCGMNTWY", "CFVZ", "CGJNOPQSTU", "ABCDEFGLPQRT"],
            },
            Group {
                letter: 'E',
                name: "Structured instruments",
                attributes: ["ABCDEM", "DFMVY", "CMRST", "BCDIMNST"],
            },
            Group {
                letter: 'G',
                name: "Mortgage-backed securities",
                attributes: ["FVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            Group {
                letter: 'M',
                name: "Others",
                attributes: ["BMP", "", "", "BMNR"],
            },
            Group {
                letter: 'N',
                name: "Municipal bonds",
                attributes: ["FVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            Group {
                letter: 'S',
                name: "Structured instruments",
                attributes: ["ABCDM", "DFMVY", "FMV", "BCDIMNST"],
            },
            Group {
                letter: 'T',
                name: "Medium-term notes",
                attributes: ["FKVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            Group {
                letter: 'W',
                name: "Bonds with warrants attached",
                attributes: ["FKVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            Group {
                letter: 'Y',
                name: "Money market instruments",
                attributes: ["FKVZ", "CGJNOPQSTU", "", "BMNR"],
            },
        ],
    },
    Category {
        letter: 'R',
        name: "Entitlement (rights)",
        groups: &[
            Group {
                letter: 'A',
                name: "Allotment",
                attributes: ["", "", "", "BMNR"],
            },
            Group {
                letter: 'D',
                name: "Depositary receipts on entitlements",
                attributes: ["AMPSW", "", "", "BMNR"],
            },
            Group {
                letter: 'F',
                name: "Mini-future certificates, constant leverage certificates",
                attributes: ["BCDIMST", "MNT", "CMP", "ABEM"],
            },
            Group {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", ""],
            },
            Group {
                letter: 'P',
                name: "Purchase rights",
                attributes: ["BCFIMPS", "", "", "BMNR"],
            },
            Group {
                letter: 'S',
                name: "Subscription rights",
                attributes: ["BCFIMPS", "", "", "BMNR"],
            },
            Group {
                letter: 'W',
                name: "Warrants",
                attributes: ["BCDIMST", "CNT", "BCP", "ABEM"],
            },
        ],
    },
    Category {
        letter: 'O',
        name: "Listed options",
        groups: &[
            Group {
                letter: 'C',
                name: "Call options",
                attributes: ["ABE", "BCDFIMNOSTW", "CENP", "NS"],
            },
            Group {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", ""],
            },
            Group {
                letter: 'P',
                name: "Put options",
                attributes: ["ABE", "BCDFIMNOSTW", "CENP", "NS"],
            },
        ],
    },
    Category {
        letter: 'F',
        name: "Futures",
        groups: &[
            Group {
                letter: 'C',
                name: "Commodities futures",
                attributes: ["AEHIMNPS", "CNP", "NS", ""],
            },
            Group {
                letter: 'F',
                name: "Financial futures",
                attributes: ["BCDFIMNOSVW", "CNP", "NS", ""],
            },
        ],
    },
    Category {
        letter: 'S',
        name: "Swaps",
        groups: &[
            Group {
                letter: 'C',
                name: "Credit",
                attributes: ["BIMUV", "CMT", "CLS", "ACP"],
            },
            Group {
                letter: 'E',
                name: "Equity",
                attributes: ["BIMS", "CDLMPTV", "", "CEP"],
            },
            Group {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["ACM", "", "", "CP"],
            },
            Group {
                letter: 'M',
                name: "Others",
                attributes: ["MP", "", "", "CEP"],
            },
            Group {
                letter: 'R',
                name: "Rates",
                attributes: ["ACDGHMZ", "CDIY", "CS", "DN"],
            },
            Group {
                letter: 'T',
                name: "Commodities",
                attributes: ["ABCGHIJKMNPQST", "CT", "", "CEP"],
            },
        ],
    },
    Category {
        letter: 'H',
        name: "Non-listed and complex listed options",
        groups: &[
            Group {
                letter: 'C',
                name: "Credit",
                attributes: ["IMUVW", "ABCDEFGHI", "ABDGLMPV", "CEP"],
            },
            Group {
                letter: 'E',
                name: "Equity",
                attributes: ["BFIMORS", "ABCDEFGHI", "ABDGLMPV", "CEP"],
            },
            Group {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["BCDEFMQRTUVWY", "JKL", "ABDGLMPV", "CEP"],
            },
            Group {
                letter: 'M',
                name: "Others",
                attributes: ["MP", "ABCDEFGHIJKL", "ABDGLMPV", "ACENP"],
            },
            Group {
                letter: 'R',
                name: "Rates",
                attributes: ["ACDEFGHMOR", "ABCDEFGHI", "ABCDFGLMPV", "CEP"],
            },
            Group {
                letter: 'T',
                name: "Commodities",
                attributes: ["ABCFGHIJKMNOPRSTW", "ABCDEFGHI", "ABDGLMPV", "CEP"],
            },
        ],
    },
    Category {
        letter: 'I',
        name: "Spot",
        groups: &[
            Group {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["", "", "", "P"],
            },
            Group {
                letter: 'T',
                name: "Commodities",
                attributes: ["AJKMNPST", "", "", ""],
            },
        ],
    },
    Category {
        letter: 'J',
        name: "Forwards",
        groups: &[
            Group {
                letter: 'C',
                name: "Credit",
                attributes: ["ABCDGIO", "", "CFS", "CP"],
            },
            Group {
                letter: 'E',
                name: "Equity",
                attributes: ["BFIOS", "", "CFS", "CP"],
            },
            Group {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["FJKLNORSTUVW", "", "CFRS", "CP"],
            },
            Group {
                letter: 'R',
                name: "Rates",
                attributes: ["IMO", "", "CFS", "CP"],
            },
            Group {
                letter: 'T',
                name: "Commodities",
                attributes: ["ABCGHIJKMNPST", "", "CFS", "CP"],
            },
        ],
    },
    Category {
        letter: 'K',
        name: "Strategies",
        groups: &[
            Group {
                letter: 'C',
                name: "Credit",
                attributes: ["", "", "", ""],
            },
            Group {
                letter: 'E',
                name: "Equity",
                attributes: ["", "", "", ""],
            },
            Group {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["", "", "", ""],
            },
            Group {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", ""],
            },
            Group {
                letter: 'R',
                name: "Rates",
                attributes: ["", "", "", ""],
            },
            Group {
                letter: 'T',
                name: "Commodities",
                attributes: ["", "", "", ""],
            },
            Group {
                letter: 'Y',
                name: "Mixed assets",
                attributes: ["", "", "", ""],
            },
        ],
    },
    Category {
        letter: 'L',
        name: "Financing",
        groups: &[
            Group {
                letter: 'L',
                name: "Loan-lease",
                attributes: ["ABJKMNPST", "", "", "CP"],
            },
            Group {
                letter: 'R',
                name: "Repurchase agreements",
                attributes: ["CGS", "FNOT", "", "DHT"],
            },
            Group {
                letter: 'S',
                name: "Securities lending",
                attributes: ["CDEGKLMPTW", "NOT", "", "DFHT"],
            },
        ],
    },
    Category {
        letter: 'T',
        name: "Referential instruments",
        groups: &[
            Group {
                letter: 'B',
                name: "Baskets",
                attributes: ["CDEFIMT", "", "", ""],
            },
            Group {
                letter: 'C',
                name: "Currencies",
                attributes: ["CLMN", "", "", ""],
            },
            Group {
                letter: 'D',
                name: "Stock dividends",
                attributes: ["CFKLMPS", "", "", ""],
            },
            Group {
                letter: 'I',
                name: "Indices",
                attributes: ["CDEFMRT", "CEFMP", "GMNP", ""],
            },
            Group {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", ""],
            },
            Group {
                letter: 'R',
                name: "Interest rates",
                attributes: ["FMNRV", "ADMNQSW", "", ""],
            },
            Group {
                letter: 'T',
                name: "Commodities",
                attributes: ["AEHIMNPS", "", "", ""],
            },
        ],
    },
    Category {
        letter: 'M',
        name: "Others (miscellaneous)",
        groups: &[
            Group {
                letter: 'C',
                name: "Combined instruments",
                attributes: ["ABHMSUW", "TU", "", "BMNR"],
            },
            Group {
                letter: 'M',
                name: "Other assets",
                attributes: ["EIMNPRST", "", "", ""],
            },
        ],
    },
];
/// The category a letter names, or `None` where no category has it.
#[must_use]
pub(super) fn category_of(letter: char) -> Option<&'static Category> {
    CATEGORIES.iter().find(|held| held.letter == letter)
}

impl Category {
    /// The group a letter names within this category.
    fn group_of(&self, letter: char) -> Option<&'static Group> {
        self.groups.iter().find(|held| held.letter == letter)
    }

    /// This category's "Others" group, which is how the standard spells a
    /// known category whose group nothing licensed.
    ///
    /// Every category publishes one, and it is `M` in all fourteen.
    fn others(&'static self) -> Option<&'static Group> {
        self.group_of('M')
    }
}

/// Whether `code` is a well-formed ISO 10962 code: six uppercase letters, a
/// real category, one of that category's groups, and an attribute each
/// position accepts or `X`.
///
/// ```
/// # use yggdryl::fix_cfi_is_valid;
/// assert!(fix_cfi_is_valid("ESVUFR"));
/// assert!(fix_cfi_is_valid("ESXXXX"));
/// // `X` is not a category and not a group.
/// assert!(!fix_cfi_is_valid("XXXXXX"));
/// assert!(!fix_cfi_is_valid("EXXXXX"));
/// // `Z` is no voting right a common share has.
/// assert!(!fix_cfi_is_valid("ESZUFR"));
/// // Position 5 is not applicable to a hedge fund, so only `X` reads there.
/// assert!(fix_cfi_is_valid("CHAXXX"));
/// assert!(!fix_cfi_is_valid("CHAAXX"));
/// ```
#[must_use]
pub fn fix_cfi_is_valid(code: &str) -> bool {
    parsed(code).is_some()
}

/// The category and group a well-formed code names, with its four attributes.
fn parsed(code: &str) -> Option<(&'static Category, &'static Group, [char; 4])> {
    let held: Vec<char> = code.chars().collect();
    let [category, group, rest @ ..] = held.as_slice() else {
        return None;
    };
    let attributes: [char; 4] = rest.try_into().ok()?;
    let category = category_of(*category)?;
    let group = category.group_of(*group)?;
    for (at, held) in attributes.iter().enumerate() {
        if *held == CFI_UNKNOWN {
            continue;
        }
        if !group.attributes[at].contains(*held) {
            return None;
        }
    }
    Some((category, group, attributes))
}

/// Two statements about one instrument as one, position by position.
///
/// `None` when either is not a well-formed code, or when they name different
/// instruments - a different category or a different group is not a
/// disagreement to resolve but two subjects, and merging them would invent an
/// instrument neither message described.
///
/// Within one `(category, group)`, a stated attribute fills an `X` and two
/// different stated attributes answer `X`: ambiguity answers nothing, which
/// is the rule the rest of this crate already follows.
///
/// ```
/// # use yggdryl::fix_cfi_merged;
/// // What one message left unsaid, the other says.
/// assert_eq!(fix_cfi_merged("ESXXXX", "ESVUFR").as_deref(), Some("ESVUFR"));
/// assert_eq!(fix_cfi_merged("ESVUFR", "ESXXXX").as_deref(), Some("ESVUFR"));
/// // Each fills the other's gaps.
/// assert_eq!(fix_cfi_merged("ESVXXX", "ESXUFR").as_deref(), Some("ESVUFR"));
/// // A disagreement inside one instrument is unknown, not a winner.
/// assert_eq!(fix_cfi_merged("ESVUFR", "ESNUFR").as_deref(), Some("ESXUFR"));
/// // Two different instruments are not one.
/// assert_eq!(fix_cfi_merged("ESVUFR", "DBFNFB"), None);
/// assert_eq!(fix_cfi_merged("ESVUFR", "EPVNFR"), None);
/// ```
#[must_use]
pub fn fix_cfi_merged(left: &str, right: &str) -> Option<SmolStr> {
    let (category, group, mine) = parsed(left)?;
    let (other_category, other_group, theirs) = parsed(right)?;
    if category.letter != other_category.letter || group.letter != other_group.letter {
        return None;
    }
    let mut held = String::with_capacity(CFI_LENGTH);
    held.push(category.letter);
    held.push(group.letter);
    for (mine, theirs) in mine.into_iter().zip(theirs) {
        held.push(match (mine, theirs) {
            (CFI_UNKNOWN, held) | (held, CFI_UNKNOWN) => held,
            (mine, theirs) if mine == theirs => mine,
            // Two voices, two answers, and picking one is a guess.
            _ => CFI_UNKNOWN,
        });
    }
    Some(SmolStr::new(held))
}

/// One code from a category and a group this crate inferred, with every
/// attribute unknown.
///
/// The group falls back to the category's own "Others" rather than to `X`,
/// because `X` is not a group and the standard's way of saying "this
/// category, kind unspecified" is that category's Others group.
pub(super) fn coarse(category: char, group: Option<char>) -> Option<SmolStr> {
    let category = category_of(category)?;
    let group = match group.and_then(|letter| category.group_of(letter)) {
        Some(group) => group,
        None => category.others()?,
    };
    let mut held = String::with_capacity(CFI_LENGTH);
    held.push(category.letter);
    held.push(group.letter);
    for _ in 0..4 {
        held.push(CFI_UNKNOWN);
    }
    Some(SmolStr::new(held))
}

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
                    Some(fix_cfi_merged(current, &candidate).unwrap_or_else(|| current.clone()))
                }
            };
        };
        // Read once, because it is the group of a listed option rather than
        // an attribute filled after the fact.
        let listed_option_group = number(201).and_then(option_group);
        fold(
            text(461)
                .map(SmolStr::new)
                .filter(|held| fix_cfi_is_valid(held)),
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
                    coarse(category, group)
                }),
        );
        fold(
            number(460)
                .and_then(category_of_product)
                .and_then(|category| coarse(category, None)),
        );
        fold(
            text(22)
                .as_deref()
                .and_then(category_of_id_source)
                .and_then(|category| coarse(category, None)),
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
                if fix_cfi_is_valid(&candidate) {
                    held = Some(SmolStr::new(candidate));
                }
            }
        }
        held
    }
}
