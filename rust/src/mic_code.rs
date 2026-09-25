//! ISO 10383 market identifier codes.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::{code_leaf, code_value};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// Which dxFeed table gives a regional exchange code its meaning.
///
/// dxFeed reuses one-character codes between feeds, so the feed is part of
/// the value: `Q` is Nasdaq (`XNAS`) under CTA/UTP and Nasdaq Basic, but the
/// Nasdaq Options Market (`XNDQ`) under US Options.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DxFeedExchangeFeed {
    /// The consolidated CTA/UTP US equities feed.
    CtaUtp,
    /// Cboe's US equities feed.
    Cboe,
    /// Nasdaq Basic.
    NasdaqBasic,
    /// NYSE Best Quote and Trades.
    NyseBqt,
    /// The US OTC equities feed.
    Otc,
    /// CME data, whose published exchange codes name no single MIC.
    Cme,
    /// US options.
    UsOptions,
}

impl DxFeedExchangeFeed {
    /// The source's published name for this feed.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CtaUtp => "CTA/UTP",
            Self::Cboe => "Cboe",
            Self::NasdaqBasic => "Nasdaq Basic",
            Self::NyseBqt => "NYSE BQT",
            Self::Otc => "OTC",
            Self::Cme => "CME",
            Self::UsOptions => "US Options",
        }
    }
}

impl fmt::Display for DxFeedExchangeFeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_leaf!(MicCode, MIC_WIDTH);

impl MicCode {
    /// ISO 10383's code for no market.
    const NONE: &str = "XXXX";

    /// The market stated as none: ISO 10383's `XXXX`, which a merge takes
    /// the other market over.
    #[must_use]
    pub fn none() -> Self {
        Self(SmolStr::new_static(Self::NONE))
    }

    /// Whether `text` is shaped as an ISO 10383 code: exactly four of
    /// `[A-Z0-9]`. A Reuters mnemonic - `S`, `TW` - is not;
    /// [`MicCode::from_reuters_exchange_code`] resolves one.
    /// [`MicCode::new`] stays permissive, because a stored column may hold
    /// a short code a venue wrote.
    ///
    /// ```
    /// # use yggdryl::MicCode;
    /// assert!(MicCode::is_iso("XSWX"));
    /// assert!(MicCode::is_iso("RJEA"));
    /// assert!(!MicCode::is_iso("S"));
    /// assert!(!MicCode::is_iso("TW"));
    /// assert!(!MicCode::is_iso("xswx"));
    /// assert!(!MicCode::is_iso("XSWXX"));
    /// ```
    #[must_use]
    pub fn is_iso(text: &str) -> bool {
        text.len() == 4
            && text
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    }

