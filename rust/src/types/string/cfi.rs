//! What a [`Cfi`] code means: ISO 10962, six characters.
//!
//! The [code registry](super::codes) already owns a CFI's width and its
//! ASCII validity, which is all a *storage* layer needs. This is the rest of
//! it - what the six characters say - and it lives here rather than in any
//! protocol module because a CFI is a value, not a message: FIX's
//! `CFICode(461)`, an ISIN registry's classification and a lake column all
//! read the same six characters the same way, and a protocol that owned the
//! reading would be a second one.
//!
//! # Six positions, and only four of them take `X`
//!
//! Position 1 is the **category** and position 2 the **group** within it.
//! Positions 3 to 6 are four **attributes** whose meaning depends on the
//! `(category, group)` pair - position 5 is "payment status" for `ES`,
//! "income" for `EP`, "assets" for `CI` and not applicable at all for `SE` -
//! so nothing here reads an attribute by position alone.
//!
//! [`Cfi::UNKNOWN`] means "not applicable or unknown" and is valid **only in
//! positions 3 to 6**. There is no valid category `X` and no valid group `X`,
//! which is what stops a caller filling a code it does not have: a value that
//! knows no category is absent, never `XXXXXX`.
//!
//! Where a category is known and its group is not, [`Cfi::coarse`] answers
//! that category's **Others** group rather than an `X`, because that is how
//! the standard itself spells "this kind of thing, kind unspecified": `EM` is
//! Equities/Others, `DM` Debt/Others, `CM` CIVs/Others.
//!
//! # Merging two statements
//!
//! [`Cfi::merged`] folds two codes for one instrument position by position,
//! and only when they agree on what the instrument *is*: same category, same
//! group. A stated attribute fills an unknown one, so `ESXXXX` merged with
//! `ESVUFR` is `ESVUFR`. Two different stated attributes are a conflict, and
//! this crate has one answer for two voices that disagree - ambiguity answers
//! nothing - so that position answers `X` rather than picking a winner. Two
//! different categories or groups are not a merge at all: they are two
//! statements about two different instruments, and the answer is `None`.
//!
//! # Provenance
//!
//! The tables are the ISO 10962:2021 code list published by SIX Financial
//! Information, the standard's appointed Maintenance Agency, as
//! `cfi-20210507-current`. The 2021 edition moved the code list out of the
//! standard document and publishes it as a free external list, so this is the
//! list itself rather than a reading of it.

use smol_str::SmolStr;

use super::code::Cfi;

/// One ISO 10962 category and the groups it contains.
pub struct CfiCategory {
    letter: char,
    name: &'static str,
    groups: &'static [CfiGroup],
}

/// One group within a category, and what its four attribute positions accept.
pub struct CfiGroup {
    letter: char,
    name: &'static str,
    /// Positions 3, 4, 5 and 6, each the letters that position accepts
    /// besides [`Cfi::UNKNOWN`]. An empty string means only `X` reads there.
    attributes: [&'static str; 4],
}

impl CfiCategory {
    /// The letter this category is written as.
    #[must_use]
    pub const fn letter(&self) -> char {
        self.letter
    }

    /// The standard's own name for this category.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The groups this category contains, in letter order.
    #[must_use]
    pub const fn groups(&self) -> &'static [CfiGroup] {
        self.groups
    }

    /// The group a letter names within this category.
    #[must_use]
    pub fn group(&self, letter: char) -> Option<&'static CfiGroup> {
        self.groups.iter().find(|held| held.letter == letter)
    }
}

impl CfiGroup {
    /// The letter this group is written as.
    #[must_use]
    pub const fn letter(&self) -> char {
        self.letter
    }

    /// The standard's own name for this group.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The letters position `at` (0 for position 3 through 3 for position 6)
    /// accepts besides [`Cfi::UNKNOWN`]; empty where the position does not
    /// apply to this group.
    #[must_use]
    pub fn attributes(&self, at: usize) -> &'static str {
        self.attributes.get(at).copied().unwrap_or_default()
    }
}

