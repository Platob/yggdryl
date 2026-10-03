//! ISO 10962 classification codes, and what their six characters say.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::{code_leaf, code_value};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// Every category, its groups, and each group's four attribute sets.
///
/// Generated from the ISO 10962:2021 code list published by SIX Financial
/// Information, the standard's Maintenance Agency (`cfi-20210507-current`).
/// An attribute set is the letters that position accepts *besides* `X`; an
/// empty one means the position is not applicable to that group and only `X`
/// reads there. Where the list and the standard's own group pages disagree
/// the pages win, and the row says so: JF and SF deliver non-deliverable
/// (`N`) in position 6.
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
            // ISO 10962:2021's SF group page lists `N`, non-deliverable, as a
            // delivery beside physical; the generated list lacked it.
            CfiGroup {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["ACM", "", "", "CNP"],
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
            // ISO 10962:2021's JF group page lists `N`, non-deliverable, as a
            // delivery beside physical and cash; the generated list lacked it.
            CfiGroup {
                letter: 'F',
                name: "Foreign exchange",
                attributes: ["FJKLNORSTUVW", "", "CFRS", "CNP"],
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
    /// A code a landed `cfi` column already holds, adopted as it stands:
    /// the landing read the cell under this type's rule.
    pub(crate) fn from_proven(text: &str) -> Self {
        Self(SmolStr::new(text))
    }

    /// How many characters a CFI code has, in every edition of the standard.
    pub const LENGTH: usize = 6;

    /// The character every attribute position accepts, meaning "not
    /// applicable or unknown". Never valid as a category or a group.
    pub const UNKNOWN: char = 'X';

    /// The code that classifies nothing: every position unknown.
    pub(crate) const UNCLASSIFIED: &str = "XXXXXX";

    /// The better of two classifications, as
    /// [`CodeValue::merge_with`](crate::CodeValue::merge_with)
    /// answers it: [`Self::refined`] with this code leading, and this code as
    /// it is where the two do not describe one instrument.
    ///
    /// ```
    /// use yggdryl::{Cfi, CodeValue};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(Cfi::new("XXXXXX")?.merge_with(&Cfi::new("ESVUFR")?).as_str(), "ESVUFR");
    /// assert_eq!(Cfi::new("ESVXXX")?.merge_with(&Cfi::new("ESXUFR")?).as_str(), "ESVUFR");
    /// // A letter the other contradicts keeps this code as it is.
    /// assert_eq!(Cfi::new("ESVUFR")?.merge_with(&Cfi::new("ESNUFR")?).as_str(), "ESVUFR");
    /// // Two different instruments are not one: this code stands.
    /// assert_eq!(Cfi::new("ESVUFR")?.merge_with(&Cfi::new("DBFNFB")?).as_str(), "ESVUFR");
    /// # Ok(())
    /// # }
    /// ```
    pub(super) fn filled(self, other: &Self) -> Self {
        Self::refined(self.as_str(), other.as_str())
            .and_then(|text| Self::new(text).ok())
            .unwrap_or(self)
    }

    /// [`CodeValue::rank`](crate::CodeValue::rank): zero for a code that
    /// classifies nothing, one for one that only classifies - a category
    /// and a group - and two for a detailed one. What [`Self::refined`]
    /// already prefers, read as a number.
    fn ranked(&self) -> u8 {
        if Self::is_detailed(self.as_str()) {
            2
        } else {
            u8::from(Self::is_classified(self.as_str()))
        }
    }

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
    /// # use yggdryl::Cfi;
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

    /// Whether `code` is classified and says something past its category
    /// and group: at least one of positions 3 to 6 is not `X`. A code that
    /// only classifies - `ESXXXX` - is coarse, and a market that keeps only
    /// detailed codes answers none for it; an `X` inside a detailed code is
    /// an attribute the standard leaves unknown and stays legal.
    ///
    /// ```
    /// # use yggdryl::Cfi;
    /// assert!(Cfi::is_detailed("ESVUFR"));
    /// assert!(Cfi::is_detailed("ESVXXX"));
    /// assert!(!Cfi::is_detailed("ESXXXX"));
    /// assert!(!Cfi::is_detailed("EMXXXX"));
    /// assert!(!Cfi::is_detailed("XXXXXX"));
    /// ```
    #[must_use]
    pub fn is_detailed(code: &str) -> bool {
        Self::parsed(code)
            .is_some_and(|(_, _, attributes)| attributes.iter().any(|held| *held != Self::UNKNOWN))
    }

    /// The category and group a well-formed code names, with its attributes.
    fn parsed(code: &str) -> Option<(&'static CfiCategory, &'static CfiGroup, [char; 4])> {
        let [category, group, a, b, c, d] = *code.as_bytes() else {
            return None;
        };
        let attributes = [a, b, c, d].map(char::from);
        let category = Self::category_of(char::from(category))?;
        let group = category.group(char::from(group))?;
        for (at, held) in attributes.iter().enumerate() {
            if *held != Self::UNKNOWN && !group.attributes(at).contains(*held) {
                return None;
            }
        }
        Some((category, group, attributes))
    }

    /// `lead` with every `X` it states filled from `other`, where the two
    /// describe one instrument: the one fold of two statements of a CFI code.
    ///
    /// An unclassified code - all `X`, or a letter no group accepts - states
    /// nothing, so it yields whole to a classified other, and a classified
    /// `lead` stands over an unclassified other; two unclassified codes
    /// answer `None`. Two classified codes describe one instrument when they
    /// share their category and group and no attribute position holds two
    /// different letters: then `lead` keeps every letter it states and takes
    /// the other's where it states `X`. Anything else - another category or
    /// group, which is another instrument, or an attribute the two contradict
    /// - answers `None`, and the caller keeps the statement it leads with.
    ///
    /// ```
    /// # use yggdryl::Cfi;
    /// // What one statement left unsaid, the other says.
    /// assert_eq!(Cfi::refined("ESXXXX", "ESVUFR").as_deref(), Some("ESVUFR"));
    /// assert_eq!(Cfi::refined("ESVUFR", "ESXXXX").as_deref(), Some("ESVUFR"));
    /// assert_eq!(Cfi::refined("ESVXXX", "ESXUFR").as_deref(), Some("ESVUFR"));
    /// // An unclassified code yields whole.
    /// assert_eq!(Cfi::refined("XXXXXX", "ESVUFR").as_deref(), Some("ESVUFR"));
    /// assert_eq!(Cfi::refined("ESVUFR", "XXXXXX").as_deref(), Some("ESVUFR"));
    /// assert_eq!(Cfi::refined("XXXXXX", "XXXXXX"), None);
    /// // A contradicted attribute, and two different instruments, fold to none.
    /// assert_eq!(Cfi::refined("ESVUFR", "ESNUFR"), None);
    /// assert_eq!(Cfi::refined("ESVUFR", "DBFNFB"), None);
    /// assert_eq!(Cfi::refined("ESVUFR", "EPVNFR"), None);
    /// ```
    #[must_use]
    pub fn refined(lead: &str, other: &str) -> Option<SmolStr> {
        let (category, group, mine) = match (Self::parsed(lead), Self::parsed(other)) {
            (None, None) => return None,
            (None, Some(_)) => return Some(SmolStr::new(other)),
            (Some(_), None) => return Some(SmolStr::new(lead)),
            (Some(mine), Some(theirs)) => {
                if mine.0.letter != theirs.0.letter || mine.1.letter != theirs.1.letter {
                    return None;
                }
                (mine.0, mine.1, mine.2.into_iter().zip(theirs.2))
            }
        };
        let mut held = [b'X'; Self::LENGTH];
        held[0] = u8::try_from(category.letter).expect("CFI is ASCII");
        held[1] = u8::try_from(group.letter).expect("CFI is ASCII");
        for (target, (mine, theirs)) in held[2..].iter_mut().zip(mine) {
            let letter = match (mine, theirs) {
                (Self::UNKNOWN, held) | (held, Self::UNKNOWN) => held,
                (mine, theirs) if mine == theirs => mine,
                // Two different letters are two instruments' attributes.
                _ => return None,
            };
            *target = u8::try_from(letter).expect("CFI is ASCII");
        }
        Some(SmolStr::new(
            std::str::from_utf8(&held).expect("CFI is ASCII"),
        ))
    }

    /// One code from a category and a group a caller inferred, with every
    /// attribute unknown.
    ///
    /// The group falls back to the category's own Others rather than to `X`,
    /// because `X` is not a group and the standard's way of saying "this
    /// category, kind unspecified" is that category's Others group.
    ///
    /// ```
    /// # use yggdryl::Cfi;
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
        let held = [
            u8::try_from(category.letter).expect("CFI is ASCII"),
            u8::try_from(group.letter).expect("CFI is ASCII"),
            b'X',
            b'X',
            b'X',
            b'X',
        ];
        Some(SmolStr::new(
            std::str::from_utf8(&held).expect("CFI is ASCII"),
        ))
    }
}

code_leaf!(Cfi, CFI_WIDTH);

code_value!(
    Cfi,
    Cfi,
    CFI_WIDTH,
    merge = Cfi::filled,
    rank = Cfi::ranked,
    max_rank = 2
);

/// The Arrow extension name of the classification code.
pub(crate) const CFI_EXTENSION_NAME: &str = "yggdryl.cfi";

/// The most bytes ISO 10962's classification code may be.
pub(crate) const CFI_WIDTH: usize = 6;

impl DataType {
    /// Creates ISO 10962's six-character classification code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::cfi(), DataType::Cfi);
    /// assert_eq!(DataType::cfi().to_string(), "cfi");
    /// assert_eq!(DataType::cfi().code_width(), Some(6));
    /// ```
    #[must_use]
    pub const fn cfi() -> Self {
        Self::Cfi
    }
}

// /// A CFI-typed field: ISO 10962's instrument classification.
define_field_types!(CfiType, Cfi);

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