    /// Resolves one dxFeed regional exchange code under the feed that defines
    /// it into its ISO 10383 MIC.
    ///
    /// The feed is required because dxFeed's tables intentionally reuse the
    /// same code. This resolves only rows for which dxFeed publishes a MIC;
    /// an aggregate code and CME's source codes name no single market and are
    /// refused. The CTA/UTP `H` row resolves to ISO's current `EPRL` for MIAX
    /// Pearl Equities; dxFeed currently prints the nonexistent `MRPL`.
    ///
    /// ```
    /// use yggdryl::{DxFeedExchangeFeed, MicCode};
    ///
    /// assert_eq!(
    ///     MicCode::from_dxfeed_exchange_code(DxFeedExchangeFeed::CtaUtp, "Q")?.as_str(),
    ///     "XNAS"
    /// );
    /// assert_eq!(
    ///     MicCode::from_dxfeed_exchange_code(DxFeedExchangeFeed::UsOptions, "Q")?.as_str(),
    ///     "XNDQ"
    /// );
    /// assert!(
    ///     MicCode::from_dxfeed_exchange_code(DxFeedExchangeFeed::Cboe, "C").is_err()
    /// );
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the feed and code when the table has no such
    /// code or its row represents an aggregate/source with no single MIC.
    pub fn from_dxfeed_exchange_code(
        feed: DxFeedExchangeFeed,
        code: impl AsRef<str>,
    ) -> Result<Self> {
        use DxFeedExchangeFeed::{Cboe, Cme, CtaUtp, NasdaqBasic, NyseBqt, Otc, UsOptions};

        let code = code.as_ref();
        let mic = match (feed, code) {
            (CtaUtp, "A") => "XASE",
            (CtaUtp, "B") => "XBOS",
            (CtaUtp, "C") => "XCIS",
            (CtaUtp, "D") => "FINR",
            (CtaUtp, "F") => "TXSE",
            (CtaUtp, "G") => "24EQ",
            (CtaUtp, "H") => "EPRL",
            (CtaUtp, "I") => "XISE",
            (CtaUtp, "J") => "EDGA",
            (CtaUtp, "K") => "EDGX",
            (CtaUtp, "L") => "LTSE",
            (CtaUtp, "M") => "XCHI",
            (CtaUtp, "N") => "XNYS",
            (CtaUtp, "P") => "ARCX",
            (CtaUtp, "Q") => "XNAS",
            (CtaUtp, "U") => "MEMX",
            (CtaUtp, "V") => "IEXG",
            (CtaUtp, "W") => "CBSX",
            (CtaUtp, "X") => "XPSX",
            (CtaUtp, "Y") => "BATY",
            (CtaUtp, "Z") => "BATS",
            (Cboe, "A") => "EDGA",
            (Cboe, "X") => "EDGX",
            (Cboe, "Y") => "BATY",
            (Cboe, "Z") => "BATS",
            (NasdaqBasic, "B") => "XBOS",
            (NasdaqBasic, "F") => "FINC",
            (NasdaqBasic, "L") => "FINN",
            (NasdaqBasic, "Q") => "XNAS",
            (NasdaqBasic, "X") => "XPSX",
            (NyseBqt, "A") => "XASE",
            (NyseBqt, "N") => "XNYS",
            (NyseBqt, "C") => "XCIS",
            (NyseBqt, "D") => "FINY",
            (NyseBqt, "M") => "XCHI",
            (NyseBqt, "O") => "GOTC",
            (NyseBqt, "P") => "ARCX",
            (Otc, "U") => "OOTC",
            (Otc, "V") => "OTCM",
            (UsOptions, "A") => "XASE",
            (UsOptions, "B") => "XBOX",
            (UsOptions, "C") => "XCBO",
            (UsOptions, "D") => "EMLD",
            (UsOptions, "E") => "EDGO",
            (UsOptions, "H") => "GMNI",
            (UsOptions, "I") => "XISX",
            (UsOptions, "J") => "MCRY",
            (UsOptions, "M") => "XMIO",
            (UsOptions, "N") => "ARCO",
            (UsOptions, "P") => "MPRL",
            (UsOptions, "Q") => "XNDQ",
            (UsOptions, "S") => "SPHR",
            (UsOptions, "T") => "XBXO",
            (UsOptions, "U") => "MXOP",
            (UsOptions, "W") => "C2OX",
            (UsOptions, "X") => "XPHO",
            (UsOptions, "Z") => "BATO",
            (Cboe, "C" | "U") | (Cme, "G" | "B") => {
                return Err(crate::Error::InvalidDataType {
                    kind: "mic",
                    reason: smol_str::format_smolstr!(
                        "dxFeed {feed} exchange code {code:?} names no single MIC"
                    ),
                });
            }
            _ => {
                return Err(crate::Error::InvalidDataType {
                    kind: "mic",
                    reason: smol_str::format_smolstr!(
                        "expected a dxFeed {feed} exchange code, got {code:?}"
                    ),
                });
            }
        };
        Self::new(mic)
    }

