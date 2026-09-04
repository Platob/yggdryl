"""The registered code vocabularies, declared as enums over their datatypes.

Each class is the Python spelling of one registered code in the datatype
grammar: `country` is ISO 3166-1 alpha-2 in two bytes, `currency` is ISO 4217
in three, `mic` is ISO 10383 in four, and `cfi` is ISO 10962 in six. A member
*is* the integer its code packs into, so the same code is the same integer in
every process and in every column that stores it.

The declared members are the codes a caller names in source. The standards
behind them are registries, not fixed sets - ISO 4217 retires codes, ISO 10383
adds venues monthly - so every vocabulary here is open exactly as any other
declared one is: a valid code that is not declared reads back as a member under
its own packed code, registered once and announced once on the
`yggdryl.enums.ascii` logger. Nothing needs a release to read a new venue.
"""

from __future__ import annotations

from .ascii import CfiCode, CountryCode, CurrencyCode, MicCode


class Country(CountryCode):
    """ISO 3166-1 alpha-2, the two-letter country code."""

    AE = "AE"
    AT = "AT"
    AU = "AU"
    BE = "BE"
    BR = "BR"
    CA = "CA"
    CH = "CH"
    CN = "CN"
    DE = "DE"
    DK = "DK"
    ES = "ES"
    FI = "FI"
    FR = "FR"
    GB = "GB"
    HK = "HK"
    IE = "IE"
    IL = "IL"
    IN = "IN"
    IT = "IT"
    JP = "JP"
    KR = "KR"
    LU = "LU"
    MX = "MX"
    NL = "NL"
    NO = "NO"
    NZ = "NZ"
    PL = "PL"
    PT = "PT"
    SA = "SA"
    SE = "SE"
    SG = "SG"
    TR = "TR"
    TW = "TW"
    US = "US"
    ZA = "ZA"


class Currency(CurrencyCode):
    """ISO 4217, the three-letter currency code.

    The `currency` datatype is exactly three bytes, so a currency stores with
    no padding at all: `USD` is the three characters and nothing else.
    """

    AED = "AED"
    AUD = "AUD"
    BRL = "BRL"
    CAD = "CAD"
    CHF = "CHF"
    CNH = "CNH"
    CNY = "CNY"
    CZK = "CZK"
    DKK = "DKK"
    EUR = "EUR"
    GBP = "GBP"
    HKD = "HKD"
    HUF = "HUF"
    IDR = "IDR"
    ILS = "ILS"
    INR = "INR"
    JPY = "JPY"
    KRW = "KRW"
    MXN = "MXN"
    MYR = "MYR"
    NOK = "NOK"
    NZD = "NZD"
    PHP = "PHP"
    PLN = "PLN"
    RON = "RON"
    SAR = "SAR"
    SEK = "SEK"
    SGD = "SGD"
    THB = "THB"
    TRY = "TRY"
    TWD = "TWD"
    USD = "USD"
    ZAR = "ZAR"

    #: The troy-ounce metal codes ISO 4217 spells in the same three letters.
    XAG = "XAG"
    XAU = "XAU"
    XPD = "XPD"
    XPT = "XPT"


class MIC(MicCode):
    """ISO 10383, the four-character market identifier code.

    The registry lists thousands of venues and adds more every month, so the
    declared members are the operating MICs a caller names; every other one
    reads back as a member of its own.
    """

    ARCX = "ARCX"
    BATS = "BATS"
    BVMF = "BVMF"
    DSMD = "DSMD"
    EUCC = "EUCC"
    IEXG = "IEXG"
    MISX = "MISX"
    NEOE = "NEOE"
    OTCM = "OTCM"
    ROCO = "ROCO"
    TASE = "TASE"
    XADS = "XADS"
    XAMS = "XAMS"
    XASE = "XASE"
    XASX = "XASX"
    XBOM = "XBOM"
    XBRU = "XBRU"
    XBUD = "XBUD"
    XCBO = "XCBO"
    XCME = "XCME"
    XCSE = "XCSE"
    XDUB = "XDUB"
    XEEE = "XEEE"
    XETR = "XETR"
    XEUR = "XEUR"
    XHEL = "XHEL"
    XHKG = "XHKG"
    XICE = "XICE"
    XIST = "XIST"
    XJSE = "XJSE"
    XKLS = "XKLS"
    XKRX = "XKRX"
    XLIS = "XLIS"
    XLON = "XLON"
    XMAD = "XMAD"
    XMEX = "XMEX"
    XMIL = "XMIL"
    XNAS = "XNAS"
    XNSE = "XNSE"
    XNYS = "XNYS"
    XNZE = "XNZE"
    XOSL = "XOSL"
    XPAR = "XPAR"
    XPHS = "XPHS"
    XSAU = "XSAU"
    XSES = "XSES"
    XSHE = "XSHE"
    XSHG = "XSHG"
    XSTO = "XSTO"
    XSWX = "XSWX"
    XTAI = "XTAI"
    XTKS = "XTKS"
    XTSE = "XTSE"
    XWAR = "XWAR"


class CFI(CfiCode):
    """ISO 10962, the six-character classification of financial instruments.

    The first character is the category and the second the group; the last four
    narrow the instrument, and `X` is the standard's own "not applicable". The
    declared members are the whole codes a caller names; every other valid one
    reads back as a member of its own.
    """

    #: Equities: common, preferred, and depositary receipts.
    ESVUFR = "ESVUFR"
    EPVUFR = "EPVUFR"
    EDSXFR = "EDSXFR"
    #: Collective investment vehicles, including exchange-traded funds.
    CEOJEU = "CEOJEU"
    #: Debt: plain bonds and money-market instruments.
    DBFTFR = "DBFTFR"
    DYFXXR = "DYFXXR"
    #: Listed derivatives: futures and options on equities and rates.
    FFICSX = "FFICSX"
    FCEPSX = "FCEPSX"
    OCASPS = "OCASPS"
    OPASPS = "OPASPS"
    #: Rights, warrants, and spot foreign exchange.
    RWSCFR = "RWSCFR"
    MRIXXX = "MRIXXX"


__all__ = [
    "CFI",
    "Country",
    "Currency",
    "MIC",
]
