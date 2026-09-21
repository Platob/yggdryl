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