    /// Resolves one Reuters exchange mnemonic - the suffix of a RIC, and the
    /// value FIX 4.2's Appendix C gives `LastMkt(30)`, `ExDestination(100)`
    /// and `SecurityExchange(207)` - into its ISO 10383 MIC.
    ///
    /// Mnemonics are case-sensitive: `B` is Boston and `b` Belfox, `D`
    /// Dusseldorf and `d` Eurex Germany, `P` Pacific and `p` MONEP. A market
    /// that closed resolves to the MIC that carries it on: Pacific to
    /// `ARCX`, its options to `ARCO`. A row naming a segment, a scheme or no
    /// market - `TH` Third Market, `0` None, `11` OTC - and a closed market
    /// ISO 10383 never carried on are refused.
    ///
    /// ```
    /// use yggdryl::MicCode;
    ///
    /// assert_eq!(MicCode::from_reuters_exchange_code("L")?.as_str(), "XLON");
    /// assert_eq!(MicCode::from_reuters_exchange_code("TW")?.as_str(), "XTAI");
    /// assert_eq!(MicCode::from_reuters_exchange_code("d")?.as_str(), "XEUR");
    /// assert!(MicCode::from_reuters_exchange_code("TH").is_err());
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the mnemonic when Appendix C has no such
    /// mnemonic or its row names no single current MIC.
    pub fn from_reuters_exchange_code(code: impl AsRef<str>) -> Result<Self> {
        let code = code.as_ref();
        let mic = match code {
            "A" | "1" => "XASE",
            "AM" => "XAMM",
            "AS" => "XAMS",
            "AX" => "XASX",
            "B" => "XBOS",
            "BC" => "XBAR",
            "BE" => "XBER",
            "BH" => "XBAH",
            "BI" => "XBIL",
            "BIV" => "BIVA",
            "BK" => "XBKK",
            "BM" => "XBRE",
            "BN" => "XBRN",
            "BO" => "XBOM",
            "BR" => "XBRU",
            "BT" => "XBOT",
            "BY" => "XBEY",
            "C" => "XCIS",
            "CE" => "XBCL",
            "CH" => "XCIE",
            "CI" => "XBRV",
            "CL" => "XCAL",
            "CM" => "XCOL",
            "CO" => "XCSE",
            "D" => "XDUS",
            "DL" => "XDES",
            "DU" => "XDFM",
            "E" => "XEUE",
            "F" => "XFRA",
            "FU" => "XFKA",
            "GH" => "XGHA",
            "H" => "XHAM",
            "HA" => "XHAN",
            "HE" => "XHEL",
            "HK" => "XHKG",
            "I" => "XDUB",
            "IC" => "XICE",
            "IS" => "XIST",
            "J" => "XJSE",
            "JK" => "XIDX",
            "KA" => "XKAR",
            "KL" => "XKLS",
            "KQ" => "XKOS",
            "KS" => "XKRX",
            "KW" => "XKUW",
            "KY" => "XKYO",
            "KZ" => "XKAZ",
            "L" => "XLON",
            "LA" => "XLAT",
            "LG" => "XNSA",
            "LM" => "XLIM",
            "LS" => "XLIS",
            "LU" => "XLUX",
            "LZ" => "XLUS",
            "M" => "XMOD",
            "MA" => "XMAD",
            "MC" => "XMCE",
            "MD" => "XMDS",
            "MI" => "XMIL",
            "MM" => "MISX",
            "MO" => "XMOS",
            "MT" => "XMAL",
            "MU" => "XMUN",
            "MW" => "XCHI",
            "MX" => "XMEX",
            "MZ" => "XMAU",
            "N" => "XNYS",
            "NG" => "XNGO",
            "NM" => "XNAM",
            "NR" => "XNAI",
            "NS" => "XNSE",
            "NZ" => "XNZE",
            "O" => "XNAS",
            "OL" => "XOSL",
            "OM" => "XMUS",
            "OS" => "XOSE",
            "P" => "ARCX",
            "PA" => "XPAR",
            "PE" => "XPET",
            "PFT" => "PFTS",
            "PH" => "XPHL",
            "PNK" => "PINX",
            "PR" => "XPRA",
            "PS" => "XPHS",
            "Q" => "XJAS",
            "QA" => "DSMD",
            "RI" => "XRIS",
            "RQ" => "XRAS",
            "RTS" => "RTSX",
            "S" => "XSWX",
            "SA" => "BVMF",
            "SE" => "XSAU",
            "SG" => "XSTU",
            "SI" => "XSES",
            "SN" => "XSGO",
            "SP" => "XSAP",
            "SS" => "XSHG",
            "ST" => "XSTO",
            "SZ" => "XSHE",
            "T" => "XTKS",
            "TA" => "XTAE",
            "TL" => "XTAL",
            "TN" => "XTUN",
            "TO" => "XTSE",
            "TW" => "XTAI",
            "TWO" => "ROCO",
            "V" => "XTSX",
            "VA" => "XVAL",
            "VI" => "XWBO",
            "VL" => "XLIT",
            "VX" => "XVTX",
            "W" => "XCBO",
            "X" => "XPHO",
            "ZI" => "XZIM",
            "b" => "XBRD",
            "d" => "XEUR",
            "p" => "XMON",
            "2" => "XCME",
            "3" => "XLIF",
            "4" => "XPOS",
            "8" => "ARCO",
            "12" => "XNYM",
            "16" => "XMRV",
            "0" | "5" | "9" | "10" | "11" | "13" | "14" | "15" | "17" | "EB" | "IN" | "K"
            | "LN" | "ML" | "NW" | "OB" | "OD" | "OJ" | "SBI" | "SO" | "SU" | "TH" | "TP" | "Z" => {
                return Err(crate::Error::InvalidDataType {
                    kind: "mic",
                    reason: smol_str::format_smolstr!(
                        "Reuters exchange mnemonic {code:?} names no single current MIC"
                    ),
                });
            }
            _ => {
                return Err(crate::Error::InvalidDataType {
                    kind: "mic",
                    reason: smol_str::format_smolstr!(
                        "expected a Reuters exchange mnemonic, got {code:?}"
                    ),
                });
            }
        };
        Self::new(mic)
    }

    /// The market `text` names: an ISO 10383 MIC as it is, else the one a
    /// Reuters exchange mnemonic resolves to - the two readings FIX gives
    /// its market fields, which no spelling satisfies both of.
    pub(crate) fn from_market(text: &str) -> Option<Self> {
        if Self::is_iso(text) {
            Self::new(text).ok()
        } else {
            Self::from_reuters_exchange_code(text).ok()
        }
    }

    /// The better of two markets: this one, unless it is `XXXX`.
    fn merged(self, other: &Self) -> Self {
        if self.as_str() == Self::NONE {
            other.clone()
        } else {
            self
        }
    }
}

code_value!(MicCode, MicCode, MIC_WIDTH, merge = MicCode::merged);

/// The Arrow extension name of the market identifier code.
pub(crate) const MIC_EXTENSION_NAME: &str = "yggdryl.mic";

/// The most bytes ISO 10383's market identifier code may be.
pub(crate) const MIC_WIDTH: usize = 4;

impl DataType {
    /// Creates ISO 10383's four-character market identifier code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::mic(), DataType::MicCode);
    /// assert_eq!(DataType::mic().to_string(), "mic");
    /// assert_eq!(DataType::mic().code_width(), Some(4));
    /// ```
    #[must_use]
    pub const fn mic() -> Self {
        Self::MicCode
    }
}

// /// A MIC-typed field: ISO 10383's market identifier.
define_field_types!(MicCodeType, MicCode);
