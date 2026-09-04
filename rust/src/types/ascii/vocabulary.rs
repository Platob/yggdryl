//! The prebuilt ASCII vocabularies: the codes a common column starts from.
//!
//! A registered-code column carries values from a published registry, and
//! most of a stream is the handful of codes that registry actually assigns.
//! These are those listings, one constant per registry: ISO 4217 currencies,
//! ISO 3166-1 alpha-2 countries, and the ISO 10383 market identifier codes of
//! the venues those trades reach. Each is sorted, so a reviewer can diff it
//! and a repeat is visible. The MICs are a common set rather than the whole
//! ISO 10383 registry, which is thousands of segment codes: a vocabulary
//! holding all of them costs every column the whole registry and buys nothing
//! a declaration does not already give.
//!
//! [`AsciiEnum::from_logical_name`] builds one as the enum a field declares,
//! so the members a schema carries under `field:enum` come from one listing
//! rather than from a copy per language. Every value fits the width its
//! registered name resolves to, so a prebuilt vocabulary never refuses its own
//! listing.

use crate::types::parser;
use crate::{AsciiEnum, DataType, Result};

impl AsciiEnum {
    /// The currently assigned ISO 4217 alphabetic currency codes, sorted.
    ///
    /// The whole active table rather than a major-currency subset: the fund
    /// codes (`CHE`, `USN`, `UYW`), the precious metals (`XAU`, `XAG`, `XPT`,
    /// `XPD`), and the two a system needs in place of a currency - `XXX` for
    /// no currency and `XTS` for a test value - are codes a real stream
    /// carries, and a subset would push them onto auto-registration.
    ///
    /// A withdrawn code is not here, successor and all: `XCG` is assigned and
    /// `ANG` is not, `SLE` and not `SLL`, `ZWG` and not `ZWL`. A stream
    /// replaying older trades still encodes them - they register on first
    /// sight, which is what the constant leaves auto-registration for.
    pub const CURRENCIES: &'static [&'static str] = &[
        "AED", "AFN", "ALL", "AMD", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT", "BHD",
        "BIF", "BMD", "BND", "BOB", "BOV", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD", "CDF",
        "CHE", "CHF", "CHW", "CLF", "CLP", "CNY", "COP", "COU", "CRC", "CUP", "CVE", "CZK", "DJF",
        "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS", "GIP",
        "GMD", "GNF", "GTQ", "GYD", "HKD", "HNL", "HTG", "HUF", "IDR", "ILS", "INR", "IQD", "IRR",
        "ISK", "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD", "KZT",
        "LAK", "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP",
        "MRU", "MUR", "MVR", "MWK", "MXN", "MXV", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR",
        "NZD", "OMR", "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD", "RUB",
        "RWF", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLE", "SOS", "SRD", "SSP", "STN",
        "SVC", "SYP", "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS", "UAH",
        "UGX", "USD", "USN", "UYI", "UYU", "UYW", "UZS", "VED", "VES", "VND", "VUV", "WST", "XAF",
        "XAG", "XAU", "XBA", "XBB", "XBC", "XBD", "XCD", "XCG", "XDR", "XOF", "XPD", "XPF", "XPT",
        "XSU", "XTS", "XUA", "XXX", "YER", "ZAR", "ZMW", "ZWG",
    ];

    /// The ISO 3166-1 alpha-2 country codes, sorted.
    ///
    /// Every currently assigned code, territories and dependencies included.
    /// The transitionally reserved codes (`AN`, `CS`, `YU`) and the
    /// user-assigned range (`AA`, `QM` through `QZ`, `XA` through `XZ`, `ZZ`)
    /// are not assigned, so a stream carrying one registers it.
    pub const COUNTRIES: &'static [&'static str] = &[
        "AD", "AE", "AF", "AG", "AI", "AL", "AM", "AO", "AQ", "AR", "AS", "AT", "AU", "AW", "AX",
        "AZ", "BA", "BB", "BD", "BE", "BF", "BG", "BH", "BI", "BJ", "BL", "BM", "BN", "BO", "BQ",
        "BR", "BS", "BT", "BV", "BW", "BY", "BZ", "CA", "CC", "CD", "CF", "CG", "CH", "CI", "CK",
        "CL", "CM", "CN", "CO", "CR", "CU", "CV", "CW", "CX", "CY", "CZ", "DE", "DJ", "DK", "DM",
        "DO", "DZ", "EC", "EE", "EG", "EH", "ER", "ES", "ET", "FI", "FJ", "FK", "FM", "FO", "FR",
        "GA", "GB", "GD", "GE", "GF", "GG", "GH", "GI", "GL", "GM", "GN", "GP", "GQ", "GR", "GS",
        "GT", "GU", "GW", "GY", "HK", "HM", "HN", "HR", "HT", "HU", "ID", "IE", "IL", "IM", "IN",
        "IO", "IQ", "IR", "IS", "IT", "JE", "JM", "JO", "JP", "KE", "KG", "KH", "KI", "KM", "KN",
        "KP", "KR", "KW", "KY", "KZ", "LA", "LB", "LC", "LI", "LK", "LR", "LS", "LT", "LU", "LV",
        "LY", "MA", "MC", "MD", "ME", "MF", "MG", "MH", "MK", "ML", "MM", "MN", "MO", "MP", "MQ",
        "MR", "MS", "MT", "MU", "MV", "MW", "MX", "MY", "MZ", "NA", "NC", "NE", "NF", "NG", "NI",
        "NL", "NO", "NP", "NR", "NU", "NZ", "OM", "PA", "PE", "PF", "PG", "PH", "PK", "PL", "PM",
        "PN", "PR", "PS", "PT", "PW", "PY", "QA", "RE", "RO", "RS", "RU", "RW", "SA", "SB", "SC",
        "SD", "SE", "SG", "SH", "SI", "SJ", "SK", "SL", "SM", "SN", "SO", "SR", "SS", "ST", "SV",
        "SX", "SY", "SZ", "TC", "TD", "TF", "TG", "TH", "TJ", "TK", "TL", "TM", "TN", "TO", "TR",
        "TT", "TV", "TW", "TZ", "UA", "UG", "UM", "US", "UY", "UZ", "VA", "VC", "VE", "VG", "VI",
        "VN", "VU", "WF", "WS", "YE", "YT", "ZA", "ZM", "ZW",
    ];

    /// The ISO 10383 market identifier codes of the common venues, sorted.
    ///
    /// The operating and segment MICs a multi-asset or commodity system
    /// actually meets - the listed exchanges, the derivatives and commodity
    /// venues, the large MTFs - plus `XOFF` for an off-exchange trade and
    /// `XXXX` for no market. It is deliberately not the whole registry, which
    /// is thousands of segment codes: a venue outside this set registers on
    /// first sight, at the cost of a code that is this dictionary's own.
    pub const MICS: &'static [&'static str] = &[
        "AQEU", "AQXE", "ARCX", "BATD", "BATE", "BATS", "BATY", "BCXE", "BMTF", "BVMF", "C2OX",
        "CCFX", "CEDX", "CEUX", "CHID", "CHIX", "DIFX", "DUMX", "EDGA", "EDGX", "EPEX", "GMNI",
        "IEXG", "IFAD", "IFEU", "IFLL", "IFSG", "IFUS", "MCRY", "MEMX", "MISX", "NDEX", "NEOE",
        "NORX", "OTCM", "RTSX", "SGMX", "TRQX", "XADS", "XAMS", "XASE", "XASX", "XATH", "XBER",
        "XBKK", "XBOM", "XBOS", "XBRU", "XBUD", "XCBF", "XCBO", "XCBT", "XCEC", "XCHI", "XCIS",
        "XCME", "XCSE", "XDCE", "XDFM", "XDUB", "XDUS", "XEEE", "XETR", "XEUR", "XFRA", "XHEL",
        "XHKG", "XICE", "XIDX", "XINE", "XIST", "XISX", "XJSE", "XKFE", "XKLS", "XKOS", "XKRX",
        "XLIS", "XLIT", "XLME", "XLON", "XMAD", "XMAT", "XMEX", "XMIL", "XMOD", "XMON", "XMUN",
        "XNAS", "XNGO", "XNSE", "XNYM", "XNYS", "XOFF", "XOSE", "XOSL", "XPAR", "XPHL", "XPRA",
        "XRIS", "XSAU", "XSES", "XSFE", "XSGE", "XSHE", "XSHG", "XSIM", "XSTO", "XSTU", "XSWX",
        "XTAE", "XTAI", "XTAL", "XTKS", "XTKT", "XTSE", "XTSX", "XVTX", "XWAR", "XWBO", "XXXX",
        "XZCE",
    ];

    /// The prebuilt vocabularies, by the logical name that spells them.
    ///
    /// `exchange` and `mic` name one list because they name one thing: FIX
    /// calls the ISO 10383 code an `Exchange`, and ISO calls it a MIC.
    pub const PREBUILT: &'static [(&'static str, &'static [&'static str])] = &[
        ("currency", Self::CURRENCIES),
        ("country", Self::COUNTRIES),
        ("mic", Self::MICS),
        ("exchange", Self::MICS),
    ];

    /// Creates the enum a registered logical name prebuilds.
    ///
    /// The enum is named for the registration and holds one member per value
    /// of its constant, each named by [`AsciiEnum::member_name`] - which, for
    /// an ISO code, is the code itself. A registered name with no constant -
    /// `language`, `monthyear`, `tenor` - answers an enum of no members,
    /// because a listing is what it has to offer and it has none.
    ///
    /// ```
    /// use yggdryl::{AsciiEnum, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let venues = AsciiEnum::from_logical_name("mic")?;
    /// assert_eq!(venues.len(), AsciiEnum::MICS.len());
    /// assert_eq!(venues.get("XCME"), Some("XCME"));
    ///
    /// // A member's code is the value's own bytes under the resolved width.
    /// assert_eq!(
    ///     venues.into_members(&DataType::Mic)?[0].1,
    ///     DataType::Mic.ascii_packed(AsciiEnum::MICS[0].as_bytes())?
    /// );
    ///
    /// // `exchange` is FIX's name for the same list, under the same type.
    /// assert_eq!(
    ///     AsciiEnum::from_logical_name("Exchange")?.len(),
    ///     AsciiEnum::from_logical_name("mic")?.len()
    /// );
    ///
    /// // A name with no listing answers an enum of no members.
    /// assert!(AsciiEnum::from_logical_name("tenor")?.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the vocabulary when `name` is not a registered
    /// logical name.
    pub fn from_logical_name(name: &str) -> Result<Self> {
        // The name has to resolve, so a caller cannot prebuild a vocabulary
        // for a registration that does not exist.
        DataType::from_logical_name(name)?;
        Self::from_members(
            parser::normalized(name.trim()),
            Self::prebuilt_values(name)
                .iter()
                .map(|value| (Self::member_name(value), *value)),
        )
    }

    /// The constant a logical name prebuilds, empty when it has none.
    ///
    /// The name folds the way [`DataType::from_logical_name`] folds it, so one
    /// spelling reaches one list.
    pub fn prebuilt_values(name: &str) -> &'static [&'static str] {
        let folded = parser::normalized(name.trim());
        Self::PREBUILT
            .iter()
            .find(|(registered, _)| *registered == folded)
            .map_or(&[], |(_, values)| *values)
    }
}