/// Every category, its groups, and each group's four attribute sets.
///
/// Generated from the ISO 10962:2021 code list published by SIX Financial
/// Information, the standard's Maintenance Agency (`cfi-20210507-current`).
/// An attribute set is the letters that position accepts *besides* `X`; an
/// empty one means the position is not applicable to that group and only `X`
/// reads there.
pub const CFI_CATEGORIES: [CfiCategory; 14] = [
    CfiCategory {
        letter: 'E',
        name: "Equities",
        groups: &[
            CfiGroup {
                letter: 'C',
                name: "Common/ordinary convertible shares",
                attributes: ["ENRV", "TU", "FOP", "BMNR"],
            },
            CfiGroup {
                letter: 'D',
                name: "Depositary receipts on equities",
                attributes: ["CFLMPS", "BDNR", "ACDFNPQU", "BMNR"],
            },
            CfiGroup {
                letter: 'F',
                name: "Preferred/preference convertible shares",
                attributes: ["ENRV", "ACEGNRT", "ACFNPQU", "BMNR"],
            },
            CfiGroup {
                letter: 'L',
                name: "Limited partnership units",
                attributes: ["ENRV", "TU", "FOP", "BMNR"],
            },
            CfiGroup {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", "BMNR"],
            },
            CfiGroup {
                letter: 'P',
                name: "Preferred/preference shares",
                attributes: ["ENRV", "ACEGNRT", "ACFNPQU", "BMNR"],
            },
            CfiGroup {
                letter: 'S',
                name: "Common/ordinary shares",
                attributes: ["ENRV", "TU", "FOP", "BMNR"],
            },
            CfiGroup {
                letter: 'Y',
                name: "Structured instruments",
                attributes: ["ABCDEM", "DMY", "EFMV", "BCDGIMNST"],
            },
        ],
    },
    CfiCategory {
        letter: 'C',
        name: "CIVs",
        groups: &[
            CfiGroup {
                letter: 'B',
                name: "Real estate investment trust",
                attributes: ["CMO", "GIJ", "", "QSUY"],
            },
            CfiGroup {
                letter: 'E',
                name: "Exchange traded funds",
                attributes: ["CMO", "GIJ", "BCDEFKLMRV", "SU"],
            },
            CfiGroup {
                letter: 'F',
                name: "Funds of funds",
                attributes: ["CMO", "GIJ", "BEHIMP", "QSUY"],
            },
            CfiGroup {
                letter: 'H',
                name: "Hedge funds",
                attributes: ["ADELMNRS", "", "", ""],
            },
            CfiGroup {
                letter: 'I',
                name: "Standard",
                attributes: ["CMO", "GIJ", "BCDEFKLMRV", "QSUY"],
            },
            CfiGroup {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", "QSUY"],
            },
            CfiGroup {
                letter: 'P',
                name: "Private equity funds",
                attributes: ["CMO", "GIJ", "BCDEFKLMRV", "QSUY"],
            },
            CfiGroup {
                letter: 'S',
                name: "Pension funds",
                attributes: ["CMO", "BGLM", "BMR", "SU"],
            },
        ],
    },
    CfiCategory {
        letter: 'D',
        name: "Debt instruments",
        groups: &[
            CfiGroup {
                letter: 'A',
                name: "Asset-backed securities",
                attributes: ["FVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            CfiGroup {
                letter: 'B',
                name: "Bonds",
                attributes: ["CFKVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            CfiGroup {
                letter: 'C',
                name: "Convertible bonds",
                attributes: ["FKVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            CfiGroup {
                letter: 'D',
                name: "Depositary receipts on debt instruments",
                attributes: ["ABCGMNTWY", "CFVZ", "CGJNOPQSTU", "ABCDEFGLPQRT"],
            },
            CfiGroup {
                letter: 'E',
                name: "Structured instruments",
                attributes: ["ABCDEM", "DFMVY", "CMRST", "BCDIMNST"],
            },
            CfiGroup {
                letter: 'G',
                name: "Mortgage-backed securities",
                attributes: ["FVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            CfiGroup {
                letter: 'M',
                name: "Others",
                attributes: ["BMP", "", "", "BMNR"],
            },
            CfiGroup {
                letter: 'N',
                name: "Municipal bonds",
                attributes: ["FVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            CfiGroup {
                letter: 'S',
                name: "Structured instruments",
                attributes: ["ABCDM", "DFMVY", "FMV", "BCDIMNST"],
            },
            CfiGroup {
                letter: 'T',
                name: "Medium-term notes",
                attributes: ["FKVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            CfiGroup {
                letter: 'W',
                name: "Bonds with warrants attached",
                attributes: ["FKVZ", "CGJNOPQSTU", "ABCDEFGLPQRT", "BMNR"],
            },
            CfiGroup {
                letter: 'Y',
                name: "Money market instruments",
                attributes: ["FKVZ", "CGJNOPQSTU", "", "BMNR"],
            },
        ],
    },
    CfiCategory {
        letter: 'R',
        name: "Entitlement (rights)",
        groups: &[
            CfiGroup {
                letter: 'A',
                name: "Allotment",
                attributes: ["", "", "", "BMNR"],
            },
            CfiGroup {
                letter: 'D',
                name: "Depositary receipts on entitlements",
                attributes: ["AMPSW", "", "", "BMNR"],
            },
            CfiGroup {
                letter: 'F',
                name: "Mini-future certificates, constant leverage certificates",
                attributes: ["BCDIMST", "MNT", "CMP", "ABEM"],
            },
            CfiGroup {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", ""],
            },
            CfiGroup {
                letter: 'P',
                name: "Purchase rights",
                attributes: ["BCFIMPS", "", "", "BMNR"],
            },
            CfiGroup {
                letter: 'S',
                name: "Subscription rights",
                attributes: ["BCFIMPS", "", "", "BMNR"],
            },
            CfiGroup {
                letter: 'W',
                name: "Warrants",
                attributes: ["BCDIMST", "CNT", "BCP", "ABEM"],
            },
        ],
    },
    CfiCategory {
        letter: 'O',
        name: "Listed options",
        groups: &[
            CfiGroup {
                letter: 'C',
                name: "Call options",
                attributes: ["ABE", "BCDFIMNOSTW", "CENP", "NS"],
            },
            CfiGroup {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", ""],
            },
            CfiGroup {
                letter: 'P',
                name: "Put options",
                attributes: ["ABE", "BCDFIMNOSTW", "CENP", "NS"],
            },
        ],
    },
    CfiCategory {
        letter: 'F',
        name: "Futures",
        groups: &[
            CfiGroup {
                letter: 'C',
                name: "Commodities futures",
                attributes: ["AEHIMNPS", "CNP", "NS", ""],
            },
            CfiGroup {
                letter: 'F',
                name: "Financial futures",
                attributes: ["BCDFIMNOSVW", "CNP", "NS", ""],
            },
        ],
    },
    CfiCategory {
        letter: 'S',
        name: "Swaps",
        groups: &[
            CfiGroup {
                letter: 'C',
                name: "Credit",
                attributes: ["BIMUV", "CMT", "CLS", "ACP"],
            },
            CfiGroup {
                letter: 'E',
                name: "Equity",
                attributes: ["BIMS", "CDLMPTV", "", "CEP"],
            },
            CfiGroup {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["ACM", "", "", "CP"],
            },
            CfiGroup {
                letter: 'M',
                name: "Others",
                attributes: ["MP", "", "", "CEP"],
            },
            CfiGroup {
                letter: 'R',
                name: "Rates",
                attributes: ["ACDGHMZ", "CDIY", "CS", "DN"],
            },
            CfiGroup {
                letter: 'T',
                name: "Commodities",
                attributes: ["ABCGHIJKMNPQST", "CT", "", "CEP"],
            },
        ],
    },
    CfiCategory {
        letter: 'H',
        name: "Non-listed and complex listed options",
        groups: &[
            CfiGroup {
                letter: 'C',
                name: "Credit",
                attributes: ["IMUVW", "ABCDEFGHI", "ABDGLMPV", "CEP"],
            },
            CfiGroup {
                letter: 'E',
                name: "Equity",
                attributes: ["BFIMORS", "ABCDEFGHI", "ABDGLMPV", "CEP"],
            },
            CfiGroup {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["BCDEFMQRTUVWY", "JKL", "ABDGLMPV", "CEP"],
            },
            CfiGroup {
                letter: 'M',
                name: "Others",
                attributes: ["MP", "ABCDEFGHIJKL", "ABDGLMPV", "ACENP"],
            },
            CfiGroup {
                letter: 'R',
                name: "Rates",
                attributes: ["ACDEFGHMOR", "ABCDEFGHI", "ABCDFGLMPV", "CEP"],
            },
            CfiGroup {
                letter: 'T',
                name: "Commodities",
                attributes: ["ABCFGHIJKMNOPRSTW", "ABCDEFGHI", "ABDGLMPV", "CEP"],
            },
        ],
    },
    CfiCategory {
        letter: 'I',
        name: "Spot",
        groups: &[
            CfiGroup {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["", "", "", "P"],
            },
            CfiGroup {
                letter: 'T',
                name: "Commodities",
                attributes: ["AJKMNPST", "", "", ""],
            },
        ],
    },
    CfiCategory {
        letter: 'J',
        name: "Forwards",
        groups: &[
            CfiGroup {
                letter: 'C',
                name: "Credit",
                attributes: ["ABCDGIO", "", "CFS", "CP"],
            },
            CfiGroup {
                letter: 'E',
                name: "Equity",
                attributes: ["BFIOS", "", "CFS", "CP"],
            },
            CfiGroup {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["FJKLNORSTUVW", "", "CFRS", "CP"],
            },
            CfiGroup {
                letter: 'R',
                name: "Rates",
                attributes: ["IMO", "", "CFS", "CP"],
            },
            CfiGroup {
                letter: 'T',
                name: "Commodities",
                attributes: ["ABCGHIJKMNPST", "", "CFS", "CP"],
            },
        ],
    },
    CfiCategory {
        letter: 'K',
        name: "Strategies",
        groups: &[
            CfiGroup {
                letter: 'C',
                name: "Credit",
                attributes: ["", "", "", ""],
            },
            CfiGroup {
                letter: 'E',
                name: "Equity",
                attributes: ["", "", "", ""],
            },
            CfiGroup {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["", "", "", ""],
            },
            CfiGroup {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", ""],
            },
            CfiGroup {
                letter: 'R',
                name: "Rates",
                attributes: ["", "", "", ""],
            },
            CfiGroup {
                letter: 'T',
                name: "Commodities",
                attributes: ["", "", "", ""],
            },
            CfiGroup {
                letter: 'Y',
                name: "Mixed assets",
                attributes: ["", "", "", ""],
            },
        ],
    },
    CfiCategory {
        letter: 'L',
        name: "Financing",
        groups: &[
            CfiGroup {
                letter: 'L',
                name: "Loan-lease",
                attributes: ["ABJKMNPST", "", "", "CP"],
            },
            CfiGroup {
                letter: 'R',
                name: "Repurchase agreements",
                attributes: ["CGS", "FNOT", "", "DHT"],
            },
            CfiGroup {
                letter: 'S',
                name: "Securities lending",
                attributes: ["CDEGKLMPTW", "NOT", "", "DFHT"],
            },
        ],
    },
    CfiCategory {
        letter: 'T',
        name: "Referential instruments",
        groups: &[
            CfiGroup {
                letter: 'B',
                name: "Baskets",
                attributes: ["CDEFIMT", "", "", ""],
            },
            CfiGroup {
                letter: 'C',
                name: "Currencies",
                attributes: ["CLMN", "", "", ""],
            },
            CfiGroup {
                letter: 'D',
                name: "Stock dividends",
                attributes: ["CFKLMPS", "", "", ""],
            },
            CfiGroup {
                letter: 'I',
                name: "Indices",
                attributes: ["CDEFMRT", "CEFMP", "GMNP", ""],
            },
            CfiGroup {
                letter: 'M',
                name: "Others",
                attributes: ["", "", "", ""],
            },
            CfiGroup {
                letter: 'R',
                name: "Interest rates",
                attributes: ["FMNRV", "ADMNQSW", "", ""],
            },
            CfiGroup {
                letter: 'T',
                name: "Commodities",
                attributes: ["AEHIMNPS", "", "", ""],
            },
        ],
    },
    CfiCategory {
        letter: 'M',
        name: "Others (miscellaneous)",
        groups: &[
            CfiGroup {
                letter: 'C',
                name: "Combined instruments",
                attributes: ["ABHMSUW", "TU", "", "BMNR"],
            },
            CfiGroup {
                letter: 'M',
                name: "Other assets",
                attributes: ["EIMNPRST", "", "", ""],
            },
        ],
    },
];
impl Cfi {
    /// How many characters a CFI code has, in every edition of the standard.
    pub const LENGTH: usize = 6;

    /// The character every attribute position accepts, meaning "not
    /// applicable or unknown". Never valid as a category or a group.
    pub const UNKNOWN: char = 'X';

    /// The category a letter names, or `None` where no category has it.
    #[must_use]
    pub fn category_of(letter: char) -> Option<&'static CfiCategory> {
        CFI_CATEGORIES.iter().find(|held| held.letter == letter)
    }

    /// Whether `code` is a well-formed ISO 10962 code: six uppercase letters,
    /// a real category, one of that category's groups, and an attribute each
    /// position accepts or `X`.
    ///
    /// The code registry already answered width and ASCII; this answers
    /// meaning, which is the part a caller filling a classification needs.
    ///
    /// ```
    /// # use yggdryl::types::Cfi;
    /// assert!(Cfi::is_classified("ESVUFR"));
    /// assert!(Cfi::is_classified("ESXXXX"));
    /// // `X` is not a category and not a group.
    /// assert!(!Cfi::is_classified("XXXXXX"));
    /// assert!(!Cfi::is_classified("EXXXXX"));
    /// // `Z` is no voting right a common share has.
    /// assert!(!Cfi::is_classified("ESZUFR"));
    /// // Position 5 is not applicable to a hedge fund, so only `X` reads there.
    /// assert!(Cfi::is_classified("CHAXXX"));
    /// assert!(!Cfi::is_classified("CHAAXX"));
    /// ```
    #[must_use]
    pub fn is_classified(code: &str) -> bool {
        Self::parsed(code).is_some()
    }

    /// The category and group a well-formed code names, with its attributes.
    fn parsed(code: &str) -> Option<(&'static CfiCategory, &'static CfiGroup, [char; 4])> {
        let held: Vec<char> = code.chars().collect();
        let [category, group, rest @ ..] = held.as_slice() else {
            return None;
        };
        let attributes: [char; 4] = rest.try_into().ok()?;
        let category = Self::category_of(*category)?;
        let group = category.group(*group)?;
        for (at, held) in attributes.iter().enumerate() {
            if *held != Self::UNKNOWN && !group.attributes(at).contains(*held) {
                return None;
            }
        }
        Some((category, group, attributes))
    }

    /// Two statements about one instrument as one, position by position.
    ///
    /// `None` when either is not a well-formed code, or when they name
    /// different instruments - a different category or group is not a
    /// disagreement to resolve but two subjects, and merging them would
    /// invent an instrument neither statement described.
    ///
    /// Within one `(category, group)`, a stated attribute fills an unknown
    /// one and two different stated attributes answer `X`: ambiguity answers
    /// nothing.
    ///
    /// ```
    /// # use yggdryl::types::Cfi;
    /// // What one statement left unsaid, the other says.
    /// assert_eq!(Cfi::merged("ESXXXX", "ESVUFR").as_deref(), Some("ESVUFR"));
    /// assert_eq!(Cfi::merged("ESVUFR", "ESXXXX").as_deref(), Some("ESVUFR"));
    /// // Each fills the other's gaps.
    /// assert_eq!(Cfi::merged("ESVXXX", "ESXUFR").as_deref(), Some("ESVUFR"));
    /// // A disagreement inside one instrument is unknown, not a winner.
    /// assert_eq!(Cfi::merged("ESVUFR", "ESNUFR").as_deref(), Some("ESXUFR"));
    /// // Two different instruments are not one.
    /// assert_eq!(Cfi::merged("ESVUFR", "DBFNFB"), None);
    /// assert_eq!(Cfi::merged("ESVUFR", "EPVNFR"), None);
    /// ```
    #[must_use]
    pub fn merged(left: &str, right: &str) -> Option<SmolStr> {
        let (category, group, mine) = Self::parsed(left)?;
        let (other_category, other_group, theirs) = Self::parsed(right)?;
        if category.letter != other_category.letter || group.letter != other_group.letter {
            return None;
        }
        let mut held = String::with_capacity(Self::LENGTH);
        held.push(category.letter);
        held.push(group.letter);
        for (mine, theirs) in mine.into_iter().zip(theirs) {
            held.push(match (mine, theirs) {
                (Self::UNKNOWN, held) | (held, Self::UNKNOWN) => held,
                (mine, theirs) if mine == theirs => mine,
                // Two voices, two answers, and picking one is a guess.
                _ => Self::UNKNOWN,
            });
        }
        Some(SmolStr::new(held))
    }

    /// One code from a category and a group a caller inferred, with every
    /// attribute unknown.
    ///
    /// The group falls back to the category's own Others rather than to `X`,
    /// because `X` is not a group and the standard's way of saying "this
    /// category, kind unspecified" is that category's Others group.
    ///
    /// ```
    /// # use yggdryl::types::Cfi;
    /// assert_eq!(Cfi::coarse('E', Some('S')).as_deref(), Some("ESXXXX"));
    /// assert_eq!(Cfi::coarse('E', None).as_deref(), Some("EMXXXX"));
    /// // A group the category does not have is not invented.
    /// assert_eq!(Cfi::coarse('E', Some('Q')).as_deref(), Some("EMXXXX"));
    /// assert_eq!(Cfi::coarse('X', None), None);
    /// ```
    #[must_use]
    pub fn coarse(category: char, group: Option<char>) -> Option<SmolStr> {
        let category = Self::category_of(category)?;
        let group = group
            .and_then(|letter| category.group(letter))
            .or_else(|| category.group('M'))?;
        let mut held = String::with_capacity(Self::LENGTH);
        held.push(category.letter);
        held.push(group.letter);
        for _ in 0..4 {
            held.push(Self::UNKNOWN);
        }
        Some(SmolStr::new(held))
    }
}
