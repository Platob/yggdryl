//! The legal tender of every country, generated from ISO 4217 list one.
//!
//! Regenerate with `python scripts/generate_country_currency.py`; the same
//! script run with `--check` fetches the list again and fails when this file
//! is stale. Nothing here is edited by hand: list one names a country by its
//! English short name, and the script reads every name through its own pinned
//! table of ISO 3166-1 alpha-2 codes, failing on a name it does not hold.
//!
//! Source: <https://www.six-group.com/dam/download/financial-information/data-center/iso-currrency/lists/list-one.xml>,
//! published 2026-09-17.
//!
//! One currency per country. A fund code is left out, a country list one
//! gives two tenders takes the one the script's override table names, and a
//! country list one gives no currency - Antarctica, Palestine - has no row,
//! nor has an entry that names no country.

/// `(alpha-2, currency)`, sorted by the alpha-2 code: what
/// [`Country::currency`](crate::Country::currency) binary-searches.
#[rustfmt::skip]
pub(crate) static COUNTRY_CURRENCY: [(&str, &str); 246] = [
    ("AD", "EUR"), ("AE", "AED"), ("AF", "AFN"), ("AG", "XCD"), ("AI", "XCD"), ("AL", "ALL"), ("AM", "AMD"), ("AO", "AOA"),
    ("AR", "ARS"), ("AS", "USD"), ("AT", "EUR"), ("AU", "AUD"), ("AW", "AWG"), ("AX", "EUR"), ("AZ", "AZN"), ("BA", "BAM"),
    ("BB", "BBD"), ("BD", "BDT"), ("BE", "EUR"), ("BF", "XOF"), ("BG", "EUR"), ("BH", "BHD"), ("BI", "BIF"), ("BJ", "XOF"),
    ("BL", "EUR"), ("BM", "BMD"), ("BN", "BND"), ("BO", "BOB"), ("BQ", "USD"), ("BR", "BRL"), ("BS", "BSD"), ("BT", "BTN"),
    ("BV", "NOK"), ("BW", "BWP"), ("BY", "BYN"), ("BZ", "BZD"), ("CA", "CAD"), ("CC", "AUD"), ("CD", "CDF"), ("CF", "XAF"),
    ("CG", "XAF"), ("CH", "CHF"), ("CI", "XOF"), ("CK", "NZD"), ("CL", "CLP"), ("CM", "XAF"), ("CN", "CNY"), ("CO", "COP"),
    ("CR", "CRC"), ("CU", "CUP"), ("CV", "CVE"), ("CW", "XCG"), ("CX", "AUD"), ("CY", "EUR"), ("CZ", "CZK"), ("DE", "EUR"),
    ("DJ", "DJF"), ("DK", "DKK"), ("DM", "XCD"), ("DO", "DOP"), ("DZ", "DZD"), ("EC", "USD"), ("EE", "EUR"), ("EG", "EGP"),
    ("EH", "MAD"), ("ER", "ERN"), ("ES", "EUR"), ("ET", "ETB"), ("FI", "EUR"), ("FJ", "FJD"), ("FK", "FKP"), ("FM", "USD"),
    ("FO", "DKK"), ("FR", "EUR"), ("GA", "XAF"), ("GB", "GBP"), ("GD", "XCD"), ("GE", "GEL"), ("GF", "EUR"), ("GG", "GBP"),
    ("GH", "GHS"), ("GI", "GIP"), ("GL", "DKK"), ("GM", "GMD"), ("GN", "GNF"), ("GP", "EUR"), ("GQ", "XAF"), ("GR", "EUR"),
    ("GT", "GTQ"), ("GU", "USD"), ("GW", "XOF"), ("GY", "GYD"), ("HK", "HKD"), ("HM", "AUD"), ("HN", "HNL"), ("HR", "EUR"),
    ("HT", "HTG"), ("HU", "HUF"), ("ID", "IDR"), ("IE", "EUR"), ("IL", "ILS"), ("IM", "GBP"), ("IN", "INR"), ("IO", "USD"),
    ("IQ", "IQD"), ("IR", "IRR"), ("IS", "ISK"), ("IT", "EUR"), ("JE", "GBP"), ("JM", "JMD"), ("JO", "JOD"), ("JP", "JPY"),
    ("KE", "KES"), ("KG", "KGS"), ("KH", "KHR"), ("KI", "AUD"), ("KM", "KMF"), ("KN", "XCD"), ("KP", "KPW"), ("KR", "KRW"),
    ("KW", "KWD"), ("KY", "KYD"), ("KZ", "KZT"), ("LA", "LAK"), ("LB", "LBP"), ("LC", "XCD"), ("LI", "CHF"), ("LK", "LKR"),
    ("LR", "LRD"), ("LS", "LSL"), ("LT", "EUR"), ("LU", "EUR"), ("LV", "EUR"), ("LY", "LYD"), ("MA", "MAD"), ("MC", "EUR"),
    ("MD", "MDL"), ("ME", "EUR"), ("MF", "EUR"), ("MG", "MGA"), ("MH", "USD"), ("MK", "MKD"), ("ML", "XOF"), ("MM", "MMK"),
    ("MN", "MNT"), ("MO", "MOP"), ("MP", "USD"), ("MQ", "EUR"), ("MR", "MRU"), ("MS", "XCD"), ("MT", "EUR"), ("MU", "MUR"),
    ("MV", "MVR"), ("MW", "MWK"), ("MX", "MXN"), ("MY", "MYR"), ("MZ", "MZN"), ("NA", "NAD"), ("NC", "XPF"), ("NE", "XOF"),
    ("NF", "AUD"), ("NG", "NGN"), ("NI", "NIO"), ("NL", "EUR"), ("NO", "NOK"), ("NP", "NPR"), ("NR", "AUD"), ("NU", "NZD"),
    ("NZ", "NZD"), ("OM", "OMR"), ("PA", "PAB"), ("PE", "PEN"), ("PF", "XPF"), ("PG", "PGK"), ("PH", "PHP"), ("PK", "PKR"),
    ("PL", "PLN"), ("PM", "EUR"), ("PN", "NZD"), ("PR", "USD"), ("PT", "EUR"), ("PW", "USD"), ("PY", "PYG"), ("QA", "QAR"),
    ("RE", "EUR"), ("RO", "RON"), ("RS", "RSD"), ("RU", "RUB"), ("RW", "RWF"), ("SA", "SAR"), ("SB", "SBD"), ("SC", "SCR"),
    ("SD", "SDG"), ("SE", "SEK"), ("SG", "SGD"), ("SH", "SHP"), ("SI", "EUR"), ("SJ", "NOK"), ("SK", "EUR"), ("SL", "SLE"),
    ("SM", "EUR"), ("SN", "XOF"), ("SO", "SOS"), ("SR", "SRD"), ("SS", "SSP"), ("ST", "STN"), ("SV", "USD"), ("SX", "XCG"),
    ("SY", "SYP"), ("SZ", "SZL"), ("TC", "USD"), ("TD", "XAF"), ("TF", "EUR"), ("TG", "XOF"), ("TH", "THB"), ("TJ", "TJS"),
    ("TK", "NZD"), ("TL", "USD"), ("TM", "TMT"), ("TN", "TND"), ("TO", "TOP"), ("TR", "TRY"), ("TT", "TTD"), ("TV", "AUD"),
    ("TW", "TWD"), ("TZ", "TZS"), ("UA", "UAH"), ("UG", "UGX"), ("UM", "USD"), ("US", "USD"), ("UY", "UYU"), ("UZ", "UZS"),
    ("VA", "EUR"), ("VC", "XCD"), ("VE", "VES"), ("VG", "USD"), ("VI", "USD"), ("VN", "VND"), ("VU", "VUV"), ("WF", "XPF"),
    ("WS", "WST"), ("YE", "YER"), ("YT", "EUR"), ("ZA", "ZAR"), ("ZM", "ZMW"), ("ZW", "ZWG"),
];
