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

    /// FIX's `SideCodeSet`, the union across every version, sorted.
    ///
    /// A datatype is parameter-free and a listing is a constant, so every
    /// reader answers the same members and one datatype serves every version:
    /// a 4.2 message and a newest one agree about what `1` means. Per-member
    /// pedigree stays in the field's own `fix:codes` document, because that is
    /// where a version can be asked about.
    ///
    /// The listing is a vocabulary, never a whitelist: a venue's own side is
    /// stored, not refused.
    pub const SIDES: &'static [&'static str] = &[
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D", "E", "F", "G", "H",
    ];

    /// FIX's `MsgTypeCodeSet`, the union across every version, sorted.
    ///
    /// Case-bearing, and that is load-bearing: `A` is Logon and `a` is
    /// QuoteStatusRequest, `Q` is DontKnowTrade and `q` is
    /// OrderMassCancelRequest, `S` is Quote and `s` is NewOrderCross. The
    /// crate's one fold serves names, keys and code spellings and must never
    /// touch one of these values.
    ///
    /// That is also why this listing is **not** in [`Self::PREBUILT`], where
    /// every other vocabulary sits. A `field:enum` document maps a member
    /// *name* to a value, and [`AsciiEnum::member_name`] upper-cases, so `A`
    /// and `a` would name one member and twenty-three of these values would
    /// be lost. Inventing a distinguishing spelling would be inventing a name
    /// the specification does not have, so the constant stays the datatype's
    /// vocabulary and `msgtype` answers no prebuilt enum.
    pub const MSGTYPES: &'static [&'static str] = &[
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "AA", "AB", "AC", "AD", "AE", "AF",
        "AG", "AH", "AI", "AJ", "AK", "AL", "AM", "AN", "AO", "AP", "AQ", "AR", "AS", "AT", "AU",
        "AV", "AW", "AX", "AY", "AZ", "B", "BA", "BB", "BC", "BD", "BE", "BF", "BG", "BH", "BI",
        "BJ", "BK", "BL", "BM", "BN", "BO", "BP", "BQ", "BR", "BS", "BT", "BU", "BV", "BW", "BX",
        "BY", "BZ", "C", "CA", "CB", "CC", "CD", "CE", "CF", "CG", "CH", "CI", "CJ", "CK", "CL",
        "CM", "CN", "CO", "CP", "CQ", "CR", "CS", "CT", "CU", "CV", "CW", "CX", "CY", "CZ", "D",
        "DA", "DB", "DC", "DD", "DE", "DF", "DG", "DH", "DI", "DJ", "DK", "DL", "DM", "DN", "DO",
        "E", "F", "G", "H", "J", "K", "L", "M", "N", "P", "Q", "R", "S", "T", "V", "W", "X", "Y",
        "Z", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q",
        "r", "s", "t", "u", "v", "w", "x", "y", "z",
    ];

    /// Which way a captured line moved.
    ///
    /// Two members and no third. A row whose line does not say which way it
    /// moved has no direction, and the crate already spells "no answer" one
    /// way: a member meaning *unknown* would be a second spelling of null,
    /// two things to check at every read and the one a caller forgets.
    pub const DIRECTIONS: &'static [&'static str] = &["RECV", "SENT"];

    /// Every state one thing can be in, ordered from first to last.
    ///
    /// One vocabulary over two worlds. FIX names an order's state twice -
    /// `OrdStatus` says where the order stands and `ExecType` says what the
    /// report is - and a scheduler names a job's state in ordinary English.
    /// They are the same shape: a thing is created, it works, and it ends one
    /// of three ways. A capture and the pipeline that reads it should not need
    /// two vocabularies and a join to answer "what happened".
    ///
    /// # The first two bytes are the rank
    ///
    /// A value is two decimal digits of rank then a name of up to eight
    /// bytes, and the rank is what makes the *stored bytes* sort from first
    /// state to terminal. That matters because most things that sort a column
    /// are not this crate: a Parquet row group's min and max, an external
    /// sort, a `ORDER BY` in whatever reads the file. Ordering by name would
    /// put `CANCELED` before `NEW`; ordering by these bytes puts every live
    /// state before every ended one, and that ordering survives every format
    /// the value crosses.
    ///
    /// Ranks run `00`-`99`. Every shipped state sits on a round rank, and the
    /// digits between two of them - `01`-`09`, `11`-`19`, and so on - are the
    /// placeholders a state that belongs between two ranks takes, so adding
    /// one moves nothing already stored:
    ///
    /// | rank | meaning |
    /// | --- | --- |
    /// | `00` | stated, but not a state anything reached |
    /// | `10` | asked for, not yet acknowledged |
    /// | `20` | acknowledged, not yet working |
    /// | `30` | working |
    /// | `40` | working, and something has happened |
    /// | `50` | halted, and able to resume |
    /// | `60` | a change is outstanding |
    /// | `70` | changed, and the new thing carries on |
    /// | `80` | ended, having done what was asked |
    /// | `90` | ended, because someone stopped it |
    /// | `95` | ended, because it could not be done |
    ///
    /// The three endings are ranked apart deliberately: "did it finish" and
    /// "did it work" are different questions, and a single terminal rank would
    /// answer neither without reading the name. Each ending owns a band -
    /// `80`-`89` done, `90`-`94` cancelled, `95`-`99` failed - and
    /// [`State::is_done`](crate::types::State::is_done),
    /// [`State::is_cancelled`](crate::types::State::is_cancelled) and
    /// [`State::is_failed`](crate::types::State::is_failed) read the band, so
    /// a placeholder inside one answers as its ending does.
    pub const STATES: &'static [&'static str] = &[
        "00UNKNOWN",
        "10PENDING",
        "10PENDNEW",
        "10QUEUED",
        "20ACCEPTED",
        "20NEW",
        "20STARTING",
        "20SUBMITTD",
        "30RUNNING",
        "30STATUS",
        "30TRIGGER",
        "40INPROGR",
        "40PARTFILL",
        "40TRADE",
        "40TRDCORR",
        "40TRDCXL",
        "40TRDHOLD",
        "50PAUSED",
        "50STOPPED",
        "50SUSPEND",
        "60PENDCXL",
        "60PENDRPL",
        "70REPLACED",
        "80CALCULAT",
        "80COMPLETE",
        "80DONEDAY",
        "80FILLED",
        "80SUCCESS",
        "80TRDRELS",
        "90CANCELED",
        "95EXPIRED",
        "95FAILED",
        "95REJECTED",
        "95TIMEOUT",
    ];

    /// FIX's `TimeInForceCodeSet`, the union across every version, sorted.
    ///
    /// The wire values rather than the names, exactly as [`Self::SIDES`] is:
    /// a code set's value is what a message carries, and the name is what a
    /// dictionary translates it to.
    pub const TIMESINFORCE: &'static [&'static str] = &[
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D",
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
        ("side", Self::SIDES),
        ("msgdirection", Self::DIRECTIONS),
        ("state", Self::STATES),
        ("timeinforce", Self::TIMESINFORCE),
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
