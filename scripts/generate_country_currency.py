#!/usr/bin/env python3
"""Generate ``rust/src/country/tables.rs`` from ISO 4217 list one.

A country's legal tender is one fact per country, and ISO 4217's list one -
the currency and fund code list SIX Financial Information publishes as the
standard's maintenance agency - states every one of them. Transcribing two
hundred and fifty of them by hand is how a wrong currency reaches a released
crate, so this driver reads the list and writes the Rust source
``Country::currency`` binary-searches.

List one names a country by its English short name, never by its ISO 3166-1
code, so ``NAMES`` pins every spelling the list uses to the alpha-2 code it
names - or to none, for the entries that name no country: the European Union,
the IMF, the regional units and the ``ZZ`` rows. A name the table does not
hold fails the run rather than being skipped: a new spelling is a decision.

One currency per country:

* a fund code (``<CcyNm IsFund="true">``) is left out - the WIR units, the US
  next-day dollar, the unidades de fomento;
* an entry with no ``Ccy`` - Antarctica, Palestine - gives its country none;
* a country list one gives two or more tenders takes the one ``OVERRIDES``
  names, and such a country ``OVERRIDES`` does not name fails the run, as does
  an override naming a currency list one no longer gives that country.

Run with ``--check`` to fetch the list again and fail when the committed file
is stale instead of rewriting it; ``--source`` reads a saved copy - a path or
a URL - so the script also runs offline. The file states the date list one
says it was published, never the day it was fetched, so ``--check`` passes for
as long as the list is the same release.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
import unicodedata
import urllib.request
import xml.etree.ElementTree as ElementTree

# The official list one, as SIX publishes it for the ISO 4217 maintenance
# agency (the doubled `rrr` in `iso-currrency` is the agency's own path).
SOURCE = (
    "https://www.six-group.com/dam/download/financial-information/"
    "data-center/iso-currrency/lists/list-one.xml"
)

TARGET = pathlib.Path(__file__).resolve().parent.parent / "rust" / "src" / "country" / "tables.rs"

USER_AGENT = "yggdryl-generate-country-currency"

# An override that takes the currency list one states first for the country.
FIRST = None

# The tender of each country list one gives two or more, and of the countries
# whose single tender is a decision worth stating. Each must name a currency
# list one gives that country, or the run fails as stale.
OVERRIDES: dict[str, str | None] = {
    # Bhutan: the ngultrum, its own; list one also gives the Indian rupee.
    "BT": "BTN",
    # Cuba: the peso; the convertible peso (`CUC`) left list one in 2021.
    "CU": "CUP",
    # Ecuador: dollarized since 2000; the sucre is gone.
    "EC": "USD",
    # Haiti: the gourde, its own; list one also gives the US dollar.
    "HT": "HTG",
    # Lesotho: the loti, its own; list one also gives the rand.
    "LS": "LSL",
    # Namibia: the Namibia dollar, its own; list one also gives the rand.
    "NA": "NAD",
    # Panama: the balboa, its own; list one also gives the US dollar.
    "PA": "PAB",
    # El Salvador: dollarized since 2001; list one still gives the colon.
    "SV": "USD",
    # Timor-Leste: the US dollar, its legal tender.
    "TL": "USD",
    # The United States: the dollar; the next-day dollar (`USN`) is a fund.
    "US": "USD",
    # Uruguay: the peso; `UYI` and `UYW` are funds.
    "UY": "UYU",
    # Venezuela: the sovereign bolivar (`VES`); list one also gives the
    # digital bolivar (`VED`), its 2021 redenomination's second code.
    "VE": "VES",
    # Zimbabwe: the currency list one states first, whatever it is called
    # in this release (`ZWG`, the gold-backed Zimbabwe Gold, since 2024).
    "ZW": FIRST,
}

# Every country spelling list one uses, to its ISO 3166-1 alpha-2 code, or to
# none where the entry names no country. Matched after Unicode NFC
# normalisation and with surrounding whitespace (a trailing no-break space
# included) removed.
NAMES: dict[str, str | None] = {
    "AFGHANISTAN": "AF",
    "ÅLAND ISLANDS": "AX",
    "ALBANIA": "AL",
    "ALGERIA": "DZ",
    "AMERICAN SAMOA": "AS",
    "ANDORRA": "AD",
    "ANGOLA": "AO",
    "ANGUILLA": "AI",
    "ANTARCTICA": "AQ",
    "ANTIGUA AND BARBUDA": "AG",
    "ARAB MONETARY FUND": None,
    "ARGENTINA": "AR",
    "ARMENIA": "AM",
    "ARUBA": "AW",
    "AUSTRALIA": "AU",
    "AUSTRIA": "AT",
    "AZERBAIJAN": "AZ",
    "BAHAMAS (THE)": "BS",
    "BAHRAIN": "BH",
    "BANGLADESH": "BD",
    "BARBADOS": "BB",
    "BELARUS": "BY",
    "BELGIUM": "BE",
    "BELIZE": "BZ",
    "BENIN": "BJ",
    "BERMUDA": "BM",
    "BHUTAN": "BT",
    "BOLIVIA (PLURINATIONAL STATE OF)": "BO",
    "BONAIRE, SINT EUSTATIUS AND SABA": "BQ",
    "BOSNIA AND HERZEGOVINA": "BA",
    "BOTSWANA": "BW",
    "BOUVET ISLAND": "BV",
    "BRAZIL": "BR",
    "BRITISH INDIAN OCEAN TERRITORY (THE)": "IO",
    "BRUNEI DARUSSALAM": "BN",
    "BULGARIA": "BG",
    "BURKINA FASO": "BF",
    "BURUNDI": "BI",
    "CABO VERDE": "CV",
    "CAMBODIA": "KH",
    "CAMEROON": "CM",
    "CANADA": "CA",
    "CAYMAN ISLANDS (THE)": "KY",
    "CENTRAL AFRICAN REPUBLIC (THE)": "CF",
    "CHAD": "TD",
    "CHILE": "CL",
    "CHINA": "CN",
    "CHRISTMAS ISLAND": "CX",
    "COCOS (KEELING) ISLANDS (THE)": "CC",
    "COLOMBIA": "CO",
    "COMOROS (THE)": "KM",
    "CONGO (THE DEMOCRATIC REPUBLIC OF THE)": "CD",
    "CONGO (THE)": "CG",
    "COOK ISLANDS (THE)": "CK",
    "COSTA RICA": "CR",
    "CÔTE D'IVOIRE": "CI",
    "CROATIA": "HR",
    "CUBA": "CU",
    "CURAÇAO": "CW",
    "CYPRUS": "CY",
    "CZECHIA": "CZ",
    "DENMARK": "DK",
    "DJIBOUTI": "DJ",
    "DOMINICA": "DM",
    "DOMINICAN REPUBLIC (THE)": "DO",
    "ECUADOR": "EC",
    "EGYPT": "EG",
    "EL SALVADOR": "SV",
    "EQUATORIAL GUINEA": "GQ",
    "ERITREA": "ER",
    "ESTONIA": "EE",
    "ESWATINI": "SZ",
    "ETHIOPIA": "ET",
    # `EU` is exceptionally reserved in ISO 3166-1, never assigned.
    "EUROPEAN UNION": None,
    "FALKLAND ISLANDS (THE) [MALVINAS]": "FK",
    "FAROE ISLANDS (THE)": "FO",
    "FIJI": "FJ",
    "FINLAND": "FI",
    "FRANCE": "FR",
    "FRENCH GUIANA": "GF",
    "FRENCH POLYNESIA": "PF",
    "FRENCH SOUTHERN TERRITORIES (THE)": "TF",
    "GABON": "GA",
    "GAMBIA (THE)": "GM",
    "GEORGIA": "GE",
    "GERMANY": "DE",
    "GHANA": "GH",
    "GIBRALTAR": "GI",
    "GREECE": "GR",
    "GREENLAND": "GL",
    "GRENADA": "GD",
    "GUADELOUPE": "GP",
    "GUAM": "GU",
    "GUATEMALA": "GT",
    "GUERNSEY": "GG",
    "GUINEA": "GN",
    "GUINEA-BISSAU": "GW",
    "GUYANA": "GY",
    "HAITI": "HT",
    "HEARD ISLAND AND McDONALD ISLANDS": "HM",
    "HOLY SEE (THE)": "VA",
    "HONDURAS": "HN",
    "HONG KONG": "HK",
    "HUNGARY": "HU",
    "ICELAND": "IS",
    "INDIA": "IN",
    "INDONESIA": "ID",
    "INTERNATIONAL MONETARY FUND (IMF)": None,
    "IRAN (ISLAMIC REPUBLIC OF)": "IR",
    "IRAQ": "IQ",
    "IRELAND": "IE",
    "ISLE OF MAN": "IM",
    "ISRAEL": "IL",
    "ITALY": "IT",
    "JAMAICA": "JM",
    "JAPAN": "JP",
    "JERSEY": "JE",
    "JORDAN": "JO",
    "KAZAKHSTAN": "KZ",
    "KENYA": "KE",
    "KIRIBATI": "KI",
    "KOREA (THE DEMOCRATIC PEOPLE’S REPUBLIC OF)": "KP",
    "KOREA (THE REPUBLIC OF)": "KR",
    "KUWAIT": "KW",
    "KYRGYZSTAN": "KG",
    "LAO PEOPLE’S DEMOCRATIC REPUBLIC (THE)": "LA",
    "LATVIA": "LV",
    "LEBANON": "LB",
    "LESOTHO": "LS",
    "LIBERIA": "LR",
    "LIBYA": "LY",
    "LIECHTENSTEIN": "LI",
    "LITHUANIA": "LT",
    "LUXEMBOURG": "LU",
    "MACAO": "MO",
    "MADAGASCAR": "MG",
    "MALAWI": "MW",
    "MALAYSIA": "MY",
    "MALDIVES": "MV",
    "MALI": "ML",
    "MALTA": "MT",
    "MARSHALL ISLANDS (THE)": "MH",
    "MARTINIQUE": "MQ",
    "MAURITANIA": "MR",
    "MAURITIUS": "MU",
    "MAYOTTE": "YT",
    "MEMBER COUNTRIES OF THE AFRICAN DEVELOPMENT BANK GROUP": None,
    "MEXICO": "MX",
    "MICRONESIA (FEDERATED STATES OF)": "FM",
    "MOLDOVA (THE REPUBLIC OF)": "MD",
    "MONACO": "MC",
    "MONGOLIA": "MN",
    "MONTENEGRO": "ME",
    "MONTSERRAT": "MS",
    "MOROCCO": "MA",
    "MOZAMBIQUE": "MZ",
    "MYANMAR": "MM",
    "NAMIBIA": "NA",
    "NAURU": "NR",
    "NEPAL": "NP",
    "NETHERLANDS (THE)": "NL",
    "NEW CALEDONIA": "NC",
    "NEW ZEALAND": "NZ",
    "NICARAGUA": "NI",
    "NIGER (THE)": "NE",
    "NIGERIA": "NG",
    "NIUE": "NU",
    "NORFOLK ISLAND": "NF",
    "NORTH MACEDONIA": "MK",
    "NORTHERN MARIANA ISLANDS (THE)": "MP",
    "NORWAY": "NO",
    "OMAN": "OM",
    "PAKISTAN": "PK",
    "PALAU": "PW",
    "PALESTINE, STATE OF": "PS",
    "PANAMA": "PA",
    "PAPUA NEW GUINEA": "PG",
    "PARAGUAY": "PY",
    "PERU": "PE",
    "PHILIPPINES (THE)": "PH",
    "PITCAIRN": "PN",
    "POLAND": "PL",
    "PORTUGAL": "PT",
    "PUERTO RICO": "PR",
    "QATAR": "QA",
    "RÉUNION": "RE",
    "ROMANIA": "RO",
    "RUSSIAN FEDERATION (THE)": "RU",
    "RWANDA": "RW",
    "SAINT BARTHÉLEMY": "BL",
    "SAINT HELENA, ASCENSION AND TRISTAN DA CUNHA": "SH",
    "SAINT KITTS AND NEVIS": "KN",
    "SAINT LUCIA": "LC",
    "SAINT MARTIN (FRENCH PART)": "MF",
    "SAINT PIERRE AND MIQUELON": "PM",
    "SAINT VINCENT AND THE GRENADINES": "VC",
    "SAMOA": "WS",
    "SAN MARINO": "SM",
    "SAO TOME AND PRINCIPE": "ST",
    "SAUDI ARABIA": "SA",
    "SENEGAL": "SN",
    "SERBIA": "RS",
    "SEYCHELLES": "SC",
    "SIERRA LEONE": "SL",
    "SINGAPORE": "SG",
    "SINT MAARTEN (DUTCH PART)": "SX",
    'SISTEMA UNITARIO DE COMPENSACION REGIONAL DE PAGOS "SUCRE"': None,
    "SLOVAKIA": "SK",
    "SLOVENIA": "SI",
    "SOLOMON ISLANDS": "SB",
    "SOMALIA": "SO",
    "SOUTH AFRICA": "ZA",
    "SOUTH GEORGIA AND THE SOUTH SANDWICH ISLANDS": "GS",
    "SOUTH SUDAN": "SS",
    "SPAIN": "ES",
    "SRI LANKA": "LK",
    "SUDAN (THE)": "SD",
    "SURINAME": "SR",
    "SVALBARD AND JAN MAYEN": "SJ",
    "SWEDEN": "SE",
    "SWITZERLAND": "CH",
    "SYRIAN ARAB REPUBLIC": "SY",
    "TAIWAN (PROVINCE OF CHINA)": "TW",
    "TAJIKISTAN": "TJ",
    "TANZANIA, UNITED REPUBLIC OF": "TZ",
    "THAILAND": "TH",
    "TIMOR-LESTE": "TL",
    "TOGO": "TG",
    "TOKELAU": "TK",
    "TONGA": "TO",
    "TRINIDAD AND TOBAGO": "TT",
    "TUNISIA": "TN",
    "TÜRKİYE": "TR",
    "TURKMENISTAN": "TM",
    "TURKS AND CAICOS ISLANDS (THE)": "TC",
    "TUVALU": "TV",
    "UGANDA": "UG",
    "UKRAINE": "UA",
    "UNITED ARAB EMIRATES (THE)": "AE",
    "UNITED KINGDOM OF GREAT BRITAIN AND NORTHERN IRELAND (THE)": "GB",
    "UNITED STATES MINOR OUTLYING ISLANDS (THE)": "UM",
    "UNITED STATES OF AMERICA (THE)": "US",
    "URUGUAY": "UY",
    "UZBEKISTAN": "UZ",
    "VANUATU": "VU",
    "VENEZUELA (BOLIVARIAN REPUBLIC OF)": "VE",
    "VIET NAM": "VN",
    "VIRGIN ISLANDS (BRITISH)": "VG",
    "VIRGIN ISLANDS (U.S.)": "VI",
    "WALLIS AND FUTUNA": "WF",
    "WESTERN SAHARA": "EH",
    "YEMEN": "YE",
    "ZAMBIA": "ZM",
    "ZIMBABWE": "ZW",
    # The agency's own rows: units, metals and the codes for testing and for
    # no currency, under `ZZ` names that are no country.
    "ZZ01_Bond Markets Unit European_EURCO": None,
    "ZZ02_Bond Markets Unit European_EMU-6": None,
    "ZZ03_Bond Markets Unit European_EUA-9": None,
    "ZZ04_Bond Markets Unit European_EUA-17": None,
    "ZZ06_Testing_Code": None,
    "ZZ07_No_Currency": None,
    "ZZ08_Gold": None,
    "ZZ09_Palladium": None,
    "ZZ10_Platinum": None,
    "ZZ11_Silver": None,
}


def fetch(source: str) -> bytes:
    """Return the bytes of list one: a URL fetched, else a saved file read."""
    try:
        if source.startswith(("https://", "http://")):
            request = urllib.request.Request(source, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=120) as response:
                return response.read()
        return pathlib.Path(source).read_bytes()
    except OSError as error:
        raise SystemExit(f"cannot read list one from {source}: {error}") from error


def parse(payload: bytes) -> tuple[str, list[tuple[str, str | None, bool]]]:
    """Return the publication date and every entry as ``(name, code, fund)``."""
    root = ElementTree.fromstring(payload)
    published = root.get("Pblshd", "")
    if root.tag != "ISO_4217" or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", published):
        raise SystemExit(f"not ISO 4217 list one: root <{root.tag} Pblshd={published!r}>")
    entries = []
    for entry in root.iter("CcyNtry"):
        name = unicodedata.normalize("NFC", (entry.findtext("CtryNm") or "").strip())
        if not name:
            raise SystemExit("list one holds an entry with no CtryNm")
        code = (entry.findtext("Ccy") or "").strip() or None
        if code is not None and not re.fullmatch(r"[A-Z]{3}", code):
            raise SystemExit(f"{name}: {code!r} is not three upper-case letters")
        currency_name = entry.find("CcyNm")
        fund = currency_name is not None and currency_name.get("IsFund", "").lower() == "true"
        entries.append((name, code, fund))
    if not entries:
        raise SystemExit("list one holds no CcyNtry")
    return published, entries


def tenders(entries: list[tuple[str, str | None, bool]]) -> dict[str, list[str]]:
    """Return the tenders list one gives each country, in list order."""
    unmatched = sorted({name for name, _, _ in entries if name not in NAMES})
    if unmatched:
        raise SystemExit(
            "NAMES holds no alpha-2 code for: "
            + ", ".join(repr(name) for name in unmatched)
            + " - add each spelling to NAMES in scripts/generate_country_currency.py"
        )
    by_country: dict[str, list[str]] = {}
    for name, code, fund in entries:
        country = NAMES[name]
        if country is None:
            continue
        held = by_country.setdefault(country, [])
        if code is not None and not fund and code not in held:
            held.append(code)
    return by_country


def settle(by_country: dict[str, list[str]]) -> dict[str, str]:
    """Return one currency per country, every choice the overrides make checked."""
    absent = sorted(set(OVERRIDES) - set(by_country))
    if absent:
        raise SystemExit(f"OVERRIDES names countries list one does not: {', '.join(absent)}")
    settled: dict[str, str] = {}
    for country, codes in sorted(by_country.items()):
        if country in OVERRIDES:
            chosen = OVERRIDES[country]
            if chosen is FIRST:
                if not codes:
                    raise SystemExit(f"OVERRIDES takes the first tender of {country}, list one gives none")
                chosen = codes[0]
            elif chosen not in codes:
                raise SystemExit(
                    f"OVERRIDES gives {country} {chosen}, list one gives it {', '.join(codes) or 'none'}"
                )
        elif len(codes) > 1:
            raise SystemExit(
                f"list one gives {country} {len(codes)} tenders ({', '.join(codes)});"
                " name the one it takes in OVERRIDES"
            )
        elif codes:
            chosen = codes[0]
        else:
            continue
        settled[country] = chosen
    return settled


def rows(items: list[str], per_row: int, indent: str) -> str:
    lines = []
    for start in range(0, len(items), per_row):
        lines.append(indent + " ".join(items[start : start + per_row]))
    return "\n".join(lines)


def render(published: str, settled: dict[str, str]) -> str:
    items = [f'("{country}", "{currency}"),' for country, currency in sorted(settled.items())]
    return f"""//! The legal tender of every country, generated from ISO 4217 list one.
