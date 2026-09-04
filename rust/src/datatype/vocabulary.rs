//! The prebuilt ASCII vocabularies: the codes a common column starts from.
//!
//! An [`AsciiDictionary`] auto-registers, so an unseen value always takes the
//! next code. That is the right default and a poor wire contract: two
//! processes that met their values in different orders agree about no code. A
//! prebuilt vocabulary fixes the head of the code space to a constant, so a
//! code below that constant's length names the same value in every process
//! reading this version, and auto-registration continues past it for whatever
//! the constant does not hold.
//!
//! The three constants are the code sets a trading column actually carries:
//! ISO 4217 currencies, ISO 3166-1 alpha-2 countries, and the ISO 10383
//! market identifier codes of the venues those trades reach. Each is sorted,
//! so a reviewer can diff it and a repeat is visible. The MICs are a common
//! set rather than the whole ISO 10383 registry, which is thousands of
//! segment codes: a vocabulary holding all of them costs every column the
//! whole registry and buys nothing auto-registration does not already give.
//!
//! Every value fits its width, so a prebuilt vocabulary never refuses its own
//! seed. A currency and a MIC are `ascii32`, and so is a country: the
//! narrowest width is 4 bytes and a two-byte code stores `FR\0\0`.

use crate::{AsciiDictionary, DataType, Result};

impl AsciiDictionary {
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

    /// Creates the vocabulary a registered logical name prebuilds.
    ///
    /// The width is whatever [`DataType::from_logical_name`] resolves and the
    /// values are this name's constant, registered in its order. A registered
    /// name over an ASCII width with no constant - `language`, `monthyear`,
    /// `tenor` - answers the empty auto-registering vocabulary [`Self::new`]
    /// builds, because the width is what it has to offer.
    ///
    /// ```
    /// use yggdryl::{AsciiDictionary, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let venues = AsciiDictionary::from_logical_name("mic")?;
    /// assert_eq!(venues.values_dtype(), &DataType::Ascii32);
    /// assert_eq!(venues.len(), AsciiDictionary::MICS.len());
    ///
    /// // A prebuilt code is a constant: it is the position in the constant.
    /// let xcme = venues.get_code("XCME").expect("a prebuilt venue");
    /// assert_eq!(venues.get(xcme), Some("XCME"));
    /// assert_eq!(AsciiDictionary::MICS[xcme as usize], "XCME");
    ///
    /// // `exchange` is FIX's name for the same list.
    /// assert_eq!(
    ///     AsciiDictionary::from_logical_name("Exchange")?,
    ///     AsciiDictionary::from_logical_name("mic")?
    /// );
    ///
    /// // Auto-registration continues past the constant: `ZZ` is ISO 3166's
    /// // user-assigned range, so no assigned country holds it.
    /// let mut countries = AsciiDictionary::from_logical_name("Country")?;
    /// assert!(countries.get_code("FR").is_some());
    /// assert_eq!(countries.push("ZZ")?, AsciiDictionary::COUNTRIES.len() as i64);
    ///
    /// // A name that resolves to something else is refused by width.
    /// let refused = AsciiDictionary::from_logical_name("price").unwrap_err().to_string();
    /// assert!(refused.contains("decimal64(18,8)"), "{refused}");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the vocabulary when `name` is not a registered
    /// logical name, and one naming the datatype when that name resolves to
    /// anything but an ASCII width.
    pub fn from_logical_name(name: &str) -> Result<Self> {
        Self::from_values(
            DataType::from_logical_name(name)?,
            Self::prebuilt_values(name),
        )
    }

    /// The constant a logical name prebuilds, empty when it has none.
    ///
    /// The name folds the way [`DataType::from_logical_name`] folds it, so one
    /// spelling reaches one list.
    pub fn prebuilt_values(name: &str) -> &'static [&'static str] {
        let folded = super::parser::normalized(name.trim());
        Self::PREBUILT
            .iter()
            .find(|(registered, _)| *registered == folded)
            .map_or(&[], |(_, values)| *values)
    }
}