//!
//! Regenerate with `python scripts/generate_country_currency.py`; the same
//! script run with `--check` fetches the list again and fails when this file
//! is stale. Nothing here is edited by hand: list one names a country by its
//! English short name, and the script reads every name through its own pinned
//! table of ISO 3166-1 alpha-2 codes, failing on a name it does not hold.
//!
//! Source: <{SOURCE}>,
//! published {published}.
//!
//! One currency per country. A fund code is left out, a country list one
//! gives two tenders takes the one the script's override table names, and a
//! country list one gives no currency - Antarctica, Palestine - has no row,
//! nor has an entry that names no country.

/// `(alpha-2, currency)`, sorted by the alpha-2 code: what
/// [`Country::currency`](crate::Country::currency) binary-searches.
#[rustfmt::skip]
pub(crate) static COUNTRY_CURRENCY: [(&str, &str); {len(items)}] = [
{rows(items, 8, "    ")}
];
"""


def build(source: str) -> tuple[str, int]:
    published, entries = parse(fetch(source))
    settled = settle(tenders(entries))
    return render(published, settled), len(settled)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail when the committed table file is stale instead of rewriting it",
    )
    parser.add_argument(
        "--source",
        default=SOURCE,
        help="list one to read, a URL or a saved file (default: the official list)",
    )
    arguments = parser.parse_args()

    generated, count = build(arguments.source)
    if arguments.check:
        current = TARGET.read_text(encoding="utf-8") if TARGET.exists() else ""
        if current != generated:
            print(f"{TARGET} is stale; run python scripts/generate_country_currency.py")
            return 1
        print(f"{TARGET} is current ({count} countries)")
        return 0

    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(generated, encoding="utf-8", newline="\n")
    print(f"wrote {TARGET} ({count} countries)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
